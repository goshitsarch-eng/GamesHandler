//! Sizes and spacing, in numbers the layout tests can assert on.
//!
//! These live in the view crate rather than in `gamehandler-core` because they
//! are not behaviour: nothing in `models.py` or `covers.py` knows how tall a
//! library tile is. They are collected here so the numbers appear once and the
//! arithmetic over them is testable without a renderer.
//!
//! # Where the values come from
//!
//! The **structure** is ported from the QML and the **numbers** are mostly
//! kept, because they encode two things worth preserving:
//!
//! - `GRID_CELL` is 200×300 (`LibraryPage.qml:139-140`). The 2:3 ratio is not
//!   arbitrary — it is the aspect the store art in `COVER_ASSETS`
//!   (`covers.py:35-43`) is authored for, so changing it starts cropping or
//!   letterboxing art that currently fills the tile exactly.
//! - the radii and the icon inset are `LibraryPage.qml:155` and
//!   `CoverArt.qml`'s `radius` / `largeSpacing`.
//!
//! # The one number that is an estimate
//!
//! Kirigami derived its sizes from a font-relative unit — `gridUnit * 3.4` for
//! a list row, `gridUnit * 2.8` for the cover in it — and COSMIC has no such
//! unit. Rather than scatter the expression's results as four unexplained
//! literals, the QML's expressions are kept and [`GRID_UNIT`] is Kirigami's
//! default of 18, which is what the app was rendering at. That is an estimate
//! of what the QML looked like, not a translation of it, and it is the one
//! thing here to check against a screenshot.

/// Kirigami's default font-relative unit, which the QML's sizes are
/// expressions over.
///
/// See the module note: this is an estimate, and it is the only number here
/// that is not read directly off the QML.
pub const GRID_UNIT: f32 = 18.0;

/// Width and height of one library cell, in logical pixels.
///
/// Ported from `LibraryPage.qml:139-140`; see the module note on the 2:3 ratio.
pub const GRID_CELL: (f32, f32) = (200.0, 300.0);

/// Corner radius of a library tile — `LibraryPage.qml:155`.
pub const CARD_RADIUS: f32 = 14.0;

/// Corner radius of a cover in the full-size grid tile — `CoverArt.qml`
/// (`radius: compact ? 6 : 10`).
pub const TILE_RADIUS: f32 = 10.0;

/// Corner radius of a cover in a list row, where the art is small —
/// `CoverArt.qml`'s `compact` branch.
pub const COMPACT_RADIUS: f32 = 6.0;

/// Inset around an icon-derived cover, so an app icon does not touch the tile
/// edges. `CoverArt.qml`: `anchors.margins: coverIsIcon ? largeSpacing : 0`.
pub const ICON_INSET: f32 = 18.0;

/// Inset of a tile's contents from its edge. `LibraryPage.qml` used
/// `Kirigami.Units.smallSpacing`; the value is ours, the structure is ported.
pub const CARD_MARGIN: f32 = 6.0;

/// Height of one list row — `LibraryPage.qml:237`, `gridUnit * 3.4`.
pub const LIST_ROW_HEIGHT: f32 = GRID_UNIT * 3.4;

/// Width of the cover inside a list row — `LibraryPage.qml:250`,
/// `gridUnit * 2`.
pub const LIST_COVER_WIDTH: f32 = GRID_UNIT * 2.0;

/// Height of the cover inside a list row — `LibraryPage.qml:250`,
/// `gridUnit * 2.8`.
pub const LIST_COVER_HEIGHT: f32 = GRID_UNIT * 2.8;

/// Width of the form's cover preview — `GameFormPage.qml:141`,
/// `gridUnit * 2`.
pub const FORM_PREVIEW_WIDTH: f32 = GRID_UNIT * 2.0;

/// Height of the form's cover preview — `GameFormPage.qml:142`,
/// `gridUnit * 3`.
pub const FORM_PREVIEW_HEIGHT: f32 = GRID_UNIT * 3.0;

/// iced's default line height for a text run, as a factor of its size.
///
/// `LineHeight::default()` is `Relative(1.4)` (`iced/core/src/text.rs:244-248`)
/// and `Relative(factor)` resolves to `Pixels(factor * text_size.0)` (`:238`),
/// so a run's height is its size times this and nothing else. Cited rather than
/// measured because it is what lets [`CARD_TEXT_BLOCK_HEIGHT`] be a `const`: a
/// number read off one rendered card would be a measurement of the font stack,
/// and this check has to hold for every stack.
pub const LINE_HEIGHT_RATIO: f32 = 1.4;

/// The size a game's name is set at, in a card and in a list row.
///
/// Moved here from the builders, where it was a literal inside
/// `view::widgets::name_and_subtitle`. A size that a *height* is derived from is
/// a number in two places the moment anything else asks how tall the block is —
/// which is exactly what [`CARD_CHROME_HEIGHT`] does.
pub const CARD_NAME_SIZE: f32 = 14.0;

/// The size the subtitle under a game's name is set at.
pub const CARD_SUBTITLE_SIZE: f32 = 11.0;

/// The gap between a game's name and the subtitle under it.
///
/// # This is the **row's** spacing, and the card's is not the same number
///
/// 2.0 is the reference's own literal for the list row: its `ColumnLayout` sets
/// `spacing: 2` (`LibraryPage.qml:260`). The **card's** two labels sit in a
/// `ColumnLayout` whose spacing is `Kirigami.Units.smallSpacing`
/// (`:168-171`) — 6, the same value [`CARD_MARGIN`] already carries — so the card
/// draws its two lines **4 pixels closer together than the reference does**, and
/// the block is 37.0 where the reference's is 41.0.
///
/// It is shared by both widgets rather than split in two, and that is recorded
/// here rather than silently repaired: changing the card's copy moves its cover
/// box again (207 → 203), which is a parity change of its own and not part of
/// #96. Whoever picks it up should split this into a card constant and a row
/// constant rather than re-pointing one value at two widgets — the two QMLs
/// genuinely disagree, and one number cannot be right for both.
pub const CARD_TEXT_SPACING: f32 = 2.0;

/// The height a card's or row's name-and-subtitle block takes.
///
/// Two lines of text and the gap between them, and it is **exact** rather than
/// approximate: [`LINE_HEIGHT_RATIO`] makes a line's height arithmetic over its
/// size, so this needs no font stack to evaluate. At 14.0 and 11.0 it is
/// 19.6 + 2.0 + 15.4 = 37.0, which is what a card lays out to — measured, in
/// `view::widgets`'s `a_long_name_does_not_squeeze_the_play_control`.
pub const CARD_TEXT_BLOCK_HEIGHT: f32 = CARD_NAME_SIZE * LINE_HEIGHT_RATIO
    + CARD_TEXT_SPACING
    + CARD_SUBTITLE_SIZE * LINE_HEIGHT_RATIO;

/// The height the Play control lays out to at the layout tests' text size.
///
/// **The one number in this module that is another crate's, and the only thing
/// here that is measured rather than derived.** libcosmic's `button::standard`
/// is `height: Length::Shrink` with `padding: Padding::new(5.0)`
/// (`src/widget/button/widget.rs:73-75`), so its height is its content row plus
/// its padding — it has no height of its own that this module could compute.
/// **Measured: 32.0**, at the text size the layout tests build their renderer
/// with (`view::widgets`'s `renderer`, `Pixels(16.0)`), of which the label is
/// 20.0 and the icon 16.0.
///
/// It is not left unguarded for that reason:
/// `view::widgets`'s `the_play_control_is_the_height_the_card_reserves` lays a
/// real `button::standard` out and asserts its own height is exactly this, and
/// then lays out a real card and asserts the control inside it gets exactly this
/// much room — so a libcosmic change that moves it fails a test naming this
/// constant rather than quietly re-running #96 below.
///
/// # What this constant was before #96, and why that was wrong
///
/// It was `21.0`, described as "its padding plus its tallest child". That is not
/// what 21 was. It was **the space left over** after a 218-tall cover, two
/// 6-pixel gaps and a 37-tall text block — a number defined by the thing it was
/// supposed to be the input to, which is why the card drew the control 11 pixels
/// shorter than it asks for and squeezed its padding from 5.0 to 0.5. Both
/// numbers were visible in the same app: the row drew the same control at 32.0.
///
/// # The residual, which no const can close
///
/// The app's theme may set a different text size from the tests', and a theme
/// whose button is *taller* than this would re-introduce a squeezed control —
/// the card's column is a fixed 300 pixels and the deficit goes to its last
/// child. That is not checkable from here, and it is the reason the measured
/// guard is `view::widgets`'s layout test rather than this line: it pins the
/// number to a renderer whose text size is written down.
pub const PLAY_BUTTON_HEIGHT: f32 = 32.0;

/// What a card's name, subtitle, play control and the gaps between them take
/// out of its cell. The cover gets the rest.
///
/// # This is a derivation, not the leftover
///
/// [`CARD_MARGIN`] twice — the column's two gaps, cover → text and text →
/// control — over [`CARD_TEXT_BLOCK_HEIGHT`] and [`PLAY_BUTTON_HEIGHT`]:
/// 12.0 + 37.0 + 32.0 = **81.0**. It was a bare `70.0` until #96, described as
/// "an estimate, not a port", which is the shape of a number nobody can check.
///
/// The 11-pixel difference is the "before" of this constant's own defect: 70.0
/// is what a chrome comes to when the control's height is *defined* as what is
/// left over rather than measured. Re-deriving it from its parts made the card's
/// cover 207.0 tall instead of 218.0 — 5% shorter, which is the visible cost of
/// no longer drawing the Play control 11 pixels shorter than it asks for. See
/// [`PLAY_BUTTON_HEIGHT`], and note that the reference has the same structure:
/// its cover is `Layout.fillHeight: true` (`LibraryPage.qml:174-175`), so the
/// cover absorbs and the control is never squeezed.
///
/// # Why the arithmetic is worth this much prose: there is no slack
///
/// A cell's interior is 300 − 2×6 = 288 and the four parts sum to exactly 288:
/// cover 207, gap 6, text 37, gap 6, control 32. **Zero.** iced's `Column` gives
/// a deficit to its last child, so *anything* that makes the text block taller
/// is taken out of the Play control — and a name that wrapped to a second line
/// did exactly that. On "The Elder Scrolls V Skyrim Special Edition" the control
/// laid out at **1.4000015 px**: nothing failed, the card compiled and rendered,
/// and a click still published, so the only symptom was a Play button with
/// nothing legible in it.
///
/// So the chrome is still exact, deliberately: it is what makes one part growing
/// show up in one place. The fix for the *name* is in
/// `view::widgets::name_and_subtitle`, which is where the block was allowed to
/// grow: the name and the subtitle are single-line, which is what the reference
/// does (`LibraryPage.qml`'s card labels set `elide: Text.ElideRight` and no
/// `wrapMode`, and `QQC2.Label`'s default is `Text.NoWrap`). The checks below are
/// the compile-time half; the measured half is that test.
pub const CARD_CHROME_HEIGHT: f32 =
    CARD_TEXT_BLOCK_HEIGHT + 2.0 * CARD_MARGIN + PLAY_BUTTON_HEIGHT;

/// A card's chrome must leave a cover behind it.
///
/// The card's counterpart of the list row's checks below, and it is the one that
/// would have caught #96 had the chrome been the thing that grew. It bounds the
/// reserve rather than the drawing — nothing const can know how tall a name
/// lays out, so the drawing is `view::widgets`'s
/// `a_long_name_does_not_squeeze_the_play_control`, and this is what fails first
/// if someone raises the chrome until the cover is squeezed to nothing.
const _: () = assert!(
    CARD_CHROME_HEIGHT < GRID_CELL.1 - 2.0 * CARD_MARGIN,
    "a card's chrome must leave room for its cover inside the cell"
);

/// The chrome must cover the parts it is a sum of.
///
/// Tautological while [`CARD_CHROME_HEIGHT`] is defined as that sum, and written
/// anyway: it is what fails if the definition is ever replaced by a literal, or
/// if a part grows while the total does not. `clippy::assertions_on_constants`
/// accepted the row's sibling checks for the same reason — the alternative was a
/// runtime `assert!` over two constants, which can only ever report what the
/// compiler already knew.
const _: () = assert!(
    CARD_CHROME_HEIGHT >= CARD_TEXT_BLOCK_HEIGHT + 2.0 * CARD_MARGIN + PLAY_BUTTON_HEIGHT,
    "a card's chrome must cover its text block, its gaps and its Play control"
);

/// A list row must leave room for the text beside its cover.
///
/// A compile-time check rather than a test, because both operands are
/// compile-time constants: a build that gets them the wrong way round is
/// broken, not merely failing, and a runtime `assert!` over two constants can
/// never tell you anything the compiler did not already know. (This was a
/// test until `clippy::assertions_on_constants` pointed out exactly that.)
const _: () = assert!(
    LIST_COVER_HEIGHT < LIST_ROW_HEIGHT,
    "a list row's cover must be shorter than the row it sits in"
);

/// The gap a list row leaves for its text must be big enough to set a name and
/// a subtitle in.
///
/// A `const {}` for the same reason as the check above: `LIST_ROW_HEIGHT -
/// LIST_COVER_HEIGHT` is a constant, so a runtime `assert!` over it can only
/// ever report something the compiler already knew. It was a test until
/// `clippy::assertions_on_constants` said so.
const _: () = assert!(
    LIST_ROW_HEIGHT - LIST_COVER_HEIGHT >= 10.0,
    "a list row must leave room for the name and the subtitle"
);

/// The form's cover preview is a portrait, taller than it is wide
/// (`GameFormPage.qml:141-142`).
const _: () = assert!(
    FORM_PREVIEW_HEIGHT > FORM_PREVIEW_WIDTH,
    "the form's cover preview must be a portrait"
);

/// Width-to-height ratio of a library tile.
///
/// Derived from [`GRID_CELL`] rather than written as its own number: the tile
/// is one shape, and two independent constants is two chances for the grid and
/// the single-tile view to disagree about it. The 2:3 portrait is set by the
/// store art in `COVER_ASSETS` (`covers.py:35-43`), which is authored for it.
pub const PORTRAIT_RATIO: f32 = GRID_CELL.1 / GRID_CELL.0;

/// The height of a tile of the given width — see [`PORTRAIT_RATIO`].
pub fn tile_height(width: f32) -> f32 {
    width * PORTRAIT_RATIO
}

/// The cover's box inside a library card.
///
/// The card is a fixed cell; the cover takes what the text block and the
/// padding do not. Both subtractions are clamped at zero rather than allowed
/// to go negative: a cell shorter than its own chrome is a layout bug, and a
/// negative size would reach the renderer as a confusing error instead of an
/// empty box.
pub fn card_cover_box(cell: (f32, f32), chrome_height: f32, margin: f32) -> (f32, f32) {
    let (cell_w, cell_h) = cell;
    let width = (cell_w - 2.0 * margin).max(0.0);
    let height = (cell_h - 2.0 * margin - chrome_height).max(0.0);
    (width, height)
}

/// The font size for a plate's initials.
///
/// Ported from `CoverArt.qml`:
/// `compact ? Math.round(height * 0.42) : Math.round(Math.min(width, height) * 0.34)`.
/// The `round` is kept because the QML rounds, and a fractional pixel size is
/// resolved by the font stack rather than by the layout.
pub fn initials_size(width: f32, height: f32, compact: bool) -> f32 {
    if compact {
        (height * 0.42).round()
    } else {
        (width.min(height) * 0.34).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cover fills the cell's width less its margins, and the cell's height
    /// less the margins and the chrome below it. Both halves are asserted, so a
    /// change that mixed up the axes fails rather than passing on one number.
    #[test]
    fn a_cover_takes_the_cell_minus_its_margins_and_chrome() {
        let (w, h) = card_cover_box(GRID_CELL, 70.0, CARD_MARGIN);
        assert_eq!(w, 200.0 - 12.0);
        assert_eq!(h, 300.0 - 12.0 - 70.0);
    }

    /// The clamp is the whole reason this is a function rather than two
    /// subtractions at the call site. Without it the height is negative.
    #[test]
    fn a_cell_shorter_than_its_chrome_yields_an_empty_box_not_a_negative_one() {
        let (w, h) = card_cover_box((200.0, 40.0), 70.0, CARD_MARGIN);
        assert_eq!(w, 188.0);
        assert_eq!(h, 0.0, "a negative height would reach the renderer as an error");
    }

    /// A cell narrower than twice its margin is degenerate for the same reason.
    #[test]
    fn a_cell_narrower_than_its_margins_yields_an_empty_box() {
        let (w, _) = card_cover_box((10.0, 300.0), 70.0, CARD_MARGIN);
        assert_eq!(w, 0.0);
    }

    /// The box tracks the cell: growing the cell by *n* grows the cover by *n*,
    /// and growing the chrome shrinks it by the same. Pinning the direction as
    /// well as the value is what catches a swapped operand.
    #[test]
    fn the_box_moves_with_the_cell_and_against_the_chrome() {
        let base = card_cover_box(GRID_CELL, 70.0, CARD_MARGIN);
        let wider = card_cover_box((GRID_CELL.0 + 20.0, GRID_CELL.1), 70.0, CARD_MARGIN);
        let taller_chrome = card_cover_box(GRID_CELL, 90.0, CARD_MARGIN);
        assert_eq!(wider.0, base.0 + 20.0);
        assert_eq!(taller_chrome.1, base.1 - 20.0);
        assert_eq!(wider.1, base.1, "a wider cell must not change the height");
    }

    /// The list row's cover is smaller than the row, and the gap it leaves is
    /// usable.
    ///
    /// Both are compile-time facts and are checked by the `const {}` blocks
    /// above, which are strictly stronger than a test — they stop the crate
    /// building rather than failing one case. What is asserted here is the
    /// part a relation cannot state: the resolved numbers, so that a change to
    /// the estimate is visible against the QML's own arithmetic rather than
    /// only against itself.
    #[test]
    fn the_list_sizes_are_the_qmls_arithmetic_at_the_default_grid_unit() {
        // `LibraryPage.qml:237` is `gridUnit * 3.4`; `:250` is `gridUnit * 2`
        // by `gridUnit * 2.8`; `GameFormPage.qml:141-142` is `gridUnit * 2` by
        // `gridUnit * 3`. At Kirigami's default of 18 those are these numbers,
        // written out — NOT as `GRID_UNIT * 3.4`, which would be a restatement
        // of the constant's own definition and could never fail.
        assert_eq!(GRID_UNIT, 18.0);
        assert_eq!(LIST_ROW_HEIGHT, 61.2); // 18 * 3.4 is exact in f32 here
        assert_eq!(LIST_COVER_WIDTH, 36.0);
        assert_eq!(FORM_PREVIEW_WIDTH, 36.0);
        assert_eq!(FORM_PREVIEW_HEIGHT, 54.0);

        // Two of them are **not** exact, and the reason is worth knowing: QML
        // evaluates `gridUnit * 2.8` in a double and gets 50.4, while f32
        // cannot represent 2.8 and lands on 50.399998. That is 2e-6 of a
        // pixel — it will never be visible — but it means the port's list
        // cover is not bit-identical to the QML's, so the assertion is a
        // tolerance rather than an equality. Measured, not guessed: `rustc`
        // prints `18.0f32 * 2.8f32` as `50.399998`.
        assert!(
            (LIST_COVER_HEIGHT - 50.4).abs() < 1e-4,
            "expected the f32 evaluation of 18 * 2.8 to be within a ten-thousandth of 50.4, got {LIST_COVER_HEIGHT}"
        );
        assert!(
            (LIST_ROW_HEIGHT - LIST_COVER_HEIGHT - 10.8).abs() < 1e-4,
            "the row's gap for text should be 10.8, got {}",
            LIST_ROW_HEIGHT - LIST_COVER_HEIGHT
        );
    }

    /// A tile is the 2:3 portrait the store art is authored for, and
    /// [`tile_height`] reproduces the cell exactly at the cell's own width.
    #[test]
    fn a_tile_is_the_two_to_three_portrait_of_its_width() {
        assert_eq!(PORTRAIT_RATIO, 1.5);
        assert_eq!(tile_height(GRID_CELL.0), GRID_CELL.1);
        assert_eq!(tile_height(100.0), 150.0);
    }

    /// The initials scale with the tile, and the compact rule keys off the
    /// height while the full rule keys off the shorter side. Asserted at a
    /// tile where the two rules disagree, so a swap of the branches fails.
    #[test]
    fn initials_scale_with_the_tile_and_the_compact_rule_differs() {
        let (w, h) = (200.0, 300.0);
        assert_eq!(initials_size(w, h, false), 68.0); // 200 * 0.34
        assert_eq!(initials_size(w, h, true), 126.0); // 300 * 0.42
        assert_ne!(
            initials_size(w, h, false),
            initials_size(w, h, true),
            "a compact tile must not set its initials at the full size"
        );
        // Bigger tile, bigger initials.
        assert!(initials_size(100.0, 150.0, false) < initials_size(w, h, false));
    }

    /// The full rule takes the *shorter* side, so a tile wider than it is tall
    /// does not overflow. This is the case the QML's `Math.min` exists for.
    #[test]
    fn initials_use_the_shorter_side_of_a_wide_tile() {
        assert_eq!(initials_size(300.0, 100.0, false), 34.0); // 100 * 0.34, not 300 * 0.34
    }
}
