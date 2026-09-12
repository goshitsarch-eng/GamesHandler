//! Cover art: where the bytes come from, how they fit, and what stands in for
//! them when there are none.
//!
//! # The rule this module exists to hold
//!
//! **A cover's file extension does not describe its contents** (FINDINGS F-N).
//! `save_cover_from_urls` (`covers.py:283-292`) writes every downloaded
//! candidate to a *hardcoded* `<id>.jpg`, while the candidate list
//! (`COVER_ASSETS`, `covers.py:35-43`) includes `portrait.png` — so PNG bytes
//! in a file called `.jpg` is a **normal outcome**, not a corner case. (F-N
//! also records that `_request` sends `Accept: */*`, so nothing negotiates a
//! format either.) A `.webp` is a user's own import and an `.ico` comes from an
//! executable's resources.
//!
//! So nothing here decides *whether* or *how* to decode from a suffix. The
//! decode is left to the image loader, which sniffs: iced opens covers through
//! `ImageReader::open(path).with_guessed_format()` (verified in the vendored
//! `iced_graphics/src/image.rs:126-131`), and D-29 turns on the codecs that
//! make webp and gif decodable. A Rust `is_this_really_a_jpeg` check, or a
//! format picked from the suffix, would reintroduce in the port exactly the bug
//! F-N records in the reference.
//!
//! # Why `.ico` is still special, and why it is not an exception to that rule
//!
//! An icon extracted from an executable must be **letterboxed**, not cropped:
//! it is square and mostly transparent, and cropping it to a 2:3 tile cuts the
//! logo in half (`CoverArt.qml` says the same). That is a *layout* choice, and
//! it needs to be made before any decode happens.
//!
//! The reference makes it from the suffix — `bridge.py:313` is
//! `cover.lower().endswith(".ico")`. This port makes it from the **content**,
//! by reading the ICO magic, for two reasons:
//!
//! - it is the same rule as everywhere else in this module, so there is one
//!   thing to remember rather than two;
//! - the suffix is the less reliable signal. `copy_custom_cover`
//!   (`covers.py:300-301`) keeps a suffix only from
//!   `{".png", ".jpg", ".jpeg", ".webp"}` and otherwise writes `.jpg`, so a
//!   user who picks an `.ico` as a custom cover gets ICO bytes in a `.jpg`
//!   file — which the reference would crop (wrongly — an icon is still square)
//!   and this port letterboxes.
//!
//! The two agree on every file the reference can produce today, because
//! `save_exe_icon` is the only writer of `.ico` (`covers.py:312`) and it writes
//! real ICO bytes. They differ only where the reference is wrong.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use cosmic::iced::ContentFit;

/// The placeholder's text, from `GameFormPage.qml:148-151`.
///
/// This belongs to the **game form's cover picker**, and not to a library
/// tile. `GameFormPage.qml` puts it where a 36×54 preview would be when there
/// is nothing to show; the library plate in `CoverArt.qml` draws the game's
/// initials and no words at all. The two are one function call apart in
/// [`super::widgets`], so the distinction is pinned by a test there.
pub const NO_COVER_LABEL: &str = "No cover yet";

/// How many placeholder shades there are. `covers.py:95` (`COVER_ACCENTS`).
///
/// Re-exported from `core` rather than declared here, so the count and
/// [`accent_index`]'s bucketing cannot drift apart — they are two halves of one
/// rule, and a second copy of the number is a second place to change it.
///
/// [`accent_index`]: gamehandler_core::covers::accent_index
pub use gamehandler_core::covers::COVER_ACCENTS;

/// The eight tile gradients, as `CoverArt.qml`'s `gradients` array lists them.
///
/// Kept in the same order and with the same values: the shade is a function of
/// the game's id (`covers.py:107`), so a game's tile keeps its colour across
/// sessions — and across the migration, which is why these are copied rather
/// than re-chosen.
pub const COVER_GRADIENTS: [([u8; 3], [u8; 3]); COVER_ACCENTS] = [
    ([0x3f, 0x6f, 0xd8], [0x23, 0x40, 0x7f]),
    ([0x8a, 0x4f, 0xd6], [0x4b, 0x23, 0x80]),
    ([0x1f, 0x8f, 0x78], [0x10, 0x51, 0x3f]),
    ([0xc1, 0x53, 0x3f], [0x6f, 0x2a, 0x20]),
    ([0xb8, 0x86, 0x2c], [0x6d, 0x4a, 0x12]),
    ([0x2c, 0x7f, 0xa8], [0x16, 0x4b, 0x64]),
    ([0xa4, 0x40, 0x6e], [0x5d, 0x1f, 0x3c]),
    ([0x4c, 0x6b, 0x8a], [0x2a, 0x3c, 0x50]),
];

/// The first four bytes of every ICO file: reserved 0, type 1 (icon).
///
/// A cursor is type 2 and is deliberately not accepted — `extract_icon`
/// (`exe_icons.py:238`) reassembles `RT_GROUP_ICON`, which is always type 1.
const ICO_MAGIC: [u8; 4] = [0x00, 0x00, 0x01, 0x00];

/// What a game's cover should be drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverSource {
    /// No cover at all: initials on a gradient plate.
    Plate,
    /// Store art or a user's own image. Fills the tile; cropped if the aspect
    /// differs, because a photo that is letterboxed reads as a broken tile.
    Photo,
    /// An icon extracted from the game's executable. Sits on the plate,
    /// uncropped and inset.
    Icon,
}

impl CoverSource {
    /// Decide how to draw `cover_path`.
    ///
    /// The path is otherwise opaque: this reads four bytes to tell an icon
    /// from a photograph and never inspects the suffix. See the module docs.
    pub fn classify(cover_path: &str) -> Self {
        // The reference checks this too (`bridge.py:311`): a game can carry a
        // `cover_path` that has since been deleted, or moved with the library.
        if cover_path.is_empty() || !Path::new(cover_path).is_file() {
            return Self::Plate;
        }
        if has_ico_magic(cover_path) {
            Self::Icon
        } else {
            Self::Photo
        }
    }

    /// Whether there are bytes to hand to the image loader.
    pub fn has_artwork(self) -> bool {
        !matches!(self, Self::Plate)
    }

    /// Whether the artwork is drawn inside the plate's bounds rather than
    /// filling them. Only icons are.
    pub fn is_letterboxed(self) -> bool {
        matches!(self, Self::Icon)
    }

    /// How iced should fit the image to its box.
    ///
    /// [`ContentFit::Plate`] has no image, so its answer is unused; it returns
    /// [`ContentFit::Contain`] rather than panicking so a caller that renders
    /// the placeholder through the same path cannot take the app down.
    ///
    /// [`ContentFit::Plate`]: CoverSource::Plate
    pub fn content_fit(self) -> ContentFit {
        match self {
            Self::Photo => ContentFit::Cover,
            Self::Plate | Self::Icon => ContentFit::Contain,
        }
    }
}

/// The gradient for a placeholder, wrapping at [`COVER_ACCENTS`].
///
/// The wrap is defensive: `accent_index` (`covers.py:107-110`) returns
/// `digest[0] % buckets` and so is already in range, but the shape of this
/// function is what `CoverArt.qml` wrote (`gradients[Math.max(0, accent) %
/// gradients.length]`), and a caller with an index from anywhere else must not
/// be able to index out of bounds.
pub fn shade(accent: usize) -> ([u8; 3], [u8; 3]) {
    COVER_GRADIENTS[accent % COVER_ACCENTS]
}

/// Up to two uppercase initials for a placeholder plate.
///
/// [`gamehandler_core::covers::initials`], where the rule lives: the strings
/// are user-visible and must match `covers.py:97-104`, which makes them a
/// compatibility surface, and those live in `core` — the same reason
/// [`accent_of`] reaches through to [`gamehandler_core::covers::accent_index`].
pub fn initials(name: &str) -> String {
    gamehandler_core::covers::initials(name)
}

/// The plate shade index for a game.
///
/// `covers.py:107-110`, reached through
/// [`gamehandler_core::covers::accent_index`] — where the rule lives, because
/// it is a compatibility surface: a game's tile has a colour that must not
/// change between the Python app and this one. The hash stays private to
/// `core`, so a caller here cannot re-derive the bucketing and drift.
///
/// The seed is the game's id in every current caller, which is what makes a
/// tile keep its shade across sessions *and* across a rename.
pub fn accent_of(seed: &str) -> usize {
    gamehandler_core::covers::accent_index(seed)
}

/// Does this file begin with the ICO magic?
///
/// A file that cannot be opened, or is shorter than four bytes, is not an icon.
/// That is the safe answer: the else-branch crops, which is what the reference
/// does for everything it cannot call an icon, and an undecodable file is the
/// loader's error to report rather than this function's.
fn has_ico_magic(path: &str) -> bool {
    let mut magic = [0u8; 4];
    match File::open(path).and_then(|mut file| file.read_exact(&mut magic)) {
        Ok(()) => magic == ICO_MAGIC,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// The real leading bytes of each format, so the fixtures below are
    /// genuinely "PNG bytes" and not arbitrary filler that happens to be
    /// non-ICO. Only the prefix matters to `classify`, which reads four bytes.
    const PNG_PREFIX: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    const JPEG_PREFIX: &[u8] = &[0xff, 0xd8, 0xff, 0xe0];
    const WEBP_PREFIX: &[u8] = b"RIFF\x00\x00\x00\x00WEBPVP8 ";
    const ICO_PREFIX: &[u8] = &[0x00, 0x00, 0x01, 0x00, 0x01, 0x00];

    /// Write `bytes` to `name` inside a fresh directory and return its path.
    ///
    /// A per-test directory rather than one shared path, so the cases cannot
    /// interfere when the harness runs them in parallel.
    fn fixture(name: &str, label: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("gh-cover-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(label);
        let mut file = File::create(&path).expect("create the fixture");
        file.write_all(bytes).expect("write the fixture");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_game_with_no_cover_path_gets_the_plate() {
        assert_eq!(CoverSource::classify(""), CoverSource::Plate);
    }

    /// `bridge.py:311` guards on the file existing; a library moved without its
    /// covers must show plates rather than a broken image.
    #[test]
    fn a_cover_path_that_is_not_a_file_gets_the_plate() {
        assert_eq!(
            CoverSource::classify("/nonexistent/definitely/not/here.jpg"),
            CoverSource::Plate
        );
        // A directory is not a file either, and `is_file()` is what says so.
        let dir = std::env::temp_dir().join(format!("gh-cover-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        assert_eq!(
            CoverSource::classify(&dir.to_string_lossy()),
            CoverSource::Plate
        );
    }

    /// **The F-N case.** PNG bytes in a file named `.jpg` are the *normal*
    /// outcome of a Steam lookup, not an edge case, and they must be treated as
    /// a photograph.
    ///
    /// This test fails if `classify` starts consulting the extension: a
    /// "does a `.jpg` really hold JPEG?" check would reject this file, and a
    /// "PNG should be letterboxed" rule would return [`CoverSource::Icon`].
    #[test]
    fn png_bytes_in_a_jpg_named_file_are_a_photo() {
        let path = fixture("png-in-jpg", "1.jpg", PNG_PREFIX);
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// The same for webp, which is the format D-29 exists to decode. The
    /// *decoder* is the thing that must cope with webp; the classification must
    /// not care either way.
    #[test]
    fn webp_bytes_in_a_jpg_named_file_are_a_photo() {
        let path = fixture("webp-in-jpg", "2.jpg", WEBP_PREFIX);
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// And the reverse, which is what makes the rule "read the bytes" rather
    /// than "read the bytes for `.jpg` only": JPEG bytes under a `.png` name
    /// are still a photo.
    #[test]
    fn jpeg_bytes_in_a_png_named_file_are_a_photo() {
        let path = fixture("jpeg-in-png", "3.png", JPEG_PREFIX);
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// An icon is letterboxed whatever it is called. This is the case the
    /// reference gets wrong: an `.ico` picked as a custom cover is stored as
    /// `<id>.jpg` (`covers.py:300-301`), and `bridge.py:313`'s suffix test
    /// would crop it.
    #[test]
    fn ico_bytes_are_an_icon_whatever_the_file_is_called() {
        let honest = fixture("ico-honest", "4.ico", ICO_PREFIX);
        let mislabelled = fixture("ico-mislabelled", "4.jpg", ICO_PREFIX);
        assert_eq!(CoverSource::classify(&honest), CoverSource::Icon);
        assert_eq!(
            CoverSource::classify(&mislabelled),
            CoverSource::Icon,
            "an icon's layout must come from its bytes; the reference's suffix \
             test would crop this one"
        );
    }

    /// The other direction of the same rule: a file named `.ico` that is not an
    /// icon is cropped like any other photograph. This is the one place this
    /// port can differ from the reference, and it is recorded in the module
    /// docs — the divergence is only reachable for a file the reference cannot
    /// itself produce, since `save_exe_icon` (`covers.py:312`) is the only
    /// writer of `.ico` and it writes real ICO bytes.
    #[test]
    fn a_file_named_ico_that_is_not_an_icon_is_a_photo() {
        let path = fixture("fake-ico", "5.ico", PNG_PREFIX);
        assert_eq!(
            CoverSource::classify(&path),
            CoverSource::Photo,
            "the suffix must not be able to claim a file is an icon"
        );
    }

    /// A truncated file must not be mistaken for an icon, and must not panic.
    #[test]
    fn a_file_shorter_than_the_magic_is_a_photo() {
        let path = fixture("stub", "6.ico", &[0x00, 0x00]);
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// A cursor (type 2) is not an icon: `extract_icon` only ever produces
    /// `RT_GROUP_ICON`, which is type 1.
    #[test]
    fn a_cursor_is_not_an_icon() {
        let path = fixture("cursor", "7.cur", &[0x00, 0x00, 0x02, 0x00]);
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// Icons letterbox and photographs fill. Asserted through the named
    /// predicates *and* through the fit they produce, because the widget calls
    /// the latter and a future refactor could keep the predicates honest while
    /// swapping the match arms.
    #[test]
    fn icons_letterbox_and_photographs_fill() {
        assert!(CoverSource::Icon.is_letterboxed());
        assert!(!CoverSource::Photo.is_letterboxed());
        assert!(!CoverSource::Plate.is_letterboxed());
        assert_eq!(CoverSource::Icon.content_fit(), ContentFit::Contain);
        assert_eq!(CoverSource::Photo.content_fit(), ContentFit::Cover);
    }

    /// Only the plate has nothing to draw.
    #[test]
    fn only_the_plate_lacks_artwork() {
        assert!(!CoverSource::Plate.has_artwork());
        assert!(CoverSource::Photo.has_artwork());
        assert!(CoverSource::Icon.has_artwork());
    }

    /// The shade must be stable and in range for any input, including one past
    /// the end of the table.
    #[test]
    fn the_shade_wraps_and_is_total() {
        assert_eq!(shade(0), COVER_GRADIENTS[0]);
        assert_eq!(shade(COVER_ACCENTS - 1), COVER_GRADIENTS[COVER_ACCENTS - 1]);
        assert_eq!(shade(COVER_ACCENTS), COVER_GRADIENTS[0]);
        assert_eq!(shade(usize::MAX), COVER_GRADIENTS[usize::MAX % COVER_ACCENTS]);
    }

    /// Every entry must be a real two-stop gradient. A zero-filled row would
    /// render as a black tile that no other test would notice.
    #[test]
    fn every_gradient_has_two_distinct_non_black_stops() {
        assert_eq!(COVER_GRADIENTS.len(), COVER_ACCENTS);
        for (index, (top, bottom)) in COVER_GRADIENTS.iter().enumerate() {
            assert_ne!(top, bottom, "gradient {index} has no visible gradient");
            assert_ne!(*top, [0, 0, 0], "gradient {index} starts at black");
            assert_ne!(*bottom, [0, 0, 0], "gradient {index} ends at black");
        }
    }

    /// `initials` against the exact strings `covers.py:88-99` produces. The
    /// expectations are the Python's own outputs, not this implementation's.
    #[test]
    fn initials_match_the_reference() {
        assert_eq!(initials("Half-Life 2"), "HL");
        assert_eq!(initials("Celeste"), "CE");
        assert_eq!(initials("The Witcher 3: Wild Hunt"), "TW");
        assert_eq!(initials("  spaced   out  "), "SO");
        assert_eq!(initials("1234"), "12");
        assert_eq!(initials("a"), "A");
    }

    /// The empty and punctuation-only cases, which the reference answers with
    /// the same `"?"` `CoverArt.qml` defaults to.
    #[test]
    fn initials_of_nothing_are_a_question_mark() {
        assert_eq!(initials(""), "?");
        assert_eq!(initials("!!!"), "?");
        assert_eq!(initials(" - _ - "), "?");
    }

    /// The word split is ASCII-only, as Python's `[^0-9A-Za-z]+` is.
    ///
    /// `"Pokémon Go"` is the case that shows it, and the answer is the
    /// surprising one: the accent is a separator, so the words are
    /// `["Pok", "mon", "Go"]` and the initials are **"PM"**, not "PG". That is
    /// what `covers.py:88-99` produces — checked against the Python, not
    /// assumed — and it is exactly the kind of result a port silently
    /// "improves" by reaching for `char::is_alphanumeric`. Ugly, and faithful.
    #[test]
    fn initials_split_on_ascii_only_like_the_reference() {
        assert_eq!(initials("Pokémon Go"), "PM");
        assert_eq!(initials("é"), "?");
    }

    /// A one-character first word contributes one character, not a repeated
    /// one: the two-word branch takes `words[0][0] + words[1][0]`.
    #[test]
    fn a_one_letter_first_word_is_not_doubled() {
        assert_eq!(initials("A Plague Tale"), "AP");
    }

    /// The placeholder is gone: `accent_of` now returns *Python's* bucket.
    ///
    /// This replaces `the_plate_shade_is_a_placeholder_until_core_hashes`,
    /// which pinned the stand-in and was written to fail on the day the hash
    /// landed. It failed, as intended, and this is what it became.
    ///
    /// The values are CPython's, and the seeds are deliberately different in
    /// *length as well as content* — the original version of this test used
    /// `"game-a"` and `"game-b"`, both six characters, and so passed against a
    /// `seed.len() % COVER_ACCENTS` implementation. Any stand-in that keys off
    /// the shape of the string has to fail here.
    ///
    /// `""` is the interesting one: it is 3, not 0, because Python hashes the
    /// empty string rather than short-circuiting — so the *first* shade is not
    /// where nameless games land.
    #[test]
    fn the_plate_shade_is_pythons_hash_bucket() {
        for (seed, expected) in [
            ("", 3),
            ("a", 2),
            ("bb", 3),
            ("ccc", 4),
            ("game-a", 7),
            ("a-very-long-id", 3),
            ("\u{1f600}", 0),
        ] {
            assert_eq!(accent_of(seed), expected, "for {seed:?}");
        }
    }

    /// The view's index is core's index, with no local reinterpretation.
    ///
    /// A tautology today, and kept anyway: it fails if `accent_of` ever grows a
    /// transformation of its own — a modulo against this module's own count, a
    /// clamp, a fallback — which is exactly how the two would drift apart a
    /// second time.
    #[test]
    fn the_plate_shade_defers_to_core() {
        for seed in ["", "Half-Life", "game-a", "\u{1f600}"] {
            assert_eq!(
                accent_of(seed),
                gamehandler_core::covers::accent_index(seed)
            );
        }
    }

    /// Whatever [`accent_of`] becomes, its result must index a shade.
    #[test]
    fn the_plate_shade_is_always_in_range() {
        for seed in ["", "game-a", "a-very-long-id", "0"] {
            assert!(accent_of(seed) < COVER_ACCENTS);
        }
    }
}
