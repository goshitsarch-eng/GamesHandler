//! File-picker URLs and network shares resolved onto real filesystem paths.
//!
//! A port of `gamehandler/netpaths.py` (186 lines). A game library does not
//! always live on the local disk: Linux file choosers happily list SMB/SFTP/
//! WebDAV shares but hand back `smb://server/share/...` style URLs, and Wine can
//! only execute a real filesystem path. GVFS solves this — once a share is
//! mounted its contents appear through a FUSE filesystem under
//! `$XDG_RUNTIME_DIR/gvfs` — and this module maps a share URL onto that path so
//! a picked executable on a network share launches exactly like a local one.
//!
//! # The parser is not here
//!
//! `url_parts` lives in [`crate::installers`] and is used by both this module
//! and the download allowlist. That is deliberate: two ports of CPython's
//! `urlsplit` would be two fidelities for one grammar, and only one of them
//! would be under the 58-vector battery in that module — the other is the copy
//! that rots. `url_parts` is `pub(crate)` for exactly this reason.
//!
//! That last fact is also why the private items in this file's docs —
//! `url_parts`, `real_uid`, `scan_gvfs_for`, `UrlParts::netloc` — are written
//! as plain code rather than as links. A public doc cannot address a
//! `pub(crate)` item, so a link would only add a `private_intra_doc_links`
//! warning to the backlog `verify.sh` already carries and deliberately does not
//! deny. [`crate::paths`] sets the same precedent for `FakeEnv`.
//!
//! # What is faithful, and the three things that are not
//!
//! Every rule below was measured against the reference on this machine rather
//! than recalled, and the measured values are in the tests at the foot of this
//! file.
//!
//! 1. **`getuid()`, not `geteuid()`** — `real_uid`. The reference calls
//!    `os.getuid()` (`netpaths.py:45`), which is the *real* uid. [`crate::plugins`]
//!    has an `effective_uid` that reads the *second* field of `/proc/self/status`
//!    for `os.geteuid()`, and the two genuinely differ under `setuid`. This is
//!    not a bug to reconcile: a real-uid path and a privilege decision ask
//!    different questions. **Do not "fix" either one to match the other.**
//! 2. **There is no filesystem seam.** The reference's own hook is
//!    `GAMEHANDLER_GVFS_ROOT` (`netpaths.py:39`), so tests point that at a real
//!    temporary directory — the same pattern as `installers`' scratch dirs —
//!    rather than swapping a trait in to make a temp dir testable.
//! 3. **[`gvfs_root_in`] returns `Option`** where `gvfs_root()` returns a `Path`.
//!    Rust's `std` cannot ask for a uid without `unsafe` (which this crate
//!    denies) or `rustix`'s `process` feature (a manifest change, and the
//!    dependency set is not this task's), so the read is done from `/proc` and
//!    can fail where Python's cannot. `None` propagates as "no mount will be
//!    found", which is the same outcome as the directory not existing, and that
//!    is the reference's own behaviour on the `OSError` path at
//!    `netpaths.py:114`.
//!
//! # One place this is stricter than the reference, on purpose
//!
//! `scan_gvfs_for` matches a directory by `to_string_lossy().to_lowercase()`
//! but **joins the real `OsString`** from the directory entry. Python keeps
//! filenames as `str` with surrogate escapes and round-trips any byte sequence;
//! a `to_string_lossy` filename would turn a non-UTF-8 byte into `U+FFFD` and
//! then go looking for a path that does not exist. Only the returned
//! [`String`] is lossy, and only because the signature is.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::installers::{UrlParts, url_parts};
use crate::paths::Env;
use crate::runners::launch_opts::{ShareResolver, python_trim};

/// Schemes GVFS exposes through its FUSE daemon (`netpaths.py:28`).
///
/// Anything else — `http`, `steam` — is not a browsable file location and is
/// left untouched.
pub const REMOTE_SCHEMES: [&str; 8] = ["smb", "sftp", "ssh", "ftp", "ftps", "dav", "davs", "nfs"];

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// `netpaths.is_remote_url` (`netpaths.py:31-34`) — whether *value* is a
/// network-share URL rather than a local path.
///
/// The reference strips the *whole input* with `str.strip()` before parsing,
/// and then asks `UrlParts::netloc` rather than "is there a
/// hostname". Those are different questions and the difference is reachable:
/// `smb:///x` has a scheme and no netloc, and `//host/x` has a netloc and no
/// scheme. Both are `false` here, for two different reasons.
pub fn is_remote_url(value: &str) -> bool {
    let parts = url_parts(python_trim(value));
    REMOTE_SCHEMES.contains(&parts.scheme.as_str()) && !parts.netloc.is_empty()
}

/// `netpaths.as_local_path` (`netpaths.py:144-167`) — a picker result or a
/// hand-typed location turned into a local path, against the real host.
pub fn as_local_path(value: &str) -> String {
    as_local_path_in(value, &crate::paths::SystemEnv)
}

/// [`as_local_path`] against an injected environment, for tests.
///
/// `file://` URLs are unwrapped, network-share URLs are mapped onto their GVFS
/// FUSE mount when one exists, and anything already a plain path is returned
/// unchanged. A share URL that cannot be resolved returns the **original**
/// value, so the caller can say something accurate about it rather than
/// reporting success on a path that does not exist.
pub fn as_local_path_in(value: &str, env: &dyn Env) -> String {
    let raw = python_trim(value);
    if raw.is_empty() {
        return String::new();
    }
    let parts = url_parts(raw);

    // `file://` is the one scheme unwrapped directly. Note `unquote(path) or
    // raw`: `file://` (the whole URL, no path at all) decodes to the empty
    // string, and the reference returns the raw value in that case rather than
    // an empty path — which is what makes `file://` an error the caller can
    // report instead of a silent "no game here".
    if parts.scheme == "file" {
        let decoded = unquote(&parts.path);
        return if decoded.is_empty() {
            raw.to_string()
        } else {
            decoded
        };
    }

    if REMOTE_SCHEMES.contains(&parts.scheme.as_str()) && !parts.netloc.is_empty() {
        for candidate in mount_candidates(&parts, env) {
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
        if let Some(found) = scan_gvfs_for(&parts, env) {
            return found.to_string_lossy().into_owned();
        }
        return raw.to_string();
    }

    raw.to_string()
}

/// `netpaths.unreachable_share_message` (`netpaths.py:170-177`) — a user-facing
/// explanation for a share URL that has no local mount.
///
/// **This one does not strip its input.** `urlsplit(str(value or ""))` at
/// `netpaths.py:173` has no `.strip()` where the two functions above do, and the
/// omission is visible: the message interpolates `value` verbatim, so a value
/// with a trailing newline keeps it in the sentence. Faithful rather than
/// tidied, because this string is matched by tests on both sides.
pub fn unreachable_share_message(value: &str) -> String {
    let parts = url_parts(value);
    let host = parts.host.unwrap_or_else(|| "the server".to_string());
    format!(
        "{value} is a network location that is not mounted yet. Open {host} \
         in your file manager once so the share is mounted, then try again."
    )
}

/// [`ShareResolver`] backed by this module, so the launch flow's seam is live.
///
/// [`crate::runners::launch_opts`] defines `ShareResolver` and
/// `resolve_game_paths` against it, and says in its own doc that `netpaths`
/// supplies the implementation when it lands. This is that implementation: the
/// three methods are the three entry points above, and nothing else is added.
///
/// It takes an injected [`Env`] rather than reading the process environment
/// directly, matching this crate's `foo_in(env)` convention, so a launch test
/// can point `GAMEHANDLER_GVFS_ROOT` at a scratch directory and assert that a
/// share-hosted title resolves.
pub struct NetpathsShares<'a> {
    env: &'a dyn Env,
}

impl<'a> NetpathsShares<'a> {
    pub fn new(env: &'a dyn Env) -> Self {
        Self { env }
    }
}

impl ShareResolver for NetpathsShares<'_> {
    fn is_remote_url(&self, value: &str) -> bool {
        is_remote_url(value)
    }

    fn as_local_path(&self, value: &str) -> String {
        as_local_path_in(value, self.env)
    }

    fn unreachable_share_message(&self, value: &str) -> String {
        unreachable_share_message(value)
    }
}

// ---------------------------------------------------------------------------
// The GVFS root
// ---------------------------------------------------------------------------

/// [`gvfs_root_in`] against the real host.
pub fn gvfs_root() -> Option<PathBuf> {
    gvfs_root_in(&crate::paths::SystemEnv)
}

/// Where GVFS mounts network shares as regular directories
/// (`netpaths.py:37-45`).
///
/// Three sources, in the reference's order: `$GAMEHANDLER_GVFS_ROOT`, then
/// `$XDG_RUNTIME_DIR/gvfs`, then `/run/user/{uid}/gvfs`. The first two are
/// tested for truthiness, not merely for presence — an empty `XDG_RUNTIME_DIR`
/// falls through to the uid form rather than becoming a relative `gvfs` — and
/// that is reproduced here.
pub fn gvfs_root_in(env: &dyn Env) -> Option<PathBuf> {
    if let Some(override_root) = env.var("GAMEHANDLER_GVFS_ROOT").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(override_root));
    }
    if let Some(runtime) = env.var("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return Some(Path::new(&runtime).join("gvfs"));
    }
    Some(PathBuf::from(format!("/run/user/{}/gvfs", real_uid()?)))
}

/// `os.getuid()` — the **real** uid, read from `/proc/self/status`.
///
/// There is no `std` way to ask for a uid and this crate denies `unsafe_code`,
/// so the syscall path is closed; `rustix` is a dependency but only with its
/// `fs` feature, and `rustix::process::getuid` sits behind `process`. Enabling
/// it is a manifest change that belongs to whoever owns the dependency set, so
/// this reads the file instead.
///
/// The `Uid:` line is `real effective saved filesystem`, so the **first** field
/// is `getuid()` and the second is `geteuid()`. [`crate::plugins`] reads the
/// second for its own reasons and that is not an inconsistency to reconcile:
/// the reference asks `os.getuid()` here (`netpaths.py:45`) and `os.geteuid()`
/// there, and the two differ in a setuid context. Changing this to match would
/// make the port wrong about the reference; changing that one to match would
/// make a privilege decision on the wrong number.
///
/// `/proc` is mounted on every kernel GameHandler runs on, including inside the
/// Flatpak sandbox; `None` is returned rather than a guessed uid if it is not,
/// and the caller's behaviour on `None` is documented on [`gvfs_root_in`].
fn real_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Mount names
// ---------------------------------------------------------------------------

/// `netpaths._smb_mount_names` (`netpaths.py:48-59`) — the directory names
/// `gvfsd-fuse` uses for an SMB share, most specific first.
///
/// Three names, in this order: with the user, without it, and — for the older
/// gvfs that lowercases the share component too — with the share lowercased.
/// The host is lowercased; the **share is not** except in that third name, and
/// SMB share names are case-insensitive in practice which is why the third
/// exists at all.
fn smb_mount_names(host: &str, share: &str, user: &str) -> Vec<String> {
    let host = host.to_lowercase();
    let mut names = Vec::new();
    if !user.is_empty() {
        names.push(format!("smb-share:server={host},share={share},user={user}"));
    }
    names.push(format!("smb-share:server={host},share={share}"));
    let lowered = share.to_lowercase();
    if lowered != share {
        names.push(format!("smb-share:server={host},share={lowered}"));
    }
    names
}

/// `netpaths._generic_mount_names` (`netpaths.py:62-80`) — the `gvfsd-fuse`
/// names for sftp/ftp/dav style mounts, most specific first.
///
/// Two renames and one flag: `ssh` is really `sftp` to gvfs, and `davs`/`ftps`
/// are `dav`/`ftp` with `,ssl=true` appended. The second name omits the user
/// and port and is only produced when there was one to omit.
///
/// **A port of zero counts as absent.** The reference's guard is `if port:`
/// (`netpaths.py:75`), and `0` is falsy in Python, so `sftp://host:0/x` builds
/// no `port=` detail and produces no second name. The parser at
/// [`crate::installers`] faithfully reports `.port` as `Some(0)` — that is what
/// CPython does — which is why the filter below exists rather than a bare
/// `is_some()`. Both halves are load-bearing.
fn generic_mount_names(scheme: &str, host: &str, user: &str, port: Option<u32>) -> Vec<String> {
    let scheme = match scheme {
        "ssh" => "sftp",
        other => other,
    };
    let (scheme, ssl) = match scheme {
        "davs" => ("dav", ",ssl=true"),
        "ftps" => ("ftp", ",ssl=true"),
        other => (other, ""),
    };
    let host = host.to_lowercase();

    let mut details = Vec::new();
    if !user.is_empty() {
        details.push(format!("user={user}"));
    }
    details.push(format!("host={host}"));
    let port = port.filter(|port| *port != 0);
    if let Some(port) = port {
        details.push(format!("port={port}"));
    }

    let mut names = vec![format!("{scheme}:{}{ssl}", details.join(","))];
    if !user.is_empty() || port.is_some() {
        names.push(format!("{scheme}:host={host}{ssl}"));
    }
    names
}

// ---------------------------------------------------------------------------
// Candidates and the scan
// ---------------------------------------------------------------------------

/// `netpaths._mount_candidates` (`netpaths.py:83-101`) — the constructed
/// `gvfsd-fuse` paths for a share URL, before any of them is checked.
///
/// Note what is unquoted and what is not: the **path segments and the username**
/// go through `unquote`, the **host does not**. The host came back from
/// `urlsplit` already lowercased but still percent-encoded, so a host written
/// `smb://my%20server/share/x` has the hostname `my%20server` in the reference
/// too, and neither side decodes it. Faithful: a mount name that does not match
/// costs a scan, not a wrong answer.
///
/// The host is `.strip()`ed here where [`scan_gvfs_for`] does not strip it, and
/// that asymmetry is the reference's (`netpaths.py:86` against `:123`).
fn mount_candidates(url: &UrlParts, env: &dyn Env) -> Vec<PathBuf> {
    let Some(root) = gvfs_root_in(env) else {
        return Vec::new();
    };
    let host = python_trim(url.host.as_deref().unwrap_or(""));
    if host.is_empty() {
        return Vec::new();
    }
    let user = unquote(url.user.as_deref().unwrap_or(""));
    let parts: Vec<String> = url
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(unquote)
        .collect();

    let mut candidates = Vec::new();
    if url.scheme == "smb" {
        // An SMB URL with no path has no share, so there is nothing to
        // construct — `netpaths.py:93`. The generic schemes have no such rule:
        // `sftp://host` is a valid mount with no path under it.
        let Some((share, rest)) = parts.split_first() else {
            return Vec::new();
        };
        for name in smb_mount_names(host, share, &user) {
            candidates.push(join_all(&root.join(&name), rest));
        }
    } else {
        for name in generic_mount_names(&url.scheme, host, &user, url.port) {
            candidates.push(join_all(&root.join(&name), &parts));
        }
    }
    candidates
}

/// The reference's `root.joinpath(name, *parts)`.
///
/// `Path::join` is not `joinpath` for every input — a component that is
/// absolute *replaces* in both, which is the one surprising case, and neither
/// side normalises `..`. Here every component comes from splitting a URL path
/// on `/`, so none is empty and none is absolute, and the two agree.
fn join_all(root: &Path, parts: &[String]) -> PathBuf {
    parts
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// `netpaths._scan_gvfs_for` (`netpaths.py:104-141`) — the fallback that reads
/// the GVFS root and looks for a mount naming the same scheme and host.
///
/// Mount-name details (user, port, domain) vary between gvfs versions, so when
/// the constructed names all miss, any mounted entry whose name contains
/// `server=<host>` or `host=<host>` is tried with the URL's path appended. For
/// `smb` the entry is rewritten when it already names the share, because the
/// share is a component of the mount name rather than of the path.
///
/// Two things are faithful and look like bugs until you check them:
///
/// * **No strip on the host.** `(url.hostname or "").lower()`
///   (`netpaths.py:123`) has no `.strip()` where `_mount_candidates` does, and
///   since `hostname` is already lowercased the `.lower()` is a no-op. Kept
///   because a host with spaces either matches a mount name that has them or
///   does not, and stripping here would make this scan disagree with the
///   construction above it.
/// * **The first match wins, in `readdir` order.** Both sides use the
///   filesystem's own order and neither sorts, so which of two mounts for one
///   host is chosen is unspecified in the reference too. Sorting here would be
///   a different behaviour, not a more correct one.
fn scan_gvfs_for(url: &UrlParts, env: &dyn Env) -> Option<PathBuf> {
    let root = gvfs_root_in(env)?;
    // `list(root.iterdir())` inside a `try` — an error anywhere in the listing
    // is `None`, not a partial scan (`netpaths.py:112-115`).
    let entries: Vec<(OsString, PathBuf)> = std::fs::read_dir(&root)
        .ok()?
        .map(|entry| entry.map(|entry| (entry.file_name(), entry.path())))
        .collect::<std::io::Result<Vec<_>>>()
        .ok()?;

    let scheme = match url.scheme.as_str() {
        "ssh" => "sftp",
        "davs" => "dav",
        "ftps" => "ftp",
        other => other,
    };
    let host = url.host.as_deref().unwrap_or("").to_lowercase();
    let parts: Vec<String> = url
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(unquote)
        .collect();

    let dash = format!("{scheme}-");
    let colon = format!("{scheme}:");
    let by_server = format!("server={host}");
    let by_host = format!("host={host}");

    for (file_name, path) in entries {
        // Matched by the lossy lowercased name, **joined by the real one** — see
        // this module's note. A non-UTF-8 directory name still has to resolve.
        let name = file_name.to_string_lossy().to_lowercase();
        if !name.starts_with(&dash) && !name.starts_with(&colon) {
            continue;
        }
        if !name.contains(&by_server) && !name.contains(&by_host) {
            continue;
        }
        let candidate = if scheme == "smb" && !parts.is_empty() {
            let share = parts[0].to_lowercase();
            if name.contains(&format!("share={share}")) {
                join_all(&path, &parts[1..])
            } else {
                join_all(&path, &parts)
            }
        } else {
            join_all(&path, &parts)
        };
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// unquote
// ---------------------------------------------------------------------------

/// `urllib.parse.unquote(value, encoding="utf-8", errors="replace")`.
///
/// Measured against CPython rather than recalled, and it is not `percent_decode`
/// from any crate:
///
/// * A `%` that is not followed by **two hex digits** is left as written, and so
///   is everything after it: `%zz` stays `%zz`, `%2` stays `%2`, and `%%41` is
///   `%A`. The escape is all-or-nothing, not best-effort per character.
/// * Escapes decode to **bytes**, and the bytes are decoded as one UTF-8
///   stream, so `%C3%A9` is `é` while `%C3%41` is `U+FFFD` followed by `A`.
///   Invalid bytes become `U+FFFD`, which is `String::from_utf8_lossy` —
///   confirmed on the multi-byte cases in the tests, including the surrogate
///   `%ED%A0%80`, where the replacement is one per byte.
/// * **Non-ASCII characters in the input are emitted verbatim**, never routed
///   through the byte decoder. This is CPython's `_generate_unquoted_parts`,
///   which splits the string into maximal ASCII runs and non-ASCII gaps and
///   only runs the decoder over the ASCII ones. The split is the reference's
///   and is reproduced rather than reasoned away: `é%20x` is `é x`, and the
///   two halves come from different branches.
fn unquote(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while !rest.is_empty() {
        // The non-ASCII gap before the next ASCII run, verbatim.
        match rest.find(|character: char| character.is_ascii()) {
            Some(0) => {}
            Some(start) => {
                out.push_str(&rest[..start]);
                rest = &rest[start..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
        // The maximal ASCII run, through the byte decoder.
        let end = rest
            .find(|character: char| !character.is_ascii())
            .unwrap_or(rest.len());
        out.push_str(&unquote_ascii_run(&rest[..end]));
        rest = &rest[end..];
    }
    out
}

/// CPython's `_unquote_impl` over one run, which the caller guarantees is ASCII.
///
/// The ASCII guarantee is what makes `split('%')` here the same operation as
/// CPython's `string.encode().split(b'%')`.
fn unquote_ascii_run(run: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(run.len());
    let mut bits = run.split('%');
    if let Some(head) = bits.next() {
        bytes.extend_from_slice(head.as_bytes());
    }
    for item in bits {
        match hex_pair(item.as_bytes()) {
            Some(byte) => {
                bytes.push(byte);
                // `item[2..]` is safe: `hex_pair` only matches on two bytes.
                bytes.extend_from_slice(&item.as_bytes()[2..]);
            }
            // CPython's `except KeyError` branch: keep the `%` and the rest.
            None => {
                bytes.push(b'%');
                bytes.extend_from_slice(item.as_bytes());
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `_hextobyte[item[:2]]` — `None` is CPython's `KeyError`.
fn hex_pair(item: &[u8]) -> Option<u8> {
    let pair = item.get(..2)?;
    Some((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::FakeEnv;
    use crate::runners::env::tests::scratch;

    /// An environment whose `GAMEHANDLER_GVFS_ROOT` is a fresh scratch dir.
    ///
    /// Returned as a pair so the caller can keep the path for building mounts
    /// and remove it at the end.
    fn gvfs(label: &str) -> (FakeEnv, PathBuf) {
        let root = scratch(label);
        let env = FakeEnv::new(&[("GAMEHANDLER_GVFS_ROOT", root.to_str().unwrap())]);
        (env, root)
    }

    /// `mkdir -p` inside the scratch root, returning the path created.
    fn mount(root: &Path, name: &str, under: &str) -> PathBuf {
        let path = root.join(name).join(under);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    // -- unquote ------------------------------------------------------------

    /// 36 vectors, every one the output of CPython's `unquote` on this
    /// machine, with the one duplicate case in the generated list dropped.
    #[test]
    fn unquote_matches_cpython() {
        let vectors: [(&str, &str); 36] = [
            ("", ""),
            ("a", "a"),
            ("a%", "a%"),
            ("%", "%"),
            ("%2", "%2"),
            ("%zz", "%zz"),
            ("%41", "A"),
            ("%7a", "z"),
            ("%7A", "z"),
            ("us%40er", "us@er"),
            ("%20", " "),
            ("a%20b", "a b"),
            ("%C3%A9", "\u{e9}"),
            ("%E9", "\u{fffd}"),
            ("%FF", "\u{fffd}"),
            ("%C3%41", "\u{fffd}A"),
            ("%C3", "\u{fffd}"),
            ("%C3%C3", "\u{fffd}\u{fffd}"),
            ("a%C3%A9b", "a\u{e9}b"),
            ("%C3%A9%C3%A9", "\u{e9}\u{e9}"),
            ("é", "\u{e9}"),
            ("é%20x", "\u{e9} x"),
            ("é%C3%A9", "\u{e9}\u{e9}"),
            ("%C3é", "\u{fffd}\u{e9}"),
            ("é%", "\u{e9}%"),
            ("%00", "\u{0}"),
            ("%2F", "/"),
            ("%2525", "%25"),
            ("%%41", "%A"),
            ("%41%42", "AB"),
            ("GAME%20(1).exe", "GAME (1).exe"),
            ("Games%2FMy%20Game", "Games/My Game"),
            ("%F0%9F%8E%AE", "\u{1f3ae}"),
            ("%ED%A0%80", "\u{fffd}\u{fffd}\u{fffd}"),
            ("x%y%z", "x%y%z"),
            ("100%", "100%"),
        ];
        for (input, expected) in vectors {
            assert_eq!(unquote(input), expected, "unquote({input:?})");
        }
    }

    /// The `%%41` and `%C3%41` rows above are the control arms for the two
    /// branches: a `%` that is not an escape is kept *and so is the text after
    /// it*, and a valid escape that starts an invalid byte sequence still
    /// consumes its own two digits. Without them a decoder that skipped a bad
    /// `%` and rescanned would pass the simple cases.
    #[test]
    fn an_invalid_escape_does_not_rescan_the_text_after_it() {
        // `%` + `z` is not hex, so the `%` is kept and the *rest of the string*
        // with it — `%z41` is not `%zA`.
        assert_eq!(unquote("%z41"), "%z41");
        assert_eq!(unquote("%2z1"), "%2z1");
        // A valid escape before an invalid one is still decoded.
        assert_eq!(unquote("%41%zz"), "A%zz");
        // And the digit after a truncated escape is not consumed by it.
        assert_eq!(unquote("%2%41"), "%2A");
    }

    // -- is_remote_url ------------------------------------------------------

    #[test]
    fn only_a_remote_scheme_with_a_netloc_is_remote() {
        for value in [
            "smb://server/share/game.exe",
            "sftp://server/pub/game.exe",
            "ssh://server/pub/game.exe",
            "ftp://server/pub/game.exe",
            "ftps://server/pub/game.exe",
            "dav://server/pub/game.exe",
            "davs://server/pub/game.exe",
            "nfs://server/export/game.exe",
            // Whitespace around it is stripped by the reference, so this is a
            // share URL rather than a local path with spaces in the name.
            "  smb://server/share/game.exe\n",
            // The scheme is lowercased before the lookup.
            "SMB://server/share/game.exe",
            // A **netloc with no host** is still remote, because the reference
            // asks `bool(parsed.netloc)` and not `hostname is not None`. Both
            // measured. `as_local_path` on these cannot resolve — there is no
            // host to build a mount name from — so they come back unchanged
            // *and* remote, which is a share the user is told to go and mount.
            "smb://:8080/x",
            "smb://:8080",
            "sftp://:22/x",
            "smb://@/x",
        ] {
            assert!(is_remote_url(value), "{value:?} is a remote URL");
        }

        for value in [
            // Remote scheme, **no netloc** — `netpaths.py:34` requires both.
            // This is the arm that makes the netloc test load-bearing rather
            // than decorative.
            "smb:///share/game.exe",
            "sftp:///pub/game.exe",
            // A netloc but a local-looking scheme.
            "file:///home/user/game.exe",
            // Not a share scheme at all.
            "https://example.com/game.exe",
            "steam://run/12345",
            // Plain paths, including one that looks like a URL.
            "/home/user/game.exe",
            "game.exe",
            "C:/games/game.exe",
            "",
            "   ",
            // A scheme with no `//` is not a netloc — the parser's rule, and
            // the reason `smb:/share/x` is not remote.
            "smb:/share/game.exe",
        ] {
            assert!(!is_remote_url(value), "{value:?} is not a remote URL");
        }
    }

    // -- gvfs_root ----------------------------------------------------------

    #[test]
    fn the_gvfs_root_follows_the_reference_order() {
        // The override wins over everything, and wins *whole* — it is not
        // joined with `gvfs`.
        let env = FakeEnv::new(&[
            ("GAMEHANDLER_GVFS_ROOT", "/tmp/override"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        assert_eq!(gvfs_root_in(&env), Some(PathBuf::from("/tmp/override")));

        // Without it, the runtime dir with `gvfs` appended.
        let env = FakeEnv::new(&[("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert_eq!(
            gvfs_root_in(&env),
            Some(PathBuf::from("/run/user/1000/gvfs"))
        );

        // **An empty value is falsy to Python**, so each of these falls
        // through rather than becoming a relative path. This is the pair that
        // makes `if override:` mean "non-empty" instead of "present".
        let env = FakeEnv::new(&[("GAMEHANDLER_GVFS_ROOT", ""), ("XDG_RUNTIME_DIR", "")]);
        let root = gvfs_root_in(&env).expect("/proc is readable on this host");
        if let Some(uid) = real_uid() {
            assert_eq!(root, PathBuf::from(format!("/run/user/{uid}/gvfs")));
        }

        // And the last resort really is the uid form.
        let env = FakeEnv::new(&[]);
        assert_eq!(
            gvfs_root_in(&env),
            real_uid().map(|uid| PathBuf::from(format!("/run/user/{uid}/gvfs")))
        );
    }

    /// The real-uid read is the **first** field of `Uid:`, where
    /// [`crate::plugins`] reads the second. `plugins` is a different module with
    /// a different question, so this asserts the position rather than comparing
    /// the two functions — under a setuid test both would agree and the
    /// comparison would prove nothing.
    #[test]
    fn real_uid_reads_the_first_uid_field_and_not_the_second() {
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        let line = status
            .lines()
            .find(|line| line.starts_with("Uid:"))
            .expect("a Uid: line");
        let fields: Vec<&str> = line["Uid:".len()..].split_whitespace().collect();
        // `real effective saved filesystem`, so there are four on Linux.
        assert_eq!(fields.len(), 4, "Uid: is {line:?}");
        assert_eq!(
            real_uid(),
            Some(fields[0].parse().unwrap()),
            "real_uid must read field 0"
        );
        // The control arm: field 1 is a *different* number to read, which is
        // what `plugins::effective_uid` does. Not asserted to differ (they are
        // equal in an ordinary session) — asserted to exist, so this test is
        // about the position and cannot pass by accident on a one-field line.
        assert!(fields[1].parse::<u32>().is_ok());
    }

    // -- mount names --------------------------------------------------------

    #[test]
    fn smb_mount_names_are_most_specific_first() {
        assert_eq!(
            smb_mount_names("server", "Share", "user"),
            vec![
                "smb-share:server=server,share=Share,user=user".to_string(),
                "smb-share:server=server,share=Share".to_string(),
                "smb-share:server=server,share=share".to_string(),
            ]
        );
        // Without a user the first name is skipped entirely rather than left
        // with an empty `user=`.
        assert_eq!(
            smb_mount_names("server", "Share", ""),
            vec![
                "smb-share:server=server,share=Share".to_string(),
                "smb-share:server=server,share=share".to_string(),
            ]
        );
        // The share is lowercased for the third name **only when it differs**,
        // so an already-lowercase share produces no duplicate.
        assert_eq!(
            smb_mount_names("server", "share", ""),
            vec!["smb-share:server=server,share=share".to_string()]
        );
        // The host is lowercased; the share keeps its case in the first two.
        assert_eq!(
            smb_mount_names("SERVER", "Share", "User"),
            vec![
                "smb-share:server=server,share=Share,user=User".to_string(),
                "smb-share:server=server,share=Share".to_string(),
                "smb-share:server=server,share=share".to_string(),
            ]
        );
    }

    #[test]
    fn generic_mount_names_rename_and_flag_the_ssl_schemes() {
        assert_eq!(
            generic_mount_names("sftp", "server", "", None),
            vec!["sftp:host=server".to_string()]
        );
        // `ssh` is `sftp` to gvfs.
        assert_eq!(
            generic_mount_names("ssh", "server", "", None),
            vec!["sftp:host=server".to_string()]
        );
        // `davs` is `dav` with `,ssl=true`, and the flag goes **after** the
        // details and after the host in the fallback name.
        assert_eq!(
            generic_mount_names("davs", "server", "user", None),
            vec![
                "dav:user=user,host=server,ssl=true".to_string(),
                "dav:host=server,ssl=true".to_string(),
            ]
        );
        assert_eq!(
            generic_mount_names("ftps", "server", "", Some(21)),
            vec![
                "ftp:host=server,port=21,ssl=true".to_string(),
                "ftp:host=server,ssl=true".to_string(),
            ]
        );
        // A user or a port earns the second, less specific name; neither
        // earns nothing.
        assert_eq!(
            generic_mount_names("sftp", "server", "user", Some(2222)),
            vec![
                "sftp:user=user,host=server,port=2222".to_string(),
                "sftp:host=server".to_string(),
            ]
        );
        // The host is lowercased here too.
        assert_eq!(
            generic_mount_names("sftp", "SERVER", "", None),
            vec!["sftp:host=server".to_string()]
        );
    }

    /// The port-zero pair: the parser reports `Some(0)` because CPython does,
    /// and the mount name treats it as absent because Python's `if port:` is
    /// falsy for zero. A `is_some()` here would add a `port=0` detail and a
    /// second name that the reference never builds.
    #[test]
    fn a_port_of_zero_is_absent_from_a_mount_name() {
        assert_eq!(
            generic_mount_names("sftp", "server", "", Some(0)),
            vec!["sftp:host=server".to_string()]
        );
        // Control arm: the same call with a real port does produce both names,
        // so the assertion above cannot pass by ignoring the port entirely.
        assert_eq!(
            generic_mount_names("sftp", "server", "", Some(22)),
            vec![
                "sftp:host=server,port=22".to_string(),
                "sftp:host=server".to_string(),
            ]
        );
        // And the parser really does report zero rather than `None`, which is
        // the half that makes this test about *this* function.
        assert_eq!(url_parts("sftp://server:0/x").port, Some(0));
    }

    // -- as_local_path ------------------------------------------------------

    #[test]
    fn a_plain_path_and_an_unrelated_scheme_come_back_unchanged() {
        let (env, root) = gvfs("netpaths-plain");
        for value in [
            "/home/user/Games/x.exe",
            "relative/x.exe",
            "https://example.com/x.exe",
            "steam://run/7",
            // Not a remote scheme, so no mount lookup happens even though a
            // matching directory exists — this is the control arm for the
            // scheme check below.
            "http://server/share/x.exe",
        ] {
            assert_eq!(as_local_path_in(value, &env), value, "{value:?}");
        }
        // The directory that would match if the scheme were remote.
        mount(&root, "smb-share:server=server,share=share", "x.exe");
        assert_eq!(
            as_local_path_in("http://server/share/x.exe", &env),
            "http://server/share/x.exe"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_url_is_unwrapped_and_percent_decoded() {
        let (env, root) = gvfs("netpaths-file");
        assert_eq!(
            as_local_path_in("file:///home/user/My%20Game/x.exe", &env),
            "/home/user/My Game/x.exe"
        );
        assert_eq!(
            as_local_path_in("file:///home/user/x.exe", &env),
            "/home/user/x.exe"
        );
        // `unquote(path) or raw` — a `file://` URL with no path at all keeps the
        // raw value rather than resolving to the empty string. Empty would be
        // indistinguishable from "no game was ever set".
        assert_eq!(as_local_path_in("file://", &env), "file://");
        assert_eq!(as_local_path_in("file:///", &env), "/");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mounted_share_resolves_onto_its_gvfs_path() {
        let (env, root) = gvfs("netpaths-mounted");
        // `smb://server/Share/dir/game.exe` -> the mount directory named for
        // the server *and* the share, with the path after the share appended.
        let expected = mount(&root, "smb-share:server=server,share=Share", "dir/game.exe");
        assert_eq!(
            as_local_path_in("smb://server/Share/dir/game.exe", &env),
            expected.to_string_lossy()
        );
        // The same URL with the user in it matches the longer name, which is
        // why the user is part of the constructed candidate at all.
        let with_user = mount(
            &root,
            "smb-share:server=server,share=Share,user=user",
            "dir/game.exe",
        );
        assert_eq!(
            as_local_path_in("smb://user@server/Share/dir/game.exe", &env),
            with_user.to_string_lossy()
        );
        // The user is percent-decoded before it goes into the mount name:
        // `us%40er` is the user `us@er`, not `us%40er`.
        let quoted_user = mount(
            &root,
            "smb-share:server=server,share=Share,user=us@er",
            "dir/game.exe",
        );
        assert_eq!(
            as_local_path_in("smb://us%40er@server/Share/dir/game.exe", &env),
            quoted_user.to_string_lossy()
        );
        // A generic scheme hangs the whole path off the mount name.
        let sftp = mount(&root, "sftp:host=server", "pub/game.exe");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            sftp.to_string_lossy()
        );
        // `ssh` resolves through the `sftp` mount name, and `davs` through
        // `dav,ssl=true` — the renames are the whole point of the pair.
        assert_eq!(
            as_local_path_in("ssh://server/pub/game.exe", &env),
            sftp.to_string_lossy()
        );
        let davs = mount(&root, "dav:host=server,ssl=true", "pub/game.exe");
        assert_eq!(
            as_local_path_in("davs://server/pub/game.exe", &env),
            davs.to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unmounted_share_comes_back_unchanged_so_the_error_can_name_it() {
        let (env, root) = gvfs("netpaths-unmounted");
        for value in [
            "smb://server/Share/dir/game.exe",
            "sftp://server/pub/game.exe",
            // A scheme with a netloc but a **path of nothing**, which is the
            // one arm that returns early out of `mount_candidates` for `smb`.
            "smb://server",
        ] {
            assert_eq!(as_local_path_in(value, &env), value, "{value:?}");
            // The pair with the resolution test above: the value is unchanged
            // *and* still remote, which is exactly the state
            // `resolve_game_paths` turns into `UnreachableShare`.
            assert!(is_remote_url(value));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- the scan fallback --------------------------------------------------

    #[test]
    fn an_unmatched_mount_name_is_found_by_scanning_the_gvfs_root() {
        let (env, root) = gvfs("netpaths-scan");
        // A name the constructor would never build — a domain the reference's
        // `_smb_mount_names` knows nothing about — which is the case the scan
        // exists for.
        let scanned = mount(
            &root,
            "smb-share:server=server,share=Share,domain=WORKGROUP",
            "dir/game.exe",
        );
        assert_eq!(
            as_local_path_in("smb://server/Share/dir/game.exe", &env),
            scanned.to_string_lossy()
        );
        // A generic mount the constructor missed, for the same reason.
        let generic = mount(&root, "sftp:host=server,user=user", "pub/game.exe");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            generic.to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The scan's two filters, each with a control arm: a directory whose name
    /// has the right scheme but the wrong host, and one with the right host but
    /// the wrong scheme. Both must be skipped — if either filter were dropped,
    /// the first would be returned.
    #[test]
    fn the_scan_requires_both_the_scheme_and_the_host_to_match() {
        let (env, root) = gvfs("netpaths-scan-filters");
        mount(&root, "sftp:host=other", "pub/game.exe");
        mount(&root, "smb-share:server=server", "pub/game.exe");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            "sftp://server/pub/game.exe"
        );
        // The control arm: adding the directory that *does* match makes the
        // same call resolve, so the assertion above is about the filters and
        // not about the scan never working.
        let matched = mount(&root, "sftp:host=server", "pub/game.exe");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            matched.to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A name that matches by scheme and host but has no such file under it
    /// must not be returned: the scan checks `candidate.exists()` and keeps
    /// looking. Without that check the mount *directory* would be handed back
    /// for any path, and Wine would be pointed at a directory.
    #[test]
    fn the_scan_does_not_return_a_directory_for_a_file_it_does_not_have() {
        let (env, root) = gvfs("netpaths-scan-missing");
        mount(&root, "sftp:host=server", "pub");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            "sftp://server/pub/game.exe"
        );
        // Control arm: the file appears and the same call resolves, so the
        // check above is about `exists()` and not about the scan.
        let present = mount(&root, "sftp:host=server", "pub/game.exe");
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            present.to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_gvfs_root_is_not_an_error() {
        // `list(root.iterdir())` raises into the `except OSError` at
        // `netpaths.py:114` when the directory does not exist, which is the
        // ordinary case for a user with no network shares at all.
        let env = FakeEnv::new(&[("GAMEHANDLER_GVFS_ROOT", "/nonexistent/gvfs")]);
        assert_eq!(
            as_local_path_in("smb://server/Share/dir/game.exe", &env),
            "smb://server/Share/dir/game.exe"
        );
        assert_eq!(
            as_local_path_in("sftp://server/pub/game.exe", &env),
            "sftp://server/pub/game.exe"
        );
    }

    // -- unreachable_share_message -----------------------------------------

    #[test]
    fn the_unreachable_message_names_the_host() {
        assert_eq!(
            unreachable_share_message("smb://server/Share/game.exe"),
            "smb://server/Share/game.exe is a network location that is not mounted yet. \
             Open server in your file manager once so the share is mounted, then try again."
        );
        // The host is lowercased by the parser, and the URL in the message is
        // the value **verbatim**.
        assert_eq!(
            unreachable_share_message("SMB://SERVER/Share/game.exe"),
            "SMB://SERVER/Share/game.exe is a network location that is not mounted yet. \
             Open server in your file manager once so the share is mounted, then try again."
        );
        // No host at all falls back to the phrase rather than the empty string.
        assert_eq!(
            unreachable_share_message(""),
            " is a network location that is not mounted yet. Open the server \
             in your file manager once so the share is mounted, then try again."
        );
        assert_eq!(
            unreachable_share_message("smb:///share/game.exe"),
            "smb:///share/game.exe is a network location that is not mounted yet. \
             Open the server in your file manager once so the share is mounted, then try again."
        );
    }

    /// This function is the one entry point that does **not** strip its input,
    /// so a trailing newline survives into the sentence. Faithful to
    /// `netpaths.py:173`, and pinned because the tidier version is the one a
    /// later reader would write.
    #[test]
    fn the_unreachable_message_does_not_trim_its_input() {
        // The leading-space form. `urlsplit` strips leading C0 and space
        // itself, so the message text keeps the space while the *host* does
        // not — this row alone cannot tell a trimmed parse from an untrimmed
        // one, which is why the next pair exists.
        let value = "  smb://server/Share/game.exe";
        let message = unreachable_share_message(value);
        assert!(
            message.starts_with("  smb://server/Share/game.exe is a network location"),
            "{message:?}"
        );
        assert!(
            message.contains("Open server in your file manager"),
            "{message:?}"
        );

        // **The rows that actually separate the two.** Here the whitespace is
        // *inside the netloc*, at the end of the value, so it lands in the
        // host: the reference names `server ` (with the space) where a trimmed
        // parse would name `server`. Both strings measured.
        for value in ["smb://server ", "smb://server \t"] {
            let message = unreachable_share_message(value);
            assert!(
                message.contains("Open server  in your file manager"),
                "the reference keeps the trailing whitespace in the host: {message:?}"
            );
        }
        // And a trailing `\n` in the value survives in *both* places, because
        // the URL is interpolated verbatim and the newline is not in the netloc.
        let value = "smb://server/Share/game.exe\n";
        let message = unreachable_share_message(value);
        assert!(
            message.starts_with("smb://server/Share/game.exe\n is a network location"),
            "{message:?}"
        );

        // Control arm: the two functions that *do* strip, on the last value, so
        // this is about this function rather than about the parser.
        let env = FakeEnv::new(&[("GAMEHANDLER_GVFS_ROOT", "/nonexistent")]);
        assert_eq!(
            as_local_path_in("smb://server/Share/game.exe\n", &env),
            "smb://server/Share/game.exe"
        );
        assert!(is_remote_url("smb://server/Share/game.exe\n"));
        assert_eq!(
            url_parts("  smb://server/Share/game.exe\n").host.as_deref(),
            Some("server")
        );
    }

    // -- the ShareResolver adapter -----------------------------------------

    /// The adapter is three delegations, so what is worth pinning is that it
    /// delegates to *these* functions and threads the injected environment
    /// through — a `NetpathsShares` that read the process environment would
    /// pass a resolution test only by accident.
    #[test]
    fn the_share_resolver_adapter_resolves_against_the_injected_environment() {
        let (env, root) = gvfs("netpaths-adapter");
        let expected = mount(&root, "smb-share:server=server,share=Share", "dir/game.exe");
        let shares = NetpathsShares::new(&env);

        assert!(shares.is_remote_url("smb://server/Share/dir/game.exe"));
        assert_eq!(
            shares.as_local_path("smb://server/Share/dir/game.exe"),
            expected.to_string_lossy()
        );
        assert_eq!(
            shares.unreachable_share_message("sftp://other/pub/x.exe"),
            "sftp://other/pub/x.exe is a network location that is not mounted yet. \
             Open other in your file manager once so the share is mounted, then try again."
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
