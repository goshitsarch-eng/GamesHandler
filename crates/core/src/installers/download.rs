//! The download half of the easy installer (`installers.py:533-646`).
//!
//! Origin and payload validation, Authenticode verification and the download
//! itself. See [`crate::installers`] for the catalog, the error type and the
//! seams.

use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand};
use std::time::Duration;

use crate::installers::command::{MAX_INSTALLER_BYTES, safe_download_name};
use crate::installers::{Installer, InstallerError, Kind};
use crate::paths::Env;
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

/// The bundled Microsoft root, if this build can find it
/// (`installers.py:537-548`).
///
/// The reference's four candidates, in order. The second is
/// `Path(__file__).with_name(...)` — the `.pem` sitting beside the Python
/// module — and has no counterpart here because there is no Python module; the
/// third is the repository's `data/` directory relative to the module, which
/// translates to a path relative to this crate's source; the fourth is Flatpak's
/// `/app/share/gamehandler`.
///
/// The first candidate is the environment override, and it is the one that
/// makes this testable and packagable: `GAMEHANDLER_AUTHENTICODE_ROOT` pointing
/// at a real `.pem` satisfies the check without anything having to be installed
/// beside the binary.
pub fn authenticode_root_path(env: &dyn Env) -> Result<PathBuf, InstallerError> {
    let override_path = env
        .var("GAMEHANDLER_AUTHENTICODE_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = override_path {
        candidates.push(path);
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(AUTHENTICODE_ROOT_NAME),
    );
    candidates.push(Path::new("/app/share/gamehandler").join(AUTHENTICODE_ROOT_NAME));
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(InstallerError::AuthenticodeRootUnavailable)
}

/// The filename the bundled root is expected under (`installers.py:534`).
pub const AUTHENTICODE_ROOT_NAME: &str = "microsoft-identity-verification-root-ca-2020.pem";

/// Seconds the signature verifier is given before it is killed
/// (`installers.py:586`).
const SIGNATURE_TIMEOUT: Duration = Duration::from_secs(90);

/// Verify the Authenticode chain and the expected publisher before execution
/// (`installers.py:569-595`).
///
/// # The argv, and why it is two inserts rather than a conditional tail
///
/// The reference builds `[verifier, "verify", "-in", path]` and then, when the
/// recipe pins the Microsoft root, splices two option *pairs* in at index 2 —
/// so they end up between the subcommand and `-in`, and the result is
/// `[verifier, "verify", "-CAfile", root, "-TSA-CAfile", root, "-in", path]`.
/// Building it in that order matters: appending the options at the end would
/// put them after the file argument, which `osslsigncode` parses differently.
///
/// # The two independent conditions
///
/// The chain must verify — the output must contain `Signature verification:
/// ok` **and** the exit status must be 0 — and one of the recipe's publisher
/// strings must appear in the subject of a certificate the signature carries.
///
/// # Where the publisher string is read from (SEC-03, SEC-11)
///
/// The reference tests the publisher against the *whole* merged output
/// (`installers.py:594-595`: `any(publisher.casefold() in folded ...)` where
/// `folded = output.casefold()`), and the merged output carries fields the
/// **signer** chooses, not the certificate: `osslsigncode` 2.14 prints
/// `Text description:` and `URL description:` out of the signature's
/// authenticated attributes, and `-n`/`-u` set them at signing time. Measured
/// against the bundled 2.14 binary, a self-signed payload signed with
/// `-n "Valve Corp."` produced an output containing `Text description: Valve
/// Corp.` under a `Subject:` of `CN=Totally Unrelated Signer,O=Evil Example
/// Ltd`, so the reference's predicate accepted a certificate that has nothing
/// to do with Valve.
///
/// Reading the `Subject:` *field* was this port's first answer to that
/// (`SEC-03`), and it is not enough: `subject_values` tested
/// `line.trim().strip_prefix("Subject:")`, so the prefix test ran **after** the
/// trim and accepted any line that trims to a `Subject:`. The signer's own
/// `-n` value is printed verbatim and may contain newlines, so `-n "$(printf
/// 'Totally Unrelated\n\t\tSubject: C=US,O=Valve Corp.,CN=Valve Corp.')"`
/// appends a line that is indistinguishable from a certificate subject — and
/// the trimmed shape it needs is exactly what `verify`'s own indentation
/// provides. Measured on the bundled 2.14: that command signs a payload with a
/// certificate whose subject is `C=US, O=Evil Example Ltd, CN=Totally Unrelated
/// Signer`, `verify -CAfile` exits 0, prints the forged `Subject: C=US,O=Valve
/// Corp.,CN=Valve Corp.` line, and the gate returned `Ok(())` for it.
///
/// So the publisher is now read from the **certificates themselves** and never
/// from the verifier's report: [`signed_subject_values`] pulls the signature's
/// PKCS#7 blob out of the file and asks `openssl` for the RFC2253 subject of
/// every certificate in it. Nothing the signer types can reach that path — the
/// only bytes parsed out of it are the certificate extensions the signature
/// covers.
///
/// **Both are deliberate divergences from the reference**, on the brief's
/// decision order: `installers.py:594-595` folds the output and searches all of
/// it, and a port that reproduced that would reproduce both holes. The message
/// the user sees is unchanged, so no P-item is affected.
///
/// # The catalog strings are written in `osslsigncode`'s rendering
///
/// What the divergence above does **not** change is the shape of the catalog
/// strings, because they were authored against the `Subject:` lines the
/// reference's own verifier printed. That rendering is
/// `X509_NAME_print_ex(…, XN_FLAG_RFC2253)`: fields in reverse order,
/// `+`/`,`-joined, no space after each comma, and `\,` inside a value that
/// contains one. `C=US,O=Valve Corp.,CN=Valve Corp.` is that form.
///
/// The consequence is measured, not assumed, on the **GOG recipe's own
/// installer**: its signer's certificate is `CN=GOG  sp. z o.o,O=GOG  sp. z
/// o.o,L=WARSZAWA,C=PL` in that rendering, and the catalog's
/// `CN=GOG  sp. z o.o,O=GOG  sp. z o.o` is a substring of it. The one-call
/// shortcut that `openssl pkcs7 -print_certs -noout` offers renders the *same*
/// certificate as `C=PL, L=WARSZAWA, O=GOG  sp. z o.o, CN=GOG  sp. z o.o` —
/// field order, space after each comma — of which the catalog string is **not**
/// a substring, so that shortcut would refuse a signature that is genuinely
/// GOG's. `openssl x509 -nameopt RFC2253` reproduces the verifier's rendering
/// byte for byte (checked on the real Steam, Amazon and GOG certificates,
/// including one with `\,`-escaped commas and a twelve-field DN), which is why
/// the extraction below pays for one call per certificate. "Tidying" the
/// catalog's double space or its field order still stops matching a signature
/// that is genuinely GOG's, exactly as before.
///
/// # The trust anchor is `osslsigncode`'s, not this function's
///
/// Nine of the ten recipes set no `microsoft_trust_root`, so no `-CAfile` is
/// passed and the chain is checked against the host's store. That is not the
/// hole the paragraph above closes: measured with the same binary, a
/// self-signed payload **without** `-CAfile` exits 1, prints `Error:
/// self-signed certificate` and `Signature verification: failed`, and never
/// prints the success line this function requires. A certificate the signer
/// minted for itself therefore fails before the publisher test is reached.
pub fn verify_installer_authenticity(
    installer: &Installer,
    path: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<(), InstallerError> {
    let Some(verifier) = launch_env.which("osslsigncode") else {
        return Err(InstallerError::SignatureToolMissing);
    };
    let mut command: Vec<String> = vec![
        verifier.to_string_lossy().into_owned(),
        "verify".to_string(),
        "-in".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    if installer.microsoft_trust_root {
        let root = authenticode_root_path(launch_env)?
            .to_string_lossy()
            .into_owned();
        command.splice(
            2..2,
            [
                "-CAfile".to_string(),
                root.clone(),
                "-TSA-CAfile".to_string(),
                root,
            ],
        );
    }

    let output = run_signature_tool(&command, installer.name)?;

    let text = output.text;
    if output.status != Some(0) || !text.contains("Signature verification: ok") {
        return Err(InstallerError::SignatureInvalid {
            name: installer.name.to_string(),
            tail: output_tail(&text),
        });
    }
    // The certificates themselves, and nothing the verifier printed about
    // them. See the doc comment: naming the `Subject:` field is not enough,
    // because the signer's `-n` value can contain a newline and a line that
    // trims to a `Subject:`.
    let subjects = signed_subject_values(installer, path, &verifier, launch_env)?;
    if !installer.publishers.iter().any(|publisher| {
        let publisher = publisher.to_lowercase();
        subjects.iter().any(|subject| subject.contains(&publisher))
    }) {
        return Err(InstallerError::PublisherUnapproved {
            name: installer.name.to_string(),
        });
    }
    Ok(())
}

/// The case-folded subject of every certificate the installer's signature
/// carries (`SEC-11`).
///
/// Three steps, none of which reads the verifier's human-readable report:
///
/// 1. `osslsigncode extract-signature -in <installer> -out <p7b>` — the
///    Authenticode signature as the PKCS#7 blob it is, with no rendering in
///    between. Fails (exit 255, `No signature found`) when the file carries no
///    signature at all.
/// 2. `openssl pkcs7 -inform DER -in <p7b> -print_certs` — the certificates
///    that blob carries, as a PEM bundle. The "catalog strings are written in
///    `osslsigncode`'s rendering` section of [`verify_installer_authenticity`]
///    has the measurement for why the per-certificate step that follows is
///    worth its subprocesses.
/// 3. `openssl x509 -in <one certificate> -noout -subject -nameopt RFC2253`,
///    once per certificate, because **`openssl x509` reads exactly one
///    certificate from its input**: handed a bundle it prints the first one and
///    stops (measured — a six-certificate bundle yields one `subject=` line).
///
/// Every failure is a refusal, never a pass, and none of them is a panic: the
/// bytes come from a downloaded file and the tools come from the host, so both
/// are treated as untrusted input and as a dependency that may be absent.
fn signed_subject_values(
    installer: &Installer,
    path: &Path,
    verifier: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<Vec<String>, InstallerError> {
    let name = installer.name;

    // Looked up before the scratch directory exists, because a host without
    // `openssl` has nothing to extract *for*. `openssl` is the reader, a
    // different dependency from the verifier: a host can have one without the
    // other, so a missing reader is its own error rather than
    // `SignatureToolMissing`, which means the verifier is absent.
    let Some(reader) = launch_env.which("openssl") else {
        return Err(InstallerError::CertificateReaderMissing);
    };

    // `path` is the downloaded file, and this is the one place in this module
    // that writes next to it rather than into the destination directory: the
    // blob is derived data, is read once, and must not be visible to another
    // local user while it exists.
    let scratch = SignatureScratch::create().map_err(InstallerError::Io)?;
    let signature = scratch.path().join("signature.p7b");
    let certificate = scratch.path().join("certificate.pem");
    let signature_arg = signature.to_string_lossy().into_owned();
    let certificate_arg = certificate.to_string_lossy().into_owned();
    let installer_arg = path.to_string_lossy().into_owned();

    // 1. The signature blob, as PKCS#7.
    let command = tool_argv(
        verifier,
        &[
            "extract-signature",
            "-in",
            &installer_arg,
            "-out",
            &signature_arg,
        ],
    );
    let output = run_signature_tool(&command, name)?;
    if output.status != Some(0) || !signature.is_file() {
        // The `is_file` half is not redundant: the check is that there is a
        // blob to read, and a tool that reports success without writing one
        // would otherwise hand `openssl` a path that does not exist — a
        // different error than the one the user needs to see.
        return Err(InstallerError::SignatureUnreadable {
            name: name.to_string(),
            tail: output_tail(&output.text),
        });
    }

    // 2. The certificates the blob carries, as PEM.
    let command = tool_argv(
        &reader,
        &[
            "pkcs7",
            "-inform",
            "DER",
            "-in",
            &signature_arg,
            "-print_certs",
        ],
    );
    let output = run_signature_tool(&command, name)?;
    let blocks = pem_certificates(&output.text);
    if output.status != Some(0) || blocks.is_empty() {
        return Err(InstallerError::CertificateReadFailed {
            name: name.to_string(),
            tail: output_tail(&output.text),
        });
    }

    // 3. Each certificate's subject, in the one rendering the catalog strings
    //    are written in (see [`verify_installer_authenticity`]). One
    //    `openssl x509` per certificate, and the file it reads is rewritten
    //    rather than duplicated: `openssl x509` will not read past the first
    //    certificate of a multi-certificate file, so handing it the whole
    //    bundle is not an option.
    let mut subjects = Vec::with_capacity(blocks.len());
    for block in &blocks {
        std::fs::write(&certificate, block).map_err(InstallerError::Io)?;
        let command = tool_argv(
            &reader,
            &[
                "x509",
                "-in",
                &certificate_arg,
                "-noout",
                "-subject",
                "-nameopt",
                "RFC2253",
            ],
        );
        let output = run_signature_tool(&command, name)?;
        if output.status != Some(0) {
            return Err(InstallerError::CertificateReadFailed {
                name: name.to_string(),
                tail: output_tail(&output.text),
            });
        }
        subjects.extend(x509_subject_values(&output.text));
    }

    if subjects.is_empty() {
        // Every certificate read cleanly and none of them had a subject, which
        // is not a certificate subject at all. Refused rather than returned as
        // an empty list, so "no subjects" can never read as "no constraint".
        return Err(InstallerError::CertificateReadFailed {
            name: name.to_string(),
            tail: String::new(),
        });
    }
    Ok(subjects)
}

/// The argv of a signature tool: the program, then its arguments.
///
/// The same shape [`run_capturing`] takes, built here rather than at each call
/// site because the three calls in [`signed_subject_values`] mix an owned path
/// with borrowed flags and none of them is more readable for spelling that out.
fn tool_argv(program: &Path, arguments: &[&str]) -> Vec<String> {
    let mut argv = vec![program.to_string_lossy().into_owned()];
    argv.extend(arguments.iter().map(|argument| (*argument).to_string()));
    argv
}

/// Run one of the signature tools, mapping its failure modes onto the errors
/// this module already reports.
///
/// The bound is the verifier's own [`SIGNATURE_TIMEOUT`], reused rather than
/// given a constant of its own: this is the same work on the same file, and a
/// second number would be a second thing to justify. A spawn that fails is
/// [`InstallerError::Io`], as it is for the verifier itself.
fn run_signature_tool(argv: &[String], name: &str) -> Result<CommandOutput, InstallerError> {
    match run_capturing(argv, SIGNATURE_TIMEOUT) {
        Ok(output) => Ok(output),
        Err(RunFailure::TimedOut) => Err(InstallerError::SignatureTimedOut {
            name: name.to_string(),
        }),
        Err(RunFailure::Failed(error)) => Err(error.into()),
    }
}

/// The last eight lines a tool printed, joined with newlines — what
/// [`InstallerError::SignatureInvalid`] has always carried, now shared with the
/// two errors the extraction can raise.
fn output_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(8)..].join("\n")
}

/// Every certificate in a PEM bundle, as its own PEM block.
///
/// `openssl pkcs7 -print_certs` writes one `-----BEGIN CERTIFICATE-----` block
/// per certificate, and this splits on those markers rather than on a line
/// count or a byte length — the bundle is whatever the file's signature
/// carries, so its size is not knowable in advance. A `BEGIN` seen before the
/// previous `END` means the output is not the bundle this expects, and the
/// partial block is dropped rather than concatenated onto the next one; a
/// truncated trailing block is dropped the same way, and
/// [`signed_subject_values`] then has one certificate fewer to match, never one
/// forged certificate more.
fn pem_certificates(text: &str) -> Vec<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line == BEGIN {
            current.clear();
            current.push(line);
        } else if !current.is_empty() {
            current.push(line);
            if line == END {
                blocks.push(current.join("\n"));
                current.clear();
            }
        }
    }
    blocks
}

/// The case-folded value of every `subject=` line in `openssl x509`'s output.
///
/// `-noout -subject` makes that the whole of its stdout: one `subject=<dn>` line
/// per certificate read, in the `KEY=VALUE` spelling `-nameopt RFC2253` gives
/// (`subject=CN=Valve Corp.,O=Valve Corp.,…`, no space around the `=`). The
/// split is on the **first** `=` and the label is matched case-insensitively,
/// so a value that itself contains an `=` keeps all of it, and a future spelling
/// change to the label is a mismatch rather than a silently empty list — which
/// is the failure that would matter, because "no subjects" must never read as
/// "no constraint".
///
/// A value is taken to the end of its line. `openssl` never wraps one: it
/// escapes a newline inside a name as `\0A` rather than printing it (measured
/// with a certificate whose CN contains one), so unlike the verifier's report —
/// whose `-n` continuation lines are what `SEC-11` was — an `openssl x509`
/// subject line cannot be continued by data the signer chose.
fn x509_subject_values(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(label, _)| label.trim().eq_ignore_ascii_case("subject"))
        .map(|(_, value)| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

/// The scratch directory the extracted signature is written to, removed when it
/// goes out of scope.
///
/// A directory rather than a single file, and `0700` rather than the default,
/// for the reason [`create_partial`] gives about the download's `.part`: the
/// contents are attacker-influenced bytes that a later step parses, so no other
/// local user should be able to read or replace them. It is created under
/// `std::env::temp_dir()` — `$TMPDIR` or `/tmp`, the same choice the tests'
/// `Scratch` makes — and named with this process's id and a counter, so two
/// concurrent verifications cannot collide and a leftover from a crashed run
/// cannot be opened as if it were ours (`create` is `O_EXCL`-equivalent: it
/// fails on a path that already exists, including a symlink).
///
/// The [`Drop`] is what makes "cleaned up on every path" true rather than
/// repeated three times: every return in [`signed_subject_values`] is an
/// ordinary `?`/`return`, so the guard runs on success and on all three of the
/// failures alike — including the one where a tool timed out and the caller is
/// unwinding.
struct SignatureScratch(PathBuf);

impl SignatureScratch {
    fn create() -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        /// `TMP_MAX`-ish: `mkstemp` gives up after a bounded number of attempts.
        const ATTEMPTS: u32 = 1_000;
        let parent = std::env::temp_dir();
        let process = std::process::id();
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        for attempt in 0..ATTEMPTS {
            let path = parent.join(format!("gh-signature-{process}-{attempt}"));
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create a temporary directory for the signature",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SignatureScratch {
    fn drop(&mut self) {
        // Best effort: a failure here must not mask the error that caused the
        // drop (`StagingDirectory` in `runners::proton` makes the same choice
        // for the same reason).
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What running a child produced.
pub(crate) struct CommandOutput {
    /// The exit status, or `None` when the child was killed by a signal.
    pub status: Option<i32>,
    /// `stdout` and `stderr` merged, as `stderr=subprocess.STDOUT` gives.
    pub text: String,
}

/// Why a child could not be run to completion.
pub(crate) enum RunFailure {
    /// It outlived its bound and was killed.
    TimedOut,
    /// It could not be spawned, or its streams could not be read.
    Failed(std::io::Error),
}

/// How many times a spawn is retried after `ETXTBSY`, and how long between
/// attempts. Five retries over ten milliseconds is fifty milliseconds — three
/// orders of magnitude more than the window they are covering, and small enough
/// that a caller's own timeout still means what it says.
pub(crate) const SPAWN_RETRIES: u32 = 5;
/// See [`SPAWN_RETRIES`].
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// `Command::spawn`, retrying the one failure that is not a property of the
/// program being run.
///
/// `execve` reports `ETXTBSY` while the target file is open for writing by
/// *any* process. In a threaded program that is a race rather than a fact about
/// the file: `fork` copies the whole descriptor table, so a child another
/// thread is midway through spawning inherits the write descriptor some third
/// thread had open on a file it has already closed and is about to execute. The
/// window is the microseconds between that `fork` and its `exec`, so the
/// condition is always momentary — which is exactly why it is safe to wait it
/// out and why it is never worth reporting.
///
/// # This is a divergence from the reference, and it is here for a named reason
///
/// `subprocess.run` spawns once and lets the `OSError` out. The port cannot do
/// that and keep a hermetic suite: the fakes below are shell scripts written a
/// moment before they are executed, in a test binary where four hundred cases
/// fork concurrently, so the reference's single-spawn behaviour turned into a
/// flake at roughly one run in six — measured, not feared (see
/// `a_held_open_executable_is_retried_instead_of_reported`). The alternative
/// was to sprinkle blind retries through the tests, which would re-run whole
/// downloads and would be unable to tell this transient apart from a real
/// refusal; retrying at the one place that can see the errno is both narrower
/// and cheaper.
///
/// It is bounded, and it is keyed on that single error: every other reason a
/// spawn can fail is returned on the first attempt, and once the retries are
/// exhausted the original error is returned unchanged. So the only observable
/// difference from the reference is on a call that was going to fail for a
/// reason that had already stopped being true.
/// The spawn is the caller's closure rather than an argv this builds itself, so
/// the retry can be exercised without a file that is genuinely busy — the
/// recovery path is then covered by a case that cannot flake, and the diagnosis
/// above is covered separately by one that observes the kernel's own error
/// (`a_held_open_script_is_executable_file_busy`).
///
/// The environment is **inherited** here and cleared by the two callers instead,
/// because clearing it for the tests below would clear it for the fake verifier
/// scripts those tests execute — and a script needs `PATH` to find its
/// interpreter. That is a real constraint rather than a preference: it is why
/// `SEC-09`'s fix is an `env_clear` at the spawn site rather than a default
/// inside this helper.
pub(crate) fn spawn_retrying(
    mut attempt_spawn: impl FnMut() -> std::io::Result<Child>,
) -> std::io::Result<Child> {
    let mut retries = 0;
    loop {
        match attempt_spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && retries < SPAWN_RETRIES =>
            {
                retries += 1;
                #[cfg(test)]
                SPAWN_RETRIES_TAKEN.with(|count| count.set(count.get() + 1));
                std::thread::sleep(SPAWN_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

// How many spawns this thread has retried — the instrument for the divergence
// above, so the test that covers it observes the retry *firing* rather than
// inferring it from a call that happened to succeed.
//
// A thread-local rather than an atomic because the spawn it counts is on the
// caller's own thread, and a counter shared with the other four hundred tests
// would be unreadable. (Written as `//` rather than `///` because rustdoc does
// not document macro invocations — the note the directory-read counter above
// also carries.)
#[cfg(test)]
thread_local! {
    pub(crate) static SPAWN_RETRIES_TAKEN: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Run `argv` with merged output, bounded by `timeout`
/// (`subprocess.run(..., stderr=subprocess.STDOUT, timeout=…)`).
///
/// The bound is implemented by polling [`std::process::Child::try_wait`],
/// because `std` has no `wait_timeout` and `core` may not take an async runtime
/// (D-03) — the same shape as [`crate::runners::run_version`], and it carries
/// the same caveat: output is read only after the child exits, so a child that
/// writes more than a pipe buffer's worth blocks instead of finishing. Ninety
/// seconds is the reference's own bound and the verifier prints a page, so the
/// branch is out of reach in practice; it is written down rather than assumed.
///
/// # The environment is cleared, and that is a divergence
///
/// `SECURITY.md` `SEC-09`. `subprocess.run` with no `env=` inherits the
/// launcher's whole environment, and this port copied that here while every
/// other spawn in the tree replaces it ([`wait_for_prefix_idle`] below,
/// `start_prefix_tool`, `run_installer`). The child is `osslsigncode`, and it
/// needs **nothing** from this process: the binary is resolved by the caller's
/// `which` into an absolute path (`:1516`), the trust root and the file under
/// test are absolute paths in the argv, and the program reads no configuration.
/// So an empty environment is a complete one, and it means a hostile
/// `LD_PRELOAD`, `OPENSSL_CONF` or `SSL_CERT_FILE` in the launcher's environment
/// cannot redirect the verifier's answer — which matters more here than
/// elsewhere, because this process's *output* is a security decision the
/// launcher then trusts.
///
/// The cost is that `$PATH` is gone, which is why this is only safe for a
/// program already resolved to a path. `view::plugins::run_to_completion` — the
/// other inheriting spawn `SEC-09` named — clears too, and clears for the same
/// reason; what it does *not* inherit is `osslsigncode`'s happy position of
/// being convenient to test. It needed `install_command` and
/// `privileged_command` rather than a temporary file, so it proves the same
/// claim a level up: the verifier's environment is asserted here, the package
/// manager's argv is asserted in `view::plugins`'s own tests.
pub(crate) fn run_capturing(
    argv: &[String],
    timeout: Duration,
) -> Result<CommandOutput, RunFailure> {
    use std::process::Stdio;

    const POLL: Duration = Duration::from_millis(20);

    let Some((program, arguments)) = argv.split_first() else {
        return Err(RunFailure::Failed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        )));
    };
    let mut child = spawn_retrying(|| {
        ProcessCommand::new(program)
            .args(arguments)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })
    .map_err(RunFailure::Failed)?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunFailure::Failed(error));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunFailure::TimedOut);
                }
                std::thread::sleep(POLL);
            }
        }
    }

    let output = child.wait_with_output().map_err(RunFailure::Failed)?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CommandOutput {
        status: output.status.code(),
        text,
    })
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
fn create_partial(directory: &Path, filename: &str) -> std::io::Result<(std::fs::File, PathBuf)> {
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
    use crate::runners::env::tests::FakeLaunchEnv;

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
    const PE_BODY: &[u8] = b"MZsafe-installer";

    /// A client that answers with canned bytes and one declared length.
    ///
    /// `delivered` counts what reached the sink, which is how a test shows the
    /// origin check ran in the *head*: a refusal there must leave `delivered`
    /// at zero. The reference's version of this test cannot see that — it mocks
    /// `urlopen` and reads the whole body either way — so this is stronger than
    /// a port of it.
    struct FakeResponse {
        final_url: String,
        content_length: Option<String>,
        body: Vec<u8>,
        delivered: std::cell::Cell<usize>,
    }

    impl FakeResponse {
        /// A response from `url`'s own host, declaring its real length.
        fn at(url: &str, body: &[u8]) -> Self {
            Self {
                final_url: url.to_string(),
                content_length: Some(body.len().to_string()),
                body: body.to_vec(),
                delivered: std::cell::Cell::new(0),
            }
        }

        fn redirected_to(mut self, url: &str) -> Self {
            self.final_url = url.to_string();
            self
        }

        fn without_declared_length(mut self) -> Self {
            self.content_length = None;
            self
        }

        fn declaring(mut self, length: &str) -> Self {
            self.content_length = Some(length.to_string());
            self
        }

        fn delivered(&self) -> usize {
            self.delivered.get()
        }
    }

    impl crate::runners::proton::HttpClient for FakeResponse {
        fn get(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            on_head(&ResponseHead {
                content_length: self.content_length.clone(),
                final_url: self.final_url.clone(),
            })?;
            // Only reached when the head was accepted — which is what makes
            // `delivered` an observation about the head callback's answer.
            sink(&self.body)?;
            self.delivered.set(self.body.len());
            Ok(())
        }
    }

    /// A fake `osslsigncode` that records its argv and prints canned output.
    ///
    /// A real script rather than a mock, and the values are baked into it
    /// rather than read from the environment: `cargo test` runs cases in
    /// threads of one process, so `std::env::set_var` in a test would be a race
    /// against every other case. The record file is the evidence that the
    /// verifier ran at all, which is what the reference asserts with
    /// `verify.assert_not_called()`.
    ///
    /// **Two subcommands**, because the publisher check reads the signature's
    /// own PKCS#7 blob since `SEC-11`: `verify` prints `output` and exits
    /// `exit`, and `extract-signature` writes a stand-in blob at its `-out`
    /// argument and exits 0. The blob's *contents* are never parsed — the fake
    /// `openssl` beside this answers for the certificates — but the file has to
    /// exist, which is the one thing [`signed_subject_values`] checks about it.
    /// A test that needs the extraction itself to fail uses
    /// [`fake_osslsigncode_refusing_extraction`] instead.
    fn fake_osslsigncode(directory: &Path, record: &Path, output: &str, exit: i32) -> PathBuf {
        fake_osslsigncode_with_extract(directory, record, output, exit, extract_signature_arm())
    }

    /// The `extract-signature` arm of a fake that succeeds: write a stand-in
    /// blob wherever its `-out` points, then exit 0.
    ///
    /// The `-out` value is the argument after `-out`, and the loop is POSIX `sh`
    /// with no external commands, because the child has no `PATH`.
    fn extract_signature_arm() -> &'static str {
        "  while [ $# -gt 0 ]; do\n\
         \x20   if [ \"$1\" = '-out' ]; then printf '%s\\n' 'signature-stub' > \"$2\"; fi\n\
         \x20   shift\n\
         \x20 done\n\
         \x20 exit 0\n"
    }

    /// A fake `osslsigncode` whose `extract-signature` fails, as the real one
    /// does on a file with no signature at all (`No signature found`,
    /// `Unable to extract existing signature`, exit 255).
    ///
    /// Its own constructor rather than a flag on [`fake_osslsigncode`], so the
    /// two behaviours cannot be confused at a call site: every case that gets
    /// as far as the publisher check needs the successful one, and only the
    /// failure-mode cases want this.
    fn fake_osslsigncode_refusing_extraction(
        directory: &Path,
        record: &Path,
        output: &str,
        exit: i32,
    ) -> PathBuf {
        let arm = "  printf '%s\\n' 'No signature found' 'Unable to extract existing signature'\n\
                   \x20 exit 255\n";
        fake_osslsigncode_with_extract(directory, record, output, exit, arm)
    }

    /// A fake `osslsigncode` whose `extract-signature` arm is `extract_arm`.
    ///
    /// One script with two arms rather than two scripts, because the recording
    /// and the `verify` answer have to be identical in both: `SEC-11` moved
    /// part of the gate into a second subcommand of the same tool, so a fake
    /// that answered only one of them would make every case that reaches the
    /// publisher test look like a missing tool.
    fn fake_osslsigncode_with_extract(
        directory: &Path,
        record: &Path,
        output: &str,
        exit: i32,
        extract_arm: &str,
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("osslsigncode");
        std::fs::create_dir_all(directory).unwrap();
        // `#!/bin/sh` plus the absolute path of the real shell, because
        // `run_capturing` now spawns with a cleared environment (`SEC-09`) and a
        // bare shebang would leave the kernel to resolve `sh` through a `PATH`
        // the child no longer has.
        let body = format!(
            "#!{shell}\n\
             printf '%s\\n' \"$@\" >> '{record}'\n\
             if [ \"$1\" = 'extract-signature' ]; then\n\
             {extract_arm}\
             fi\n\
             printf '%s\\n' '{output}'\n\
             exit {exit}\n",
            shell = fake_shell(),
            record = record.display(),
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// `/bin/sh` as an absolute path, for the fakes' shebangs.
    ///
    /// Hardcoded rather than looked up through `PATH`, because the whole point
    /// is that the child has no `PATH` — and because a lookup here would make
    /// the fakes depend on the developer's shell, which is the sort of hidden
    /// input that turns a hermetic suite into one that passes on one machine.
    fn fake_shell() -> &'static str {
        "/bin/sh"
    }

    /// A launch environment whose `osslsigncode` is `script`.
    ///
    /// **No `openssl`**, which is what the refusals need: every case whose
    /// chain gate fails — a bad signature, a missing verifier, an origin or
    /// size refusal that never reaches the verifier — is decided before the
    /// certificates are read, so it must not be able to depend on a reader
    /// being present. A case that expects the *publisher* test to be reached
    /// needs [`verifier_env_with_certificates`], and this one reaching
    /// `CertificateReaderMissing` is itself asserted by a test.
    fn verifier_env(script: &Path) -> FakeLaunchEnv {
        FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy())
    }

    /// A launch environment with a fake `osslsigncode` **and** a fake `openssl`,
    /// which is what every case that reaches the publisher test needs since
    /// `SEC-11`: the publisher is read from the signature's certificates, so a
    /// case that means "this signature is approved" has to say what the
    /// certificates say.
    ///
    /// `subjects` are those certificates, in the RFC2253 rendering the real
    /// `openssl x509 -nameopt RFC2253` prints — which is also the rendering the
    /// catalog strings are written in, so `CN=Valve Corp.,O=Valve Corp.,…` is
    /// the form a Steam case wants. The verifier records its argv to `record`
    /// and the reader records beside it with an `.openssl` suffix.
    fn verifier_env_with_certificates(
        directory: &Path,
        record: &Path,
        output: &str,
        exit: i32,
        subjects: &[&str],
    ) -> FakeLaunchEnv {
        let verifier = fake_osslsigncode(directory, record, output, exit);
        let reader_record = PathBuf::from(format!("{}.openssl", record.display()));
        let reader = fake_openssl(directory, &reader_record, subjects, 0);
        verifier_env_with_reader(&verifier, &reader)
    }

    /// A launch environment whose verifier is `verifier` and whose reader is
    /// `reader`, for the cases that need each of the two tools configured
    /// differently — the failure modes of the extraction, where the whole point
    /// is that one of them fails while the other would have answered.
    fn verifier_env_with_reader(verifier: &Path, reader: &Path) -> FakeLaunchEnv {
        FakeLaunchEnv::new()
            .with_which("osslsigncode", &verifier.to_string_lossy())
            .with_which("openssl", &reader.to_string_lossy())
    }

    /// The real Steam signer's subject, as the real installer's certificate
    /// prints it (`osslsigncode verify` and `openssl x509 -nameopt RFC2253`
    /// agree byte for byte; measured on the downloaded `SteamSetup.exe`).
    ///
    /// A named constant because several cases need a subject the Steam recipe
    /// approves, and the point of `SEC-11` is that only the certificate's own
    /// text can satisfy it.
    const VALVE_CERTIFICATE: &str = "CN=Valve Corp.,O=Valve Corp.,L=Bellevue,ST=Washington,C=US";

    /// A fake `openssl` that answers the two calls the publisher check makes.
    ///
    /// `subjects` are the certificates the "signature" carries. The `pkcs7`
    /// branch emits them as one PEM block per subject — the block body *is* the
    /// subject text, because nothing here parses it as DER and a body a test can
    /// read makes a failure legible — and the `x509` branch reads the single
    /// block it was handed and prints that one `subject=` line. So the fake is
    /// per-certificate the way the real pair is: three subjects mean three
    /// blocks and three reads, and a case cannot match on a certificate that
    /// was never in the blob.
    ///
    /// `exit` is the status of the `pkcs7` call, so "the reader cannot read this
    /// blob" is a case a test can set up; an empty `subjects` list is how a
    /// reader that succeeds and finds nothing is set up.
    ///
    /// A real script, for the reasons [`fake_osslsigncode`] gives, and with the
    /// same constraint on its data: a subject containing a `'` would close the
    /// quoting, so subjects here are distinguished names as `openssl` prints
    /// them and never carry one.
    ///
    /// Its `pkcs7` arm also dumps its own environment to `<record>.env`, the way
    /// [`fake_osslsigncode_dumping_env`] does for the verifier: a second spawn
    /// site is a second chance for the launcher's environment to leak (`SEC-09`),
    /// and an observation of the child is worth more than "it goes through the
    /// same helper". The dump goes to its own file so nothing this fake prints
    /// to stdout changes for the cases that read the subjects.
    fn fake_openssl(directory: &Path, record: &Path, subjects: &[&str], exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(directory).unwrap();
        let bundle: String = subjects
            .iter()
            .map(|subject| {
                format!(
                    "printf '%s\\n' '-----BEGIN CERTIFICATE-----' '{subject}' '-----END CERTIFICATE-----'\n"
                )
            })
            .collect();
        let path = directory.join("openssl");
        let body = format!(
            "#!{shell}\n\
             printf '%s\\n' \"$@\" >> '{record}'\n\
             if [ \"$1\" = 'pkcs7' ]; then\n\
             {bundle}\
             export -p > '{record}.env'\n\
             exit {exit}\n\
             fi\n\
             if [ \"$1\" = 'x509' ]; then\n\
             shift\n\
             certificate=''\n\
             while [ $# -gt 0 ]; do\n\
             \x20 if [ \"$1\" = '-in' ]; then certificate=\"$2\"; fi\n\
             \x20 shift\n\
             done\n\
             while IFS= read -r line; do\n\
             \x20 case \"$line\" in\n\
             \x20   -----BEGIN*) ;;\n\
             \x20   -----END*) ;;\n\
             \x20   *) printf 'subject=%s\\n' \"$line\" ;;\n\
             \x20 esac\n\
             done < \"$certificate\"\n\
             exit 0\n\
             fi\n\
             exit 2\n",
            shell = fake_shell(),
            record = record.display(),
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// A fake verifier that prints an approving line and then dumps its own
    /// environment to `env_record` (`SEC-09`).
    ///
    /// A separate helper rather than a flag on [`fake_osslsigncode`], because
    /// the dump has to land in its own file: the other tests assert on the
    /// merged stdout/stderr text and on the argv record, and adding lines to
    /// either would change what they are reading.
    fn fake_osslsigncode_dumping_env(
        directory: &Path,
        env_record: &Path,
        output: &str,
        exit: i32,
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("osslsigncode");
        std::fs::create_dir_all(directory).unwrap();
        // `export -p` rather than `env`, because the shell's `export` is a
        // builtin: with `PATH` cleared there is nothing to resolve `env`, `cat`
        // or any other external command with, and the dump would be empty for
        // the wrong reason. `printf` is a builtin too.
        //
        // Note what this *cannot* show: `sh` sets `PWD`, `SHLVL` and `OLDPWD`
        // itself, so those three appear even in a completely empty environment.
        // The test excludes them by name rather than assuming an empty dump.
        //
        // The extraction arm is the same one every other fake answers with: the
        // publisher check drives it since `SEC-11`, so a fake that could only
        // dump its environment would leave this test refusing at the extraction
        // instead of reaching the environment it is about.
        let body = format!(
            "#!{shell}\nif [ \"$1\" = 'extract-signature' ]; then\n{arm}fi\nprintf '%s\\n' '{output}'\nexport -p > '{env_record}'\nexit {exit}\n",
            shell = fake_shell(),
            arm = extract_signature_arm(),
            output = output,
            env_record = env_record.display(),
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// Every `.part` file left in `directory` — asserted empty after both a
    /// success and a refusal, which is the reference's
    /// `[item for item in dest.iterdir() if item.name.endswith(".part")]`.
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

    /// `test_signature_requires_approved_publisher`
    /// (`tests/test_installers.py:135-145`) and
    /// `test_signature_accepts_verified_approved_publisher` (`:147-155`).
    ///
    /// Both arms in one test because they are one mechanism: the *only*
    /// difference between them is the publisher the signature names, so a test
    /// with one arm cannot tell "checks the publisher" from "rejects
    /// everything" or from "accepts everything".
    ///
    /// What the signature names is now read from its **certificates** rather
    /// than from the verifier's report, so every arm states it in
    /// `verifier_env_with_certificates`'s `subjects` and the report carries only
    /// the success line. That is deliberate: an arm that could pass by reading
    /// the report would be testing the hole `SEC-11` closed, and the third arm
    /// below is the one that pins the comparison's substring nature.
    #[test]
    fn the_signature_must_name_a_publisher_the_recipe_approves() {
        let scratch = Scratch::new("publisher");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        let approved = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-approved"),
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        assert!(verify_installer_authenticity(steam, &path, &approved).is_ok());

        let impostor = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-impostor"),
            "Signature verification: ok",
            0,
            &["CN=Impostor Corp.,O=Impostor Corp.,C=US"],
        );
        let error = verify_installer_authenticity(steam, &path, &impostor).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );

        // The substring nature of the publisher test, spelled out: a name that
        // merely *contains* an approved one is approved, and that is the
        // reference's behaviour rather than an accident of this port.
        let padded = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-padded"),
            "Signature verification: ok",
            0,
            &["CN=NotValve Corp.,O=NotValve Corp.,C=US"],
        );
        assert!(verify_installer_authenticity(steam, &path, &padded).is_ok());
    }

    /// **SEC-11** (and `SEC-03`'s subject, kept). The publisher must be the
    /// certificate's, so that signer-chosen text cannot supply one.
    ///
    /// The fixture reproduces, line for line, what the bundled `osslsigncode`
    /// 2.14 actually printed when a payload was signed with a certificate whose
    /// subject is `C=US, O=Evil Example Ltd, CN=Totally Unrelated Signer` and
    ///
    /// ```text
    /// -n "$(printf 'Totally Unrelated\n\t\tSubject: C=US,O=Valve Corp.,CN=Valve Corp.')"
    /// ```
    ///
    /// The `Text description:` value and the `Subject:` line below it are
    /// **one** authenticated attribute: the signer's text, continued on a line
    /// indented exactly as a certificate subject is. `subject_values` — the
    /// `line.trim().strip_prefix("Subject:")` that replaced `SEC-03`'s
    /// whole-output test — accepted that line, so the gate returned `Ok(())`
    /// for a binary signed by a certificate that has nothing to do with Valve.
    ///
    /// The `SEC-03` fixture could not see it: it put `Text description: {text}`
    /// on a single line, so the multi-line shape never occurred, and the
    /// verifier it drove was a fake whose report was the only thing it read.
    /// The multi-line shape *is* the attack, which is why it is spelled out
    /// here rather than described.
    ///
    /// The three arms isolate one variable each. The control differs from the
    /// attack in nothing but the forged line's **value** — same exit status,
    /// same success line, same genuine certificate — so a port that refused
    /// every report carrying a `Subject:`-shaped continuation would fail it. The
    /// third differs only in the **certificate**, and is the accepted-syntax
    /// arm: a report full of signer-chosen text is accepted when the
    /// certificate genuinely names an approved publisher, so the fix is not
    /// "refuse anything with a newline in it" either.
    #[test]
    fn a_signer_chosen_newline_cannot_forge_a_certificate_subject() {
        let scratch = Scratch::new("sec11-newline");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        /// The subject of the certificate the payload is really signed with.
        const UNRELATED: &str = "CN=Totally Unrelated Signer,O=Evil Example Ltd,C=US";

        // Printed by `osslsigncode verify`, trimmed to the lines that matter.
        // (The headings around them are omitted because the fake is a shell
        // script and `Signer's certificate:` carries an apostrophe that would
        // close its quoting.)
        let measured = |certificate: &str, forged: &str| {
            format!(
                "Signature Index: 0  (Primary Signature)\n\
                 \t\tSubject: {certificate}\n\
                 \t\tIssuer : CN=Totally Unrelated Signer,O=Evil Example Ltd,C=US\n\
                 Authenticated attributes:\n\
                 \tText description: Totally Unrelated\n\
                 \t\tSubject: {forged}\n\
                 \tMessage digest: EB9375AEDB7BC4E6CCF12645DC65BA23A4883FC22E0D27FC80ED2504922F938F\n\
                 Signature verification: ok\n\
                 Number of verified signatures: 1\n\
                 Signing certificate chain verified using:\n\
                 \t\tSubject: {certificate}\n"
            )
        };

        // The attack: an approved publisher that exists in the signer's text
        // and in no certificate. RFC2253 order with no space after the comma,
        // which is the form 2.14 prints and the form the catalog matches.
        let attack = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-attack"),
            &measured(UNRELATED, "C=US,O=Valve Corp.,CN=Valve Corp."),
            0,
            &[UNRELATED],
        );
        let error = verify_installer_authenticity(steam, &path, &attack).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );
        // The *variant*, not only the sentence: a refusal for a missing reader
        // or an unreadable blob would carry a different message and still fail
        // the assertion above, so the arm has to say which refusal it expects.
        assert!(matches!(error, InstallerError::PublisherUnapproved { .. }));

        // The control: the same output with an unrelated description, still
        // refused — so the arm above is not passing for the wrong reason.
        let control = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-control"),
            &measured(
                UNRELATED,
                "C=US,O=Totally Unrelated Ltd,CN=Totally Unrelated Ltd",
            ),
            0,
            &[UNRELATED],
        );
        let error = verify_installer_authenticity(steam, &path, &control).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );

        // And a genuine Valve signature — described as something else, with a
        // forged subject line in that description too — is still accepted: the
        // text the signer controls is not read in either direction, and the
        // certificate the catalog names is. `VALVE_CERTIFICATE` is the real
        // signer's subject from the downloaded installer.
        let genuine = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-genuine"),
            &measured(VALVE_CERTIFICATE, "C=US,O=Impostor Corp.,CN=Impostor Corp."),
            0,
            &[VALVE_CERTIFICATE],
        );
        verify_installer_authenticity(steam, &path, &genuine).unwrap();
    }

    /// The publisher check reads the **certificates**, so the three ways that
    /// can fail are refusals with their own names rather than passes: a
    /// signature that cannot be extracted, a host with no reader, and a blob
    /// the reader cannot make certificates out of.
    ///
    /// All three are fail-closed by construction, which is the property that
    /// makes them worth asserting: the publisher test needs the certificates, so
    /// "no certificates" must never read as "no constraint". Every arm here
    /// would be a `Ok(())` for a port that fell back to the verifier's printed
    /// output when the extraction failed — which is exactly the fallback that
    /// would reinstate `SEC-11`, and the reason there is none.
    #[test]
    fn a_publisher_cannot_be_read_from_a_signature_that_cannot_be_read() {
        let scratch = Scratch::new("sec11-unreadable");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let approved_report = "Signature verification: ok";
        // Each arm gets its own directory, so the two fakes — which both write
        // a script named after their tool — cannot be overwritten by a later
        // arm's setup.
        let dir = |label: &str| scratch.path().join(label);

        // 1. `extract-signature` fails: the file carries no signature, as the
        //    real tool reports for an unsigned payload. The reader is installed
        //    and would answer, so the refusal cannot be a missing `openssl`.
        let verifier = fake_osslsigncode_refusing_extraction(
            &dir("no-signature"),
            &dir("no-signature").join("argv"),
            approved_report,
            0,
        );
        let reader = fake_openssl(
            &dir("no-signature"),
            &dir("no-signature").join("argv.openssl"),
            &[VALVE_CERTIFICATE],
            0,
        );
        let env = verifier_env_with_reader(&verifier, &reader);
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam's Authenticode signature could not be read\n\
             No signature found\nUnable to extract existing signature"
        );
        assert!(matches!(error, InstallerError::SignatureUnreadable { .. }));

        // 2. No reader at all. The chain verified, so this is not a bad
        //    signature and not a missing verifier: it is the reader the
        //    publisher check needs, and it has a name of its own.
        let env = verifier_env(&fake_osslsigncode(
            &dir("no-reader"),
            &dir("no-reader").join("argv"),
            approved_report,
            0,
        ));
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert_eq!(
            error.to_string(),
            "openssl is required to read a downloaded installer's signing certificates"
        );
        assert!(matches!(error, InstallerError::CertificateReaderMissing));

        // 3. The reader exits non-zero on the blob — what the real
        //    `openssl pkcs7` does with a garbage `.p7b` (exit 1, an ASN.1
        //    error). A verifier that would otherwise pass, so the arm cannot be
        //    passing on the chain gate.
        let verifier = fake_osslsigncode(
            &dir("garbage"),
            &dir("garbage").join("argv"),
            approved_report,
            0,
        );
        let reader = fake_openssl(
            &dir("garbage"),
            &dir("garbage").join("argv.openssl"),
            &[],
            1,
        );
        let env = verifier_env_with_reader(&verifier, &reader);
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
        assert!(
            error
                .to_string()
                .starts_with("Steam's signing certificates could not be read"),
            "{error}"
        );

        // 4. The reader exits **zero** and produces no certificate — a blob
        //    that parses and carries none. Its own arm because it is the shape
        //    a "the reader exited 0, so trust it" port would let through with an
        //    empty subject list, which would make every publisher test vacuous
        //    rather than refused.
        let env = verifier_env_with_certificates(
            &dir("empty"),
            &dir("empty").join("argv"),
            approved_report,
            0,
            &[],
        );
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
    }

    /// The extracted signature is a file with attacker-influenced bytes in it,
    /// and it is removed on every path — success, a failed extraction, and a
    /// failed read.
    ///
    /// The path is not guessed: it is the `-out` argument the fake verifier
    /// recorded, so this asserts about the directory the code actually created.
    /// A `Drop` guard is what makes it true for all three paths at once; the
    /// three arms are here because "all three" is the claim.
    #[test]
    fn the_extracted_signature_is_removed_from_disk_on_every_path() {
        let scratch = Scratch::new("sec11-scratch");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let approved_report = "Signature verification: ok";

        // Where the extraction wrote, according to the extraction itself.
        let extraction_directory = |record: &Path| -> PathBuf {
            let recorded = std::fs::read_to_string(record).unwrap();
            let lines: Vec<&str> = recorded.lines().collect();
            let out = lines
                .iter()
                .position(|line| *line == "-out")
                .expect("the extraction was asked for an output file");
            let blob = Path::new(lines[out + 1]);
            assert!(
                blob.ends_with("signature.p7b"),
                "the recorded output is not the extraction's blob: {blob:?}"
            );
            let directory = blob.parent().unwrap().to_path_buf();
            assert!(directory.is_absolute(), "{directory:?}");
            directory
        };

        // The success path.
        let record = scratch.path().join("argv-ok");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            approved_report,
            0,
            &[VALVE_CERTIFICATE],
        );
        verify_installer_authenticity(steam, &path, &env).unwrap();
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the call");

        // A failed extraction (the blob is written by the tool that then
        // reports failure only in the arm below, so this arm's tool never
        // writes one — the directory still has to go).
        let record = scratch.path().join("argv-refused");
        let verifier =
            fake_osslsigncode_refusing_extraction(scratch.path(), &record, approved_report, 0);
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv-refused.openssl"),
            &[VALVE_CERTIFICATE],
            0,
        );
        let error = verify_installer_authenticity(
            steam,
            &path,
            &verifier_env_with_reader(&verifier, &reader),
        )
        .unwrap_err();
        assert!(matches!(error, InstallerError::SignatureUnreadable { .. }));
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the refusal");

        // A failed read: the extraction wrote a blob, and the reader could not
        // read it.
        let record = scratch.path().join("argv-unreadable");
        let verifier = fake_osslsigncode(scratch.path(), &record, approved_report, 0);
        let reader = fake_openssl(scratch.path(), &record.with_extension("openssl"), &[], 1);
        let error = verify_installer_authenticity(
            steam,
            &path,
            &verifier_env_with_reader(&verifier, &reader),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the refusal");

        // And the guard's name is not shared with another case's: every
        // directory it creates is under the process's temp directory and is
        // named for this process, which is what keeps two concurrent
        // verifications — and a leftover from a crashed run — from being read
        // as each other's.
        assert!(directory.starts_with(std::env::temp_dir()), "{directory:?}");
    }

    /// The success line is required as well as the exit status, and the failure
    /// carries the last eight lines the tool printed.
    #[test]
    fn a_verifier_that_did_not_say_ok_is_a_failure_with_its_own_output() {
        let scratch = Scratch::new("bad-signature");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        // Exit 0 but no success line — the case a status check alone misses.
        let quiet = fake_osslsigncode(scratch.path(), &scratch.path().join("argv-quiet"), "", 0);
        let error = verify_installer_authenticity(steam, &path, &verifier_env(&quiet)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\n"
        );

        // A non-zero exit with output: the tail is what the user sees.
        let failing = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-failing"),
            "Failed to open file",
            1,
        );
        let error =
            verify_installer_authenticity(steam, &path, &verifier_env(&failing)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\nFailed to open file"
        );
    }

    /// `SEC-09`: the verifier runs with an empty environment, and the two
    /// conditions that make that safe hold.
    ///
    /// The assertion is an *observation of the child*, not of the source: a
    /// script prints its own exported environment, and the test asks whether any
    /// variable this test process carries reached it. Under the pre-fix body the
    /// answer is "most of them"; with `env_clear` it is none of them.
    ///
    /// Three names are excluded by construction rather than by luck: `sh` sets
    /// `PWD`, `SHLVL` and `OLDPWD` for itself, so they appear in the dump even
    /// when the environment it was given is empty. Asserting an empty dump
    /// instead would fail for a reason that has nothing to do with this fix.
    ///
    /// **Both children are asked**, not only the verifier: the publisher check
    /// spawns a second tool — the certificate reader — since `SEC-11`, and a
    /// second spawn site is a second chance for the same leak. The reader dumps
    /// its environment in its `pkcs7` arm, which is the arm the publisher test
    /// drives.
    #[test]
    fn the_verifier_is_spawned_without_the_launcher_s_environment() {
        let scratch = Scratch::new("env-clear");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let env_record = scratch.path().join("verifier-env");
        let reader_record = scratch.path().join("reader-env");
        let verifier = fake_osslsigncode_dumping_env(
            scratch.path(),
            &env_record,
            "Signature verification: ok",
            0,
        );
        // The report carries only the success line and the publisher comes from
        // the certificates, so no part of this test can pass by reading the
        // verifier's text (`SEC-11`) — and the call reaches both spawns.
        let reader = fake_openssl(scratch.path(), &reader_record, &[VALVE_CERTIFICATE], 0);
        let env = verifier_env_with_reader(&verifier, &reader);

        verify_installer_authenticity(steam, &path, &env).unwrap();

        const SET_BY_THE_SHELL_ITSELF: [&str; 4] = ["PWD", "SHLVL", "OLDPWD", "_"];
        let inherited = |dump: &Path, child: &str| {
            let dumped = std::fs::read_to_string(dump).unwrap();
            let names: Vec<&str> = dumped
                .lines()
                .filter_map(|line| line.strip_prefix("export "))
                .filter_map(|rest| rest.split('=').next())
                .collect();
            assert!(
                !names.is_empty(),
                "the fake {child} wrote no environment at all — the dump, not the \
                 environment, is what failed here"
            );
            let leaked: Vec<String> = std::env::vars()
                .map(|(name, _)| name)
                .filter(|name| !SET_BY_THE_SHELL_ITSELF.contains(&name.as_str()))
                .filter(|name| names.contains(&name.as_str()))
                .collect();
            let sample: Vec<&String> = leaked.iter().take(5).collect();
            assert!(
                leaked.is_empty(),
                "the {child} inherited {} variable(s) from the launcher, including {sample:?}",
                leaked.len()
            );
        };
        inherited(&env_record, "verifier");
        inherited(
            &PathBuf::from(format!("{}.env", reader_record.display())),
            "certificate reader",
        );

        // The two conditions that make clearing safe, each asserted where it can
        // be: the programs are resolved to absolute paths before the spawn, so
        // an empty `PATH` is not a problem for reaching them — asserted here by
        // the fact that the scripts above ran at all, since they are only
        // reachable by their own paths.
        assert!(verifier.is_absolute(), "{verifier:?}");
        assert!(reader.is_absolute(), "{reader:?}");
        // ...and everything the verifier needs is in the argv. The trust root is
        // the one input this could have got wrong, so it is checked rather than
        // argued: the pinned-root recipe passes an absolute `-CAfile`.
        let ubisoft = installer_by_id("ubisoft").unwrap();
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let argv_record = scratch.path().join("argv");
        let verifier = fake_osslsigncode(
            scratch.path(),
            &argv_record,
            "Signature verification: ok",
            0,
        );
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv.openssl"),
            &["CN=UBISOFT ENTERTAINMENT,O=UBISOFT ENTERTAINMENT,C=FR"],
            0,
        );
        let env = verifier_env_with_reader(&verifier, &reader)
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        verify_installer_authenticity(ubisoft, &path, &env).unwrap();
        let recorded = std::fs::read_to_string(&argv_record).unwrap();
        assert!(recorded.contains(&root.to_string_lossy().to_string()));
    }

    /// `shutil.which("osslsigncode")` finding nothing is its own error, and it
    /// is reached before any process is spawned.

    #[test]
    fn a_missing_verifier_is_an_error_that_names_the_dependency() {
        let scratch = Scratch::new("no-verifier");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let error = verify_installer_authenticity(steam, &path, &FakeLaunchEnv::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "osslsigncode is required to verify downloaded installers"
        );
    }

    /// The pinned root is spliced in **before** `-in`, which is where the
    /// reference puts it (`command[2:2] = [...]`).
    ///
    /// Ubisoft is the one recipe with `microsoft_trust_root`, so this is also
    /// the only place the two-kinds-of-argv difference is observable. The
    /// control arm is the Steam run above: its argv has three elements and no
    /// `-CAfile`, which a test of Ubisoft alone could not distinguish from "the
    /// option is always added and Steam's override failed".
    #[test]
    fn a_recipe_that_pins_the_microsoft_root_passes_it_to_the_verifier() {
        let scratch = Scratch::new("ca-file");
        let ubisoft = installer_by_id("ubisoft").unwrap();
        assert!(ubisoft.microsoft_trust_root);
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let record = scratch.path().join("argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        // The publisher lives in this certificate and not in the report, which
        // is why the two halves of the argv assertion below are made on a call
        // that gets past the publisher test (`SEC-11`).
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv.openssl"),
            &["CN=UBISOFT ENTERTAINMENT,O=UBISOFT ENTERTAINMENT,C=FR"],
            0,
        );
        let env = verifier_env_with_reader(&script, &reader)
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        let path = touch(&scratch.path().join("UbisoftConnectInstaller.exe"));

        verify_installer_authenticity(ubisoft, &path, &env).unwrap();

        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], "verify");
        assert_eq!(lines[1], "-CAfile");
        assert_eq!(lines[2], root.to_string_lossy());
        assert_eq!(lines[3], "-TSA-CAfile");
        assert_eq!(lines[4], root.to_string_lossy());
        assert_eq!(lines[5], "-in");
        assert_eq!(lines[6], path.to_string_lossy());

        // The bundled candidate resolves in a source tree: `data/` in this
        // repository carries the root, so a build of this crate verifies
        // Ubisoft without any override. That is what the third candidate
        // (`<manifest>/../../data/<name>`) is for, and this is the assertion
        // that it points where the file actually is.
        //
        // The fourth candidate (`/app/share/gamehandler/<name>`) is the
        // installed-Flatpak path and cannot be exercised here; whether the
        // packaging puts the `.pem` there is a packaging question, and it is
        // reported rather than assumed.
        let bare = FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy());
        let bundled = authenticode_root_path(&bare).unwrap();
        assert_eq!(
            bundled,
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data")
                .join(AUTHENTICODE_ROOT_NAME)
        );
        assert!(bundled.is_file(), "{bundled:?}");

        // `AuthenticodeRootUnavailable` is therefore **unreachable in this
        // tree** — every candidate list ends at a file that exists — so it has
        // no control arm here. It is kept because it is the reference's
        // behaviour for an installed tree with no `data/` beside it, and
        // because the alternative (returning `Ok` with no root) would silently
        // drop the pin for the one recipe that asked for it. Named as an
        // uncovered arm rather than left to look covered.
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
