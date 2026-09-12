//! Cover artwork: the pure rules the interface renders with.
//!
//! Port of the deterministic half of `gamehandler/covers.py`. Only the parts
//! the interface needs *without* touching the network or the filesystem live
//! here — the fetching half (`fetch_cover`, `download_image`, `steam_search`)
//! lands with T-05, which is where the injected HTTP client of D-26 belongs.
//!
//! # Why [`accent_index`] is in `core` rather than in the view
//!
//! `crates/app/src/view/cover.rs` needs it to pick a game's placeholder shade,
//! and it is a hash — so it was tempting to put it where it is used. It is here
//! because it is a **port of a Python function whose value is observable**: a
//! game's tile has a colour that must not change between the Python app and
//! this one, or across sessions. That makes it a compatibility surface, and
//! compatibility surfaces belong where the oracle fixtures can reach them,
//! which is `core`.
//!
//! The alternative the view originally reached for — widening
//! [`crate::hash::sha256_hex`] to `pub` — was rejected for the reason the view
//! itself gave: it exports the *primitive* while the thing that must not drift
//! is the *rule*. A caller holding `sha256_hex` can bucket the digest its own
//! way, and the drift would be invisible until someone compared two screenshots.
//! Exposing the decision keeps the rule in one place, and keeps the hash an
//! implementation detail of it.

use std::fmt;
use std::path::{Path, PathBuf};

/// How many placeholder shades there are. `covers.py:95`.
pub const COVER_ACCENTS: usize = 8;

/// The built-in category list, `covers.py:75-89`.
///
/// The game form offers these plus whatever the library already holds
/// (`bridge.py:344-350`), and [`models::UNCATEGORIZED`] is deliberately the
/// first entry — it is the one the reference sorts last in a library listing
/// and the one a blank category folds to, so a form that seeded its selector
/// with anything else would offer a category no game can be in.
///
/// It lives here rather than in `models` because `covers.py` is where the
/// reference declares it, and because the module that answers "what shelf is
/// this game on" is the one that owns the shelf list. The order is
/// user-visible: it is the order the form's category selector reads.
///
/// [`models::UNCATEGORIZED`]: crate::models::UNCATEGORIZED
pub const DEFAULT_CATEGORIES: [&str; 13] = [
    "Uncategorized",
    "Action",
    "Adventure",
    "RPG",
    "Strategy",
    "Shooter",
    "Racing",
    "Simulation",
    "Sports",
    "Puzzle",
    "Indie",
    "Utility",
    "Emulation",
];

/// A stable colour bucket for `seed`, so a game's tile never changes shade.
/// `accent_index` (`covers.py:107-110`).
///
/// `sha256(seed.encode("utf-8")).digest()[0] % COVER_ACCENTS` — the **first byte**
/// of the digest, not the first hex digit. Reading the digest as text and
/// taking one character would give a value in `0..16` bucketed against 8 and so
/// happen to produce the same answer, which is exactly the kind of coincidence
/// that stops being harmless the moment the bucket count changes. The
/// arithmetic is done on a byte here, as in Python.
///
/// `seed` being empty is handled by hashing the empty string rather than by a
/// special case, because that is what `(seed or "")` does and the two agree:
/// `sha256(b"")` is a real digest whose first byte is 227, so the empty seed is
/// bucket 3, not bucket 0. A port that short-circuited the empty case would put
/// every nameless game in the first shade.
///
/// UTF-8 rather than the platform encoding, so the bucket for a non-ASCII name
/// is the same on every machine — which is what makes this reproducible at all.
pub fn accent_index(seed: &str) -> usize {
    accent_index_in(seed, COVER_ACCENTS)
}

/// [`accent_index`] with an explicit bucket count.
///
/// Exists because Python's signature is `accent_index(seed, buckets=COVER_ACCENTS)`
/// and the default is the *only* thing that differs: the corpus exercises other
/// counts to reach the clamp below, and a caller that needs a different number
/// of shades should not have to re-derive the rule to get one.
///
/// `buckets` is clamped to at least 1, as `max(1, buckets)` does. A zero would
/// otherwise be a division by zero — in Python it raises, here it would panic,
/// and neither is a useful behaviour for a caller that computed a count from a
/// config file. Negative counts cannot reach the clamp from Python's `int`, but
/// they can from a `usize`-underflowing caller, so the clamp is written on the
/// way in rather than assumed.
/// Writing an executable's icon into the covers directory failed.
///
/// Port of the two failures `save_exe_icon` (`covers.py:307-317`) surfaces:
/// the executable carries no icon (a `RuntimeError` there, [`SaveIconError::NoIcon`]
/// here, with the reference's message byte for byte), or the filesystem
/// refused the write.
#[derive(Debug)]
pub enum SaveIconError {
    /// `RuntimeError(f"{name} carries no icon to use as a cover")`.
    ///
    /// `exe` is the executable's file name, as `Path(exe_path).name` is —
    /// including when there is no icon because the path is not a PE at all,
    /// since [`crate::exe_icons::extract_icon`] folds every unreadable input
    /// into `None` before this error is built.
    NoIcon { exe: String },
    /// Creating the covers directory, writing the temporary file, or the
    /// rename into place failed.
    Io(std::io::Error),
}

impl From<std::io::Error> for SaveIconError {
    fn from(error: std::io::Error) -> Self {
        SaveIconError::Io(error)
    }
}

impl fmt::Display for SaveIconError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveIconError::NoIcon { exe } => {
                write!(formatter, "{exe} carries no icon to use as a cover")
            }
            SaveIconError::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SaveIconError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveIconError::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Write the icon embedded in a Windows executable into the covers directory.
///
/// Port of `save_exe_icon` (`covers.py:307-317`): the icon bytes come from
/// [`crate::exe_icons::extract_icon`], the destination is
/// `{covers_dir()}/{game_id}.ico`, and the write is atomic — a temporary
/// `{game_id}.ico.tmp` beside the target, renamed into place — so an
/// interrupted save leaves the previous cover (or nothing), never half a file.
pub fn save_exe_icon(exe_path: &Path, game_id: &str) -> Result<PathBuf, SaveIconError> {
    save_exe_icon_to(exe_path, game_id, &crate::paths::covers_dir())
}

/// [`save_exe_icon`] into an explicit directory, as the reference's tests do
/// by patching `config.covers_dir` (`tests/test_covers.py:142-146`).
pub fn save_exe_icon_to(
    exe_path: &Path,
    game_id: &str,
    covers_dir: &Path,
) -> Result<PathBuf, SaveIconError> {
    let icon = crate::exe_icons::extract_icon(exe_path).ok_or_else(|| SaveIconError::NoIcon {
        exe: exe_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })?;
    let destination = covers_dir.join(format!("{game_id}.ico"));
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("ico.tmp");
    std::fs::write(&temporary, icon)?;
    std::fs::rename(&temporary, &destination)?;
    Ok(destination)
}

pub fn accent_index_in(seed: &str, buckets: usize) -> usize {
    let buckets = buckets.max(1);
    let digest = crate::hash::sha256_hex(seed.as_bytes());
    // The digest is hex; the first byte is its first two characters. Taking the
    // byte rather than parsing a `u64` keeps this equal to Python's
    // `digest[0]` for every seed, including the ones whose first byte is above
    // `0x7f` — where a naive `hex[0]` would be an ASCII letter.
    let first = u8::from_str_radix(&digest[0..2], 16)
        .expect("sha256_hex always writes at least two hex characters");
    usize::from(first) % buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_seed_is_hashed_rather_than_short_circuited() {
        // `(seed or "")` hashes the empty string. The value below is CPython's:
        // `int.from_bytes(hashlib.sha256(b"").digest()[:1])` is 227, and
        // `227 % 8` is 3. A port that returned 0 for the empty seed would put
        // every unnamed game in the first shade, and this is the assertion that
        // says so.
        assert_eq!(accent_index(""), 3);
        assert_ne!(accent_index(""), 0);
    }

    #[test]
    fn the_bucket_is_the_first_digest_byte_not_the_first_hex_digit() {
        // `sha256("")` is `e3b0c442…`. Its first byte is `0xe3` = 227, bucket 3.
        // Reading the digest as text and taking `'e'` would give 14, bucket 6 —
        // a value this test rejects. The two differ only because 227 and 14 are
        // incongruent mod 8, so the case has to be one where they are.
        assert_eq!(crate::hash::sha256_hex(b"")[0..2], *"e3");
        assert_eq!(accent_index(""), 227 % COVER_ACCENTS);
    }

    #[test]
    fn known_seeds_land_in_cpython_s_buckets() {
        // Every pair below was produced by running the Python implementation.
        // They are here as a smoke test with readable values; the exhaustive
        // check is the vector corpus, which carries the same cases plus the
        // awkward ones (non-ASCII, long ids, the clamp).
        assert_eq!(accent_index("a"), 2);
        assert_eq!(accent_index("bb"), 3);
        assert_eq!(accent_index("ccc"), 4);
        assert_eq!(accent_index("game-a"), 7);
        assert_eq!(accent_index("a-very-long-id"), 3);
        assert_eq!(accent_index("Half-Life"), 5);
        assert_eq!(accent_index("0".repeat(32).as_str()), 4);
    }

    #[test]
    fn a_non_ascii_seed_is_hashed_as_utf8() {
        // The seed is a game id or name, either of which can be non-ASCII. The
        // encoding is what makes the bucket reproducible across machines, so it
        // is pinned rather than assumed: CPython's `"😀".encode("utf-8")` is
        // `f0 9f 98 80` and this crate hashes the same four bytes.
        assert_eq!(accent_index("\u{1f600}"), 0);
        assert_eq!(crate::hash::sha256_hex("\u{1f600}".as_bytes())[0..2], *"f0");
    }

    #[test]
    fn the_result_is_always_a_usable_shade_index() {
        // The property that matters to the view: whatever the seed, the value
        // indexes `COVER_GRADIENTS` without wrapping. `shade` wraps defensively,
        // but it should never have to.
        for seed in ["", "a", "\u{1f600}", &"x".repeat(500)] {
            assert!(
                accent_index(seed) < COVER_ACCENTS,
                "{seed:?} escaped the range"
            );
        }
    }

    #[test]
    fn the_same_seed_always_gets_the_same_shade() {
        // Stability is the whole point of hashing here: a tile that changed
        // colour between sessions would be worse than a shared colour.
        let first = accent_index("Half-Life");
        for _ in 0..8 {
            assert_eq!(accent_index("Half-Life"), first);
        }
    }

    #[test]
    fn a_bucket_count_of_zero_is_clamped_rather_than_dividing_by_zero() {
        // `max(1, buckets)` in Python. Reaching this needs an explicit count,
        // which is why `accent_index_in` is public — a caller that computed zero
        // from a config file must not panic.
        assert_eq!(accent_index_in("a", 0), 0);
        assert_eq!(accent_index_in("a", 1), 0);
        for buckets in [2usize, 3, 5, 8, 100] {
            assert!(accent_index_in("a", buckets) < buckets);
        }
        // The value for a real bucket count is still Python's.
        assert_eq!(accent_index_in("a", 3), 1);
    }

    #[test]
    fn seeds_that_look_alike_do_not_share_a_shade_by_accident() {
        // Guards against a stand-in that keys off the *shape* of the string
        // rather than its hash — the view's placeholder did exactly that before
        // it was wired up, and its own test caught it. The seeds here are
        // deliberately pairwise different in length as well as content, so a
        // `len() % buckets` implementation cannot pass.
        let seeds = ["", "a", "bb", "ccc", "game-a", "a-very-long-id"];
        let buckets: Vec<usize> = seeds.iter().map(|seed| accent_index(seed)).collect();
        let mut distinct = buckets.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() > 1,
            "every seed got the same bucket ({buckets:?}) — this is not hashing"
        );
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gh-covers-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// A real PE carrying one icon (`tests/test_covers.py:24-29`).
    fn windows_executable(path: &Path) -> PathBuf {
        use crate::exe_icons::builders::{build_pe, dib_icon, group_icon, resource_section};
        let payload = dib_icon(32, 0x5A);
        let section = resource_section(
            &[(1, payload.clone())],
            &group_icon(&[(32, payload.len(), 1)]),
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the exe directory");
        }
        std::fs::write(path, build_pe(&section, false, None)).expect("write the fixture");
        path.to_path_buf()
    }

    #[test]
    fn saves_the_executables_icon_into_the_covers_directory() {
        let root = scratch_dir("save");
        let covers = root.join("covers");
        let exe = windows_executable(&root.join("Steam").join("steam.exe"));
        let path = save_exe_icon_to(&exe, "abc123", &covers).expect("a saved icon");
        assert_eq!(path, covers.join("abc123.ico"));
        let saved = std::fs::read(&path).expect("read the saved icon");
        assert!(
            saved.starts_with(b"\x00\x00\x01\x00"),
            "an .ico header, not whatever the exe happened to hold"
        );
        // Saving is atomic: the temporary file is renamed away, not left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&covers)
            .expect("read the covers directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a .tmp file was left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_executable_without_an_icon_is_reported_not_silently_empty() {
        let root = scratch_dir("noicon");
        let covers = root.join("covers");
        let bare = root.join("bare.exe");
        let mut bytes = b"MZ".to_vec();
        bytes.resize(4098, 0);
        std::fs::write(&bare, bytes).expect("write the fixture");
        let error = save_exe_icon_to(&bare, "abc123", &covers).expect_err("no icon to save");
        assert!(
            error.to_string().contains("no icon"),
            "the reference's message, byte for byte: {error}"
        );
        assert_eq!(
            error.to_string(),
            "bare.exe carries no icon to use as a cover"
        );
        assert!(!covers.join("abc123.ico").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_executable_names_itself_in_the_error() {
        // `extract_icon` folds a missing file into `None`, so the error is
        // still `NoIcon` — and it still names the file, as `Path(name).name`
        // does when there is a name to take.
        let root = scratch_dir("missing");
        let error = save_exe_icon_to(&root.join("gone.exe"), "abc123", &root.join("covers"))
            .expect_err("no file, no icon");
        assert_eq!(
            error.to_string(),
            "gone.exe carries no icon to use as a cover"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
