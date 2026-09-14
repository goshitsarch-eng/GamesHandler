//! The download half of the easy installer (`installers.py:598-646`).
//!
//! Origin and payload validation and the download itself. The Authenticode
//! verification the pipe calls mid-stream is [`signature`](crate::installers::signature)
//! — split under `ARCH-11` because the two concerns share only
//! [`run_capturing`](crate::installers::process::run_capturing). See
//! [`crate::installers`] for the catalog, the error type and the seams.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::installers::command::{MAX_INSTALLER_BYTES, safe_download_name};
use crate::installers::signature::verify_installer_authenticity;
use crate::installers::{Installer, InstallerError, Kind};

use crate::runners::proton::{HttpClient, ResponseHead};
use crate::runners::{LaunchEnv, RunnerError, USER_AGENT};

/// `urlparse(url).scheme.lower()` and `urlparse(url).hostname`
/// (`installers.py:552-554`).
///
/// # Why this is a port and not `split("://")`
///
/// The one caller is an allowlist check whose whole value is that it cannot be
/// talked out of: `_validate_download_origin` compares the *final* host against
/// `installer.allowed_hosts`, so a host this function reports as an allowed one
/// is a host the download is then accepted from. A naive parse that reported a
/// host the reference would not is an allowlist bypass, not a cosmetic
/// difference — `https://allowed.example@evil.example/x` is exactly that case,
/// and it lands the other way depending on whether userinfo is split at the
/// first `@` or the last.
///
/// So the rules below are CPython's `urlsplit`/`_hostinfo`, taken from the
/// implementation and then checked against it: the vector battery in the tests
/// was produced by running `urllib.parse` on this machine, and every line of it
/// has to agree. The rules that matter:
///
/// * Leading C0 controls and spaces are stripped, and `\t`, `\r` and `\n` are
///   removed **everywhere** — CPython's `_UNSAFE_URL_BYTES_TO_REMOVE`. Google's
///   URL spec strips both ends; CPython deliberately does not strip the right
///   end, and neither does this.
/// * The scheme is only recognised when the text before the first `:` is a
///   valid scheme, so `cdn.example/x` has no scheme and no host. A URL with no
///   scheme can never pass the `https` test, which is the fail-closed direction.
/// * The netloc runs from `//` to the first `/`, `?` or `#`.
/// * Userinfo is split at the **last** `@` (`netloc.rpartition('@')`).
/// * A bracketed host is everything up to the closing `]`; otherwise the host
///   is everything before the first `:`. The port is neither validated nor
///   reported, which is why `https://allowed.example:notaport/x` resolves to
///   `allowed.example`.
/// * The host is lowercased but **not** IDNA-encoded and **not** stripped of a
///   trailing dot, both of which are what the measured CPython does — and both
///   of which mean `allowed.example.` is a *different* host from
///   `allowed.example`, so a host that merely looks like an allowed one fails
///   closed.
pub(crate) struct UrlParts {
    /// `urlsplit(url).scheme`, lowercased.
    pub scheme: String,
    /// The raw netloc, exactly as CPython keeps it — case preserved, and `""`
    /// when there is no `//`. Kept because `bool(parsed.netloc)` is what
    /// `netpaths.is_remote_url` tests, and that is **not** the same question as
    /// "is there a hostname": `smb:///x` has a scheme and no netloc, and
    /// `//host/x` has a netloc and no scheme.
    pub netloc: String,
    /// `urlsplit(url).hostname`, lowercased, or `None` when empty.
    pub host: Option<String>,
    /// `urlsplit(url).username` — **not** percent-decoded, because CPython's is
    /// not; `netpaths` decodes it itself with `unquote`.
    pub user: Option<String>,
    /// `urlsplit(url).port`, or `None`. See this function's note on the one
    /// case where CPython raises instead.
    pub port: Option<u32>,
    /// `urlsplit(url).path` — the path component only, with any `?query` and
    /// `#fragment` already removed.
    pub path: String,
}

/// `urlsplit()`'s five components (`urlparse` in `installers.py:552`,
/// `urlsplit` in `netpaths.py`), in one place because there is only one
/// CPython URL parser worth having a fidelity opinion about.
///
/// # Why this is one function and not two
///
/// It used to return `(scheme, Option<host>)`, which is all the download
/// origin allowlist needs. `netpaths` needs the username, port and path as
/// well, and the tempting move is to parse again inside `netpaths`. That would
/// be two independent ports of `urlsplit` for one behaviour: two fidelities,
/// only one of which the vector battery below exercises, and the unexercised
/// one is the copy that rots. So the components were added here instead, and
/// the battery was extended over all five rather than a second one started.
///
/// **The widening is return-type-only.** Every caller that read
/// `(scheme, host)` before still reads exactly that and nothing else —
/// [`validate_download_origin`] is the one that matters, and the security
/// property is untouched by construction because widening what a parser
/// *returns* cannot change what a caller *reads*. The 27 allowlist vectors are
/// the evidence that it did not.
///
/// # The rules, all of them measured against CPython rather than recalled
///
/// See the notes on the scheme, netloc, userinfo and host rules that were
/// already here; the additions are that the username is the text before the
/// **first** `:` of the userinfo (which was itself split at the **last** `@`,
/// so `a@b@host` has the username `a@b`), and that `port` is `None` both when
/// the port is absent or empty and when it is not a number.
///
/// That last one is a **divergence**, and it is exact rather than a rounding:
/// CPython's `.port` property raises `ValueError` on a port that is not ASCII
/// digits within `0..=65535`, so `netpaths.as_local_path("sftp://host:abc/x")`
/// raises out of the reference and takes the caller with it. Returning `None`
/// here cannot crash, and the difference is observable only on a URL the
/// reference cannot handle at all. The accepted *text*, though, is reproduced
/// exactly — see the ASCII-digit guard in the body, which is why `+8` and
/// `8_0` are `None` here as well as a raise there, and not `8`.
pub(crate) fn url_parts(url: &str) -> UrlParts {
    /// CPython's `_WHATWG_C0_CONTROL_OR_SPACE`.
    fn is_c0_control_or_space(character: char) -> bool {
        character == ' ' || (character as u32) <= 0x1f
    }

    let url = url.trim_start_matches(is_c0_control_or_space);
    let url: String = url
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();

    // The scheme is recognised only when everything before the first `:` is a
    // scheme character *and* the first character is an ASCII letter — the
    // `url[0].isascii() and url[0].isalpha()` guard in CPython, which is why
    // `1http://x` has no scheme.
    let mut scheme = String::new();
    let mut rest = url.as_str();
    if let Some(colon) = url.find(':') {
        let candidate = &url[..colon];
        let first_is_letter = candidate
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic());
        let all_scheme_chars = candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
        if colon > 0 && first_is_letter && all_scheme_chars {
            scheme = candidate.to_lowercase();
            rest = &url[colon + 1..];
        }
    }

    // `_splitnetloc`: the netloc runs from just after the `//` to the first
    // `/`, `?` or `#`. When there is no `//` there is no netloc but there is
    // still a path — `https:/host/x` keeps `/host/x` — so this cannot return
    // early the way the two-component version did.
    let (netloc, after_netloc) = match rest.strip_prefix("//") {
        Some(remainder) => {
            let end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
            (&remainder[..end], &remainder[end..])
        }
        None => ("", rest),
    };
    // Fragment first, then query — CPython's order, and the two are equivalent
    // for the path only because neither can contain the other's delimiter.
    let path = after_netloc.split('#').next().unwrap_or("");
    let path = path.split('?').next().unwrap_or("");

    // `_userinfo`, then `_hostinfo`: both split at the **last** `@`.
    let at = netloc.rfind('@');
    let user = at.map(|at| {
        let userinfo = &netloc[..at];
        // `userinfo.partition(':')` — the **first** colon, and a username with
        // no colon at all is the whole thing.
        let end = userinfo.find(':').unwrap_or(userinfo.len());
        userinfo[..end].to_string()
    });
    let hostinfo = match at {
        Some(at) => &netloc[at + 1..],
        None => netloc,
    };

    let (hostname, port_text) = match hostinfo.find('[') {
        Some(open) => {
            let bracketed = &hostinfo[open + 1..];
            match bracketed.find(']') {
                // The port, when there is one, is after the `]` — so
                // `[::1]:8080` reports `::1` and `8080`, and `[::1]` reports
                // no port rather than a port of `:1`.
                Some(close) => {
                    let tail = &bracketed[close + 1..];
                    (&bracketed[..close], tail.strip_prefix(':').unwrap_or(""))
                }
                None => (bracketed, ""),
            }
        }
        None => match hostinfo.find(':') {
            Some(colon) => (&hostinfo[..colon], &hostinfo[colon + 1..]),
            None => (hostinfo, ""),
        },
    };

    // `SplitResult.hostname` lowercases **only up to the first `%`**, because a
    // scoped IPv6 literal carries its zone there and the zone is
    // case-significant: `smb://[fe80::1%tESt]/x` has the host
    // `fe80::1%tESt`, not `fe80::1%test`. Lowercasing the whole thing would
    // quietly rewrite the zone and pick a different interface.
    let host = if hostname.is_empty() {
        None
    } else {
        match hostname.split_once('%') {
            Some((before, zone)) => Some(format!("{}%{zone}", before.to_lowercase())),
            None => Some(hostname.to_lowercase()),
        }
    };
    // CPython guards with `port.isdigit() and port.isascii()` **before**
    // converting, so the accepted text is ASCII digits and nothing else — no
    // sign, no underscores, no whitespace, no Unicode digits. `+8` and `8_0`
    // both raise there. The `is_ascii_digit` check below reproduces that
    // guard exactly, which matters because `"+8".parse::<u32>()` is `Ok(8)`
    // in Rust and would otherwise turn a URL the reference rejects into a
    // port of 8 here.
    let digits_only = !port_text.is_empty() && port_text.chars().all(|c| c.is_ascii_digit());
    // `0..=65535` is CPython's own range check; anything else it would raise
    // on, and this returns `None` instead. See this function's note.
    let port = port_text
        .parse::<u32>()
        .ok()
        .filter(|port| digits_only && *port <= 65_535);

    UrlParts {
        scheme,
        netloc: netloc.to_string(),
        host,
        user,
        port,
        path: path.to_string(),
    }
}

/// Refuse a response that did not come from where the recipe says it may come
/// from (`installers.py:551-558`).
///
/// Two conditions, both required: the scheme is `https` and the host is one of
/// `installer.allowed_hosts`. The reference raises a `RuntimeError` whose
/// message is [`InstallerError::UntrustedOrigin`]'s Display.
///
/// `final_url` is the URL after redirects, which is the whole point — see
/// [`ResponseHead::final_url`]. An empty string (a client that cannot report
/// one) has no host and is rejected.
pub fn validate_download_origin(
    installer: &Installer,
    final_url: &str,
) -> Result<(), InstallerError> {
    // Reads the scheme and the host and **nothing else**. The parser was
    // widened to five components for `netpaths`; this caller's inputs are
    // unchanged, which is what makes the widening unable to affect it.
    let parts = url_parts(final_url);
    let (scheme, host) = (parts.scheme, parts.host);
    let allowed = installer
        .allowed_hosts
        .iter()
        .map(|item| item.to_lowercase())
        .collect::<Vec<_>>();
    let host_is_allowed = host.as_ref().is_some_and(|host| allowed.contains(host));
    if scheme != "https" || !host_is_allowed {
        return Err(InstallerError::UntrustedOrigin {
            name: installer.name.to_string(),
            url: final_url.to_string(),
        });
    }
    Ok(())
}

/// Refuse a payload whose first bytes are not the container the recipe declared
/// (`installers.py:561-566`).
///
/// `MZ` for an exe, the OLE compound-file header for an MSI. This is a cheap
/// shape check, not a security control — a real attacker's payload passes it —
/// and it is here to catch a *wrong download* (an HTML error page, a captive
/// portal's redirect body) with a message that says what happened, before
/// spending ninety seconds on the signature verifier. The tests assert that
/// ordering: a non-PE payload must fail here and the verifier must not be
/// called.
pub fn validate_installer_magic(installer: &Installer, path: &Path) -> Result<(), InstallerError> {
    use std::io::Read;
    const OLE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    let mut magic = [0u8; 8];
    let mut file = std::fs::File::open(path)?;
    let read = file.read(&mut magic)?;
    let magic = &magic[..read];
    let expected: &[u8] = match installer.kind {
        Kind::Exe => b"MZ",
        Kind::Msi => &OLE,
    };
    // `magic.startswith(expected)`, including for a file shorter than the
    // header: a one-byte `M` does not start with `MZ`.
    if magic.len() < expected.len() || &magic[..expected.len()] != expected {
        return Err(InstallerError::NotValidPayload {
            name: installer.name.to_string(),
            kind: installer.kind,
        });
    }
    Ok(())
}

/// Create the `.part` file beside the target
/// (`tempfile.mkstemp(dir=dest, prefix=f".{filename}.", suffix=".part")`).
///
/// `mkstemp` guarantees two things that matter and one that does not. The
/// exclusive create is the load-bearing one: `create_new` is `O_CREAT|O_EXCL`,
/// which fails on an existing path *including* a symlink, so a `.part` name an
/// attacker guessed cannot be used to redirect the write. The other is 0600
/// permissions, which `mkstemp` gives and `create_new` does not: `create_new`
/// is bounded by the process umask, so under the usual 022 the download sits
/// at 0644 for the length of the transfer, readable by every local user. That
/// is the file the mode is set on below — an explicit `0o600` on the handle
/// `create_new` just returned, which is the mode the file has from the instant
/// it exists rather than from the instant the download finishes, and which no
/// umask can widen. The part that does not matter is the *unpredictability* of
/// the name, which `mkstemp` gets from random bytes; exclusivity is what makes
/// the name safe, so this counts up from the process id instead, which is also
/// what makes a leftover file nameable in a bug report.
pub(crate) fn create_partial(
    directory: &Path,
    filename: &str,
) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    /// `TMP_MAX`-ish: `mkstemp` gives up after a bounded number of attempts.
    const ATTEMPTS: u32 = 1_000;
    let process = std::process::id();
    for attempt in 0..ATTEMPTS {
        let path = directory.join(format!(".{filename}.{process}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            // `mkstemp`'s own mode argument: the file is never wider than
            // 0600 even for the instant between the create and the chmod below.
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => {
                // A umask with owner bits in it would narrow the create's mode,
                // so the 0600 is then made exact. A failure here removes the
                // file, so a caller that got an `Err` has nothing to reason
                // about but the error.
                if let Err(error) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                return Ok((file, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("could not create a temporary file for {filename}"),
    ))
}

/// Atomically download and authenticate a vendor installer
/// (`installers.py:598-646`).
///
/// # The order, which is the whole function
///
/// The bytes are written to a `.part` file beside the target and the target is
/// only ever created by a rename, so an interrupted or refused download cannot
/// leave something at the target path that the caller would then treat as a
/// verified installer. Every failure — an untrusted origin, a payload that is
/// not the declared container, a signature that does not verify, a missing
/// verifier — removes the `.part` file and returns the target to whatever it
/// was before, which for a first install is nothing at all.
///
/// The checks happen in a deliberate order, and the reference's tests pin all
/// three steps of it:
///
/// 1. **Origin, in the head callback, before a byte is kept.**
///    [`validate_download_origin`] runs on the final URL the client reports.
///    Returning `Err` there abandons the transfer, so a redirect to an
///    unapproved host costs no bytes at all rather than up to
///    [`MAX_INSTALLER_BYTES`] of them.
/// 2. **Size, twice.** The declared `Content-Length` first — the cheap refusal
///    — and then the running total, because a server that understates its
///    length still cannot fill the disk.
/// 3. **Shape then signature**, both against the `.part` file. Shape first so a
///    captive portal's HTML costs no verifier run, and the verifier only after
///    the file is complete.
///
/// `progress` is called with `downloaded / total` while the body arrives, only
/// when a length was declared, and exactly once with `1.0` at the end — which
/// is what the reference does, and is why the final call is outside the
/// `if declared` guard.
pub fn download_installer(
    installer: &Installer,
    dest_dir: &Path,
    progress: Option<&dyn Fn(f64)>,
    timeout: Duration,
    client: &dyn HttpClient,
    launch_env: &dyn LaunchEnv,
) -> Result<PathBuf, InstallerError> {
    download_installer_with(
        installer,
        dest_dir,
        progress,
        timeout,
        client,
        launch_env,
        &|| false,
    )
}

/// [`download_installer`] with a cancellation predicate.
///
/// `cancelled` is polled inside both transfer callbacks and again after the
/// body lands but before shape or signature checks run, so an abort stops the
/// transfer mid-stream and never pays a verifier run for a download the user
/// already walked away from. A `true` answer returns
/// [`InstallerError::Cancelled`] through the same refusal cell the origin and
/// size checks use, and the `.part` file is removed by
/// [`download_installer`]'s existing error path — the half-written download is
/// never renamed over the target.
///
/// The `_with` split is this codebase's own shape (`install`/`install_with`,
/// `extract_archive`/`extract_archive_with`): the reference has no cancel, so
/// the parameter-free spelling stays the port-shaped default and only callers
/// with a button to answer carry the predicate.
#[allow(clippy::too_many_arguments)]
pub fn download_installer_with(
    installer: &Installer,
    dest_dir: &Path,
    progress: Option<&dyn Fn(f64)>,
    timeout: Duration,
    client: &dyn HttpClient,
    launch_env: &dyn LaunchEnv,
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, InstallerError> {
    std::fs::create_dir_all(dest_dir)?;
    let filename = safe_download_name(installer.filename, installer.id);
    let target = dest_dir.join(&filename);
    let (file, temporary) = create_partial(dest_dir, &filename)?;

    let result = download_into(
        installer, &temporary, file, progress, timeout, client, launch_env, cancelled,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, &target) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Some(progress) = progress {
        progress(1.0);
    }
    Ok(target)
}

/// The `Err` a transfer callback returns to abandon a download it has already
/// refused.
///
/// The trait's callbacks speak [`RunnerError`] while this module speaks
/// [`InstallerError`], so a refusal recorded in the callback and returned as
/// `RunnerError` would lose the variant that says *which* refusal it was —
/// `UntrustedOrigin` and `TooLarge` are different things to report, and the
/// reference's tests distinguish them by message. So the callback records the
/// real error in [`download_into`]'s `refusal` cell and returns this to stop the
/// transfer; the cell is what gets returned to the caller.
///
/// **The message is never rendered** — the cell always wins — but it carries
/// the refusal's own text anyway, so that a future caller which propagated this
/// instead would still print the right sentence rather than a placeholder.
fn abandon(reason: &InstallerError) -> RunnerError {
    RunnerError::Http {
        message: reason.to_string(),
    }
}

/// The body of [`download_installer`], split out so every exit from it — errors
/// included — is a single `Result` the caller can clean up after.
///
/// # Why the shared state is in cells rather than `mut` locals
///
/// `on_head` and `sink` are two simultaneous `&mut dyn FnMut`, so anything both
/// of them touch cannot be a plain `&mut` local — the second closure's borrow
/// would overlap the first's. A `Cell`/`RefCell` behind a shared reference is
/// borrowed by each closure rather than moved into one, which is what lets the
/// head record `total` and the sink read it, and lets either of them record a
/// refusal. This is the same problem the trait's two callbacks create on the
/// client side.
#[allow(clippy::too_many_arguments)]
fn download_into(
    installer: &Installer,
    temporary: &Path,
    mut file: std::fs::File,
    progress: Option<&dyn Fn(f64)>,
    timeout: Duration,
    client: &dyn HttpClient,
    launch_env: &dyn LaunchEnv,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), InstallerError> {
    use std::cell::{Cell, RefCell};
    use std::io::Write;

    let total = Cell::new(0u64);
    let downloaded = Cell::new(0u64);
    let refusal: RefCell<Option<InstallerError>> = RefCell::new(None);

    let headers = [("User-Agent", USER_AGENT)];
    let outcome = client.get(
        installer.download_url,
        &headers,
        timeout,
        &mut |head: &ResponseHead| {
            // Checked first: a cancel that lands before the first byte is the
            // cheapest refusal there is, ahead of even the origin check.
            if cancelled() {
                let reason = InstallerError::Cancelled;
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            // Before a byte is kept: a redirect to a host the recipe never
            // approved costs nothing at all rather than the whole body.
            if let Err(reason) = validate_download_origin(installer, &head.final_url) {
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            total.set(parse_content_length(head.content_length.as_deref()));
            if total.get() > MAX_INSTALLER_BYTES {
                let reason = InstallerError::TooLarge {
                    name: installer.name.to_string(),
                };
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            Ok(())
        },
        &mut |chunk: &[u8]| {
            // Per chunk rather than at the end: a cancel should stop the
            // transfer, not acknowledge it once the body has all landed.
            if cancelled() {
                let reason = InstallerError::Cancelled;
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            downloaded.set(downloaded.get() + chunk.len() as u64);
            if downloaded.get() > MAX_INSTALLER_BYTES {
                let reason = InstallerError::TooLarge {
                    name: installer.name.to_string(),
                };
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            if let Err(error) = file.write_all(chunk) {
                let reason = InstallerError::Io(error);
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            if let Some(progress) = progress.filter(|_| total.get() > 0) {
                progress((downloaded.get() as f64 / total.get() as f64).min(1.0));
            }
            Ok(())
        },
    );

    // The recorded refusal is the specific error; `outcome`'s is the generic
    // "abandoned" that stopped the transfer.
    if let Some(reason) = refusal.into_inner() {
        return Err(reason);
    }
    outcome?;

    // The body is all on disk; a cancel that landed with the last chunk still
    // skips the signature run, which is the expensive part left.
    if cancelled() {
        return Err(InstallerError::Cancelled);
    }
    validate_installer_magic(installer, temporary)?;
    verify_installer_authenticity(installer, temporary, launch_env)?;
    Ok(())
}

/// `Content-Length`, parsed the way the reference parses it
/// (`int(resp.headers.get("Content-Length", 0) or 0)`).
///
/// A header that is absent, empty, or not a number all give `0`, which the
/// progress callback reads as "no denominator, report nothing" rather than as
/// an error. It is deliberately **not** [`crate::runners::proton::parse_content_length`]:
/// that one raises `"Runner download has an invalid Content-Length"` because
/// the runner download's size drives a staging decision, while this one only
/// ever feeds a progress bar — and the reference is equally relaxed here.
fn parse_content_length(declared: Option<&str>) -> u64 {
    declared
        .map(|value| value.trim())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installers::installer_by_id;
    use crate::installers::tests_support::*;

    // -----------------------------------------------------------------------
    // `urllib.parse` — the allowlist's host parser
    // -----------------------------------------------------------------------

    /// One row of the battery: `(url, scheme, host, user, port, path, netloc)`.
    ///
    /// A named alias rather than the tuple spelled out at the array, because
    /// clippy's `type_complexity` fires on the literal form and the gate is
    /// `-D warnings` — and because the row is easier to read named.
    type UrlVector<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Option<u32>,
        &'a str,
        &'a str,
    );

    /// The vector battery was produced by running CPython's `urllib.parse` on
    /// this machine and copying its output, then checked line by line.
    ///
    /// # The four lines that are the reason this is a table rather than an
    /// assertion about `split("://")`
    ///
    /// * `https://cdn.akamai.steamstatic.com@evil.example/x` → `evil.example`.
    ///   Userinfo is split at the last `@`, so the host is what comes *after*
    ///   it. A parser that split at the first `@` would report
    ///   `evil.example@...` — harmless here because it is not in an allowlist
    ///   either — but a parser that *ignored* userinfo would report the allowed
    ///   host and accept the download. That is the bypass this test exists for.
    /// * `https://evil.example@cdn.akamai.steamstatic.com/x` → the allowed
    ///   host, correctly: that URL really is served by the allowed host.
    ///   The pair is the control arm — one line alone cannot distinguish "splits
    ///   at the last `@`" from "splits at the first" or "ignores `@`".
    /// * `https://cdn.akamai.steamstatic.com./x` → a **different** host
    ///   (trailing dot), which is not in the allowlist and so fails closed.
    /// * `https://cdn.akamai.steamstatic.com:notaport/x` → the allowed host:
    ///   `.hostname` does not validate the port. Pinned so a later change that
    ///   "tidied" the parse by rejecting a bad port cannot silently make this
    ///   stricter than the reference.
    #[test]
    fn the_url_parser_matches_cpython() {
        // **58 vectors, and the count went up twice for a reason worth
        // naming.** The first 27 are the allowlist's own and every one of them
        // was generated by running CPython's `urlsplit` on this machine, not
        // written from memory. `netpaths` then needed the username, port and
        // path as well, so the *same* battery was extended over all five
        // components rather than a second one started — one parser, one
        // fidelity, one table. It went 27 → 47 → 58 for exactly that reason,
        // and the last eleven are the IPv6-zone and port-zero rows that caught
        // two real defects rather than illustrating a rule already known.
        //
        // Each tuple is `(url, scheme, host, user, port, path, netloc)` and
        // every value in it came out of `urlsplit(...)`, so a disagreement here
        // is a disagreement with CPython and not with a prior reading of it.
        let vectors: [UrlVector; 58] = [
            (
                "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/client/installer/SteamSetup.exe",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "HTTPS://CDN.AKAMAI.STEAMSTATIC.COM/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "CDN.AKAMAI.STEAMSTATIC.COM",
            ),
            (
                "https://cdn.akamai.steamstatic.com:443/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                Some(443),
                "/x",
                "cdn.akamai.steamstatic.com:443",
            ),
            (
                "https://user:pw@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("user"),
                None,
                "/x",
                "user:pw@cdn.akamai.steamstatic.com",
            ),
            (
                "http://cdn.akamai.steamstatic.com/x",
                "http",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://evil.example/SteamSetup.exe",
                "https",
                Some("evil.example"),
                None,
                None,
                "/SteamSetup.exe",
                "evil.example",
            ),
            (
                "//cdn.akamai.steamstatic.com/x",
                "",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "cdn.akamai.steamstatic.com/x",
                "",
                None,
                None,
                None,
                "cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https://cdn.akamai.steamstatic.com./x",
                "https",
                Some("cdn.akamai.steamstatic.com."),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com.",
            ),
            (
                "https://cdn.akamai.steamstatic.com@evil.example/x",
                "https",
                Some("evil.example"),
                Some("cdn.akamai.steamstatic.com"),
                None,
                "/x",
                "cdn.akamai.steamstatic.com@evil.example",
            ),
            (
                "https://evil.example@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("evil.example"),
                None,
                "/x",
                "evil.example@cdn.akamai.steamstatic.com",
            ),
            (
                "https://a@b@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("a@b"),
                None,
                "/x",
                "a@b@cdn.akamai.steamstatic.com",
            ),
            (
                "https://[2001:db8::1]/x",
                "https",
                Some("2001:db8::1"),
                None,
                None,
                "/x",
                "[2001:db8::1]",
            ),
            (
                "https://cdn.akamai.steamstatic.com:notaport/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com:notaport",
            ),
            ("https:///x", "https", None, None, None, "/x", ""),
            ("https://", "https", None, None, None, "", ""),
            ("", "", None, None, None, "", ""),
            (
                "https://cdn.akamai.steamstatic.com\t/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                " https://cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://\tcdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "ftp://cdn.akamai.steamstatic.com/x",
                "ftp",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://cdn.akamai.steamstatic.com.evil.example/x",
                "https",
                Some("cdn.akamai.steamstatic.com.evil.example"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com.evil.example",
            ),
            (
                "https://cdn.akamai.steamstatic.com?x=1",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://cdn.akamai.steamstatic.com#frag",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "1https://cdn.akamai.steamstatic.com/x",
                "",
                None,
                None,
                None,
                "1https://cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https:/cdn.akamai.steamstatic.com/x",
                "https",
                None,
                None,
                None,
                "/cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https://cdn.akamai.steamstatic.com:80/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                Some(80),
                "/x",
                "cdn.akamai.steamstatic.com:80",
            ),
            (
                "smb://server/share/game.exe",
                "smb",
                Some("server"),
                None,
                None,
                "/share/game.exe",
                "server",
            ),
            (
                "smb://user@server/share/game.exe",
                "smb",
                Some("server"),
                Some("user"),
                None,
                "/share/game.exe",
                "user@server",
            ),
            (
                "smb://user:pw@SERVER/Share/dir/game.exe",
                "smb",
                Some("server"),
                Some("user"),
                None,
                "/Share/dir/game.exe",
                "user:pw@SERVER",
            ),
            (
                "smb://server:445/share/game.exe",
                "smb",
                Some("server"),
                None,
                Some(445),
                "/share/game.exe",
                "server:445",
            ),
            (
                "sftp://host/pub/game.exe",
                "sftp",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ssh://host/pub/game.exe",
                "ssh",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ftp://host/pub/game.exe",
                "ftp",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ftps://host/pub/game.exe",
                "ftps",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "dav://host/pub/game.exe",
                "dav",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "davs://host/pub/game.exe",
                "davs",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "nfs://host/export/game.exe",
                "nfs",
                Some("host"),
                None,
                None,
                "/export/game.exe",
                "host",
            ),
            (
                "sftp://user@host:2222/pub/x.exe",
                "sftp",
                Some("host"),
                Some("user"),
                Some(2222),
                "/pub/x.exe",
                "user@host:2222",
            ),
            (
                "smb://server/share/a%20b.exe",
                "smb",
                Some("server"),
                None,
                None,
                "/share/a%20b.exe",
                "server",
            ),
            (
                "smb://us%40er@server/share/x.exe",
                "smb",
                Some("server"),
                Some("us%40er"),
                None,
                "/share/x.exe",
                "us%40er@server",
            ),
            (
                "smb:///nohost/x.exe",
                "smb",
                None,
                None,
                None,
                "/nohost/x.exe",
                "",
            ),
            (
                "file:///home/u/game.exe",
                "file",
                None,
                None,
                None,
                "/home/u/game.exe",
                "",
            ),
            (
                "file://host/home/u/game.exe",
                "file",
                Some("host"),
                None,
                None,
                "/home/u/game.exe",
                "host",
            ),
            (
                "https://host/x?q=1#f",
                "https",
                Some("host"),
                None,
                None,
                "/x",
                "host",
            ),
            (
                "smb://[fe80::1]/share/x.exe",
                "smb",
                Some("fe80::1"),
                None,
                None,
                "/share/x.exe",
                "[fe80::1]",
            ),
            (
                "http://host/pub/x.exe",
                "http",
                Some("host"),
                None,
                None,
                "/pub/x.exe",
                "host",
            ),
            // The rows below were added with `netpaths`, and two of them are
            // here because writing them caught a real defect in the widening
            // rather than because they were obvious:
            //
            // * `[fe80::1%tESt]` — the IPv6 zone. `.hostname` lowercases only
            //   the part before `%`, so the host is `fe80::1%tESt` with the
            //   zone's case intact. Lowercasing the whole literal, which is
            //   what the first cut of the widened parser did, silently picks a
            //   different interface.
            // * `h:0` — port **zero**, which is not `None`. `_hostinfo` only
            //   blanks a port that is the empty string, and `"0"` is a
            //   perfectly good `isdigit()`; so CPython reports 0 here while
            //   `_generic_mount_names`'s `if port:` still treats it as unset.
            //   Both halves of that are load-bearing and neither is guessable.
            (
                "smb://[fe80::1%tESt]/share/x.exe",
                "smb",
                Some("fe80::1%tESt"),
                None,
                None,
                "/share/x.exe",
                "[fe80::1%tESt]",
            ),
            ("smb://h:0/x", "smb", Some("h"), None, Some(0), "/x", "h:0"),
            (
                "smb://[::1]:8080/share/x.exe",
                "smb",
                Some("::1"),
                None,
                Some(8080),
                "/share/x.exe",
                "[::1]:8080",
            ),
            (
                "sftp://user@host:2222/pub/x",
                "sftp",
                Some("host"),
                Some("user"),
                Some(2222),
                "/pub/x",
                "user@host:2222",
            ),
            (
                "davs://host/path/x",
                "davs",
                Some("host"),
                None,
                None,
                "/path/x",
                "host",
            ),
            (
                "file:///home/u/game.exe",
                "file",
                None,
                None,
                None,
                "/home/u/game.exe",
                "",
            ),
            // The username is the text before the **first** colon of the
            // userinfo, so the password never leaks into it — while the
            // userinfo itself split at the **last** `@`, which is why the
            // `a@b` row below has a username containing an `@`.
            (
                "smb://us:er:pw@h/s/x",
                "smb",
                Some("h"),
                Some("us"),
                None,
                "/s/x",
                "us:er:pw@h",
            ),
            (
                "https://a@b@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("a@b"),
                None,
                "/x",
                "a@b@cdn.akamai.steamstatic.com",
            ),
            // A scheme with no netloc, and a netloc with no host.
            (
                "smb:///nohost/x.exe",
                "smb",
                None,
                None,
                None,
                "/nohost/x.exe",
                "",
            ),
            (
                "smb://H/S/dir/game.exe",
                "smb",
                Some("h"),
                None,
                None,
                "/S/dir/game.exe",
                "H",
            ),
            (
                "ftp://host:21/pub/x",
                "ftp",
                Some("host"),
                None,
                Some(21),
                "/pub/x",
                "host:21",
            ),
        ];
        for (url, scheme, host, user, port, path, netloc) in vectors {
            let parts = url_parts(url);
            assert_eq!(parts.scheme, scheme, "scheme of {url:?}");
            assert_eq!(parts.host.as_deref(), host, "host of {url:?}");
            assert_eq!(parts.user.as_deref(), user, "user of {url:?}");
            assert_eq!(parts.port, port, "port of {url:?}");
            assert_eq!(parts.path, path, "path of {url:?}");
            assert_eq!(parts.netloc, netloc, "netloc of {url:?}");
        }
    }

    /// The ports the reference **raises** on, where this returns `None`.
    ///
    /// These cannot live in the table above, because there is no value for
    /// CPython to have produced: `SplitResult.port` raises `ValueError` on
    /// anything that is not ASCII digits in range. This test is the control
    /// arm for the port's `isdigit()`-and-`isascii()` guard — without it, a
    /// parser that used Rust's `str::parse` and nothing else would report
    /// `+8` as port 8, a URL the reference refuses outright, and no row in the
    /// table would notice.
    ///
    /// Recorded rather than replicated: a `ValueError` out of a pure function
    /// has no honest port here, and returning `None` cannot take the caller
    /// down the way the raise does.
    #[test]
    fn a_port_the_reference_raises_on_is_none_here_and_never_a_number() {
        // Measured: every one of these raises `ValueError` from `.port` in
        // CPython, the first with "Port could not be cast to integer value".
        for url in [
            "sftp://h:any/x",
            "sftp://h:8_0/x",
            "sftp://h:+8/x",
            "sftp://h:-8/x",
            "sftp://h: 80/x",
            "sftp://h:1:2/x",
            "sftp://h:99999/x",
            "sftp://h:65536/x",
        ] {
            assert_eq!(url_parts(url).port, None, "port of {url:?}");
        }
        // The control arm: the boundary values the reference *accepts*, so the
        // assertion above cannot be satisfied by a parser that returns `None`
        // for every port.
        assert_eq!(url_parts("sftp://h:0/x").port, Some(0));
        assert_eq!(url_parts("sftp://h:65535/x").port, Some(65_535));
        assert_eq!(url_parts("sftp://h:0080/x").port, Some(80));
    }

    /// The allowlist check itself, on the two arms that matter.
    #[test]
    fn the_origin_check_accepts_the_recipes_host_and_refuses_another() {
        let steam = installer_by_id("steam").unwrap();
        assert!(validate_download_origin(steam, steam.download_url).is_ok());
        // The redirect the reference's test uses.
        let error =
            validate_download_origin(steam, "https://evil.example/SteamSetup.exe").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam redirected to an untrusted download origin: https://evil.example/SteamSetup.exe"
        );
        // The same host over http is still refused: the scheme is checked
        // independently of the host.
        assert!(validate_download_origin(steam, "http://cdn.akamai.steamstatic.com/x").is_err());
        // And a client that cannot report a final URL fails closed rather than
        // falling back to the request URL.
        assert!(validate_download_origin(steam, "").is_err());
        // A look-alike suffix is a different host.
        assert!(
            validate_download_origin(steam, "https://cdn.akamai.steamstatic.com.evil.example/x")
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // The download half — DownloadSecurityTests
    // (tests/test_installers.py:83-155)
    // -----------------------------------------------------------------------

    /// The bytes an EXE payload must start with.
    fn part_files(directory: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".part"))
            })
            .collect()
    }

    /// The `.part` file is `0600` from the moment it exists, as `mkstemp`'s is
    /// (`installers.py:612-614`), and it is `create_new` that makes the name
    /// unguessable-in-effect: `O_CREAT|O_EXCL` refuses a path that already
    /// exists *including* one that is a symlink, so a `.part` name an attacker
    /// planted cannot redirect the write.
    ///
    /// Both halves are asserted against the file the real download uses, not
    /// against a re-created temp file: the mode is read from inside the
    /// transfer's own progress callback, which is the only moment the `.part`
    /// exists, and the symlinks are planted at the names this process's own id
    /// generates, so they are the names `create_partial` actually reaches for.
    #[test]
    fn a_partial_download_is_private_to_its_owner_and_cannot_be_redirected() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("part-mode");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        // The report carries no `Subject:` line at all, and the approved
        // publisher is in the *certificates* the fake `openssl` answers with:
        // a case that could pass by reading the verifier's text would be
        // testing the hole `SEC-11` closed. `CN=Valve Corp.` is the real
        // signer's subject, measured on the downloaded `SteamSetup.exe`.
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();

        let observed: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
        let progress = |_value: f64| {
            for entry in std::fs::read_dir(&dest).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().ends_with(".part") {
                    let mode = entry
                        .metadata()
                        .expect("the temporary file is statable")
                        .permissions()
                        .mode()
                        & 0o777;
                    observed.borrow_mut().push(mode);
                }
            }
        };

        // A symlink at the *target* path is the store-installer case the
        // recipe catalogue cannot rule out, and the reason the install is a
        // rename: `rename` replaces it rather than writing through it.
        let target = dest.join("SteamSetup.exe");
        std::fs::create_dir_all(&dest).expect("the destination directory");
        std::os::unix::fs::symlink(scratch.path().join("sentinel"), &target)
            .expect("plant the target symlink");

        download_installer(
            steam,
            &dest,
            Some(&progress),
            Duration::from_secs(60),
            &FakeResponse::at(steam.download_url, PE_BODY),
            &env,
        )
        .expect("an authenticated download");

        let observed = observed.into_inner();
        assert!(
            !observed.is_empty(),
            "no .part file was observed during the transfer"
        );
        assert!(
            observed.iter().all(|mode| *mode == 0o600),
            "the .part file was readable by more than its owner: {observed:?}"
        );
        assert!(
            !target.is_symlink(),
            "the rename wrote through the target symlink"
        );
        assert_eq!(std::fs::read(&target).unwrap(), PE_BODY);
    }

    /// A hostile `.part` name cannot make `create_partial` panic, escape the
    /// destination, or open something it did not create.
    ///
    /// The name reaches it from the recipe catalogue, which is data this app
    /// did not write; a name the filesystem refuses is a failed download, not
    /// a crash. Written as a battery because the failures are all different
    /// kinds — `InvalidInput` for a NUL, `ENAMETOOLONG`, `ENOENT` for a
    /// component that is not there — and the contract is only that none of
    /// them is a panic.
    #[test]
    fn hostile_partial_names_fail_cleanly() {
        let scratch = Scratch::new("part-hostile");
        let dir = scratch.path();
        let cases = [
            String::new(),
            ".".to_string(),
            "..".to_string(),
            "a/b".to_string(),
            "..\\..\\evil.exe".to_string(),
            "nul\u{0}byte".to_string(),
            "x".repeat(4096),
        ];
        for name in cases {
            match create_partial(dir, &name) {
                Ok((file, path)) => {
                    // Accepted names still obey the two invariants.
                    assert!(path.starts_with(dir), "{name:?} left {dir:?}");
                    assert!(!path.is_symlink(), "{name:?} opened a symlink");
                    drop(file);
                }
                Err(error) => {
                    // A refused name is a value the caller reports, not a panic.
                    assert!(!error.to_string().is_empty());
                }
            }
        }
    }

    /// The same protection on the path a hostile name cannot be sanitised away
    /// on: an existing `.part` name this process would itself generate.
    #[test]
    fn a_guessed_partial_name_is_neither_followed_nor_reused() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("part-symlink");
        let dir = scratch.path();
        let victim = dir.join("victim");
        std::fs::write(&victim, b"private").expect("the victim file");
        let process = std::process::id();
        // The first three names `create_partial` tries, all pointed at the
        // victim, and the fourth already taken by a directory.
        for attempt in 0..3 {
            std::os::unix::fs::symlink(
                &victim,
                dir.join(format!(".SteamSetup.exe.{process}.{attempt}.part")),
            )
            .expect("plant the symlink");
        }
        std::fs::create_dir(dir.join(format!(".SteamSetup.exe.{process}.3.part")))
            .expect("take the fourth name");

        let (mut file, path) =
            create_partial(dir, "SteamSetup.exe").expect("the fifth name is free");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!(".SteamSetup.exe.{process}.4.part")
        );
        assert!(!path.is_symlink());
        std::io::Write::write_all(&mut file, b"payload").expect("write the partial");
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"private",
            "the write followed a planted symlink onto the victim"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// `test_download_is_authenticated_before_atomic_install`
    /// (`tests/test_installers.py:101-111`).
    ///
    /// The control arm for the three refusal tests below: with an approved
    /// origin, an `MZ` payload and a publisher the recipe approves, the
    /// download lands, keeps its bytes, and leaves no temporary file. Without
    /// it a `download_installer` that refused everything would pass all of them.
    #[test]
    fn an_authenticated_download_is_installed_atomically() {
        let scratch = Scratch::new("download-ok");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);

        let target =
            download_installer(steam, &dest, None, Duration::from_secs(60), &client, &env).unwrap();

        assert_eq!(target, dest.join("SteamSetup.exe"));
        assert_eq!(std::fs::read(&target).unwrap(), PE_BODY);
        assert!(part_files(&dest).is_empty(), "a .part file was left behind");
        // The verifier ran, and it ran on the temporary file — so the
        // signature was checked before anything existed at the target path.
        //
        // The record holds two calls since `SEC-11`, because the publisher
        // check drives a second subcommand of the same tool: `verify` on the
        // `.part` file, then `extract-signature` on **the same** `.part` file.
        // The second half is the load-bearing one to assert — an extraction
        // that read the installed path, or a path of its own, would be reading
        // bytes no chain check ever covered.
        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], "verify");
        assert_eq!(lines[1], "-in");
        assert!(lines[2].ends_with(".part"), "{lines:?}");
        assert_eq!(
            &lines[3..7],
            &["extract-signature", "-in", lines[2], "-out"],
            "{lines:?}"
        );
        assert!(
            lines[7].ends_with("signature.p7b"),
            "the extraction wrote somewhere other than its own scratch file: {lines:?}"
        );
        assert_eq!(lines.len(), 8, "the tool ran more than twice: {lines:?}");
    }

    /// `test_untrusted_redirect_is_rejected_and_removed`
    /// (`tests/test_installers.py:113-123`).
    ///
    /// # The four assertions, and why each is separate
    ///
    /// The error message is the reference's. The verifier must **not** have run
    /// — checked by the absence of its record file, which is the strongest form
    /// of `verify.assert_not_called()`: a mock can only say the function was not
    /// called, while this says the program was never executed. The target must
    /// not exist. No `.part` file may remain.
    ///
    /// And the fourth, which the reference cannot make: **no body byte was
    /// delivered**. That is the property that makes the check's placement in
    /// the head callback load-bearing rather than cosmetic — moving it after
    /// the transfer would still delete the file and still pass the other three.
    #[test]
    fn an_untrusted_redirect_is_rejected_before_a_single_byte() {
        let scratch = Scratch::new("redirect");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"MZpayload")
            .redirected_to("https://evil.example/SteamSetup.exe");

        let error = download_installer(steam, &dest, None, Duration::from_secs(60), &client, &env)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Steam redirected to an untrusted download origin: https://evil.example/SteamSetup.exe"
        );
        // The *variant*, not only the sentence. Both matter, and they are
        // separately losable: a refusal that returned a generic transport error
        // carrying this text would satisfy any message assertion while telling
        // a caller nothing about why the download stopped.
        assert!(
            matches!(error, InstallerError::UntrustedOrigin { ref name, ref url }
                if name == "Steam" && url == "https://evil.example/SteamSetup.exe"),
            "{error:?}"
        );
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
        assert_eq!(
            client.delivered(),
            0,
            "the body was transferred from a host the allowlist never approved"
        );
    }

    /// UX-27: a flag already set when the request begins is the cheapest
    /// refusal there is — ahead of even the origin check — and the `.part`
    /// file it abandons is removed by `download_installer_with`'s error path.
    ///
    /// `delivered == 0` is the load-bearing half: `FakeResponse` only counts a
    /// body it was allowed to send, so zero says the cancel landed before a
    /// single byte, not after.
    #[test]
    fn a_cancel_before_the_first_byte_leaves_no_partial_and_sends_nothing() {
        let scratch = Scratch::new("cancel-head");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);

        let error = download_installer_with(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &env,
            &|| true,
        )
        .unwrap_err();

        assert!(
            matches!(error, InstallerError::Cancelled),
            "expected Cancelled, got {error:?}"
        );
        assert_eq!(
            client.delivered(),
            0,
            "the body was sent to a cancelled job"
        );
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
    }

    /// UX-27: a cancel that lands with the last chunk still skips the
    /// signature run — the expensive part left — and the `.part` cleanup.
    ///
    /// `FakeResponse` delivers its body in one `sink` call, so a mid-stream
    /// abort is not reachable with it; what *is* reachable is the post-body
    /// check, driven here by a progress callback that flips the flag — the
    /// last moment the flag can matter before the verifier would run. The
    /// verifier's record staying absent is the assertion that distinguishes
    /// this test from a generic "the download failed".
    #[test]
    fn a_cancel_with_the_last_chunk_skips_the_verifier() {
        let scratch = Scratch::new("cancel-late");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);
        let flag = std::sync::atomic::AtomicBool::new(false);

        let error = download_installer_with(
            steam,
            &dest,
            Some(&|_| flag.store(true, std::sync::atomic::Ordering::SeqCst)),
            Duration::from_secs(60),
            &client,
            &env,
            &|| flag.load(std::sync::atomic::Ordering::SeqCst),
        )
        .unwrap_err();

        assert!(
            matches!(error, InstallerError::Cancelled),
            "expected Cancelled, got {error:?}"
        );
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
    }

    /// `test_non_pe_payload_is_rejected_before_signature_check`
    /// (`tests/test_installers.py:125-133`).
    #[test]
    fn a_payload_that_is_not_an_exe_is_refused_before_the_signature() {
        let scratch = Scratch::new("not-pe");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"not-an-executable");

        let error = download_installer(steam, &dest, None, Duration::from_secs(60), &client, &env)
            .unwrap_err();

        assert_eq!(error.to_string(), "Steam download is not a valid EXE file");
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(part_files(&dest).is_empty());
    }

    /// The MSI arm of the same check, which the reference does not have a test
    /// for: `_validate_installer_magic` branches on `kind`, and the branch that
    /// expects the OLE header is only reachable through Epic. Without this,
    /// a port that expected `MZ` for both kinds would pass every other test.
    #[test]
    fn a_payload_that_is_not_an_ole_file_is_refused_for_an_msi() {
        let scratch = Scratch::new("not-ole");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            "Signature verification: ok",
            0,
            &["CN=Epic Games Inc.,O=Epic Games Inc.,C=US"],
        );
        let epic = installer_by_id("epic").unwrap();
        assert_eq!(epic.kind, Kind::Msi);
        // An `MZ` payload for an MSI recipe is the cross-wired case.
        let client = FakeResponse::at(epic.download_url, PE_BODY);

        let error = download_installer(epic, &dest, None, Duration::from_secs(60), &client, &env)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Epic Games Launcher download is not a valid MSI file"
        );

        // The control for this arm: the real OLE header is accepted, so the
        // test above cannot pass by refusing every MSI.
        let mut ole = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend_from_slice(b"payload");
        let good = FakeResponse::at(epic.download_url, &ole);
        // The recipe's own publisher string is `Epic Games Inc.` — no comma,
        // and the difference matters because the check is a substring test.
        // It is in the certificate now rather than in the verifier's report,
        // which is the whole of `SEC-11`.
        let env = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-2"),
            "Signature verification: ok",
            0,
            &["CN=Epic Games Inc.,O=Epic Games Inc.,C=US"],
        );
        let target =
            download_installer(epic, &dest, None, Duration::from_secs(60), &good, &env).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), ole);
    }

    /// The size cap's cheap arm: a declared length over the cap is refused from
    /// the head, before the body — the same `delivered == 0` observation as the
    /// origin test, and here it is the *only* way to show the check is cheap.
    #[test]
    fn a_declared_length_over_the_cap_is_refused_before_the_body() {
        let scratch = Scratch::new("declared-cap");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY)
            .declaring(&(MAX_INSTALLER_BYTES + 1).to_string());

        let error = download_installer(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam installer exceeds the download size limit"
        );
        // The variant as well as the sentence: the head callback records
        // `TooLarge` and returns a generic error to abandon the transfer, and a
        // test that only read the message could not see the difference.
        assert!(matches!(error, InstallerError::TooLarge { ref name } if name == "Steam"));
        assert_eq!(client.delivered(), 0);
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(part_files(&dest).is_empty());
    }

    /// A server that understates its length still cannot fill the disk.
    ///
    /// **This test has no counterpart in the reference**, which covers the
    /// origin, the magic and the publisher and never the streaming cap. It is
    /// kept because `total` is checked here and `Content-Length` is checked in
    /// the head callback, and only this one is observable from a server that
    /// lies — but its cost is this port's own and is named rather than hidden:
    /// the cap is a gibibyte, so reaching the second check really does write a
    /// gibibyte to the destination before the refusal.
    ///
    /// An earlier version of this comment justified that by saying the
    /// destination is under [`std::env::temp_dir`], which is a tmpfs "in the
    /// common case". On the machine this was written on it is not — `/tmp` is
    /// ext4 there, measured — so the sentence was doing the opposite of its job:
    /// it made a gigabyte of real I/O look free to every later reader. The
    /// assertions below are what the test is for, and none of them depend on
    /// where the file lives, so the honest statement is simply that the write is
    /// real and the coverage is worth it.
    ///
    /// The client is bounded at one chunk past the cap on purpose: a cap that
    /// stopped working would otherwise make this test hang instead of fail.
    #[test]
    fn a_body_that_crosses_the_cap_is_refused_while_it_streams() {
        const CHUNK: usize = 64 * 1024 * 1024;
        struct Endless {
            chunk: Vec<u8>,
            chunk_index: std::cell::Cell<u64>,
        }
        impl crate::runners::proton::HttpClient for Endless {
            fn get(
                &self,
                _url: &str,
                _headers: &[(&str, &str)],
                _timeout: Duration,
                on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
                sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
            ) -> Result<(), RunnerError> {
                on_head(&ResponseHead {
                    // A declared length the server lies about: under the cap.
                    content_length: Some("1024".to_string()),
                    final_url: installer_by_id("steam").unwrap().download_url.to_string(),
                })?;
                loop {
                    self.chunk_index.set(self.chunk_index.get() + 1);
                    sink(&self.chunk)?;
                    // 1 GiB plus one chunk, then stop: an unbounded loop would
                    // hang rather than fail if the cap stopped working.
                    if self.chunk_index.get() > MAX_INSTALLER_BYTES / CHUNK as u64 + 1 {
                        return Ok(());
                    }
                }
            }
        }

        let scratch = Scratch::new("streamed-cap");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        let client = Endless {
            chunk: vec![0u8; CHUNK],
            chunk_index: std::cell::Cell::new(0),
        };
        let error = download_installer(
            installer_by_id("steam").unwrap(),
            &dest,
            None,
            Duration::from_secs(600),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam installer exceeds the download size limit"
        );
        assert!(!record.exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
        // The loop stops within a chunk or two of the cap rather than running
        // to its own bound, which is what "refused while it streams" means.
        assert!(
            client.chunk_index.get() <= MAX_INSTALLER_BYTES / CHUNK as u64 + 2,
            "read {} chunks",
            client.chunk_index.get()
        );
    }

    /// The progress callback reports the fraction of the declared length, and
    /// finishes at exactly `1.0` even though the last chunk usually overshoots
    /// the declaration.
    #[test]
    fn progress_is_reported_as_a_fraction_and_ends_at_one() {
        let scratch = Scratch::new("progress");
        let dest = scratch.path().join("downloads");
        // The success line only: the publisher is in the certificates since
        // `SEC-11`, so a case that reached the publisher test on the verifier's
        // text would be the hole this port closed.
        let env = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv"),
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);
        let seen = std::cell::RefCell::new(Vec::new());
        let progress = |fraction: f64| seen.borrow_mut().push(fraction);

        download_installer(
            steam,
            &dest,
            Some(&progress),
            Duration::from_secs(60),
            &client,
            &env,
        )
        .unwrap();

        let seen = seen.into_inner();
        assert_eq!(seen.last(), Some(&1.0));
        assert!(
            seen.iter().all(|value| (0.0..=1.0).contains(value)),
            "{seen:?}"
        );
        assert!(seen.len() >= 2, "the body arrives in chunks: {seen:?}");

        // With no declared length there is no denominator, so the reference
        // reports nothing during the transfer and only the final `1.0` — which
        // is why that last call sits outside the `if total` guard.
        let seen = std::cell::RefCell::new(Vec::new());
        let progress = |fraction: f64| seen.borrow_mut().push(fraction);
        let client = FakeResponse::at(steam.download_url, PE_BODY).without_declared_length();
        download_installer(
            steam,
            &scratch.path().join("downloads-undeclared"),
            Some(&progress),
            Duration::from_secs(60),
            &client,
            &verifier_env_with_certificates(
                scratch.path(),
                &scratch.path().join("argv-undeclared"),
                "Signature verification: ok",
                0,
                &[VALVE_CERTIFICATE],
            ),
        )
        .unwrap();
        assert_eq!(seen.into_inner(), vec![1.0]);
    }

    /// A download that fails does not destroy an installer that is already
    /// there: the temporary file is written beside the target and the target is
    /// only ever created by a rename.
    #[test]
    fn a_refused_download_leaves_an_existing_installer_alone() {
        let scratch = Scratch::new("keep-existing");
        let dest = scratch.path().join("downloads");
        std::fs::create_dir_all(&dest).unwrap();
        let existing = dest.join("SteamSetup.exe");
        std::fs::write(&existing, b"the good copy").unwrap();
        let script = fake_osslsigncode(scratch.path(), &scratch.path().join("argv"), "", 0);
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"not-an-executable");

        assert!(
            download_installer(
                steam,
                &dest,
                None,
                Duration::from_secs(60),
                &client,
                &verifier_env(&script),
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&existing).unwrap(), b"the good copy");
        assert!(part_files(&dest).is_empty());
    }

    // -----------------------------------------------------------------------
}
