//! Bounded extraction of runner archives, and validation of what came out.
//!
//! This is the one module in the crate where a defect is a vulnerability rather
//! than a bug. The input is a tarball fetched from GitHub, so a compromised
//! upstream account controls every byte of it, and the output is a directory
//! tree under the user's data home that the app will later execute. Everything
//! here is written on that basis.
//!
//! # The invariants, because they are not the obvious ones
//!
//! Three behaviours in this file invert the instinct that "refuse anything
//! suspicious" is the safe choice. Each is pinned by a test ported from
//! `tests/test_security.py`, and each would have been got wrong by porting from
//! a summary rather than from the test:
//!
//! 1. **An absolute member is sanitised, not refused.** Python's
//!    `tarfile.data_filter` strips a leading `/` and only rejects the path if it
//!    is *still* absolute — which on Linux cannot happen, since the Windows
//!    `C:/foo` case is what that second check exists for. So `/tmp/evil` is
//!    extracted to `<destination>/tmp/evil`. Refusing it instead reads safer and
//!    fails `test_absolute_member_is_confined_to_the_destination`, which asserts
//!    the member **is** written, inside the destination. This is the trap.
//! 2. **The byte counter counts the stream, not the payloads.** Python wraps the
//!    decompressor in a reader that counts every byte pulled through it, *before*
//!    `tarfile` parses anything, so PAX/GNU metadata counts toward the limit.
//!    Summing `member.size` is not equivalent and
//!    `test_pax_metadata_counts_toward_decompressed_stream_limit` catches the
//!    difference: a 4 KiB PAX comment on a 4-byte payload trips a 1 KiB limit.
//! 3. **Members stream out, and failure leaves the earlier ones behind.** The
//!    limits are enforced as the archive is read, not in a validation pass
//!    beforehand, so a member that trips a limit is not extracted while the
//!    members before it already are. The caller relies on this: staging is
//!    always a fresh directory that is discarded on failure, which is what makes
//!    one bounded pass safe.
//!
//! Two further properties are structural rather than behavioural, and are worth
//! stating because they are the reason the port cannot simply call a library:
//!
//! * **The filter is ours.** Rust's `tar` crate has no `data_filter` equivalent
//!   (PLAN.md R-3), so the mode handling, special-file rejection and link
//!   checks below are a port of `tarfile._get_filtered_attrs`, and the crate's
//!   own `unpack_in` is deliberately not used — it *rejects* absolute paths,
//!   which is invariant 1 above.
//! * **The filter cannot be absent.** `test_missing_standard_data_filter_fails_closed`
//!   exists because Python can be built without `tarfile.data_filter` and the
//!   function refuses to run rather than extracting unfiltered. In Rust the
//!   filter is this file's own code, so the failure mode it guards against is
//!   unrepresentable; `a_special_file_is_refused` keeps the *intent* as a
//!   positive test, and REPORT.md records that the mechanism is gone rather
//!   than silently dropping the case.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use crate::hash::sha256_hex;

/// Written into an installed runner directory to record where it came from.
pub const METADATA_NAME: &str = ".gamehandler.json";

/// Cap on the *downloaded* archive, enforced against `Content-Length` and the
/// stream. Referenced by `ProtonManager::install`; the constant lives here so
/// every archive bound is in one place.
pub const MAX_RUNNER_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Cap on the sum of member sizes — what the archive claims to contain.
pub const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Cap on bytes actually pulled through the decompressor, metadata included.
pub const MAX_ARCHIVE_DECOMPRESSED_BYTES: u64 = 24 * 1024 * 1024 * 1024;
/// Cap on the number of members, which is what stops a tarball of a hundred
/// million empty files from being a denial of service.
pub const MAX_ARCHIVE_MEMBERS: u64 = 250_000;

/// Layouts used by Proton tarballs and Kron4ek Wine-Builds. `runners.py:60`.
const WINE_CANDIDATES: [&[&str]; 3] = [
    &["files", "bin", "wine"],
    &["dist", "bin", "wine"],
    &["bin", "wine"],
];

/// The bounds applied while extracting.
///
/// Bundled rather than read from the constants directly so tests can extract
/// under a tiny limit. Python does the same thing with
/// `mock.patch("gamehandler.runners.MAX_ARCHIVE_MEMBERS", 1)`, which Rust has no
/// equivalent of — module globals are not patchable — so the limits are a value
/// that gets passed in. That also makes the production bound explicit at the
/// call site instead of implicit in a name.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub members: u64,
    pub uncompressed_bytes: u64,
    pub decompressed_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            members: MAX_ARCHIVE_MEMBERS,
            uncompressed_bytes: MAX_ARCHIVE_UNCOMPRESSED_BYTES,
            decompressed_bytes: MAX_ARCHIVE_DECOMPRESSED_BYTES,
        }
    }
}

/// Every way extraction or staging validation can refuse an archive.
///
/// Distinct variants rather than a string, because the caller has to tell
/// "this archive is hostile" from "this archive is corrupt" — the first is worth
/// reporting to the user as a security event and the second as a failed
/// download. Messages carry the substrings the ported tests assert on.
#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    TooManyMembers { limit: u64 },
    PayloadTooLarge { limit: u64, seen: u64 },
    DecompressedTooLarge { limit: u64 },
    /// A member that resolves outside the destination.
    OutsideDestination(String),
    /// A member type the filter refuses: fifo, socket, char or block device.
    SpecialFile(String),
    /// A link whose target is an absolute path.
    AbsoluteLink(String),
    /// A link that would escape once the tree is renamed into place.
    EscapingLink(PathBuf),
    /// The tree to be installed contains `.gamehandler.json`, which would let an
    /// archive forge the provenance the app later trusts.
    ReservedMetadata,
    /// The staged tree is not inside its own staging directory.
    OutsideStaging(PathBuf),
    /// No Wine binary and no `proton` file — nothing runnable was extracted.
    NoUsableRunner,
    UnsafeId(String),
    UnsafeTag(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::TooManyMembers { limit } => write!(
                f,
                "Runner archive contains too many members (limit {limit})"
            ),
            Self::PayloadTooLarge { limit, seen } => write!(
                f,
                "Runner archive exceeds the uncompressed payload size limit \
                 ({seen} bytes, limit {limit})"
            ),
            Self::DecompressedTooLarge { limit } => write!(
                f,
                "Runner archive exceeds the decompressed stream limit ({limit} bytes)"
            ),
            Self::OutsideDestination(name) => {
                write!(f, "Runner archive member escapes the destination: {name}")
            }
            Self::SpecialFile(name) => {
                write!(f, "Runner archive contains an unsupported special file: {name}")
            }
            Self::AbsoluteLink(name) => {
                write!(f, "Runner archive link has an absolute target: {name}")
            }
            Self::EscapingLink(path) => {
                write!(f, "Refusing escaping link in runner: {}", path.display())
            }
            Self::ReservedMetadata => {
                write!(f, "Runner archive contains reserved file {METADATA_NAME}")
            }
            Self::OutsideStaging(path) => write!(
                f,
                "Runner archive resolved outside its private staging directory: {}",
                path.display()
            ),
            Self::NoUsableRunner => {
                write!(f, "Extracted archive does not contain a usable runner")
            }
            Self::UnsafeId(id) => write!(f, "Unsafe runner id: {id:?}"),
            Self::UnsafeTag(tag) => write!(f, "Unsafe runner tag: {tag:?}"),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

// ---------------------------------------------------------------------------
// Untrusted names
// ---------------------------------------------------------------------------

/// Reduce a remote asset name to a bare, safe filename. `runners.py:513`.
///
/// GitHub asset names are attacker-controllable if an upstream account is
/// compromised, so they are never joined onto a directory unfiltered. The
/// fallback is used for anything that is not a usable name — including `..`,
/// which is why the result is a filename and never a path.
pub fn safe_archive_name(name: &str, fallback: &str) -> String {
    // Python takes `PurePosixPath(name.replace("\\", "/")).name.strip()`, so
    // the last component after normalising separators, then trimmed. Empty,
    // "." and ".." are all rejected rather than returned.
    let normalised = name.replace('\\', "/");
    let candidate = normalised
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim();
    if candidate.is_empty() || candidate == "." || candidate == ".." || candidate.contains('/') {
        return fallback.to_string();
    }
    candidate.to_string()
}

/// Reject runner directory names that could escape the runners directory.
/// `runners.py:525`.
pub fn safe_install_id(install_id: &str) -> Result<String, ArchiveError> {
    let candidate = install_id.trim();
    if candidate.is_empty() || candidate == "." || candidate == ".." {
        return Err(ArchiveError::UnsafeId(install_id.to_string()));
    }
    if candidate.contains('/') || candidate.contains('\\') || candidate.starts_with('.') {
        return Err(ArchiveError::UnsafeId(install_id.to_string()));
    }
    Ok(candidate.to_string())
}

/// Turn an untrusted release tag into one safe, collision-resistant component.
/// `runners.py:535`.
///
/// Common upstream tags come back unchanged. When filtering or truncation
/// changes a tag, the result is bound to the complete raw value with a short
/// hash so two distinct releases cannot alias the same install directory — and
/// the `~h` marker sits in a namespace an unchanged raw tag can never occupy,
/// since `~` is not in the accepted alphabet.
pub fn sanitise_release_tag(tag: &str) -> Result<String, ArchiveError> {
    let raw = tag.trim();
    let mut candidate = String::with_capacity(raw.len());
    // `re.sub(r"[^A-Za-z0-9._+-]+", "-", raw)` — runs of disallowed characters
    // collapse to a single `-`.
    let mut in_run = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() || "._+-".contains(character) {
            candidate.push(character);
            in_run = false;
        } else if !in_run {
            candidate.push('-');
            in_run = true;
        }
    }
    let trimmed = candidate.trim_matches([' ', '.', '_', '-']).to_string();
    if trimmed.is_empty() {
        return Err(ArchiveError::UnsafeTag(tag.to_string()));
    }
    if trimmed == raw && trimmed.len() <= 180 {
        return safe_install_id(&trimmed);
    }
    let digest = &sha256_hex(raw.as_bytes())[..12];
    let prefix = {
        let head: String = trimmed.chars().take(166).collect();
        let stripped = head.trim_end_matches([' ', '.', '_', '-']);
        if stripped.is_empty() {
            "runner".to_string()
        } else {
            stripped.to_string()
        }
    };
    safe_install_id(&format!("{prefix}~h{digest}"))
}

// ---------------------------------------------------------------------------
// Bounded reading
// ---------------------------------------------------------------------------

/// A reader that counts every byte it hands out and fails once past `limit`.
///
/// This is the whole point of invariant 2 in the module docs: it sits *under*
/// the tar parser, so metadata is counted as it is read rather than inferred
/// from member sizes. Python's `_BoundedReader` (`runners.py:556`) does the same
/// thing, including over-reading by one byte past the limit so that a limit of
/// zero still makes progress and reports honestly.
struct BoundedReader<R> {
    inner: R,
    limit: u64,
    consumed: u64,
    /// Set when the limit was the reason a read failed, so the caller can tell
    /// our refusal apart from a corrupt archive the tar parser rejected.
    tripped: bool,
}

impl<R: Read> BoundedReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self { inner, limit, consumed: 0, tripped: false }
    }
}

impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.consumed);
        // Never ask for more than one byte past the limit: enough to detect the
        // overrun, not enough to let a hostile archive make us buffer it.
        let request = if buf.len() as u64 > remaining + 1 {
            (remaining + 1) as usize
        } else {
            buf.len()
        };
        let read = self.inner.read(&mut buf[..request])?;
        self.consumed += read as u64;
        if self.consumed > self.limit {
            self.tripped = true;
            return Err(io::Error::other(
                "Runner archive exceeds the decompressed stream limit",
            ));
        }
        Ok(read)
    }
}

/// Choose a decompressor by magic bytes, defaulting to a plain tar stream.
///
/// `runners.py:578`. The formats are sniffed rather than taken from the file
/// extension, because the extension comes from the same untrusted place the
/// contents do.
fn decoder_for(file: File, magic: &[u8]) -> Box<dyn Read> {
    if magic.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if magic.starts_with(b"\xfd7zXZ\x00") {
        Box::new(xz2::read::XzDecoder::new(file))
    } else if magic.starts_with(b"BZh") {
        Box::new(bzip2::read::BzDecoder::new(file))
    } else {
        Box::new(file)
    }
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

/// Resolve `path` as `realpath` would, tolerating a missing tail.
///
/// Python calls `os.path.realpath(..., strict=ALLOW_MISSING)`, which resolves
/// every symlink that already exists and leaves the rest alone. That matters
/// here because members are extracted in order: an earlier member can create a
/// symlink that redirects a later one, and a purely lexical check would not
/// notice. The longest existing prefix is canonicalised and the remainder
/// appended.
fn resolve_missing(path: &Path) -> PathBuf {
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(mut real) = fs::canonicalize(&current) {
            for part in suffix.iter().rev() {
                real.push(part);
            }
            return real;
        }
        match current.file_name() {
            Some(name) => {
                suffix.push(name.to_os_string());
                if !current.pop() {
                    return path.to_path_buf();
                }
            }
            None => return path.to_path_buf(),
        }
    }
}

/// Normalise `name` lexically, refusing anything that leaves `destination`.
///
/// Lexical normalisation rather than a blanket `..` ban, because Python permits
/// a member like `a/../b` — it normalises to `b`, inside the destination, and
/// only the *result* is checked. The result is then resolved through any
/// symlinks that exist by now (see [`resolve_missing`]) and checked again, so a
/// symlink earlier in the archive cannot redirect a later member out.
///
/// Returns `None` when the name normalises to the destination root itself.
/// Python allows that — `tar -czf x.tar.gz .` writes members named `./`, and
/// `os.path.commonpath` says a path equal to the destination is inside it — so
/// refusing it would reject ordinary archives. The caller decides what such a
/// member may become (only a directory is meaningful; a *file* or a *link*
/// there would replace the destination for everything after it, which Python
/// refuses by identity in the link branch).
fn destination_path(destination: &Path, name: &str) -> Result<Option<PathBuf>, ArchiveError> {
    let mut relative = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            // "." and repeated separators collapse; a leading "/" is dropped,
            // which is invariant 1 in the module docs — it is stripped, not
            // refused.
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                if !relative.pop() {
                    return Err(ArchiveError::OutsideDestination(name.to_string()));
                }
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }

    let resolved_root = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    let resolved = resolve_missing(&destination.join(&relative));
    if !resolved.starts_with(&resolved_root) {
        return Err(ArchiveError::OutsideDestination(name.to_string()));
    }
    Ok(Some(relative))
}

/// Normalise a link target the way `os.path.normpath` does, keeping it
/// relative.
///
/// A symlink's target is stored as written, so this is what gets created on
/// disk; the *validation* of where it lands is separate.
///
/// The one subtlety is that a `..` may only cancel a *normal* component. A
/// pending `..` is not something to pop — `../../outside` must stay
/// `../../outside`, not collapse to `outside`, or every escaping symlink would
/// be laundered into a safe-looking relative path before the containment check
/// ever saw it. (Python has the same rule in `os.path.normpath`; getting it
/// wrong is how the first draft of this port let
/// `a_symlink_escaping_the_destination_is_refused` extract successfully.)
fn normalise_link_target(target: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in Path::new(target).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::ParentDir => {
                let pops_a_normal = out
                    .file_name()
                    .is_some_and(|last| last != std::ffi::OsStr::new(".."));
                if pops_a_normal {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The filter
// ---------------------------------------------------------------------------

/// What a member will become, once it has passed the filter.
struct Planned {
    /// Path relative to the destination, or `None` for a member that *is* the
    /// destination root.
    relative: Option<PathBuf>,
    /// Permission bits to apply, or `None` to leave the default in place.
    mode: Option<u32>,
    link: Option<LinkTarget>,
}

enum LinkTarget {
    Symlink(PathBuf),
    Hardlink(PathBuf),
}

/// Apply the ported `data_filter` to one member.
///
/// The order matters and follows `tarfile._get_filtered_attrs`: name first
/// (because everything else is relative to the destination), then file type and
/// mode together (because the mode rule depends on the type), then links.
fn plan_member<R: Read>(
    entry: &tar::Entry<'_, R>,
    destination: &Path,
) -> Result<Planned, ArchiveError> {
    let name = entry
        .path()
        .map_err(|error| ArchiveError::Io(io::Error::other(error)))?
        .to_string_lossy()
        .into_owned();
    let relative = destination_path(destination, &name)?;
    let kind = entry.header().entry_type();
    let resolved_root =
        fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());

    // `member.size < 0` has no Rust equivalent — the tar header's size field is
    // unsigned here where Python's is a signed int — so the check Python makes
    // cannot fail and is not restated. A size that overflows is caught by the
    // payload limit instead.
    let raw_mode = entry.header().mode().unwrap_or(0o644);

    if kind.is_dir() {
        // Python ignores the mode of directories and symlinks entirely
        // (`mode = None`), so the extracted tree gets default permissions. A
        // `./` member is the destination itself, which already exists.
        return Ok(Planned { relative, mode: None, link: None });
    }
    if kind.is_symlink() || kind.is_hard_link() {
        let Some(relative) = relative else {
            // Python refuses this by identity, and it is the reason the check
            // exists: a link at the destination root would replace the
            // destination for every member extracted after it.
            return Err(ArchiveError::OutsideDestination(name));
        };
        let target = entry
            .link_name()
            .map_err(|error| ArchiveError::Io(io::Error::other(error)))?
            .ok_or_else(|| ArchiveError::AbsoluteLink(name.clone()))?;
        let target = target.to_string_lossy().into_owned();
        if Path::new(&target).is_absolute() {
            return Err(ArchiveError::AbsoluteLink(name));
        }
        let normalised = normalise_link_target(&target);

        // A symlink's target is relative to the directory holding the link; a
        // hardlink's is relative to the destination root. Getting this
        // backwards is the difference between catching an escape and missing
        // one, so the two cases are kept visually separate.
        let landing = if kind.is_symlink() {
            let link_dir = relative.parent().unwrap_or(Path::new(""));
            destination.join(link_dir).join(&normalised)
        } else {
            destination.join(&normalised)
        };
        let resolved = resolve_missing(&landing);
        if !resolved.starts_with(&resolved_root) {
            return Err(ArchiveError::EscapingLink(destination.join(&relative)));
        }
        let link = if kind.is_symlink() {
            LinkTarget::Symlink(normalised)
        } else {
            LinkTarget::Hardlink(normalised)
        };
        return Ok(Planned { relative: Some(relative), mode: None, link: Some(link) });
    }
    if !kind.is_file() {
        // Fifo, socket, char/block device: refused, as `data_filter` does.
        return Err(ArchiveError::SpecialFile(name));
    }
    if relative.is_none() {
        // A *file* at the destination root: Python's own filter passes it and
        // `tarfile.extract` then fails with `IsADirectoryError`. Failing here
        // instead says the same thing earlier and more clearly.
        return Err(ArchiveError::OutsideDestination(name));
    }

    // Regular file: strip high bits and group/other write, clear the execute
    // bits entirely unless the owner had them, then guarantee owner read/write.
    let mut mode = raw_mode & 0o755;
    if mode & 0o100 == 0 {
        mode &= !0o111;
    }
    mode |= 0o600;
    Ok(Planned { relative, mode: Some(mode), link: None })
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Stream a bounded Proton/Wine tarball into private staging.
///
/// The caller always supplies a fresh staging directory that is discarded on
/// failure, which is what makes validating and extracting in one bounded pass
/// safe. Returns an error rather than partial success; the partial tree is the
/// caller's to remove (`ProtonManager::install` does).
pub fn extract_archive(archive: &Path, destination: &Path) -> Result<(), ArchiveError> {
    extract_archive_with(archive, destination, Limits::default())
}

/// [`extract_archive`] with explicit bounds, for tests and for callers that
/// need a tighter cap than the defaults.
pub fn extract_archive_with(
    archive: &Path,
    destination: &Path,
    limits: Limits,
) -> Result<(), ArchiveError> {
    fs::create_dir_all(destination)?;

    let mut file = File::open(archive)?;
    // Sniff the container without consuming it. Reading fewer than six bytes is
    // fine — `startswith` on a short buffer still answers correctly, and a file
    // too short to hold a magic number is a plain tar stream that will fail
    // later with a better message.
    let mut magic = [0u8; 6];
    let read = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let decoder = decoder_for(file, &magic[..read]);

    let mut reader = BoundedReader::new(decoder, limits.decompressed_bytes);
    let mut members = 0u64;
    let mut payload = 0u64;

    let outcome = extract_members(&mut reader, destination, limits, &mut members, &mut payload);
    match outcome {
        Ok(()) => Ok(()),
        // A read error from the bounded reader is our own limit, not a corrupt
        // archive — the flag is how the two are told apart, since the tar parser
        // surfaces both as `io::Error`.
        Err(error) if reader.tripped => {
            let _ = error;
            Err(ArchiveError::DecompressedTooLarge { limit: limits.decompressed_bytes })
        }
        Err(error) => Err(error),
    }
}

fn extract_members<R: Read>(
    reader: &mut BoundedReader<R>,
    destination: &Path,
    limits: Limits,
    members: &mut u64,
    payload: &mut u64,
) -> Result<(), ArchiveError> {
    let mut tar = tar::Archive::new(reader);
    // The archive is untrusted, so a member whose header claims a size that
    // cannot be honoured is an error rather than something to skip.
    tar.set_ignore_zeros(false);

    for entry in tar.entries()? {
        let mut entry = entry?;
        *members += 1;
        if *members > limits.members {
            return Err(ArchiveError::TooManyMembers { limit: limits.members });
        }

        let size = entry.size();
        *payload = payload.saturating_add(size);
        if *payload > limits.uncompressed_bytes {
            return Err(ArchiveError::PayloadTooLarge {
                limit: limits.uncompressed_bytes,
                seen: *payload,
            });
        }

        let planned = plan_member(&entry, destination)?;
        let Some(relative) = planned.relative.clone() else {
            // A `./` member: the destination itself, which `create_dir_all`
            // above has already made. `plan_member` has refused every other
            // kind of member that could land here.
            fs::create_dir_all(destination)?;
            continue;
        };
        let path = destination.join(&relative);

        match planned.link {
            Some(LinkTarget::Symlink(target)) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(&target, &path)?;
            }
            Some(LinkTarget::Hardlink(target)) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                // Hardlink targets are relative to the destination root, and
                // the target has already been filtered because it appeared as
                // its own member.
                let source = destination.join(target);
                fs::hard_link(&source, &path)?;
            }
            None if entry.header().entry_type().is_dir() => {
                fs::create_dir_all(&path)?;
            }
            None => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = File::create(&path)?;
                io::copy(&mut entry, &mut file)?;
                file.flush()?;
                if let Some(mode) = planned.mode {
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Staging validation
// ---------------------------------------------------------------------------

/// Locate the Wine binary inside an extracted Proton or Wine build.
/// `runners.py:303`.
pub fn find_wine_binary(root: &Path) -> Option<PathBuf> {
    for parts in WINE_CANDIDATES {
        let candidate = root.join(parts.iter().collect::<PathBuf>());
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Confirm a selected staged tree is self-contained and still runnable.
/// `runners.py:632`.
///
/// The symlink checks run *after* extraction and against the tree as it will be
/// renamed into place, which is why they cannot be folded into the extraction
/// filter: a link that is safe relative to the staging root can point outside
/// once a top-level directory is moved beside existing installations.
pub fn validate_staged_runner(
    candidate: &Path,
    extraction_root: &Path,
) -> Result<(), ArchiveError> {
    let root = fs::canonicalize(extraction_root)?;
    let resolved = fs::canonicalize(candidate)?;
    if candidate.is_symlink() || (resolved != root && !resolved.starts_with(&root)) {
        return Err(ArchiveError::OutsideStaging(candidate.to_path_buf()));
    }

    if candidate.join(METADATA_NAME).exists() || candidate.join(METADATA_NAME).is_symlink() {
        return Err(ArchiveError::ReservedMetadata);
    }

    walk_links(candidate)?;

    if find_wine_binary(candidate).is_none() && !candidate.join("proton").is_file() {
        return Err(ArchiveError::NoUsableRunner);
    }
    Ok(())
}

/// Check every symlink in the staged tree without following any of them.
///
/// `os.walk(..., followlinks=False)` in Python; the recursion is bounded by the
/// tree depth of an archive we have already extracted, and symlinked
/// directories are not descended into.
fn walk_links(root: &Path) -> Result<(), ArchiveError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ArchiveError::Io(error)),
    };
    for entry in entries {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            if target.is_absolute() {
                return Err(ArchiveError::EscapingLink(path));
            }
            let relative_parent = path
                .strip_prefix(root)
                .unwrap_or(Path::new(""))
                .parent()
                .unwrap_or(Path::new(""));
            // Lexical, because the point is to predict where the link will land
            // *after the tree is moved*, not where it lands now.
            let mut lexical = PathBuf::new();
            let mut escaped = false;
            for component in relative_parent.join(&target).components() {
                match component {
                    Component::Normal(part) => lexical.push(part),
                    Component::ParentDir => {
                        if !lexical.pop() {
                            escaped = true;
                        }
                    }
                    Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
                }
            }
            if escaped {
                return Err(ArchiveError::EscapingLink(path));
            }
        } else if metadata.is_dir() {
            walk_links(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Test archives
    // -----------------------------------------------------------------------

    /// One member of a synthetic archive.
    enum Member<'a> {
        File(&'a str, &'a [u8]),
        FileMode(&'a str, &'a [u8], u32),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
    }

    /// Write `name` into a header's name field without the builder's own
    /// validation.
    ///
    /// `Header::set_path` refuses absolute paths and `..`, which is exactly the
    /// set of names the adversarial fixtures need — it is a guard for people
    /// *writing* archives, and it is not the guard under test. Going through it
    /// would mean the traversal tests silently asserted nothing, because the
    /// fixture could not be built. Nothing here is a safety bypass: these
    /// archives only ever exist in a temp directory and are only ever fed to
    /// the code that is supposed to refuse them.
    fn set_raw_name(header: &mut tar::Header, name: &str) {
        let slot = &mut header.as_old_mut().name;
        slot.fill(0);
        let bytes = name.as_bytes();
        assert!(bytes.len() < slot.len(), "test names must fit in 100 bytes");
        slot[..bytes.len()].copy_from_slice(bytes);
    }

    fn build_tar(members: &[Member<'_>]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for member in members {
            match member {
                Member::File(name, data) => append_file(&mut builder, name, data, 0o755),
                Member::FileMode(name, data, mode) => append_file(&mut builder, name, data, *mode),
                Member::Dir(name) => {
                    let mut header = tar::Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Directory);
                    set_raw_name(&mut header, name);
                    header.set_size(0);
                    header.set_mode(0o755);
                    header.set_cksum();
                    builder.append(&header, io::empty()).unwrap();
                }
                Member::Symlink(name, target) => {
                    let mut header = tar::Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Symlink);
                    set_raw_name(&mut header, name);
                    header.set_link_name(target).unwrap();
                    header.set_size(0);
                    header.set_mode(0o777);
                    header.set_cksum();
                    builder.append(&header, io::empty()).unwrap();
                }
            }
        }
        builder.into_inner().unwrap()
    }

    fn append_file(builder: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8], mode: u32) {
        let mut header = tar::Header::new_gnu();
        // Every name in these tests fits the 100-byte GNU name field, so no
        // long-name extension entry is needed — which matters, because a long
        // name would add a member and change what the member-count limits see.
        set_raw_name(&mut header, name);
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }

    /// A scratch directory that cleans itself up.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gh-archive-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
        /// Write `bytes` to a file and return its path.
        fn archive(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    // -----------------------------------------------------------------------
    // Invariant 1 — absolute members are confined, not refused
    // -----------------------------------------------------------------------

    #[test]
    fn a_normal_archive_extracts_and_keeps_the_executable_bit() {
        let scratch = Scratch::new("normal");
        let payload = build_tar(&[Member::File("GE-Proton/files/bin/wine", b"#!/bin/sh\n")]);
        let archive = scratch.archive("ok.tar", &payload);
        let destination = scratch.join("runners");
        extract_archive(&archive, &destination).unwrap();

        let wine = destination.join("GE-Proton/files/bin/wine");
        assert!(wine.is_file());
        assert_eq!(fs::read(&wine).unwrap(), b"#!/bin/sh\n");
        let mode = fs::metadata(&wine).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "Wine builds must stay executable, mode is {mode:o}"
        );
    }

    #[test]
    fn an_absolute_member_is_confined_to_the_destination() {
        // The trap: the leading separator is *stripped*, so the member lands
        // inside the destination and the absolute path is never written. A
        // filter that refused absolute names would fail this — see the module
        // docs, invariant 1.
        let scratch = Scratch::new("absolute");
        let outside = Path::new("/tmp/gh-absolute-escape");
        let _ = fs::remove_file(outside);
        let payload = build_tar(&[Member::File("/tmp/gh-absolute-escape", b"pwned")]);
        let archive = scratch.archive("evil.tar", &payload);
        let destination = scratch.join("runners");

        extract_archive(&archive, &destination).unwrap();

        assert!(!outside.exists(), "the absolute path must never be written");
        assert_eq!(
            fs::read(destination.join("tmp/gh-absolute-escape")).unwrap(),
            b"pwned",
            "the leading separator is stripped, so the member is confined"
        );
    }

    #[test]
    fn a_parent_traversal_member_is_refused() {
        let scratch = Scratch::new("traversal");
        let payload = build_tar(&[Member::File("../escaped.txt", b"pwned")]);
        let archive = scratch.archive("evil.tar", &payload);
        let destination = scratch.join("runners");

        let error = extract_archive(&archive, &destination).unwrap_err();
        assert!(
            matches!(error, ArchiveError::OutsideDestination(_)),
            "expected an outside-destination refusal, got {error}"
        );
        assert!(!scratch.join("escaped.txt").exists());
    }

    #[test]
    fn a_traversal_that_normalises_back_inside_is_allowed() {
        // Python normalises lexically and checks the *result*, so `a/../b` is
        // a member called `b`. Banning every `..` would be stricter than Python
        // and would break legitimate archives.
        let scratch = Scratch::new("lexical");
        let payload = build_tar(&[Member::Dir("a"), Member::File("a/../b", b"fine")]);
        let archive = scratch.archive("ok.tar", &payload);
        let destination = scratch.join("runners");
        extract_archive(&archive, &destination).unwrap();
        assert_eq!(fs::read(destination.join("b")).unwrap(), b"fine");
    }

    #[test]
    fn a_symlink_escaping_the_destination_is_refused() {
        let scratch = Scratch::new("symlink");
        let payload = build_tar(&[Member::Symlink("link", "../../outside")]);
        let archive = scratch.archive("evil.tar", &payload);
        let destination = scratch.join("runners");

        let error = extract_archive(&archive, &destination).unwrap_err();
        assert!(
            matches!(error, ArchiveError::EscapingLink(_) | ArchiveError::OutsideDestination(_)),
            "expected a link refusal, got {error}"
        );
    }

    #[test]
    fn a_special_file_is_refused() {
        // The positive form of `test_missing_standard_data_filter_fails_closed`:
        // the filter cannot be absent in this port, so what is worth asserting
        // is that it always runs and rejects what Python rejects.
        let scratch = Scratch::new("special");
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Fifo);
        header.set_path("runner/pipe").unwrap();
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
        let archive = scratch.archive("evil.tar", &builder.into_inner().unwrap());
        let destination = scratch.join("runners");

        let error = extract_archive(&archive, &destination).unwrap_err();
        assert!(
            matches!(error, ArchiveError::SpecialFile(_)),
            "a fifo must be refused, got {error}"
        );
        assert!(!destination.join("runner/pipe").exists());
    }

    #[test]
    fn a_member_that_would_replace_the_destination_is_refused() {
        // A link at the destination root would become the destination for every
        // later member. Python refuses this by identity, not by containment.
        let scratch = Scratch::new("replace-root");
        let payload = build_tar(&[Member::Symlink(".", "/tmp")]);
        let archive = scratch.archive("evil.tar", &payload);
        let destination = scratch.join("runners");
        assert!(extract_archive(&archive, &destination).is_err());
    }

    #[test]
    fn the_current_directory_member_that_tar_actually_writes_is_accepted() {
        // The counterpart to the refusal above, and the reason the root case is
        // not simply rejected: `tar -czf` writes a directory member named `./`
        // for the archive root, so refusing every root member would reject real
        // archives. Only the *link* and *file* cases are refused.
        let scratch = Scratch::new("dot-root");
        let payload = build_tar(&[
            Member::Dir("./"),
            Member::Dir("./runner/files"),
            Member::File("./runner/files/wine", b"wine"),
        ]);
        let archive = scratch.archive("ok.tar", &payload);
        let destination = scratch.join("runners");
        extract_archive(&archive, &destination).unwrap();
        assert!(destination.join("runner/files/wine").is_file());
    }

    #[test]
    fn a_file_at_the_destination_root_is_refused() {
        // Python's filter passes this and `tarfile.extract` then fails with
        // `IsADirectoryError`. The refusal is the same refusal, moved earlier so
        // it names the cause rather than the symptom.
        let scratch = Scratch::new("root-file");
        let payload = build_tar(&[Member::File("./", b"not a directory")]);
        let archive = scratch.archive("evil.tar", &payload);
        let destination = scratch.join("runners");
        let error = extract_archive(&archive, &destination).unwrap_err();
        assert!(
            matches!(error, ArchiveError::OutsideDestination(_)),
            "got {error}"
        );
        assert!(destination.is_dir(), "the destination must survive");
    }

    // -----------------------------------------------------------------------
    // Invariant 2 — the decompressed counter counts metadata too
    // -----------------------------------------------------------------------

    #[test]
    fn pax_metadata_counts_toward_the_decompressed_stream_limit() {
        // The trap: a 4 KiB PAX comment on a 4-byte payload. Summing member
        // sizes gives 4 and passes; counting the stream gives ~4 KiB and trips
        // a 1 KiB limit. This is why the counter sits under the tar parser.
        let scratch = Scratch::new("pax");
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::XHeader);
        header.set_path("pax").unwrap();
        let record = format!("comment={}\n", "A".repeat(4096));
        header.set_size(record.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, record.as_bytes()).unwrap();

        let mut file_header = tar::Header::new_gnu();
        file_header.set_path("runner/files/bin/wine").unwrap();
        file_header.set_size(4);
        file_header.set_mode(0o755);
        file_header.set_cksum();
        builder.append(&file_header, &b"wine"[..]).unwrap();

        let payload = gzip(&builder.into_inner().unwrap());
        let archive = scratch.archive("pax.tar.gz", &payload);
        let destination = scratch.join("runners");

        let error = extract_archive_with(
            &archive,
            &destination,
            Limits { decompressed_bytes: 1024, ..Limits::default() },
        )
        .unwrap_err();
        assert!(
            matches!(error, ArchiveError::DecompressedTooLarge { .. }),
            "metadata must count toward the stream limit, got {error}"
        );
        assert!(
            !destination.join("runner/files/bin/wine").exists(),
            "the refusal must happen before the payload is written"
        );
    }

    // -----------------------------------------------------------------------
    // Invariant 3 — stream and fail, keeping what came before
    // -----------------------------------------------------------------------

    #[test]
    fn the_member_limit_stops_streaming_inside_private_staging() {
        let scratch = Scratch::new("members");
        let payload = build_tar(&[
            Member::FileMode("runner/a", b"a", 0o644),
            Member::FileMode("runner/b", b"b", 0o644),
        ]);
        let archive = scratch.archive("ok.tar", &payload);
        let destination = scratch.join("runners");

        let error = extract_archive_with(
            &archive,
            &destination,
            Limits { members: 1, ..Limits::default() },
        )
        .unwrap_err();
        assert!(
            matches!(error, ArchiveError::TooManyMembers { .. }),
            "expected a member-limit refusal, got {error}"
        );
        assert!(
            destination.join("runner/a").is_file(),
            "members before the limit are already extracted — failure leaves them behind"
        );
        assert!(!destination.join("runner/b").exists());
    }

    #[test]
    fn the_payload_limit_is_checked_before_the_member_is_extracted() {
        let scratch = Scratch::new("payload");
        let payload = build_tar(&[Member::FileMode("runner/large", b"1234", 0o644)]);
        let archive = scratch.archive("ok.tar", &payload);
        let destination = scratch.join("runners");

        let error = extract_archive_with(
            &archive,
            &destination,
            Limits { uncompressed_bytes: 3, ..Limits::default() },
        )
        .unwrap_err();
        assert!(
            matches!(error, ArchiveError::PayloadTooLarge { .. }),
            "expected a payload-limit refusal, got {error}"
        );
        assert!(
            !destination.join("runner/large").exists(),
            "the offending member must not be written"
        );
    }

    #[test]
    fn a_corrupt_archive_is_an_error_not_a_limit() {
        // The bounded reader and the tar parser both surface failures as
        // `io::Error`, so the distinction is carried by the reader's flag.
        // Getting it backwards would report a truncated download as a security
        // refusal, or worse, report a limit as a corrupt file and retry it.
        let scratch = Scratch::new("corrupt");
        let archive = scratch.archive("bad.tar", b"not a tar archive");
        let destination = scratch.join("runners");
        let error = extract_archive(&archive, &destination).unwrap_err();
        assert!(
            matches!(error, ArchiveError::Io(_)),
            "a corrupt archive is an IO error, got {error}"
        );
    }

    // -----------------------------------------------------------------------
    // Containers
    // -----------------------------------------------------------------------

    #[test]
    fn xz_and_bzip2_streams_extract() {
        let inner = build_tar(&[Member::File("runner/files/bin/wine", b"wine")]);
        for (label, bytes) in [
            ("xz", {
                let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 6);
                encoder.write_all(&inner).unwrap();
                encoder.finish().unwrap()
            }),
            ("bz2", {
                let mut encoder = bzip2::write::BzEncoder::new(
                    Vec::new(),
                    bzip2::Compression::default(),
                );
                encoder.write_all(&inner).unwrap();
                encoder.finish().unwrap()
            }),
            ("gz", gzip(&inner)),
            ("tar", inner.clone()),
        ] {
            let scratch = Scratch::new(&format!("container-{label}"));
            let archive = scratch.archive(&format!("runner.tar.{label}"), &bytes);
            let destination = scratch.join("runners");
            extract_archive(&archive, &destination)
                .unwrap_or_else(|error| panic!("{label} should extract: {error}"));
            assert!(
                destination.join("runner/files/bin/wine").is_file(),
                "{label} did not produce the member"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Untrusted names
    // -----------------------------------------------------------------------

    #[test]
    fn asset_names_are_reduced_to_a_bare_filename() {
        assert_eq!(safe_archive_name("GE-Proton9-5.tar.gz", "runner.tar.gz"), "GE-Proton9-5.tar.gz");
        assert_eq!(safe_archive_name("../../etc/cron.d/x", "runner.tar.gz"), "x");
        assert_eq!(safe_archive_name("/etc/passwd", "runner.tar.gz"), "passwd");
        assert_eq!(safe_archive_name("..", "runner.tar.gz"), "runner.tar.gz");
        assert_eq!(safe_archive_name("", "runner.tar.gz"), "runner.tar.gz");
        assert_eq!(safe_archive_name("", "fallback.tgz"), "fallback.tgz");
        // Windows separators are normalised before the basename is taken, so a
        // backslash path cannot smuggle a directory through.
        assert_eq!(safe_archive_name(r"C:\Users\evil.exe", "runner.tar.gz"), "evil.exe");
    }

    #[test]
    fn install_ids_that_could_escape_are_rejected() {
        assert_eq!(safe_install_id("GE-Proton9-5").unwrap(), "GE-Proton9-5");
        for bad in ["..", "../../home", "a/b", "", ".hidden", "a\\b"] {
            let error = safe_install_id(bad).unwrap_err();
            assert!(
                matches!(error, ArchiveError::UnsafeId(_)),
                "{bad:?} must be refused, got {error}"
            );
        }
    }

    #[test]
    fn release_tags_are_sanitised_before_becoming_install_paths() {
        let slash = sanitise_release_tag("release/v1").unwrap();
        let parent = sanitise_release_tag("../../relocated").unwrap();
        assert!(
            slash.starts_with("release-v1~h") && slash.len() == "release-v1~h".len() + 12,
            "got {slash:?}"
        );
        assert_eq!(parent, format!("relocated~h{}", &parent[parent.len() - 12..]));
        // A tag that survives unchanged is returned as-is, so common upstream
        // tags keep their familiar directory names.
        assert_eq!(sanitise_release_tag("GE-Proton9-5").unwrap(), "GE-Proton9-5");
    }

    #[test]
    fn distinct_raw_tags_never_alias_the_same_install_id() {
        let transformed = sanitise_release_tag("release/v1").unwrap();
        // A transformed id re-sanitised must not be a fixed point, or two
        // different tags would collide on one directory.
        assert_ne!(transformed, sanitise_release_tag(&transformed).unwrap());

        let prefix = "x".repeat(180);
        assert_ne!(
            sanitise_release_tag(&format!("{prefix}A")).unwrap(),
            sanitise_release_tag(&format!("{prefix}B")).unwrap(),
            "truncation must not make two distinct long tags alias"
        );
        assert!(sanitise_release_tag(&"t".repeat(300)).unwrap().len() <= 180);
    }

    #[test]
    fn an_unsafe_tag_is_refused_rather_than_guessed_at() {
        for bad in ["", "   ", "...", "///"] {
            assert!(
                matches!(sanitise_release_tag(bad), Err(ArchiveError::UnsafeTag(_))),
                "{bad:?} has no safe form and must be refused"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Staged tree validation
    // -----------------------------------------------------------------------

    #[test]
    fn a_staged_tree_containing_metadata_is_refused() {
        // `.gamehandler.json` records provenance, so an archive that ships one
        // could forge where it came from.
        let scratch = Scratch::new("metadata");
        let root = scratch.join("stage");
        let tree = root.join("runner");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::write(tree.join("bin/wine"), b"wine").unwrap();
        fs::write(tree.join(METADATA_NAME), b"{}").unwrap();

        let error = validate_staged_runner(&tree, &root).unwrap_err();
        assert!(matches!(error, ArchiveError::ReservedMetadata), "got {error}");
    }

    #[test]
    fn a_staged_tree_without_a_runner_is_refused() {
        let scratch = Scratch::new("norunner");
        let root = scratch.join("stage");
        let tree = root.join("runner");
        fs::create_dir_all(tree.join("docs")).unwrap();

        let error = validate_staged_runner(&tree, &root).unwrap_err();
        assert!(matches!(error, ArchiveError::NoUsableRunner), "got {error}");
    }

    #[test]
    fn a_bare_proton_file_counts_as_a_runner() {
        // Proton builds have no `bin/wine`; the `proton` script is the entry
        // point, so its presence is enough.
        let scratch = Scratch::new("proton");
        let root = scratch.join("stage");
        let tree = root.join("runner");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("proton"), b"#!/bin/sh\n").unwrap();
        validate_staged_runner(&tree, &root).unwrap();
    }

    #[test]
    fn the_wine_binary_is_found_in_each_supported_layout() {
        for layout in [["files", "bin"], ["dist", "bin"], ["bin", ""]] {
            let scratch = Scratch::new("layout");
            let root = scratch.join("stage");
            let mut wine = root.clone();
            for part in layout.iter().filter(|part| !part.is_empty()) {
                wine.push(part);
            }
            fs::create_dir_all(&wine).unwrap();
            fs::write(wine.join("wine"), b"wine").unwrap();
            assert_eq!(find_wine_binary(&root), Some(wine.join("wine")));
        }
    }

    #[test]
    fn a_symlink_that_escapes_once_moved_is_refused() {
        // The reason this check runs after extraction rather than during it:
        // a link safe relative to the staging root can point outside once a
        // top-level directory is renamed beside existing installations.
        let scratch = Scratch::new("escapelink");
        let root = scratch.join("stage");
        let tree = root.join("new-runner");
        fs::create_dir_all(tree.join("files/bin")).unwrap();
        fs::write(tree.join("files/bin/wine"), b"wine").unwrap();
        std::os::unix::fs::symlink("../bridge/back", tree.join("escape")).unwrap();

        let error = validate_staged_runner(&tree, &root).unwrap_err();
        assert!(
            matches!(error, ArchiveError::EscapingLink(_)),
            "expected an escaping-link refusal, got {error}"
        );
    }

    #[test]
    fn an_absolute_symlink_in_a_staged_tree_is_refused() {
        let scratch = Scratch::new("abslink");
        let root = scratch.join("stage");
        let tree = root.join("runner");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::write(tree.join("bin/wine"), b"wine").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", tree.join("passwd")).unwrap();

        let error = validate_staged_runner(&tree, &root).unwrap_err();
        assert!(matches!(error, ArchiveError::EscapingLink(_)), "got {error}");
    }

    #[test]
    fn a_relative_symlink_that_stays_inside_is_allowed() {
        // The counterpart to the two refusals above: an internal link is
        // legitimate — Wine builds use them — and a check that refused every
        // symlink would reject real archives.
        let scratch = Scratch::new("innerlink");
        let root = scratch.join("stage");
        let tree = root.join("runner");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::write(tree.join("bin/wine"), b"wine").unwrap();
        std::os::unix::fs::symlink("bin/wine", tree.join("wine")).unwrap();

        validate_staged_runner(&tree, &root).unwrap();
    }
}
