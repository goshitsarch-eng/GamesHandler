//! The shapes a game is drawn in: the cover, the library card, the list row
//! and the form's cover picker.
//!
//! Every function here is generic over the message type and produces none:
//! `M` is a parameter the caller fixes, never a value this layer constructs.
//! That is what lets the layout be written, compiled and tested before the
//! `Message` enum has a variant for "open this game" — and bound to one
//! afterwards without touching a line of this file.
//!
//! # Why these take `&Game` rather than fields
//!
//! A caller that passed `(name, subtitle, cover_path)` would be choosing what
//! a card shows, in the place least able to test it. Taking the model keeps
//! every visible decision in [`super::cover`], [`super::meta`] and
//! [`super::metrics`], where it is a pure function with its own tests, and
//! leaves the builders here doing nothing but arranging boxes.
//!
//! # Three covers, not one
//!
//! The reference has three different cover-shaped things and they are not
//! variations on a size:
//!
//! | Where | Shape | Fitted how |
//! |---|---|---|
//! | library tile / card | the leftover box in a cell | cropped if a photo, inset if an icon |
//! | list row | 36×50.4, a fixed strip | same |
//! | the game form's picker | 36×54 | always letterboxed, and labelled when empty |
//!
//! The form's picker is the one that says "No cover yet"
//! (`GameFormPage.qml:148-151`). It is a *different widget* from the library
//! plate, which shows the game's initials and no words at all
//! (`CoverArt.qml`'s `Text` is `text: art.initials`, `visible: art.coverUrl ===
//! ""`). Putting the label on the tiles would be a plausible-looking mistake,
//! which is why the two live in separate functions with separate tests.
//!
//! # The play control, and how the launch reaches it
//!
//! The card and the row carry the reference's Play control and both are
//! double-clickable. Those are the four emission sites `LibraryPage.qml` has on
//! them: `:161` the card's double tap, `:205-207` the card's button, `:239` the
//! row's double click, `:277-279` the row's button. All four emit
//! `backend.playGame(gameId)` and all four here emit the **same message value**.
//!
//! It arrives as a parameter, not a variant this file names. That is the
//! property the top of this doc claims — `M` is a parameter the caller fixes,
//! never a value this layer constructs — and the launch control is where it
//! would have been easiest to break it, because a button needs a `Message` and
//! the game whose launch it is sits right here. It stays the caller's for three
//! reasons:
//!
//! * **The id is the caller's to resolve.** `play_button_id` needs the game id,
//!   which this file has, but *what to do with it* — launch this game, open its
//!   form, select it — is the caller's decision. A builder that hard-coded
//!   `Message::LaunchGame` would make every future card action a signature
//!   change instead of a second argument.
//! * **It keeps the guard's question answerable.** `tests/dispatch_coverage.rs`
//!   reads a `Message::<Variant>` in this file as an *emission*, and it cannot
//!   tell a construction from a match pattern (`emissions`). A file that
//!   constructs its message has one place to be wrong in; a file that receives
//!   one has none, and the emission is checked where it is written, in
//!   `view/library.rs`.
//! * **The tests stay messages-free.** Every test below builds a card with
//!   `()` as the message, so what is asserted about the layout cannot depend on
//!   the `Message` enum.
//!
//! # What is still not here
//!
//! Hover and selection styling (`LibraryPage.qml:154-158`, which reads
//! `cardHover.hovered`) needs state and is not in this file. Nor is the
//! right-click context menu, including its own Play item (`:302-304`): that
//! menu belongs to `view/library.rs`. What is here is the launch itself.
//!
//! The Play button is activated by **Enter** when focused, which iced's button
//! does itself (`iced/widget/src/button.rs:426-440`, the only keyboard arm it
//! has) and by no other key — `QQC2.Button` also accepts Space. Recorded rather
//! than fixed: the reference's keyboard behaviour is not reachable from this
//! file, and a widget that swallowed Space locally would be the wrong place to
//! answer it.
//!
//! The button's `Id` is the one part of this file that is **not** inert, and
//! that is the other reason it is set: iced's button also activates on an
//! accessibility `Action::Click` whose `event_id` equals its `Id`
//! (`iced/widget/src/button.rs:414-424`), so the id is what a screen reader's
//! activation is matched against. One id for every game would make that
//! ambiguous; see [`play_button_id`].

use cosmic::Element;
use cosmic::iced::gradient::Linear;
use cosmic::iced::widget::container;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Radians};
use cosmic::widget::{button, icon, image, mouse_area, text, Column, Row};
use gamehandler_core::models::Game;
use gamehandler_core::runners::RunnerManager;

use super::cover::{self, CoverSource};
use super::meta;
use super::metrics;

/// The plate's text. White at slightly under full opacity, as `CoverArt.qml`
/// sets it (`color: "#ffffff"; opacity: 0.92`).
const PLATE_TEXT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.92);

/// The font size of the picker's "No cover yet", from
/// `GameFormPage.qml:148-151`.
const PREVIEW_LABEL_SIZE: f32 = 11.0;

// ---- Widget ids -----------------------------------------------------------
//
// iced gives a widget an optional `Id`, and an `Id` is the one part of a widget
// that is readable from the outside: `Widget::id()` is public and `Tree` — the
// structure the framework builds from a real element — carries it. Nothing
// else is. `Widget` has no `as_any`, there is no downcast anywhere in iced, and
// the nested widgets a builder returns cannot be reached by type.
//
// So these ids are the seam the structural tests use, and they are the reason
// those tests can call the *real* builders rather than a stand-in: the test
// builds the element the app would build and walks the tree the framework would
// build from it. An id is inert at runtime — it is not drawn, and a widget
// without a `Message` has no `update` to route through it — so the cost of
// having them is a string constant each.
//
// They are named for what the node *is*, and a test asserts the whole set for
// each widget, so a builder that stopped drawing a node fails there rather than
// rendering something subtly different that no test can see.
const PLATE_ID: &str = "gamehandler.cover.plate";
const PICTURE_ID: &str = "gamehandler.cover.picture";
const INITIALS_ID: &str = "gamehandler.cover.initials";
const PREVIEW_LABEL_ID: &str = "gamehandler.cover.preview-label";

/// The Play control's label, which is the reference's word for it: `text:
/// "Play"` on both the card's button (`LibraryPage.qml:205`) and the row's
/// (`:277`).
const PLAY_LABEL: &str = "Play";

/// The icon the reference puts on the Play control (`.name:
/// "media-playback-start"`, `LibraryPage.qml:206` and `:278`), by the name
/// freedesktop icon themes carry it under.
///
/// Resolved by name rather than embedded as bytes, the same way
/// `view/installers.rs` asks for `run-install`: `icon::from_name` falls back to
/// a symbolic name when the theme has nothing, so a missing icon is a different
/// glyph rather than a missing button.
const PLAY_ICON: &str = "media-playback-start";

/// The box a caller wants a cover drawn in, and how to draw it.
///
/// A struct rather than five positional arguments because the numbers are easy
/// to transpose and impossible to read at a call site: `cover_box(game, 36.0,
/// 50.4, 6.0, true)` says nothing about which of `50.4` and `6.0` is the height
/// and which the corner radius, and a caller that swapped them would still
/// compile and still look plausible.
///
/// It is also the seam the tests read. Each builder asks for its box through
/// one of the `*_cover_spec` functions below and then **destructures the
/// result**, so the numbers exist in exactly one place — a test that pins the
/// spec is a test on what the widget draws, and there is no second copy at the
/// call site for the two to drift apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverSpec {
    /// The box's width in logical pixels.
    pub width: f32,
    /// The box's height in logical pixels.
    pub height: f32,
    /// The corner radius applied to the artwork *and* to the plate behind it.
    pub radius: f32,
    /// The compact drawing, used by a list row: a smaller maximum initial, and
    /// a shorter tile. `CoverArt.qml` reaches the same distinction through a
    /// `compact` property rather than through a second widget.
    pub compact: bool,
}

/// The cover box a library **card** asks for.
///
/// The box is the cell less its margins and chrome ([`metrics::card_cover_box`]),
/// which is the same subtraction the metrics tests assert on. The radius is
/// [`metrics::TILE_RADIUS`] and the drawing is *not* compact: a tile is the
/// full-size cover, and only a row is compact.
pub fn card_cover_spec() -> CoverSpec {
    let (width, height) = metrics::card_cover_box(
        metrics::GRID_CELL,
        metrics::CARD_CHROME_HEIGHT,
        metrics::CARD_MARGIN,
    );
    CoverSpec {
        width,
        height,
        radius: metrics::TILE_RADIUS,
        compact: false,
    }
}

/// The cover box a **list row** asks for.
///
/// The fixed strip, not the tile aspect — a row is a fixed-height line whose
/// text must not move when one game's art is a different shape from the next —
/// and [`metrics::COMPACT_RADIUS`] for the corner, because the row's cover is
/// half the size of a tile's.
pub fn row_cover_spec() -> CoverSpec {
    CoverSpec {
        width: metrics::LIST_COVER_WIDTH,
        height: metrics::LIST_COVER_HEIGHT,
        radius: metrics::COMPACT_RADIUS,
        compact: true,
    }
}

/// The cover box the game form's **picker** asks for.
///
/// `gridUnit * 2` by `gridUnit * 3` (`GameFormPage.qml:141-142`), the compact
/// radius, and a non-compact drawing: the preview is small but it is a preview
/// of a tile, not a row.
pub fn preview_cover_spec() -> CoverSpec {
    CoverSpec {
        width: metrics::FORM_PREVIEW_WIDTH,
        height: metrics::FORM_PREVIEW_HEIGHT,
        radius: metrics::COMPACT_RADIUS,
        compact: false,
    }
}

/// What [`cover_box`] composes, as a value rather than as control flow.
///
/// The three [`CoverSource`]s differ in more than the picture, and the
/// difference is a layout decision that has to be made *before* the image is
/// decoded (see [`super::cover`]'s module docs):
///
/// - a **photograph** fills the box and is cropped if it does not fit —
///   `ContentFit::Cover`;
/// - an **icon** sits inside it, uncropped, inset, on the plate —
///   `ContentFit::Contain`;
/// - a game with **no cover** gets the plate: initials on the game's gradient.
///
/// Returning this as a value rather than writing the choice inline is what
/// makes the choice testable: `cover_box` is then nothing but a renderer for a
/// plan, and every decision — which composition, how deep the inset, which
/// words — is a field of the plan that a test can read without a renderer.
#[derive(Debug, Clone, PartialEq)]
pub enum CoverPlan {
    /// A photograph alone: the picture fills the box, nothing behind it.
    Photo,
    /// An icon inset over the plate. The plate goes *behind* the icon —
    /// `CoverArt.qml`'s `showPlate: coverUrl === "" || coverIsIcon` — so an
    /// icon tile reads as artwork on a coloured card rather than as a floating
    /// logo.
    IconOnPlate {
        /// The gap between the icon and the edge of its box.
        inset: f32,
    },
    /// The game's initials on the plate. The initials are not drawn for an
    /// icon: the QML's `visible` test is `coverUrl === ""`, which an icon
    /// fails.
    InitialsOnPlate {
        /// The string drawn — [`plate_text`]'s answer for this game.
        text: String,
    },
}

/// The plan for a game's cover in a **library** context.
///
/// The text is carried *in* the plan rather than composed by the widget that
/// draws it, so "what a tile says" is a value with one definition
/// ([`plate_text`]) that the test reads directly, and [`cover_box`] has no
/// string of its own to get wrong.
pub fn cover_plan(game: &Game) -> CoverPlan {
    match CoverSource::classify(&game.cover_path) {
        CoverSource::Photo => CoverPlan::Photo,
        CoverSource::Icon => CoverPlan::IconOnPlate {
            inset: metrics::ICON_INSET,
        },
        CoverSource::Plate => CoverPlan::InitialsOnPlate {
            text: plate_text(game),
        },
    }
}

/// The words a library tile draws when there is no artwork: the game's
/// initials, and nothing else.
///
/// `CoverArt.qml`'s text is `art.initials` with `visible: art.coverUrl === ""`.
/// The "No cover yet" string belongs to the form's picker and is reached
/// through [`preview_label`] — the two are one function call apart, which is
/// why each has its own name and its own test.
pub fn plate_text(game: &Game) -> String {
    cover::initials(&game.name)
}

/// The words the game form's picker draws when there is no artwork.
///
/// No game is involved: this is a constant for a *state*, not a value derived
/// from the thing being drawn. That is the difference from [`plate_text`], and
/// the reason the two cannot be served by one function.
pub fn preview_label() -> &'static str {
    cover::NO_COVER_LABEL
}

/// A game's cover at a given *width*, its height taken from the tile aspect.
///
/// The single-argument form exists for a grid that lays out a column of tiles
/// and wants them all the same shape; a caller with its own box should use
/// [`cover_box`] instead, which is what [`card`] and [`row`] do.
pub fn cover_tile<M: Clone + 'static>(game: &Game, width: f32) -> Element<'_, M> {
    cover_box(
        game,
        CoverSpec {
            width,
            height: metrics::tile_height(width),
            radius: metrics::TILE_RADIUS,
            compact: false,
        },
    )
}

/// A game's cover in an explicit box — a renderer for [`cover_plan`].
pub fn cover_box<M: Clone + 'static>(game: &Game, spec: CoverSpec) -> Element<'_, M> {
    match cover_plan(game) {
        CoverPlan::Photo => framed(game, spec, 0.0),
        CoverPlan::IconOnPlate { inset } => {
            let inner = framed(game, spec, inset);
            plate(game, spec, inner)
        }
        CoverPlan::InitialsOnPlate { text } => {
            let inner = initials_only(text, spec);
            plate(game, spec, inner)
        }
    }
}

/// The id of the Play control on a game's card and on its row.
///
/// # Why this one is per game and the cover ids are constants
///
/// [`PLATE_ID`] and its siblings name a node that exists **once** in a tree —
/// one cover, one plate, one picker label. A Play control exists once *per
/// game*, and the library draws every game at once, so a constant here would be
/// the same key on every card. iced's `Id` is a key, and both of the things it
/// is used for are lookups that a duplicate makes ambiguous: the accessibility
/// `Action::Click` iced matches on `self.id == *event_id`
/// (`iced/widget/src/button.rs:414`), and any future `operation::focus`. The
/// game's id is what makes it unique, so it is part of the key.
///
/// It is public, and takes the id as a `&str` rather than a `&Game`, so a test
/// can name the id it expects without building a game — and so a caller that
/// has only an id (a menu acting on a game that is no longer in the list, say)
/// can still reach the control.
pub fn play_button_id(game_id: &str) -> String {
    format!("gamehandler.library.play.{game_id}")
}

/// The reference's Play control, as both the card and the row draw it.
///
/// One function rather than two call sites' worth of builder chain, because the
/// card and the row must not drift: `LibraryPage.qml` spells this button out
/// twice (`:204-208` on the card, `:276-280` on the row) with the same `text`,
/// the same `icon.name` and the same `onClicked`, which is two chances to give
/// one of them a different message. They differ in where they sit, not in what
/// they are.
///
/// `on_play` is taken and cloned rather than built here: see this module's
/// header for why the launch is the caller's message and not this file's
/// variant. It is cloned because the same control is used for the button and
/// for the double-click on the card or row around it.
fn play_button<'a, M: Clone + 'static>(game_id: &str, on_play: M) -> Element<'a, M> {
    button::standard(PLAY_LABEL)
        .leading_icon(icon::from_name(PLAY_ICON))
        .id(play_button_id(game_id).into())
        .on_press(on_play)
        .into()
}

/// A library grid tile: the cover, the name and the subtitle, with the Play
/// control under them.
///
/// The cover's box comes from [`card_cover_spec`], which is the same
/// subtraction the metrics tests assert on — so a card whose chrome grows
/// shortens its cover rather than overlapping it.
///
/// `label` is the game's resolved runner label for the subtitle; see
/// [`subtitle_of`]. It is data rather than a manager, and the caller resolves
/// it when it builds the row — never here.
///
/// `on_play` is emitted by the Play control and by a double click anywhere on
/// the card, which is the reference's pair of emission sites on this delegate
/// (`LibraryPage.qml:161` and `:205-207`) and one message in both.
///
/// # The card has no last-played label, and that is the reference's asymmetry
///
/// `LibraryPage.qml` draws the grid and the list with two separate delegates,
/// and they do **not** show the same text. The card is `subtitle` alone
/// (`:192`); the row is `subtitle + " · " + lastPlayed` (`:269`). So "add the
/// timestamp to the game widgets" is the wrong reading of P-02/P-16 — it is the
/// *list row* that gained it, and a card that showed it too would render a
/// string the reference never renders. T-30 is scoped to the row for this
/// reason; see [`row`].
pub fn card<'a, M: Clone + 'static>(
    game: &'a Game,
    label: &str,
    on_play: M,
) -> Element<'a, M> {
    let (cell_w, cell_h) = metrics::GRID_CELL;
    let spec = card_cover_spec();

    let body = Column::new()
        .push(cover_box(game, spec))
        .push(name_and_subtitle(game, subtitle_of(game, label)))
        .push(play_button(&game.id, on_play.clone()))
        .spacing(metrics::CARD_MARGIN)
        .width(Length::Fill);

    // The double click wraps the whole tile rather than sitting on the body, so
    // that the cover and the two strings are as clickable as the button — which
    // is what the reference's `TapHandler` covers (`LibraryPage.qml:159-162`,
    // `anchors.fill: parent`). A click on the Play control itself is consumed by
    // the button before it reaches here (`mouse_area` returns early on
    // `shell.is_event_captured()`, `iced/widget/src/mouse_area.rs:278-280`, and
    // iced's button captures, `:386`), so a double click on the button launches
    // once rather than twice.
    mouse_area(
        container(body)
            .width(Length::Fixed(cell_w))
            .height(Length::Fixed(cell_h))
            .padding(metrics::CARD_MARGIN)
            .align_x(Alignment::Start)
            .align_y(Alignment::Start)
            .style(card_style),
    )
    .on_double_click(on_play)
    .into()
}

/// The two strings a **list row** resolves for a game, carried together.
///
/// A struct rather than two `&str` parameters because both are `&str` and
/// adjacent: transposing them would produce a row reading
/// `"System Wine · Shooter"`, which compiles, renders, and is wrong. The same
/// hazard the installers page records for its `(installer_id, runner_id)`
/// payload, answered the same way — the type is what makes the swap
/// unrepresentable rather than a reviewer catching it.
///
/// Both fields are resolved by the caller, for the reason
/// [`resolved_runner_label`] gives: the runner label walks the filesystem and
/// must not be resolved per frame per game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowLabels<'a> {
    /// [`resolved_runner_label`]'s result for this game.
    pub runner: &'a str,
    /// [`meta::row_subtitle`]'s third part: `format_last_played` for this game,
    /// evaluated against the page's **single** instant — the reason that
    /// function takes `now` as a parameter rather than reading the clock.
    ///
    /// [`format_last_played`]: gamehandler_core::models::format_last_played
    pub last_played: &'a str,
}

/// A list row: the cover, the name and the subtitle followed by the
/// last-played label, laid out horizontally.
///
/// The cover's box comes from [`row_cover_spec`] — the fixed strip, not the
/// tile aspect. A row is a fixed-height line whose text must not move when one
/// game's art is a different shape from the next, so the cover is fitted *into*
/// the row rather than setting its height.
///
/// `labels` carries both resolved strings; see [`RowLabels`] and
/// [`subtitle_of`]. The composition itself is [`meta::row_subtitle`]'s, so the
/// row and any future caller of it cannot disagree about the separator.
///
/// `on_play` is emitted by the Play control, which sits at the end of the line
/// (`LibraryPage.qml:276-280`), and by a double click anywhere on the row
/// (`:239`). Both are the same message, for the reason [`card`] gives.
pub fn row<'a, M: Clone + 'static>(
    game: &'a Game,
    labels: &RowLabels<'_>,
    on_play: M,
) -> Element<'a, M> {
    let line = Row::new()
        .push(cover_box(game, row_cover_spec()))
        .push(name_and_subtitle(game, row_subtitle_of(game, labels)))
        .push(play_button(&game.id, on_play.clone()))
        .spacing(metrics::CARD_MARGIN)
        .align_y(Alignment::Center);

    mouse_area(
        container(line)
            .width(Length::Fill)
            .height(Length::Fixed(metrics::LIST_ROW_HEIGHT))
            .padding(metrics::ICON_INSET / 2.0)
            .align_y(Alignment::Center),
    )
    .on_double_click(on_play)
    .into()
}

/// The game form's cover picker: a small preview, or the words "No cover yet".
///
/// This is the widget [`preview_label`] belongs to. `GameFormPage.qml` shows an
/// `Image` at `gridUnit * 2` by `gridUnit * 3` with
/// `fillMode: Image.PreserveAspectFit`, and a label in its place when there is
/// nothing to show — so unlike the library plate this one letterboxes a
/// photograph as well as an icon, and unlike the plate it says what is
/// missing rather than drawing initials.
///
/// The label appears for [`CoverSource::Plate`] only, which is the same
/// condition the QML puts on `coverPreview.visible` (`source !== ""`): a game
/// whose cover file has been deleted gets the label, not a broken image.
///
/// The condition is read from [`cover_plan`] rather than from a second call to
/// `classify`, so the picker and the tile agree about which games have
/// artwork by construction — and so the one thing they *do* differently, words
/// instead of initials, is visible as a single arm.
pub fn cover_preview<M: Clone + 'static>(game: &Game) -> Element<'_, M> {
    let spec = preview_cover_spec();

    match cover_plan(game) {
        CoverPlan::InitialsOnPlate { .. } => preview_label_widget(spec),
        CoverPlan::Photo | CoverPlan::IconOnPlate { .. } => framed(game, spec, 0.0),
    }
}

/// The picker's placeholder: the words, sized into `spec`'s box.
///
/// Split out so its id and its size are set in one place, and so the test that
/// asks "does the picker draw words or initials?" has an element whose root it
/// can name.
fn preview_label_widget<'a, M: Clone + 'static>(spec: CoverSpec) -> Element<'a, M> {
    container(text(preview_label()).size(PREVIEW_LABEL_SIZE))
        .id(PREVIEW_LABEL_ID)
        .width(Length::Fixed(spec.width))
        .height(Length::Fixed(spec.height))
        .align_x(Alignment::Start)
        .align_y(Alignment::Center)
        .into()
}

/// The game's name, as every game-shaped widget shows it.
///
/// Named rather than written inline for the same reason as [`subtitle_of`]: the
/// card and the row both show it, and a test can read it without a renderer.
pub fn title_of(game: &Game) -> String {
    game.name.clone()
}

/// The name over the subtitle, as every game-shaped widget shows them.
///
/// `subtitle` is passed in already composed rather than resolved here, because
/// the card and the row show *different* strings — the row appends the
/// last-played label and the card does not (see [`card`]) — and a helper that
/// picked one would have to know which caller it had. Both compositions live in
/// [`meta`], which is the module whose job that is.
fn name_and_subtitle<'a, M: Clone + 'static>(game: &'a Game, subtitle: String) -> Element<'a, M> {
    Column::new()
        .push(text(title_of(game)).size(14.0))
        .push(text(subtitle).size(11.0))
        .spacing(2.0)
        .width(Length::Fill)
        .into()
}

/// The runner label for a game, resolved **where the row data is built**.
///
/// This is `bridge.py:298-301`'s `_runner_label` and nothing more, kept as one
/// function so the two halves of that rule cannot drift:
///
/// ```text
/// if game.is_linux: return "Linux native"
/// return self.runner_manager.label(game.runner)
/// ```
///
/// # Why this is not called from a widget
///
/// [`RunnerManager::label`] is **uncached** (`runners/mod.rs:1099`): it joins the
/// runners directory, tests `exists()`, and constructs a `ProtonRunner` to ask
/// for its family label — on every call, for every game. A builder runs every
/// frame, so resolving this inside [`card`] or [`row`] would put a filesystem
/// walk per game per frame on the render path. That is the same defect as
/// spawning a process there, only cheaper.
///
/// So the label is resolved when the row data is assembled and carried as a
/// string from then on. It is also the cheaper answer for a second reason:
/// **the runner list changes only on install or uninstall**, both of which
/// already re-read the library — so resolving at build time is correct *and*
/// free, where resolving per frame would be merely correct.
///
/// Resolving here rather than threading a `&RunnerManager` through the
/// builders also keeps them free functions over the game: their `Element`
/// borrows the game alone, so no caller's lifetime is welded to the manager's.
///
/// # The line number, and why it is not the authority
///
/// `runners/mod.rs:1099` is a **point-in-time** fact about a file that grows.
/// This citation was `mod.rs:1041` when it was written (`13e9806`), which was
/// correct then; `runners/mod.rs` has since gained 58 lines above `label`, and
/// every citation of that function's body moved with them — this one and the
/// one in the test below, both corrected together. The same drift hit
/// `view/settings.rs`'s citation of `RunnerManager::choices` (`mod.rs:1061` →
/// `runners/mod.rs:1119`), and the +58 accounts for both.
///
/// So the number is a convenience and [`RunnerManager::label`] is the
/// authority: when this citation and the symbol disagree, the symbol is right
/// and the number is stale. `view/meta.rs`'s `subtitle` doc reached the same
/// conclusion about a *prose* citation that had gone false (#69) — a pointer
/// that no longer lands is worse than no pointer, because it is trusted.
pub fn resolved_runner_label(manager: &RunnerManager, game: &Game) -> String {
    meta::runner_label(game.is_linux(), &manager.label(&game.runner))
}

/// The subtitle for a game, through the pure function.
///
/// `label` is the **already-resolved** runner label — see
/// [`resolved_runner_label`] for where it comes from and why it is a parameter
/// rather than a manager. It used to be the literal `""`, which is `#30`: every
/// Windows game rendered no runner at all, and for an uncategorised one an
/// empty subtitle, where Python renders `"System Wine"`. The empty string is a
/// legal input to [`meta::subtitle`], so a test asserting the old behaviour
/// passed in both the fixed and the unfixed state — the defect D-34 names.
///
/// A Linux game is unaffected by `label`: [`meta::runner_label`] discards it.
///
/// This is the **card's** subtitle, and it stops at [`meta::subtitle`]. The
/// list row's line is [`row_subtitle_of`], which is this plus the last-played
/// part — see [`card`] for why the two widgets deliberately differ.
fn subtitle_of(game: &Game, label: &str) -> String {
    meta::subtitle(game.display_category(), &meta::runner_label(game.is_linux(), label))
}

/// The **list row's** line: [`subtitle_of`]'s string, then the last-played
/// label. Parity items P-02 and P-16.
///
/// The composition is [`meta::row_subtitle`]'s and not this function's, for the
/// reason that module exists: the rule that an absent part is omitted rather
/// than left as a dangling separator is a rule, and it belongs where a test can
/// call it directly. This function only supplies the three inputs — and it
/// resolves the runner label through [`meta::runner_label`] exactly as
/// [`subtitle_of`] does, so a Linux game discards `labels.runner` here too.
///
/// `labels.last_played` is `format_last_played`'s result and is never empty,
/// so the row always shows a fifth-of-a-line timestamp; the absent case the
/// composer handles is for callers other than this one. See
/// [`meta::row_subtitle`] for why that branch is written anyway.
fn row_subtitle_of(game: &Game, labels: &RowLabels<'_>) -> String {
    meta::row_subtitle(
        game.display_category(),
        &meta::runner_label(game.is_linux(), labels.runner),
        labels.last_played,
    )
}

/// A cover image in a box of exactly `spec`'s size, inset by `inset`.
fn framed<'a, M: Clone + 'static>(game: &'a Game, spec: CoverSpec, inset: f32) -> Element<'a, M> {
    let source = CoverSource::classify(&game.cover_path);
    let picture = image(image::Handle::from_path(&game.cover_path))
        .content_fit(source.content_fit())
        .width(Length::Fill)
        .height(Length::Fill)
        .border_radius(spec.radius);

    // The inset is applied whether or not it is zero, so the two branches
    // cannot drift in their width/height handling.
    container(picture)
        .id(PICTURE_ID)
        .width(Length::Fixed(spec.width))
        .height(Length::Fixed(spec.height))
        .padding(inset)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

/// The initials, centred — what a library tile shows for a game with no cover.
///
/// No words: `CoverArt.qml`'s text is `art.initials` alone. The "No cover yet"
/// string belongs to [`cover_preview`] and arrives through [`preview_label`],
/// and a test below pins the difference. The string is a parameter rather than
/// a call to [`plate_text`] here so that this function has no string of its
/// own to get wrong: it draws what [`cover_plan`] put in the plan.
fn initials_only<'a, M: Clone + 'static>(text_of_game: String, spec: CoverSpec) -> Element<'a, M> {
    container(
        text(text_of_game)
            .size(metrics::initials_size(spec.width, spec.height, spec.compact))
            .class(PLATE_TEXT),
    )
    .id(INITIALS_ID)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .into()
}

/// A gradient plate of exactly `spec`'s size with `content` centred on it.
fn plate<'a, M: Clone + 'static>(
    game: &'a Game,
    spec: CoverSpec,
    content: Element<'a, M>,
) -> Element<'a, M> {
    let accent = cover::accent_of(&game.id);
    container(content)
        .id(PLATE_ID)
        .width(Length::Fixed(spec.width))
        .height(Length::Fixed(spec.height))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| plate_style(accent, spec.radius))
        .into()
}

/// The angle of a plate's gradient, in radians. See [`plate_style`].
pub const PLATE_GRADIENT_ANGLE: f32 = std::f32::consts::PI;

/// The gradient a placeholder plate is filled with, as a [`container::Style`].
///
/// Top to bottom, the way `CoverArt.qml`'s two `GradientStop`s are: offset 0.0
/// is the lighter colour at the top. The angle that produces that is
/// [`PLATE_GRADIENT_ANGLE`], which is `PI`: iced resolves an angle in
/// `to_distance` (`iced_core/src/angle.rs:86-98`), where the start point is
/// `center - r * d` with `r = (cos(a - PI/2), sin(a - PI/2))` — at `a = PI`
/// that is `(0, 1)`, so the start is the top edge. `PI/2` would be
/// left-to-right, and a gradient running the wrong way still looks plausible,
/// which is why the angle is a named constant the test resolves rather than a
/// literal written in two places.
fn plate_style(accent: usize, radius: f32) -> container::Style {
    let mut gradient = Linear::new(Radians(PLATE_GRADIENT_ANGLE));
    for (offset, colour) in plate_stops(accent) {
        gradient = gradient.add_stop(offset, colour);
    }

    container::Style {
        background: Some(Background::Gradient(gradient.into())),
        border: Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The two stops of a plate's gradient, as `(offset, colour)` in the order
/// they are added.
///
/// A pure function rather than two `add_stop` calls inside [`plate_style`],
/// because the *order* is the part that is easy to get backwards and
/// impossible to see: a gradient running bottom-to-top is still a gradient,
/// and both colours still appear. Written this way a test can assert which
/// shade colour lands at which offset against [`super::cover::shade`]'s own
/// output, which is what ties the angle to the colours.
pub fn plate_stops(accent: usize) -> [(f32, Color); 2] {
    let (top, bottom) = cover::shade(accent);
    [(0.0, rgb(top)), (1.0, rgb(bottom))]
}

/// The surface a card sits on. `LibraryPage.qml`'s card is a `Rectangle` whose
/// colour is `Kirigami.Theme.alternateBackgroundColor`, or the highlight
/// colour at 12% while hovered.
///
/// The hover half is deliberately absent: hovering needs a message, and this
/// module emits none. The theme lookup is here rather than at the call site so
/// that adding the hover state later is a change in one place.
fn card_style(theme: &cosmic::Theme) -> container::Style {
    let cosmic = theme.cosmic();
    container::Style {
        background: Some(Background::Color(
            cosmic.background(false).base.into(),
        )),
        border: Border {
            radius: metrics::CARD_RADIUS.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// An 8-bit RGB triple from [`super::cover`] as a colour.
fn rgb(channels: [u8; 3]) -> Color {
    Color::from_rgb8(channels[0], channels[1], channels[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::advanced::layout::{Limits, Node};
    use cosmic::iced::advanced::widget::operation::Focusable;
    use cosmic::iced::advanced::widget::{Operation, Tree};
    use cosmic::iced::advanced::Layout;
    // The trait, not a value: `measure_image` is `Renderer`'s method and is not
    // in scope without it.
    use cosmic::iced::advanced::image::Renderer as _;
    use cosmic::iced::advanced::Shell;
    use cosmic::iced::{Event, Point, mouse};
    use cosmic::iced::{Font, Pixels, Radius, Rectangle, Size};
    use cosmic::widget::Id;
    use gamehandler_core::models::{Game, format_last_played};

    /// The instant the last-played tests are evaluated against.
    ///
    /// Spelled out rather than taken from the clock, because the whole reason
    /// `format_last_played` takes `now` as a parameter is that a label computed
    /// from an ambient read is untestable — and worse, flaky at a boundary. The
    /// value is the same frozen epoch the oracle's `format_last_played` corpus
    /// uses (`1700000000.0`), so a label asserted here and a label asserted
    /// there are the same label.
    const FROZEN_NOW: f64 = 1_700_000_000.0;

    /// The subtitle a widget shows, reached through the same path the widgets
    /// use. These are the strings a user reads, so they are asserted as
    /// strings; the pieces they are built from are tested in [`super::meta`].
    ///
    /// This is `#30`'s assertion, and it is written with a **real** label on
    /// purpose. The previous version passed no label at all, so it asserted
    /// `"Shooter"` — a string `meta::subtitle` produces both when the runner is
    /// missing *and* when there is no runner to show. Python produces
    /// `"Shooter · System Wine"` here (`bridge.py:309-320`), and D-36 records
    /// the port rendering `"Shooter"`: the runner absent from the subtitle in
    /// the common case, not merely on uncategorised rows.
    #[test]
    fn a_row_shows_the_category_and_the_runner_for_a_windows_game() {
        let mut game = Game::new_named("Half-Life 2");
        game.category = "Shooter".into();
        game.kind = "windows".into();
        assert_eq!(subtitle_of(&game, "System Wine"), "Shooter \u{b7} System Wine");
    }

    /// A Linux game says so whatever label it is handed — the label is
    /// discarded, not merely overridden, which is why the second call passes a
    /// label a manager could never return for a Linux game. A port that
    /// consulted the manager first and the platform second would render the
    /// passed label here.
    #[test]
    fn a_linux_game_is_labelled_native_and_discards_the_runner_label() {
        let mut game = Game::new_named("Celeste");
        game.category = "Platformer".into();
        game.kind = "linux".into();
        assert_eq!(subtitle_of(&game, "System Wine"), "Platformer \u{b7} Linux native");
        assert_eq!(subtitle_of(&game, ""), "Platformer \u{b7} Linux native");
    }

    /// An uncategorised Windows game shows the runner **alone**, which is
    /// `"System Wine"` and not the empty string this used to assert.
    ///
    /// Both halves are claimed: that the runner is there, and that it is not
    /// preceded by a lone separator — the visible bug the uncategorised branch
    /// of `meta::subtitle` exists to prevent.
    #[test]
    fn an_uncategorised_windows_game_shows_the_runner_alone() {
        let mut game = Game::new_named("Mystery");
        game.kind = "windows".into();
        assert_eq!(subtitle_of(&game, "System Wine"), "System Wine");
        assert_ne!(subtitle_of(&game, "System Wine"), "");
    }

    /// The resolution itself: a Windows game with no runner id gets
    /// `"System Wine"`, which is the value `#30` says never reached the view.
    ///
    /// This is the half that makes the three tests above load-bearing rather
    /// than hypothetical. `bridge.py:300-302` resolves the label *before* the
    /// row is built, so Python can never hand `subtitle_of` an empty string;
    /// [`resolved_runner_label`] is that resolution, and `runner.label("")`
    /// returning `"System Wine"` (`runners.py:759-761`, `runners/mod.rs:1099`)
    /// is the fact that closes the gap. The manager is constructed at a path
    /// with no runners in it, so this also pins the fallback rather than a
    /// lookup.
    #[test]
    fn a_windows_game_resolves_to_a_real_label_and_never_to_the_empty_string() {
        let manager = RunnerManager::at("/nonexistent");

        let mut windows = Game::new_named("Half-Life 2");
        windows.kind = "windows".into();
        assert_eq!(resolved_runner_label(&manager, &windows), "System Wine");
        assert_ne!(resolved_runner_label(&manager, &windows), "");

        // And the resolved label is what the subtitle is built from, so the two
        // are one path rather than two that can disagree.
        let label = resolved_runner_label(&manager, &windows);
        assert_eq!(subtitle_of(&windows, &label), "System Wine");

        // A Linux game's label is the platform's, whatever the manager holds.
        let mut linux = Game::new_named("Celeste");
        linux.kind = "linux".into();
        assert_eq!(resolved_runner_label(&manager, &linux), "Linux native");
    }

    /// **The plate and the form's picker are different widgets.** The library
    /// tile shows initials and no words; the form's picker shows the words and
    /// no initials.
    ///
    /// Asserted on the strings the two builders actually hand the traversal —
    /// `cover_box` for the tile, `cover_preview` for the picker — because the
    /// two are one function call apart and swapping them would still compile,
    /// still render, and read as a design choice rather than a bug. This is the
    /// assertion the audit's "draw the initials in the picker" mutation has to
    /// get past, and it is made on a real `cover_preview`.
    #[test]
    fn a_tile_draws_the_initials_and_a_picker_draws_the_words() {
        let game = Game::new_named("Half-Life 2");

        let mut tile: Element<'_, ()> = cover_box(&game, row_cover_spec());
        assert_eq!(
            texts(&traversal(&mut tile)),
            ["HL"],
            "a library tile draws the game's initials and nothing else"
        );

        let mut picker: Element<'_, ()> = cover_preview(&game);
        assert_eq!(
            texts(&traversal(&mut picker)),
            [cover::NO_COVER_LABEL],
            "the form's picker draws the words, not the initials"
        );
    }

    /// The same claim through the two builders the library page calls, with the
    /// subtitle the third string: a game with no category shows the runner
    /// alone, and the runner is a real one. That third string is the
    /// end-to-end half of `meta`'s regression test — it is what a row actually
    /// shows, after the widget has had its turn — and it is `#30`'s symptom
    /// observed at the level a user sees it. The label passed here is the one
    /// [`resolved_runner_label`] produces for this game, so the assertion is
    /// about the shipped path rather than a hand-picked string.
    ///
    /// **The card and the row do not draw the same third string, and this test
    /// is where that is visible.** `LibraryPage.qml` gives each delegate its
    /// own text: the card is `subtitle` (`:192`) and the row is
    /// `subtitle + " · " + lastPlayed` (`:269`). So the row has a *fourth*
    /// string and the card has three — asserted separately rather than as one
    /// list, because a shared expectation is what would let the card grow a
    /// timestamp the reference never draws.
    #[test]
    fn a_card_and_a_row_draw_the_initials_and_never_the_pickers_words() {
        let game = Game::new_named("Half-Life 2");
        let manager = RunnerManager::at("/nonexistent");
        let label = resolved_runner_label(&manager, &game);

        let mut card: Element<'_, ()> = card(&game, &label, ());
        assert_eq!(
            texts(&traversal(&mut card)),
            ["HL", "Half-Life 2", "System Wine", "Play"]
        );

        let played = format_last_played(0.0, FROZEN_NOW);
        let labels = RowLabels {
            runner: &label,
            last_played: &played,
        };
        let mut row: Element<'_, ()> = row(&game, &labels, ());
        assert_eq!(
            texts(&traversal(&mut row)),
            ["HL", "Half-Life 2", "System Wine · Never played", "Play"]
        );
    }

    /// The list row shows the last-played label composed with the subtitle, and
    /// the grid card does not — the reference's asymmetry, asserted as a
    /// difference rather than as two independent expectations.
    ///
    /// This is P-02/P-16's acceptance at the widget level: the timestamp is
    /// **in the row** and **not in the card**. Written as one test because the
    /// failure it prevents is exactly a change that makes them equal — wiring
    /// the timestamp into `name_and_subtitle` instead of into the row would
    /// satisfy a "the row shows it" assertion and quietly put it on every card.
    #[test]
    fn the_row_carries_the_last_played_label_and_the_card_does_not() {
        let mut game = Game::new_named("Celeste");
        game.category = "Platformer".into();
        game.kind = "linux".into();
        // Two days before the frozen instant, so the label is a real one and
        // not the `"Never played"` this test's sibling already covers.
        game.last_played = FROZEN_NOW - 2.0 * 86_400.0;

        let manager = RunnerManager::at("/nonexistent");
        let label = resolved_runner_label(&manager, &game);
        let played = format_last_played(game.last_played, FROZEN_NOW);
        assert_eq!(played, "Played 2 days ago");

        let labels = RowLabels {
            runner: &label,
            last_played: &played,
        };
        let mut row: Element<'_, ()> = row(&game, &labels, ());
        let row_seen = traversal(&mut row);
        let row_texts = texts(&row_seen);
        assert_eq!(
            row_texts,
            [
                "CE",
                "Celeste",
                "Platformer · Linux native · Played 2 days ago",
                "Play"
            ]
        );

        let mut card: Element<'_, ()> = card(&game, &label, ());
        let card_seen = traversal(&mut card);
        let card_texts = texts(&card_seen);
        assert_eq!(
            card_texts,
            ["CE", "Celeste", "Platformer · Linux native", "Play"]
        );

        // The asymmetry itself, so a change that added the label to the card
        // fails here with a message saying why rather than with a diff.
        assert!(
            !card_texts.iter().any(|text| text.contains("Played")),
            "the card must not draw a last-played label: LibraryPage.qml:192 \
             draws `subtitle` alone, and only the list delegate (:269) appends \
             the timestamp. Got {card_texts:?}"
        );
    }

    /// A quiet game — one that has never been played — still shows a label, and
    /// it is the reference's `"Never played"` rather than a suppressed line.
    ///
    /// `bridge.py:311` computes `lastPlayed` for every row with no test, and
    /// the QML concatenates it unconditionally, so a port that hid the part for
    /// an unplayed game would render `"Shooter · System Wine"` where the
    /// reference renders three parts.
    #[test]
    fn an_unplayed_row_shows_never_played_rather_than_omitting_the_part() {
        let mut game = Game::new_named("Mystery");
        game.category = "Shooter".into();
        // `Game::new_named` leaves this at 0.0, the value `format_last_played`
        // maps to "Never played"; asserted so the fixture is what it claims.
        assert_eq!(game.last_played, 0.0);

        let played = format_last_played(game.last_played, FROZEN_NOW);
        assert_eq!(played, "Never played");
        let labels = RowLabels {
            runner: "System Wine",
            last_played: &played,
        };
        let mut row: Element<'_, ()> = row(&game, &labels, ());

        assert_eq!(
            texts(&traversal(&mut row)),
            ["MY", "Mystery", "Shooter · System Wine · Never played", "Play"]
        );
    }

    /// **The card and the row both carry the Play control, and it carries the
    /// game's id.**
    ///
    /// This is the control's existence asserted on the real builders: the word
    /// the user reads, and the id the control answers to. The two halves are
    /// separate because they fail separately — a card that drew the button
    /// without an id would satisfy the word and not the id, and an id on a
    /// control that stopped being drawn is exactly the state `#74` records
    /// (a declared thing nothing draws).
    ///
    /// The id is read through [`Operation::focusable`], not `container`, because
    /// that is where `cosmic::widget::button` reports it. The harness records
    /// both; see [`Collect::focusable`].
    #[test]
    fn both_delegates_draw_a_play_control_carrying_the_games_id() {
        // The word is the reference's, spelled out rather than taken from
        // `PLAY_LABEL`: an assertion that reads the constant it is checking
        // agrees with any typo in it. `LibraryPage.qml` says `text: "Play"` on
        // both the card's button (`:205`) and the row's (`:277`).
        assert_eq!(PLAY_LABEL, "Play");

        let game = Game::new_named("Half-Life 2");
        let id = play_button_id(&game.id);
        assert_eq!(id, format!("gamehandler.library.play.{}", game.id));

        let label = resolved_runner_label(&RunnerManager::at("/nonexistent"), &game);
        let played = format_last_played(0.0, FROZEN_NOW);
        let labels = RowLabels {
            runner: &label,
            last_played: &played,
        };

        let mut card: Element<'_, ()> = card(&game, &label, ());
        let card_seen = traversal(&mut card);
        assert!(
            texts(&card_seen).contains(&PLAY_LABEL),
            "the card must draw the Play control's word; it drew {:?}",
            texts(&card_seen)
        );
        assert!(
            ids(&card_seen).contains(&Id::from(id.clone())),
            "the card's Play control must answer to the game's id ({id:?}); the \
             traversal reported {:?}",
            ids(&card_seen)
        );

        let mut row: Element<'_, ()> = row(&game, &labels, ());
        let row_seen = traversal(&mut row);
        assert!(
            texts(&row_seen).contains(&PLAY_LABEL),
            "the row must draw the Play control's word; it drew {:?}",
            texts(&row_seen)
        );
        assert!(
            ids(&row_seen).contains(&Id::from(id.clone())),
            "the row's Play control must answer to the game's id ({id:?}); the \
             traversal reported {:?}",
            ids(&row_seen)
        );
    }

    /// **A double click on the card, or on the row, publishes the message the
    /// caller handed in — once.**
    ///
    /// Driven through a real `Widget::update` on the real builder's element, so
    /// this is the wiring and not the source text: the reference emits
    /// `backend.playGame(...)` from the card's `onDoubleTapped`
    /// (`LibraryPage.qml:161`) and from the row's `onDoubleClicked` (`:239`),
    /// and neither had any test before this one.
    ///
    /// **"Once" is the half worth having.** A double click is two presses, and
    /// the obvious wrong port is to hang `on_press` on the wrapper as well as
    /// `on_double_click` — which launches the game on the first click of every
    /// pair and again on the second. So the assertion is the whole vector, not
    /// `contains`: a `[Launch, Launch]` satisfies "it published a launch" and is
    /// the defect.
    ///
    /// The point is the card's centre. It has to be inside the tile for the
    /// wrapper to see it at all — `mouse_area` ignores events outside its
    /// bounds (`iced/widget/src/mouse_area.rs:446-464`) — which is also why a
    /// click on the Play control is not this test's subject: the button captures
    /// the event first, and what that means for the count is the next test.
    #[test]
    fn a_double_click_on_either_delegate_publishes_the_launch_once() {
        let game = Game::new_named("Half-Life 2");
        let label = resolved_runner_label(&RunnerManager::at("/nonexistent"), &game);
        let played = format_last_played(0.0, FROZEN_NOW);
        let labels = RowLabels {
            runner: &label,
            last_played: &played,
        };

        let mut card: Element<'_, &str> = card(&game, &label, "launch");
        let (cell_w, cell_h) = metrics::GRID_CELL;
        let card_hit = Point::new(cell_w / 2.0, cell_h / 2.0);
        assert_eq!(
            published_by_double_click(&mut card, card_hit),
            ["launch"],
            "a double click on the card must publish the caller's message \
             exactly once"
        );

        let mut row: Element<'_, &str> = row(&game, &labels, "launch");
        let row_hit = Point::new(20.0, metrics::LIST_ROW_HEIGHT / 2.0);
        assert_eq!(
            published_by_double_click(&mut row, row_hit),
            ["launch"],
            "a double click on the row must publish the caller's message \
             exactly once"
        );
    }

    /// **Two games' Play controls are two keys, not one key twice.**
    ///
    /// The reason [`play_button_id`] takes the game's id. iced's `Id` is a key
    /// and both of its uses are lookups — the accessibility `Action::Click` that
    /// iced's button matches on `self.id == *event_id`, and any future
    /// `operation::focus` — so a constant here would put the same key on every
    /// card in the grid. A test that only ever built one card could not tell the
    /// difference, which is why this one builds two.
    ///
    /// It also pins what the id is *made of*: a scheme that hashed the id, or
    /// dropped it, would still produce two distinct keys here, so the second
    /// assertion is the one that says the game is in the key.
    #[test]
    fn two_games_play_controls_are_two_ids() {
        let one = Game::new_named("Half-Life 2");
        let two = Game::new_named("Celeste");

        assert_ne!(play_button_id(&one.id), play_button_id(&two.id));
        assert!(play_button_id(&one.id).contains(&one.id));
        assert!(
            !play_button_id(&one.id).contains(&two.id),
            "a card must not answer to another game's key"
        );

        let mut card: Element<'_, ()> = card(&one, "", ());
        let seen = traversal(&mut card);
        assert!(
            !ids(&seen).contains(&Id::from(play_button_id(&two.id))),
            "the card for {:?} reported {:?}'s id: {:?}",
            one.id,
            two.id,
            ids(&seen)
        );
    }

    /// A card's cover box is the one the metrics module computes, and a card
    /// with no cover is asking for initials sized against that box rather than
    /// against the whole cell.
    ///
    /// This is the arithmetic the widget does, asserted directly: the cover
    /// box is the cell less its margins and chrome, and the initials are sized
    /// from the box, so a change that sized them from `GRID_CELL` fails here.
    #[test]
    fn a_cards_cover_box_and_its_initials_come_from_the_same_rectangle() {
        let (w, h) = metrics::card_cover_box(
            metrics::GRID_CELL,
            metrics::CARD_CHROME_HEIGHT,
            metrics::CARD_MARGIN,
        );
        assert_eq!((w, h), (188.0, 218.0));
        // The cover box is narrower than the cell, so the initials are sized
        // from 188 and not from 200: 64 rather than 68. Asserted as both
        // numbers, because a single `assert_ne!` would pass on a one-pixel
        // difference and this is the check that the card sizes from the box.
        assert_eq!(metrics::initials_size(w, h, false), 64.0);
        assert_ne!(metrics::initials_size(w, h, false), 68.0);
        assert_ne!(
            metrics::initials_size(w, h, false),
            metrics::initials_size(metrics::GRID_CELL.0, metrics::GRID_CELL.1, false),
            "initials sized from the cell rather than from the cover box would \
             be 4px larger here"
        );
    }

    /// **The card's initials are the full-size ones.** A card is not a compact
    /// drawing, so its initials come from `initials_size(w, h, false)` — 64
    /// points at the card's 188×218 cover box — and not from the compact rule's
    /// 92.
    ///
    /// The claim is made without knowing the font's line-height ratio, which is
    /// the property that makes this test portable: every string in a card is
    /// laid out by the same font stack, so the ratio cancels between the
    /// initials and the game's name and the quotient of the two heights *is*
    /// the quotient of the two font sizes. Asserting a height in pixels would
    /// pin the font instead, and fail on a machine with different metrics.
    ///
    /// The two candidate answers are written out rather than taken from
    /// [`metrics::initials_size`], so that a change to the rule cannot make the
    /// test agree with the widget about the wrong number.
    #[test]
    fn a_cards_initials_are_the_full_size_rule_and_not_the_compact_one() {
        assert_eq!(metrics::initials_size(188.0, 218.0, false), 64.0);
        assert_eq!(metrics::initials_size(188.0, 218.0, true), 92.0);

        // "Halo" so that the initials "HA" cannot be confused with the name.
        let game = Game::new_named("Halo");
        let mut card: Element<'_, ()> = card(&game, "", ());
        let seen = traversal(&mut card);
        let ratio = drawn(&seen, "HA").height / drawn(&seen, "Halo").height;

        // `name_and_subtitle` sets the name at 14.0.
        let expected = 64.0 / 14.0;
        assert!(
            (ratio - expected).abs() < 0.05,
            "the initials/name height quotient should be 64/14 = {expected:.3} \
             for the full-size rule; got {ratio:.3}, and the compact rule would \
             give 92/14 = {:.3}",
            92.0 / 14.0
        );
    }

    /// **The row's initials are the compact ones**, which is how the row's
    /// drawing is told apart from the tile's.
    ///
    /// This is also what catches a `row` that stopped asking for
    /// [`row_cover_spec`] and called [`cover_tile`] instead: the drawn height
    /// of a row's cover is clamped to the line, so the *box* difference (50.4
    /// against 54) never reaches the layout — but `cover_tile` passes
    /// `compact: false`, and the initials fall from 21 points to 12.
    #[test]
    fn a_rows_initials_are_the_compact_rule_and_not_the_tiles() {
        assert_eq!(metrics::initials_size(36.0, 50.4, true), 21.0);
        assert_eq!(
            metrics::initials_size(36.0, 54.0, false),
            12.0,
            "the tile's rule at the row's width, which is what a row drawn \
             through `cover_tile` would use"
        );

        let game = Game::new_named("Halo");
        // This test is about the text *sizes*, so the two labels are empty —
        // the row still draws its last-played line, but as an empty string it
        // contributes no glyphs to measure.
        let labels = RowLabels {
            runner: "",
            last_played: "",
        };
        let mut row: Element<'_, ()> = row(&game, &labels, ());
        let seen = traversal(&mut row);
        let ratio = drawn(&seen, "HA").height / drawn(&seen, "Halo").height;

        let expected = 21.0 / 14.0;
        assert!(
            (ratio - expected).abs() < 0.05,
            "the initials/name height quotient should be 21/14 = {expected:.3} \
             for the compact rule; got {ratio:.3}, and the tile's rule would \
             give 12/14 = {:.3}",
            12.0 / 14.0
        );
    }

    /// The list row's box is the fixed strip, not the tile aspect. Those two
    /// differ, and a refactor that made `row` call `cover_tile` would give
    /// 36×54 instead of 36×50.4 — close enough to look right in a screenshot.
    ///
    /// Pinned on the spec rather than on the drawn box because the row clamps
    /// its cover to the line height, so both boxes are drawn 36×43.2 and the
    /// *visible* difference is carried by the initials instead — see
    /// `a_rows_initials_are_the_compact_rule_and_not_the_tiles`, which calls
    /// the real `row`.
    #[test]
    fn a_rows_spec_is_the_fixed_strip_and_not_the_tile_aspect() {
        let row = row_cover_spec();
        assert_eq!(row.width, 36.0);
        // f32 cannot represent 2.8, so `18 * 2.8` lands on 50.399998; see the
        // note in `metrics`.
        assert!(
            (row.height - 50.4).abs() < 1e-4,
            "the row's cover should be 18 * 2.8 = 50.4 tall, got {}",
            row.height
        );
        assert!(row.compact, "a row is the compact drawing");
        assert_eq!(metrics::tile_height(row.width), 54.0);
    }

    /// **What each cover composition is made of**, read off the ids a real
    /// [`cover_box`] produces: a photograph is the picture alone, an icon is
    /// the picture inside the plate, and a game with no artwork is the plate
    /// with its initials on it.
    ///
    /// The ids are the whole test, and they are the only way to see this.
    /// `Widget` has no `as_any` and iced has no downcast, so which nodes a
    /// builder composed is otherwise invisible from outside it — and a
    /// photograph given a plate behind it renders plausibly and crops nothing,
    /// which is what makes it worth an assertion.
    #[test]
    fn a_photograph_is_drawn_alone_while_an_icon_and_a_placeholders_are_on_the_plate() {
        let spec = card_cover_spec();

        let with_photo_game = with_photo("Half-Life 2");
        let mut photo: Element<'_, ()> = cover_box(&with_photo_game, spec);
        assert_eq!(
            ids(&traversal(&mut photo)),
            [Id::from(PICTURE_ID)],
            "a photograph fills the box; there is no plate behind it"
        );

        let with_icon_game = with_icon("Half-Life 2");
        let mut icon: Element<'_, ()> = cover_box(&with_icon_game, spec);
        assert_eq!(
            ids(&traversal(&mut icon)),
            [Id::from(PLATE_ID), Id::from(PICTURE_ID)],
            "an icon sits on the plate, the way the QML's `showPlate` says"
        );

        let bare = Game::new_named("Half-Life 2");
        let mut placeholder: Element<'_, ()> = cover_box(&bare, spec);
        assert_eq!(
            ids(&traversal(&mut placeholder)),
            [Id::from(PLATE_ID), Id::from(INITIALS_ID)],
            "a game with no artwork is the plate and its initials"
        );
    }

    /// The icon's inset reaches the layout: inside the plate the picture is
    /// smaller than the box by [`metrics::ICON_INSET`] on every side, because
    /// an icon is letterboxed rather than cropped.
    ///
    /// Read from the laid-out tree, since an `Image` has no id and no
    /// `operate` — the leaf box is where its size actually lands.
    ///
    /// # What the photograph half can and cannot show (#46)
    ///
    /// That a photograph's box is the full 188x218 and not the inset one. What
    /// it **cannot** show is that a picture is there at all: [`framed`] pins the
    /// container with `width(Fixed(..))`/`height(Fixed(..))`,
    /// `Limits::width(Length::Fixed(x))` sets `min == max == x`
    /// (`iced/core/src/layout/limits.rs:60-66`), and `Image::layout` resolves
    /// `intrinsic.min(max).max(min)` (`iced/widget/src/image.rs:257-268`) — so
    /// the leaf is 188x218 for *any* intrinsic, including the `Size::ZERO` a
    /// failed decode produces. Measured, all three in this binary:
    ///
    /// | cover | `measure_image` | `leaves` |
    /// |---|---|---|
    /// | `WEBP_COVER` | `Some(24x16)` | 188x218 |
    /// | twelve bytes of PNG header | `None` | 188x218 |
    /// | no file at all (`Prey`) | `None` | 78.528x89.6 |
    ///
    /// The plate row's height is `spec.height - 2 * 64.2`, the same 89.6 for
    /// every name; its width is the drawn initials and so moves with the name
    /// (`Celeste` 76.032, `Bare` 82.496, `""` 27.776 — all measured, all at
    /// this spec). It is named here because a bare 78.528 would be a number no
    /// reader could reproduce: it is what `Prey`'s two initials measure.
    ///
    /// The third row is why this test is not worthless — it does separate a
    /// plate from a photograph. The first two are why it needed
    /// [`a_webp_cover_is_decoded`] beside it: before that, the photograph here
    /// could not decode at all and this assertion still passed.
    #[test]
    fn an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset() {
        let spec = card_cover_spec();
        assert_eq!((spec.width, spec.height), (188.0, 218.0));
        assert_eq!(metrics::ICON_INSET, 18.0);

        let with_icon_game = with_icon("Half-Life 2");
        let mut icon: Element<'_, ()> = cover_box(&with_icon_game, spec);
        assert_eq!(
            leaves(&mut icon),
            [Size::new(188.0 - 2.0 * 18.0, 218.0 - 2.0 * 18.0)],
            "the icon's box should be the plate's, less the inset on each side"
        );

        // A photograph is cropped to the box instead, so its inset is zero.
        let with_photo_game = with_photo("Half-Life 2");
        assert_decodes(&with_photo_game, "the photograph this branch draws");
        let mut photo: Element<'_, ()> = cover_box(&with_photo_game, spec);
        assert_eq!(
            leaves(&mut photo),
            [Size::new(188.0, 218.0)],
            "a photograph fills the box rather than being inset into it"
        );
    }

    /// **A `.webp` cover is decoded, not merely classified.**
    ///
    /// `CoverSource::classify` reads content, not suffixes — it is an ICO-magic
    /// check — so it answers `Photo` for these bytes exactly as it answered
    /// `Photo` for the twelve bytes of PNG header that used to stand in for a
    /// photograph. Classifying is not decoding, and D-29 exists for the decoder:
    /// `animated-image` is the only feature that turns on `image/webp`, and
    /// without it a user's own `.webp` cover — which `covers` stores verbatim
    /// under that suffix, including files the Python application already wrote
    /// to the shared data directory — fails to open.
    ///
    /// # The assertion is on `measure_image`, and it has to be
    ///
    /// It is the only observable that moves. See the table on
    /// `an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset`: the
    /// laid-out box is the pinned 188x218 for a decoded image and for an
    /// undecodable one alike, so a `.webp` assertion written the way the
    /// photograph tests are written would pass with the decoder switched off —
    /// which is the defect (#46) this test was written to close.
    ///
    /// Being a check rather than a claim: removing `"animated-image"` from
    /// `crates/app/Cargo.toml`'s `libcosmic` features makes this fail with
    /// `left: None`, while every other test in the binary stays green. Measured
    /// both ways.
    #[test]
    fn a_webp_cover_is_decoded() {
        let game = with_photo("Half-Life 2");
        assert_decodes(&game, "the .webp fixture");
    }

    /// **The icon fixture does not decode** — and the plate lays out anyway.
    ///
    /// `with_icon`'s eight bytes are the ICO magic and a claimed directory
    /// entry, and then nothing: the entry `10 10` begins is never finished, so
    /// there is no picture behind them. Nothing in this file asserted an icon
    /// draws, which is precisely why the fact was invisible and worth writing
    /// down — see `with_icon`'s own comment, which this test keeps honest.
    ///
    /// It is the same blindness #46 named, on the other fixture. The plate's
    /// size comes from the spec, so a *size* assertion is satisfied by a
    /// decoded icon and by these bytes alike; `measure_image` is the one
    /// reading that separates them, and this is what it reads here.
    ///
    /// If the fixture is ever made a real ICO, this failing is correct: it
    /// means that comment is now wrong too.
    #[test]
    fn the_icon_fixture_is_not_a_decodable_icon() {
        let game = with_icon("Half-Life 2");
        let measured = renderer().measure_image(&image::Handle::from_path(&game.cover_path));
        assert_eq!(
            measured,
            None,
            "these eight bytes are a truncated ICO header — a magic that classifies \
             as an icon and no image data — so the decoder must refuse them; \
             `Some(_)` means they now decode and `with_icon`'s comment no longer \
             describes the fixture"
        );
    }

    /// The rendered size of `game`'s cover, which is `None` when it cannot be
    /// decoded.
    ///
    /// The one reading that separates a picture from bytes that merely look
    /// like one, so it is factored out rather than spelled twice.
    fn assert_decodes(game: &Game, what: &str) {
        let measured = renderer().measure_image(&image::Handle::from_path(&game.cover_path));
        assert_eq!(
            measured,
            Some(Size::new(WEBP_COVER_SIZE.0, WEBP_COVER_SIZE.1)),
            "{what} should decode as a {0}x{1} image; `None` means the decoder \
             could not read it, which is what a build without `image/webp` does \
             to a user's .webp cover — and `leaves` cannot tell the difference",
            WEBP_COVER_SIZE.0,
            WEBP_COVER_SIZE.1
        );
    }

    /// The card's corner radius is the one the style gives the renderer.
    ///
    /// This is the only radius in this file whose value can be read back at
    /// all. A `Container`'s style is never exposed on the widget —
    /// `Container::id()` returns its *content's* id and there is no getter — so
    /// the choice is between calling the style function the widget uses and not
    /// checking the number. What this proves is that the style the card hands
    /// the renderer carries radius 14; what it cannot prove is anything about a
    /// radius that reaches no style at all.
    #[test]
    fn a_cards_corner_radius_is_the_declared_one() {
        let style = card_style(&cosmic::Theme::dark());
        // 14 is `LibraryPage.qml:155`, and `metrics::CARD_RADIUS` is where the
        // port keeps it: written out, so the assertion fails whether the
        // constant moves or the style stops reading it.
        assert_eq!(style.border.radius, Radius::from(14.0));
        assert_eq!(metrics::CARD_RADIUS, 14.0);
    }

    /// A plate's corners are the radius its spec asks for — the tile's radius
    /// for a card, the compact one for a row.
    ///
    /// [`plate_style`] is where a radius becomes a `container::Style`, and the
    /// style is the last point at which it can be read (see the card-radius
    /// test above). The link this does **not** cover is the argument `plate`
    /// passes — nothing outside the widget can see a `Style` the widget holds —
    /// so a `plate` that stopped passing `spec.radius` would survive here. That
    /// gap is real and is not papered over.
    #[test]
    fn a_plates_corners_are_the_radius_its_spec_asks_for() {
        let card = card_cover_spec();
        assert_eq!(card.radius, 10.0, "the tile's radius, `CoverArt.qml`");
        assert_eq!(
            plate_style(0, card.radius).border.radius,
            Radius::from(10.0)
        );

        let row = row_cover_spec();
        assert_eq!(row.radius, 6.0, "the compact radius, `CoverArt.qml`");
        assert_eq!(plate_style(0, row.radius).border.radius, Radius::from(6.0));
        assert_eq!(preview_cover_spec().radius, 6.0);
    }

    /// The plate's gradient runs from the shade's first colour at the top: the
    /// stops come out in order, with offset 0.0 carrying `shade`'s first
    /// colour and 1.0 its second.
    ///
    /// **This test previously asserted the wrong thing.** It checked that the
    /// two colours differ, which stays true if they are swapped — so putting
    /// the darker colour at the top, the exact mistake it was written to
    /// catch, passed. It was found by mutation-testing the implementation
    /// rather than by reading the test. Asserting *which colour is at which
    /// offset* is the version that fails when they trade places.
    #[test]
    fn the_plate_gradient_puts_the_first_shade_colour_at_the_top() {
        let (top, bottom) = cover::shade(0);
        let stops = plate_stops(0);

        assert_eq!(stops[0].0, 0.0, "the first stop must be the top edge");
        assert_eq!(stops[1].0, 1.0, "the second stop must be the bottom edge");
        assert_eq!(
            stops[0].1,
            rgb(top),
            "offset 0.0 must carry shade()'s first colour"
        );
        assert_eq!(
            stops[1].1,
            rgb(bottom),
            "offset 1.0 must carry shade()'s second colour"
        );
        assert_ne!(stops[0].1, stops[1].1, "a flat gradient is not a gradient");
    }

    /// The angle iced is given resolves to top-to-bottom rather than
    /// side-to-side.
    ///
    /// The vector is computed from the same formula iced uses
    /// (`iced_core/src/angle.rs:86-98`): `(cos(a - PI/2), sin(a - PI/2))`, and
    /// only a downward vector makes offset 0.0 the top edge.
    ///
    /// **This reads [`PLATE_GRADIENT_ANGLE`], not a literal.** The first
    /// version of this test wrote `PI` out itself, so it asserted a property
    /// of its own arithmetic and passed with the implementation set to
    /// `PI/2` — found by mutation, not by reading. Deriving the vector from
    /// the constant the widget actually uses is what makes the mutation fail.
    #[test]
    fn the_plate_gradient_angle_points_down_not_sideways() {
        let angle = PLATE_GRADIENT_ANGLE - std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (angle.cos(), angle.sin());
        assert!(dy > 0.0, "the gradient must run downwards (got dy={dy})");
        assert!(
            dx.abs() < 1e-5,
            "the gradient must not run sideways (got dx={dx})"
        );
    }

    // ---- Reaching the widget a builder returned ---------------------------
    //
    // Every test below calls the **real builder** — `card`, `row`,
    // `cover_box`, `cover_preview` — and reads what came back. That is possible
    // only through the two things iced exposes about a widget from outside: its
    // [`Id`], which `Widget::id()` returns and the operation traversal reports,
    // and the layout the framework computes from it. `Widget` has no `as_any`
    // in this version and iced has no downcast anywhere, so a builder's return
    // value cannot be inspected by type — and a `container::Style` cannot be
    // read back off the widget either, because `Container::id()` returns its
    // *content's* id and the style is only ever handed to the renderer.
    //
    // So each assertion is either "which nodes are present, in what order",
    // read from the ids those nodes carry, or "what the framework was told
    // about them" — a bounding box, or a string a text widget handed the
    // traversal. Both come from a genuine `Widget::layout` and a genuine
    // `Widget::operate` over a real element, not from a stand-in.

    /// One thing the framework reports about the widget tree.
    #[derive(Debug, Clone)]
    struct Seen {
        id: Option<Id>,
        bounds: Rectangle,
        text: Option<String>,
    }

    /// Collects what a traversal reports. `traverse` calls `operate(self)` so
    /// the widgets keep descending — the contract `Operation` documents.
    #[derive(Default)]
    struct Collect(Vec<Seen>);

    impl Operation for Collect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            self.0.push(Seen {
                id: id.cloned(),
                bounds,
                text: None,
            });
        }

        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            self.0.push(Seen {
                id: None,
                bounds,
                text: Some(text.to_string()),
            });
        }

        /// Where a `cosmic::widget::button` reports its [`Id`].
        ///
        /// Not in [`Self::container`]: the button's `operate` calls
        /// `operation.container(None, layout.bounds())` — always `None` — and
        /// then `operation.focusable(Some(&self.id), …)`
        /// (`libcosmic src/widget/button/widget.rs:345` and `:359`). So a test
        /// that read only `container` would find a button's id nowhere and
        /// conclude the control had none, which is the "check that cannot see
        /// the thing it checks" shape. Recorded here rather than asserted
        /// against a button built in the test, so the id is read off the same
        /// traversal that reads the rest of the tree.
        fn focusable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            _state: &mut dyn Focusable,
        ) {
            self.0.push(Seen {
                id: id.cloned(),
                bounds,
                text: None,
            });
        }
    }

    /// A real renderer, for measuring text.
    ///
    /// `iced_tiny_skia` is a pure-software backend, so this needs no display
    /// and draws nothing — `layout` wants it only to ask the font stack how
    /// wide a string is.
    fn renderer() -> cosmic::Renderer {
        cosmic::Renderer::new(Font::default(), Pixels(16.0))
    }

    /// Lay out a real element and traverse it, and report what the framework
    /// was told.
    fn traversal<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<Seen> {
        let renderer = renderer();
        let mut tree = Tree::new(el.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        // `Widget::layout` and `Widget::operate` both take the renderer by
        // shared reference — a widget measures text through it and does not
        // draw — so neither needs a `&mut` here and clippy says so.
        let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let mut collect = Collect::default();
        el.as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
        collect.0
    }

    /// The messages a real double click at `at` makes the element publish.
    ///
    /// A genuine `Widget::update` on a real element with a real `Shell`, so
    /// what this reports is what the runtime would route — not a reading of the
    /// builder's source.
    ///
    /// **The sequence is four events, not two, and that is the thing this
    /// helper exists to get right.** A double click is press → release → press
    /// → release, and which of the two the message lands on is not obvious: it
    /// is the **second release**.
    /// `MouseArea` checks `on_double_press` while handling a press and
    /// `on_double_click` while handling a release
    /// (`iced/widget/src/mouse_area.rs:476` against `:506`), so a helper that
    /// sent only presses would publish nothing and a test built on it would
    /// report a working card as broken. Measured: that is exactly what this
    /// test did before the helper sent releases.
    ///
    /// Consecutive is within 6 logical pixels and 300 ms
    /// (`iced/core/src/mouse/click.rs:79-89`); every event here shares a point
    /// and the loop outruns no clock.
    ///
    /// `Shell::new` takes the `Vec` it appends to, so the published messages are
    /// exactly the elements of the returned vector — each event gets a fresh
    /// `Shell` over the same vector, which is what lets one event's capture not
    /// hide the next event's publication.
    fn published_by_double_click<M: Clone + 'static>(
        el: &mut Element<'_, M>,
        at: Point,
    ) -> Vec<M> {
        let renderer = renderer();
        let mut tree = Tree::new(el.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let layout = Layout::new(&node);
        let cursor = cosmic::iced::advanced::mouse::Cursor::Available(at);
        let viewport = Rectangle::new(Point::ORIGIN, Size::new(f32::INFINITY, f32::INFINITY));
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));

        let mut published = Vec::new();
        for event in [&press, &release, &press, &release] {
            let mut clipboard = cosmic::iced::advanced::clipboard::Null;
            let mut shell = Shell::new(&mut published);
            el.as_widget_mut().update(
                &mut tree,
                event,
                layout,
                cursor,
                &renderer,
                &mut clipboard,
                &mut shell,
                &viewport,
            );
        }
        published
    }

    /// The ids the traversal reported, in the order it reported them.
    fn ids(seen: &[Seen]) -> Vec<Id> {
        seen.iter()
            .filter_map(|seen| seen.id.clone())
            .collect()
    }

    /// The strings the traversal was handed, in order.
    fn texts(seen: &[Seen]) -> Vec<&str> {
        seen.iter()
            .filter_map(|seen| seen.text.as_deref())
            .collect()
    }

    /// The box a drawn string was laid out in.
    fn drawn<'a>(seen: &'a [Seen], text: &str) -> &'a Rectangle {
        seen.iter()
            .find(|seen| seen.text.as_deref() == Some(text))
            .map(|seen| &seen.bounds)
            .unwrap_or_else(|| {
                panic!("nothing drew {text:?}; the traversal drew {:?}", texts(seen))
            })
    }

    /// The sizes of the leaves of a real element's laid-out tree, in order.
    ///
    /// A leaf is where the picture ends up: `Image` has no `Id` and does not
    /// implement `operate`, so the only place its box can be read is in the
    /// layout the framework computed for it.
    fn leaves<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<Size> {
        fn walk(node: &Node, out: &mut Vec<Size>) {
            if node.children().is_empty() {
                out.push(node.size());
            }
            for child in node.children() {
                walk(child, out);
            }
        }

        let renderer = renderer();
        let mut tree = Tree::new(el.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let mut out = Vec::new();
        walk(&node, &mut out);
        out
    }

    /// The path to a real file with exactly these bytes.
    ///
    /// [`CoverSource::classify`] reads the file — a path that does not exist is
    /// a [`CoverSource::Plate`] whatever it is called — so a test that wants a
    /// photograph or an icon has to put bytes on disk. The name carries no
    /// extension on purpose: which composition a cover gets is decided from the
    /// content and never from the suffix (see [`super::cover`]'s module docs),
    /// and a fixture called `.png` would leave that untested either way.
    fn cover_fixture(stem: &str, bytes: &[u8]) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gamehandler-widgets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temporary directory");
        // A distinct path per call, so no two tests can read a half-written
        // file from the other.
        let path = dir.join(format!("{stem}-{n}"));
        std::fs::write(&path, bytes).expect("a writable fixture");
        path.to_string_lossy().into_owned()
    }

    /// A real `.webp`: 24x16, lossless VP8L, 118 bytes, from libwebp.
    ///
    /// Embedded rather than generated at test time, and shipped without the
    /// generator, because generating one would need a webp *encoder* and the
    /// point is the *decoder*: `image/webp` is the only thing that reads these
    /// bytes, and it arrives through libcosmic's `animated-image` feature
    /// (D-29), which is the single reason that feature is on. Nothing at test
    /// time needs libwebp or a C toolchain.
    ///
    /// Generated once, off-tree, by a 30-line program against libwebp 1.6.0: a
    /// 24x16 RGBA buffer — four quadrants plus a per-pixel ramp on the blue
    /// channel, so it is a picture rather than a flat block — through
    /// `WebPEncodeLosslessRGBA(px, 24, 16, 24 * 4, &out)`, the output written
    /// verbatim. That is the whole recipe; these bytes are the whole fixture.
    const WEBP_COVER: [u8; 118] = [
        0x52, 0x49, 0x46, 0x46, 0x6e, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
        0x56, 0x50, 0x38, 0x4c, 0x62, 0x00, 0x00, 0x00, 0x2f, 0x17, 0xc0, 0x03,
        0x00, 0xcd, 0x95, 0x21, 0xa2, 0xff, 0xb1, 0x2b, 0x78, 0x14, 0xbc, 0xff,
        0x01, 0x26, 0x91, 0x24, 0x49, 0x4a, 0xbc, 0x4a, 0x56, 0xd3, 0xfa, 0x77,
        0xf2, 0x5d, 0xa1, 0xc3, 0x8a, 0x1a, 0x49, 0x8a, 0x6a, 0x2f, 0xc0, 0xff,
        0x0b, 0x11, 0x48, 0x42, 0x8c, 0xa2, 0xb6, 0x91, 0x1c, 0xbf, 0x76, 0xaf,
        0xf1, 0x87, 0x70, 0x20, 0x7b, 0x4c, 0xc0, 0xfc, 0xe8, 0xaf, 0xee, 0x70,
        0xfa, 0xa3, 0x33, 0x06, 0x00, 0x58, 0x04, 0xc2, 0x0d, 0x6c, 0xb2, 0x02,
        0x87, 0x52, 0x81, 0x4b, 0xad, 0xc0, 0xa3, 0x55, 0xe0, 0xd3, 0xab, 0xcf,
        0x8b, 0x07, 0x26, 0x3c, 0x27, 0x3a, 0x00, 0x03, 0xbf, 0x19,
    ];

    /// The size [`WEBP_COVER`] declares: 24x16, as encoded.
    const WEBP_COVER_SIZE: (u32, u32) = (24, 16);

    /// A game whose cover is a photograph: a real `.webp` that decodes.
    ///
    /// **It used to be twelve bytes of PNG header** — `\x89PNG\r\n\x1a\n` and
    /// the start of an `IHDR` — which `CoverSource::classify` reads as a
    /// photograph, because classification is an ICO-magic check and not a
    /// suffix check. The trouble was that nothing noticed the difference: a
    /// failed decode is invisible in the laid-out tree (#46), so every
    /// assertion these fixtures fed was satisfied whether or not a picture was
    /// ever drawn. Pointing the fixture at bytes that really decode is what
    /// makes "a photograph" mean a photograph.
    fn with_photo(name: &str) -> Game {
        let mut game = Game::new_named(name);
        game.cover_path = cover_fixture("photo", &WEBP_COVER);
        game
    }

    /// A game whose cover is an icon: eight bytes of ICO header, and no more.
    ///
    /// `00 00 01 00` is the ICO magic that `CoverSource::classify` checks, and
    /// `01 00` claims one directory entry — which is what makes this path an
    /// icon rather than a photograph. The file stops there: `10 10` begins a
    /// 16x16 entry that is never finished, so **these bytes do not decode**.
    /// There is no picture behind them.
    ///
    /// Nothing here asserts that one does, and that is the point of writing it
    /// down rather than a gap to be closed by the next reader. The plate's size
    /// comes from the spec, not from the intrinsic, so a failed icon decode is
    /// exactly as invisible in the laid-out tree as a failed photograph decode
    /// was (#46) — `with_photo` above is the fixture that had to be made real
    /// for that reason. This one was not, because no assertion depends on the
    /// icon drawing. Do not read it as evidence that an icon renders.
    fn with_icon(name: &str) -> Game {
        let mut game = Game::new_named(name);
        game.cover_path = cover_fixture("icon", &[0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x10, 0x10]);
        game
    }

    /// The two 8-bit triples become the colours the plate draws, with no
    /// channel swapped and no gamma applied on the way.
    #[test]
    fn the_shade_colours_reach_the_renderer_unchanged() {
        let (top, _) = cover::shade(1);
        let colour = rgb(top);
        assert_eq!(colour.r, top[0] as f32 / 255.0);
        assert_eq!(colour.g, top[1] as f32 / 255.0);
        assert_eq!(colour.b, top[2] as f32 / 255.0);
        assert_eq!(colour.a, 1.0);
    }
}
