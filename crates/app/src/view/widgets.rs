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
use cosmic::widget::{Column, Row, Space, button, image, mouse_area, text};
use gamehandler_core::models::Game;
use gamehandler_core::runners::RunnerManager;

use super::cover::{self, CoverSource};
use super::cover_cache::CoverCache;
use super::meta;
use super::metrics;

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
pub(crate) const PICTURE_ID: &str = "gamehandler.cover.picture";
const INITIALS_ID: &str = "gamehandler.cover.initials";
const PREVIEW_LABEL_ID: &str = "gamehandler.cover.preview-label";

/// The Play control's label, which is the reference's word for it: `text:
/// "Play"` on both the card's button (`LibraryPage.qml:205`) and the row's
/// (`:277`).
const PLAY_LABEL: &str = "Play";

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
///
/// # The kind comes from the cache, and that is PERF-01's fix
///
/// This function used to call [`CoverSource::classify`] itself, on every frame,
/// per tile — and [`framed`] called it again for the same tile, so the file was
/// `stat`ed and `open`ed twice per drawn cover per frame. The audit measured
/// 6,624 cover-file syscalls over a 100-redraw window in which **all 333 cover
/// paths were touched every frame**, in a window that shows a few dozen tiles
/// (`docs/audit/PERFORMANCE.md`, PERF-01). The answer is a function of the file
/// and cannot change between frames, so it is asked once and kept; the argument
/// for the cache's lifetime and invalidation is in [`CoverCache`]'s docs.
///
/// `cache` is a parameter rather than a `thread_local` on purpose: the page's
/// dependencies are readable from its signature (see
/// [`super::library::LibraryPage`]), and a cache a test cannot substitute is a
/// cache a test cannot measure. Every `*_cover_spec` caller passes the one
/// [`crate::State::cover_cache`] the app owns.
pub fn cover_plan(cache: &CoverCache, game: &Game) -> CoverPlan {
    match cache.kind(&game.cover_path) {
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

/// A game's cover in an explicit box — a renderer for [`cover_plan`].
pub fn cover_box<'a, M: Clone + 'static>(
    cache: &CoverCache,
    game: &Game,
    spec: CoverSpec,
) -> Element<'a, M> {
    let plan = cover_plan(cache, game);
    plan_box(cache, game, spec, &plan)
}

/// A `Game`'s cover in an explicit box, from a plan that has **already** been
/// decided — the renderer behind [`cover_box`] and [`cover_preview`].
///
/// `plan` is an argument rather than a decision made here so that a caller which
/// has to know the plan anyway — [`cover_preview`], which draws words instead of
/// a picture in one of its three arms — reads the same one the drawing does.
/// Deciding it twice would be a second `classify` per frame (that is PERF-01)
/// and, worse, the two calls could in principle disagree; here they cannot.
///
/// `game` is plain `&Game` rather than `&'a Game`, and the element borrows it
/// for nothing: the plan already holds the only string derived from the game
/// that any arm draws, and what is left is the id ([`cover::accent_of`]) and the
/// cover path, both read and consumed during this call. That is what lets the
/// game form draw a preview of a game that does not exist anywhere yet — see
/// [`crate::state::GameForm::as_preview_game`].
fn plan_box<'a, M: Clone + 'static>(
    cache: &CoverCache,
    game: &Game,
    spec: CoverSpec,
    plan: &CoverPlan,
) -> Element<'a, M> {
    // Resolved once, above the match, so that the plate's gradient and the ink
    // drawn on it cannot come from different shades: `plate` fills with
    // `plate_style(accent, ..)` and `initials_only` picks its colour from the
    // same number. Two calls to `accent_of` would agree — it is a hash of the
    // id — but nothing in the types would say they had to.
    let accent = cover::accent_of(&game.id);

    match plan {
        CoverPlan::Photo => framed(cache, &game.cover_path, spec, 0.0),
        CoverPlan::IconOnPlate { inset } => {
            let inner = framed(cache, &game.cover_path, spec, *inset);
            plate(spec, accent, inner)
        }
        CoverPlan::InitialsOnPlate { text } => {
            let inner = initials_only(text.clone(), spec, accent);
            plate(spec, accent, inner)
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
        // The reference's `.name: "media-playback-start"` (`LibraryPage.qml:206`
        // and `:278`): the shared play triangle. This used to resolve the name
        // by `icon::from_name`, with a comment claiming the lookup "falls back
        // to a symbolic name when the theme has nothing" — measured in U7, it
        // does not, and the name resolved nowhere. Hence the embedded glyph.
        .leading_icon(crate::icons::handle(crate::icons::Icon::Play))
        .id(play_button_id(game_id).into())
        .on_press(on_play)
        .into()
}

/// The reference's own words for the "More actions" control, used for both its
/// tooltip and its accessible name.
///
/// `QQC2.ToolTip.text: "More actions"` (`LibraryPage.qml:211`), one string
/// bound twice for the reason UX-12's four sites record: a hint and a name that
/// come from one constant cannot drift.
pub const MORE_ACTIONS_LABEL: &str = "More actions";

/// Play, and the "More actions" control beside it — the pair both delegates
/// draw (`LibraryPage.qml:203-213`, `:275-284`).
///
/// One function for the two call sites, so the card and the row cannot disagree
/// about the pair or about their spacing. `on_more` is an `Option` because the
/// caller owns whether the actions are reachable: `None` draws Play alone, which
/// is what a card whose actions have nowhere to open gets — a button that
/// publishes a message nothing answers is worse than no button.
fn action_row<'a, M: Clone + 'static>(
    game_id: &str,
    on_play: M,
    on_more: Option<M>,
) -> Element<'a, M> {
    let mut row = Row::new()
        .push(play_button(game_id, on_play))
        .spacing(metrics::CARD_MARGIN)
        .align_y(Alignment::Center);
    row = row.push_maybe(on_more.map(|message| more_actions_button(game_id, message)));
    row.into()
}

/// The id of a game's "More actions" control, on its card and on its row.
///
/// Per game, for the reason [`play_button_id`] is: the control exists once per
/// game and the library draws every game at once.
pub fn more_actions_id(game_id: &str) -> String {
    format!("gamehandler.library.more.{game_id}")
}

/// The reference's "More actions" control, as both the card and the row draw
/// it — the keyboard route to the per-game actions (**UX-16**).
///
/// `LibraryPage.qml:209-213` (card) and `:281-284` (row) each draw a
/// `QQC2.ToolButton` beside Play with `icon.name: "view-more-symbolic"` and, on
/// the card, `QQC2.ToolTip.text: "More actions"`; both call
/// `page.openGameMenu(modelData, this)`, which pops the very `gameMenu` the
/// port renders as a right-click context menu. **A `QQC2.ToolButton` is in the
/// Tab ring**, so in the reference a keyboard user reaches every per-game
/// action — including "Remove from library" — through this control.
///
/// The port had no such control, and the toolkit's context menu opens from a
/// pointer button release alone (`src/widget/context_menu.rs:441-460`), so
/// until this button existed the only keyboard-reachable control on a game was
/// Play.
///
/// # Why it is built here and not at the two call sites
///
/// The card and the row must not drift — the same reason [`play_button`] is one
/// function. The icon is embedded rather than looked up by name for the reason
/// [`play_button`]'s doc records: `icon::from_name` resolves nowhere in this
/// build.
///
/// # What its press sends
///
/// `on_more`, which each call site builds as `Message::OpenGameMenu` with that
/// game's id — the arm that opens the layer
/// [`crate::view::library::game_menu_actions`] draws. (Spelled as a plain code
/// span rather than a link because this module may not name the app's
/// `Message` type at all: `view/mod.rs`'s layer guard reads this file's text,
/// and a link would carry the path it looks for.) It is *not* the menu's "Remove from library" action: the reference's
/// control opens the menu, and a control labelled "More actions" that went
/// straight to the destructive entry would skip the other seven and lie about
/// itself. The message is the caller's rather than built here because
/// `view/widgets.rs` names no `Message` of its own, which is the property that
/// keeps its tests message-free.
///
/// # The accessible name
///
/// `button::icon` publishes a node with **no label**: it sets no `name`, and the
/// only label source in `src/widget/button/widget.rs` is the `name` field the
/// *text* variant's `From` impl fills from its label (`text.rs:143-146`). So the
/// button is wrapped in [`crate::view::a11y::tooltipped_button`] with the
/// reference's own tooltip text — the same repair UX-12 made at four other
/// icon-only controls.
fn more_actions_button<'a, M: Clone + 'static>(game_id: &str, on_more: M) -> Element<'a, M> {
    crate::view::a11y::tooltipped_button(
        button::icon(crate::icons::handle(crate::icons::Icon::ViewMore))
            .id(more_actions_id(game_id).into())
            .tooltip(MORE_ACTIONS_LABEL)
            .on_press(on_more.clone()),
        MORE_ACTIONS_LABEL,
        Some(on_more),
    )
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
    cache: &CoverCache,
    game: &'a Game,
    label: &str,
    on_play: M,
    on_more: Option<M>,
) -> Element<'a, M> {
    let (cell_w, cell_h) = metrics::GRID_CELL;
    let spec = card_cover_spec();

    let body = Column::new()
        .push(cover_box(cache, game, spec))
        .push(name_and_subtitle(game, subtitle_of(game, label)))
        .push(action_row(&game.id, on_play.clone(), on_more))
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
    cache: &CoverCache,
    game: &'a Game,
    labels: &RowLabels<'_>,
    on_play: M,
    on_more: Option<M>,
) -> Element<'a, M> {
    let line = Row::new()
        .push(cover_box(cache, game, row_cover_spec()))
        .push(name_and_subtitle(game, row_subtitle_of(game, labels)))
        .push(action_row(&game.id, on_play.clone(), on_more))
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
pub fn cover_preview<'a, M: Clone + 'static>(
    cache: &CoverCache,
    game: &Game,
    plan: &CoverPlan,
) -> Element<'a, M> {
    let spec = preview_cover_spec();

    match plan {
        CoverPlan::InitialsOnPlate { .. } => preview_label_widget(spec),
        CoverPlan::Photo | CoverPlan::IconOnPlate { .. } => plan_box(cache, game, spec, plan),
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
///
/// # Both lines are single-line, and that is not cosmetic
///
/// [`Wrapping::None`] on each, because a *wrapping* name is what broke the card:
/// a cell's interior is exactly filled by cover + text block + Play control, so
/// the block growing by one line comes out of the control, and the control is
/// what iced's `Column` squeezes last. Measured before this: on "The Elder
/// Scrolls V Skyrim Special Edition" the name took two lines (56.6 px instead of
/// 37.0) and the Play control laid out at **1.4000015 px** — measured when the
/// chrome was 70.0; at the corrected 81.0 the same wrap takes it to 12.4 px,
/// which is still a squeeze and still fails the same assertion. See
/// [`metrics::CARD_CHROME_HEIGHT`] and
/// `a_long_name_does_not_squeeze_the_play_control`.
///
/// It is also what the reference does, which is the better reason: the card's
/// two `QQC2.Label`s set `elide: Text.ElideRight` and no `wrapMode`
/// (`LibraryPage.qml:182-197`), and `QQC2.Label`'s default is `Text.NoWrap`. The
/// row's do the same (`:261-273`).
///
/// # The ellipsis, and a comment here that was wrong
///
/// The reference **elides** — it draws `The Elder Scrolls V Sky…` — and this
/// code used to say that was unreachable: "this iced has no ellipsis: `Wrapping`
/// is `None`, `Word`, `Glyph` or `WordOrGlyph`, and there is no
/// truncation-with-marker mode in this version", with the divergence filed as a
/// bound on the fix. **That was false**, and it was false in the direction that
/// costs the most: it talked the fix out of existing. [`Wrapping`] and
/// [`Ellipsize`] are *orthogonal* — `Wrapping` is how a line that does not fit
/// is broken, `Ellipsize` is what happens after that — and iced does expose the
/// second one: `Text::ellipsize` (`iced/core/src/widget/text.rs:166`) sets
/// `Format::ellipsize` (`:809`), which the graphics layer hands to the shaper as
/// `Buffer::set_ellipsize(cosmic_text::Ellipsize)`
/// (`iced/graphics/src/text/paragraph.rs:92`, `iced/graphics/src/text.rs:378`).
/// Under `Wrap::None` cosmic-text still ellipsizes: its `Wrap::None` branch calls
/// `layout_line(.., width_opt, ellipsize)`, and `layout_spans` computes
/// `check_ellipsizing = matches!(ellipsize, Start(_) | End(_)) && width_opt is
/// finite` (`cosmic-text-0.19.0/src/shape.rs:2316`, `:1741`). So the reference's
/// behaviour is reachable and this is it.
///
/// [`name_ellipsize`] holds the strategy, and its doc records what the unit test
/// can and cannot reach: the *value* is asserted, the *wiring* is iced's and is
/// verified by eye against the running app.
///
/// [`Wrapping`]: cosmic::iced::widget::text::Wrapping
/// [`Wrapping::None`]: cosmic::iced::widget::text::Wrapping::None
/// [`Ellipsize`]: cosmic::iced::widget::text::Ellipsize
fn name_and_subtitle<'a, M: Clone + 'static>(game: &'a Game, subtitle: String) -> Element<'a, M> {
    use cosmic::iced::widget::text::Wrapping;
    Column::new()
        .push(
            text(title_of(game))
                .size(metrics::CARD_NAME_SIZE)
                .wrapping(Wrapping::None)
                .ellipsize(name_ellipsize()),
        )
        .push(
            text(subtitle)
                .size(metrics::CARD_SUBTITLE_SIZE)
                .wrapping(Wrapping::None)
                .ellipsize(name_ellipsize()),
        )
        .spacing(metrics::CARD_TEXT_SPACING)
        .width(Length::Fill)
        .into()
}

/// The ellipsizing strategy for a card's or a row's name and subtitle.
///
/// `End(Lines(1))` is the reference's `Text.ElideRight` under `Text.NoWrap`
/// (`LibraryPage.qml:186`, `:195`, `:265`, `:272`): one line, cut at the end,
/// with the marker. [`Wrapping::None`] already forces the one line; the limit is
/// what makes the cut happen at the *right* place rather than at whatever byte
/// the layout ran out of room on.
///
/// # What the test below does and does not prove
///
/// [`Text`]'s `format` field is private and iced has no downcast anywhere, so
/// nothing in this suite can read an `ellipsize` back off a built widget — a
/// test that claimed to would be this project's dominant defect class, a check
/// that passes without inspecting what it claims. So the test asserts the
/// **strategy** (that this function returns what the reference asks for, and
/// that both cards and rows call it) and the doc names the **wiring** as iced's,
/// verified by eye against the running application rather than asserted here.
/// The distinction is the point: the row that replaced a false claim should not
/// replace it with an unfalsifiable one.
///
/// [`Wrapping::None`]: cosmic::iced::widget::text::Wrapping::None
/// [`Text`]: cosmic::iced::widget::Text
fn name_ellipsize() -> cosmic::iced::widget::text::Ellipsize {
    use cosmic::iced::core::text::EllipsizeHeightLimit;
    use cosmic::iced::widget::text::Ellipsize;

    Ellipsize::End(EllipsizeHeightLimit::Lines(1))
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
/// [`RunnerManager::label`] **used to be uncached** (`runners/mod.rs:1273`): it
/// joined the runners directory, tested `exists()`, and constructed a
/// `ProtonRunner` to read and parse the build's metadata and ask for its family
/// label — on every call, for every game. A builder runs every frame, so
/// resolving this inside [`card`] or [`row`] put a filesystem walk, a file open
/// and a JSON parse per game per frame on the render path. That is the same
/// defect as spawning a process there, only cheaper.
///
/// It is memoised now, against the runner directory's mtime (PERF-06), which
/// leaves one `stat` per call for the key where there was a walk. The rule
/// below did not change with it, and the reason it did not is the second
/// reason: the label is resolved when the row data is assembled and carried as
/// a string from then on, because **the runner list changes only on install or
/// uninstall**, both of which already re-read the library — so resolving at
/// build time is correct *and* free, where resolving per frame would be merely
/// correct.
///
/// Resolving here rather than threading a `&RunnerManager` through the
/// builders also keeps them free functions over the game: their `Element`
/// borrows the game alone, so no caller's lifetime is welded to the manager's.
///
/// # The line number, and why it is not the authority
///
/// `runners/mod.rs:1273` is a **point-in-time** fact about a file that grows.
/// This citation was `mod.rs:1041` when it was written (`13e9806`), which was
/// correct then; `runners/mod.rs` has since gained 58 lines above `label`, and
/// every citation of that function's body moved with them — this one and the
/// one in the test below, both corrected together. The same drift hit
/// `view/settings.rs`'s citation of `RunnerManager::choices` (`mod.rs:1061` →
/// `runners/mod.rs:1119`), and the +58 accounts for both. PERF-06 moved it
/// again: memoising `label` added the cache and the two types it keys on above
/// it, and 1273 is where that left the function. The number is rewritten here
/// because the prose beside it changed anyway — see the note below about what
/// a number that no longer lands is worth.
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
    meta::subtitle(
        game.display_category(),
        &meta::runner_label(game.is_linux(), label),
    )
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
///
/// # The pixels now come from the cache, and that is PERF-02's fix
///
/// This used to build `image::Handle::from_path(&game.cover_path)`, which hands
/// the renderer a *path*: the renderer then decodes that file at the source
/// image's own dimensions and keeps the result until a frame does not draw it
/// (`iced/tiny_skia/src/raster.rs:151-171`, `:232-237`). With PERF-03's
/// per-frame draw of every game, that is a 600×900 pixmap resident per cover —
/// the audit measured **771,656 kB** of RSS for 333 covers at that size, against
/// the 719 MB of pixels the arithmetic predicts (`docs/audit/PERFORMANCE.md`,
/// PERF-02), i.e. essentially all of them resident.
///
/// So the decode moves here: [`CoverCache::image`] decodes the file once, to at
/// most [`super::cover_cache::DECODE_MAX`] in either axis, keeps the smallest amount
/// of decoded data the UI can draw from, and holds it under a byte budget. The
/// renderer is handed `Handle::Rgba` — pixels, not a path — so it never sees
/// the source size at all. Its own copy of what we hand it is bounded by what
/// is *drawn*, which PERF-03's window bounds in turn.
///
/// # Nothing about the drawing changes
///
/// The fit, the radius, the box and the inset are the same values as before,
/// and the case where the pixels are unavailable draws what the old code drew
/// in that case: an empty box of the same size, because that is what the
/// renderer does with a handle it cannot load (`raster.rs:152-160` inserts a
/// `None` entry and `draw` returns early, `:196`). A cover file that was
/// deleted between the classify and the decode therefore renders exactly as it
/// did, rather than becoming a plate or a panic — this is external state and
/// neither of those is an acceptable answer to it.
fn framed<'a, M: Clone + 'static>(
    cache: &CoverCache,
    cover_path: &str,
    spec: CoverSpec,
    inset: f32,
) -> Element<'a, M> {
    let source = cache.kind(cover_path);
    let picture: Element<'a, M> = match cache.image(cover_path) {
        Some(handle) => image(handle)
            .content_fit(source.content_fit())
            .width(Length::Fill)
            .height(Length::Fill)
            .border_radius(spec.radius)
            .into(),
        // Fills the box without drawing anything, which is the renderer's own
        // behaviour for an image it cannot load. `Space` rather than an empty
        // `container` so that the layout contribution is explicit and the
        // `PICTURE_ID` container below still holds exactly one child.
        None => Space::new().width(Length::Fill).height(Length::Fill).into(),
    };

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

/// The factor iced's text layout gives a single line's box over its font size.
///
/// `LineHeight::default()` is `Relative(1.4)` (`iced/core/src/text.rs:244-248`),
/// so a one-line `text` widget's box is 1.4 x its size whatever the font.
/// Measured, twice, on the two plates the app draws: the initials on a 36 x 50.4
/// row plate are 21 px and their box is 29.4 = 1.4 x 21 tall, and on the 188 x
/// 207 card plate they are 64 px and 89.6 = 1.4 x 64 — the second number is the
/// one this file already records in
/// `an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset`.
///
/// It is named because [`plate_text_band`] turns it into the band of the plate
/// the glyphs cover, and a wrong factor there picks the wrong ink.
const LINE_BOX_FACTOR: f32 = 1.4;

/// WCAG 2.x relative luminance of an opaque colour.
///
/// Linearise each channel with the sRGB transfer function, then weight them
/// 0.2126 / 0.7152 / 0.0722 (`https://www.w3.org/TR/WCAG22/#dfn-relative-luminance`).
///
/// It is here, rather than only in the test, because [`plate_text_color`]
/// *chooses* with it. The test below computes the ratio again from its own copy
/// of this arithmetic and evaluates the colour this function returned, so a
/// mistake here shows up as a failing ratio instead of being mirrored on both
/// sides — which is what would happen if the two shared one implementation.
fn relative_luminance(colour: Color) -> f32 {
    fn channel(value: f32) -> f32 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * channel(colour.r) + 0.7152 * channel(colour.g) + 0.0722 * channel(colour.b)
}

/// WCAG 2.x contrast ratio between two opaque colours, from 1.0 to 21.0.
fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (first, second) = (relative_luminance(a), relative_luminance(b));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// The gradient offsets a plate's initials line box spans on a `spec` box.
///
/// The initials are centred on the plate (`initials_only` aligns both axes), the
/// plate's gradient runs from offset 0.0 at the top edge to 1.0 at the bottom
/// ([`PLATE_GRADIENT_ANGLE`]), and a line box is [`LINE_BOX_FACTOR`] of the font
/// size — so these two numbers are the top and bottom edges of the text as
/// offsets *into the gradient*.
///
/// A pure function of the spec, and public, because it is the thing a test can
/// check against a real layout: [`initials_only`] has no id to read a box off
/// and the traversal's text bounds are the only measurement of it there is.
pub fn plate_text_band(spec: CoverSpec) -> (f32, f32) {
    let size = metrics::initials_size(spec.width, spec.height, spec.compact);
    let half = LINE_BOX_FACTOR * size / 2.0;
    (
        ((spec.height / 2.0 - half) / spec.height).clamp(0.0, 1.0),
        ((spec.height / 2.0 + half) / spec.height).clamp(0.0, 1.0),
    )
}

/// The colour a plate's gradient shows at `offset` into it.
///
/// Channel by channel between [`plate_stops`]' two colours, which is what the
/// application's renderer does with them. `wgpu` is deliberately absent from the
/// feature list (DECISIONS D-11), so the drawing backend is `iced_tiny_skia`,
/// which hands the stops straight to a `tiny_skia::LinearGradient` and samples
/// it (`iced/tiny_skia/src/engine.rs:164-193`). The other backend would not
/// agree: its shader smooth-steps the mix factor
/// (`iced/wgpu/src/shader/quad/gradient.wgsl`, `smoothstep(curr, next, coord)`),
/// which leaves a *lighter* colour behind the same glyphs, so the ink derived
/// from this function is the conservative one only for the backend in use.
fn plate_colour_at(accent: usize, offset: f32) -> Color {
    let [(_, top), (_, bottom)] = plate_stops(accent);

    Color::from_rgb(
        top.r + (bottom.r - top.r) * offset,
        top.g + (bottom.g - top.g) * offset,
        top.b + (bottom.b - top.b) * offset,
    )
}

/// The colour a plate's initials are drawn in, for `accent`'s gradient.
///
/// # Why this is derived and not a constant (UX-17)
///
/// It was `Color::from_rgba(1.0, 1.0, 1.0, 0.92)` — `CoverArt.qml`'s
/// `color: "#ffffff"; opacity: 0.92`, one pair for all eight shades and every
/// size. Measured against the region the glyphs actually cover — the band
/// [`plate_text_band`] returns, not the plate's top edge — that pair was:
///
/// | shade | row plate (21 px) | card plate (64 px) |
/// |---|---|---|
/// | 0, 1, 3, 5, 6, 7 | 4.655 - 6.092 | 4.905 - 6.416 |
/// | 2 | **4.246** | 4.501 |
/// | 4 | **3.516** | 3.737 |
///
/// The audit's number for the worst of these was 2.98, measured against the
/// shade's *lightest stop* — the colour at the plate's top edge, 10.5 px above
/// the text box on a row plate, and never behind a glyph. The real worst pair is
/// 3.516, at shade 4 on a row; the arithmetic is in the test below, which
/// recomputes it from a laid-out plate. So the row's figure is wrong and its
/// conclusion is right: **two** of the eight shades, not one, were under 4.5:1.
///
/// The fix is to stop fixing the pair. The ink is now the better of opaque white
/// and opaque black *for the plate it is about to be drawn on*: the shade
/// because the gradient is the shade's, and the spec because the band is the
/// spec's. The 0.92 alpha could not stay either — compositing at 0.92 always
/// lowers the ink's luminance, and it is what put shade 2 under (4.246 against
/// the same ink opaque). For the twenty-four (shade, spec) pairs this app draws,
/// the derivation resolves to opaque white every time, so what the change buys
/// for today's palette is that alpha; it is written as a choice so that a
/// re-picked shade is re-decided and re-asserted rather than re-copied.
///
/// # What it does not close
///
/// No ink can bring shade 4 to 4.5:1 at these sizes. Over the row plate's band
/// the two candidates are 3.845 (white) and 3.226 (black) and the plate is
/// nowhere lighter than either — the gradient's own luminance range across the
/// text is the binding constraint, not the choice of ink. Shade 4 therefore
/// clears the 3:1 that applies to it as large text and not the 4.5:1 that would
/// apply if it were small; see [`initials_only`] for why it is large.
pub fn plate_text_color(accent: usize, spec: CoverSpec) -> Color {
    let (top, bottom) = plate_text_band(spec);
    let behind = [
        plate_colour_at(accent, top),
        plate_colour_at(accent, bottom),
    ];

    let worst = |ink: Color| {
        behind
            .iter()
            .map(|colour| contrast_ratio(ink, *colour))
            .fold(f32::INFINITY, f32::min)
    };

    // A tie goes to white, which is what the reference draws these in.
    if worst(Color::BLACK) > worst(Color::WHITE) {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// The initials, centred — what a library tile shows for a game with no cover.
///
/// No words: `CoverArt.qml`'s text is `art.initials` alone. The "No cover yet"
/// string belongs to [`cover_preview`] and arrives through [`preview_label`],
/// and a test below pins the difference. The string is a parameter rather than
/// a call to [`plate_text`] here so that this function has no string of its
/// own to get wrong: it draws what [`cover_plan`] put in the plan.
///
/// # The weight
///
/// **Bold**, which is `CoverArt.qml:45`'s `font.bold: true` and was missing
/// here: this drew the default regular weight, and the port had no note saying
/// it meant to. It is not decoration. The row plate's initials are 21 px
/// (`initials_size(36, 50.4, true)`), which is 15.75 pt — under the 24 px at
/// which regular text counts as large, so as regular text they would owe 4.5:1,
/// and shade 4 cannot reach it from any ink. `font.bold` makes it 15.75 pt
/// *bold*, over the 14 pt bold the same clause names, so the bar is 3:1 and
/// every shade clears it. The card plate's 64 px was never in question.
///
/// The `style: Text.Raised` / `styleColor: "#66000000"` shadow beside it is
/// still absent: it is not reachable from iced's `text`, and a drop shadow is
/// not contrast in any case.
fn initials_only<'a, M: Clone + 'static>(
    text_of_game: String,
    spec: CoverSpec,
    accent: usize,
) -> Element<'a, M> {
    container(
        text(text_of_game)
            .size(metrics::initials_size(
                spec.width,
                spec.height,
                spec.compact,
            ))
            .font(cosmic::font::bold())
            .class(plate_text_color(accent, spec)),
    )
    .id(INITIALS_ID)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .into()
}

/// A gradient plate of exactly `spec`'s size with `content` centred on it.
///
/// The `accent` is a parameter rather than looked up from the game, because its
/// other reader is the ink [`initials_only`] draws the words in: one number, two
/// consumers, and no way for them to disagree. [`cover_box`] is where it is
/// resolved.
fn plate<'a, M: Clone + 'static>(
    spec: CoverSpec,
    accent: usize,
    content: Element<'a, M>,
) -> Element<'a, M> {
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
///
/// **This is the single home for the card surface** (ARCH-17, UX-19). It was
/// three copies — here, `view::installers` and `view::runners` — and the two
/// copies' own comments named three as the threshold for moving to one place,
/// by which point the threshold had already been reached. Both of those copies
/// hardcoded `14.0` where this one read [`metrics::CARD_RADIUS`], so changing
/// the constant silently left two pages at the old radius. `pub(super)` rather
/// than `pub`: the surface is a `view`-internal idiom, and a page outside this
/// module taking it from here is the thing the copies were each warned against.
pub(super) fn card_style(theme: &cosmic::Theme) -> container::Style {
    let cosmic = theme.cosmic();
    container::Style {
        background: Some(Background::Color(cosmic.background(false).base.into())),
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
///
/// **Module scope and `pub(crate)` since the cover decode moved**, which is why
/// this is no longer declared inside `mod tests`: `super::cover_cache`'s tests
/// decode the same bytes through the decoder the app now uses, and the point of
/// D-29 is that the *application's* path reads a user's `.webp` — a fixture only
/// this module's tests can reach would leave that path untested the moment the
/// decode stopped going through `image::Handle::from_path`. One definition
/// rather than two copies of 118 bytes that could drift.
#[cfg(test)]
pub(crate) const WEBP_COVER: [u8; 118] = [
    0x52, 0x49, 0x46, 0x46, 0x6e, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x4c,
    0x62, 0x00, 0x00, 0x00, 0x2f, 0x17, 0xc0, 0x03, 0x00, 0xcd, 0x95, 0x21, 0xa2, 0xff, 0xb1, 0x2b,
    0x78, 0x14, 0xbc, 0xff, 0x01, 0x26, 0x91, 0x24, 0x49, 0x4a, 0xbc, 0x4a, 0x56, 0xd3, 0xfa, 0x77,
    0xf2, 0x5d, 0xa1, 0xc3, 0x8a, 0x1a, 0x49, 0x8a, 0x6a, 0x2f, 0xc0, 0xff, 0x0b, 0x11, 0x48, 0x42,
    0x8c, 0xa2, 0xb6, 0x91, 0x1c, 0xbf, 0x76, 0xaf, 0xf1, 0x87, 0x70, 0x20, 0x7b, 0x4c, 0xc0, 0xfc,
    0xe8, 0xaf, 0xee, 0x70, 0xfa, 0xa3, 0x33, 0x06, 0x00, 0x58, 0x04, 0xc2, 0x0d, 0x6c, 0xb2, 0x02,
    0x87, 0x52, 0x81, 0x4b, 0xad, 0xc0, 0xa3, 0x55, 0xe0, 0xd3, 0xab, 0xcf, 0x8b, 0x07, 0x26, 0x3c,
    0x27, 0x3a, 0x00, 0x03, 0xbf, 0x19,
];

/// The size [`WEBP_COVER`] declares: 24x16, as encoded.
#[cfg(test)]
pub(crate) const WEBP_COVER_SIZE: (u32, u32) = (24, 16);

/// A real cover file on disk, for the tests in this module and the game form's.
///
/// A path under the system temporary directory holding `bytes`, named `stem-N`
/// with a counter so no two calls can read a half-written file left by another.
/// Bytes rather than an extension because a cover is
/// [`CoverSource::Plate`] whatever it is called — so a test that wants a
/// photograph or an icon has to put bytes on disk. The name carries no
/// extension on purpose: which composition a cover gets is decided from the
/// content and never from the suffix (see [`super::cover`]'s module docs), and a
/// fixture called `.png` would leave that untested either way.
///
/// It is `pub(crate)` and at module scope rather than inside `tests` because the
/// game form's suite draws the same kind of cover through
/// [`cover_preview`](crate::view::widgets::cover_preview), and a second copy of
/// this would be a second answer to "what is a cover file".
#[cfg(test)]
pub(crate) fn cover_fixture(stem: &str, bytes: &[u8]) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("gamehandler-widgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a writable temporary directory");
    let path = dir.join(format!("{stem}-{n}"));
    std::fs::write(&path, bytes).expect("a writable fixture");
    path.to_string_lossy().into_owned()
}

/// The traversal the widget tests read a built element with, shared with the
/// game form's suite.
///
/// `Widget` has no `as_any` and iced has no downcast, so "which nodes did this
/// builder compose, and what strings did it hand them" is invisible from outside
/// a builder — the ids, the boxes and the text a real `Widget::operate` reports
/// are the only observation there is. Two independent traversals of the same
/// widgets would be two answers to the same question, which is why the game
/// form's `UX-11` tests call these rather than growing a collector of their own
/// (see [`crate::view::form`]).
///
/// What is left here is a re-export: the walk itself is
/// [`crate::view::testkit`]'s since `ARCH-14`, where six copies of it lived
/// before. This module kept its own `Seen`, `Collect`, `renderer` and
/// `traversal` until then.
#[cfg(test)]
pub(crate) mod harness {
    pub(crate) use crate::view::testkit::{Seen, ids, renderer, texts};

    /// Lay an element out as one line and report what the framework knows.
    pub(crate) use crate::view::testkit::traversal;

    /// The same, at a real width — see
    /// [`crate::view::testkit::traversal_at_width`].
    pub(crate) fn traversal_at_width<M: Clone + 'static>(
        el: &mut cosmic::Element<'_, M>,
        width: f32,
    ) -> Vec<Seen> {
        crate::view::testkit::traversal_at_width(el, width)
    }
}

#[cfg(test)]
mod tests {
    use super::harness::traversal_at_width;
    use super::harness::{Seen, ids, renderer, texts, traversal};
    use super::*;
    use cosmic::iced::advanced::Layout;
    use cosmic::iced::advanced::layout::{Limits, Node};
    use cosmic::iced::advanced::widget::Tree;
    // The trait, not a value: `measure_image` is `Renderer`'s method and is not
    // in scope without it.
    use cosmic::iced::advanced::Shell;
    use cosmic::iced::advanced::image::Renderer as _;
    use cosmic::iced::{Event, Point, mouse};
    use cosmic::iced::{Font, Radius, Rectangle, Size};
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

    /// A fresh [`CoverCache`] for a builder call in a test.
    ///
    /// A function rather than a shared `static`, because the cache is
    /// `RefCell`-based and not `Sync`; and an expression rather than a `let` in
    /// every test, because the builders take the cache by shared reference and
    /// hand back an `Element` whose lifetime is tied to the *game*, not to the
    /// cache (`widgets::card<'a, M>(cache: &CoverCache, game: &'a Game, …) ->
    /// Element<'a, M>`), so the temporary lives exactly as long as the call
    /// needs it.
    ///
    /// These tests are about what a tile *draws*, and every game in them has a
    /// `cover_path` pointing at a file that does not exist, so the cache takes
    /// the no-cover branch and the assertions are unchanged from before it
    /// existed. That is deliberate: a test that started passing differently
    /// because a cache appeared beside it would be pinning the cache, not the
    /// widget.
    fn cache() -> CoverCache {
        CoverCache::new()
    }

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
        assert_eq!(
            subtitle_of(&game, "System Wine"),
            "Shooter \u{b7} System Wine"
        );
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
        assert_eq!(
            subtitle_of(&game, "System Wine"),
            "Platformer \u{b7} Linux native"
        );
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
    /// returning `"System Wine"` (`runners.py:759-761`, `runners/mod.rs:1273`)
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

    /// A message type of this module's own, for the two tests that press a
    /// control. Same reason [`crate::view::a11y`]'s test module has one: the
    /// app's `Message` has no `PartialEq` (it carries a `ToastId`, a `Task` and
    /// a `GameForm`) so a comparison against it would have to be destructured,
    /// and what is under test here is which message reaches the caller — not
    /// which message the app happens to define.
    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Play,
        More,
    }

    /// **UX-16: the "More actions" control is the card's and the row's second
    /// Tab stop, and its press is the caller's message** — proved by driving the
    /// key through a real widget tree rather than by reading the builder.
    ///
    /// The order is asserted, not just the presence: the reference draws Play
    /// first and "More actions" beside it (`LibraryPage.qml:203-213`,
    /// `:275-284`), and a Tab ring that reached the actions before the game's
    /// own Play button would be a different page from the one the QML lays out.
    /// The two keys are driven in sequence against **one** element and **one**
    /// tree, because `tab_to` carries the framework's focus forward inside that
    /// tree — a fresh tree per key would focus the first stop twice and the test
    /// would pass against a widget that ignored the second key.
    #[test]
    fn the_more_actions_control_is_the_second_tab_stop_and_presses_the_callers_message() {
        use crate::view::a11y::harness;
        use cosmic::iced::keyboard::{Key, key::Named};

        let game = Game::new_named("Half-Life 2");
        let mut el: Element<'_, Msg> =
            card(&cache(), &game, "System Wine", Msg::Play, Some(Msg::More));

        let stops = harness::focusables(&mut el);
        assert_eq!(
            stops.len(),
            2,
            "the tile is Play and More actions — the reference's pair. Reported: \
             {stops:?}"
        );
        assert!(
            stops.iter().all(Option::is_some),
            "a control in the ring that reports no id cannot be addressed, which \
             is what `Shell::update`'s focus request does with the layer's first \
             control: {stops:?}"
        );

        let (mut tree, node) = harness::built(&mut el);
        let press_enter = |el: &mut Element<'_, Msg>, tree: &mut Tree, node: &Node| {
            let mut messages = Vec::new();
            let _ = harness::dispatch(
                el,
                tree,
                node,
                &harness::pressed(Key::Named(Named::Enter)),
                &mut messages,
            );
            messages
        };

        harness::tab_to(&mut el, &mut tree, &node);
        assert_eq!(
            press_enter(&mut el, &mut tree, &node),
            vec![Msg::Play],
            "the first Tab stop is Play"
        );
        harness::tab_to(&mut el, &mut tree, &node);
        assert_eq!(
            press_enter(&mut el, &mut tree, &node),
            vec![Msg::More],
            "the second Tab stop is More actions, and Enter on it publishes \
             exactly the message the caller handed the builder — one message, \
             not one from the wrapper and one from the button under it"
        );
    }

    /// **The row draws the same pair as the card**, which is the reason the two
    /// are one function rather than two builders that agree.
    ///
    /// `LibraryPage.qml:275-284` draws the row's `ToolButton` beside its Play
    /// button exactly as `:203-213` draws the card's; a port that wired the card
    /// and forgot the row would still render, still focus, and leave the list
    /// view with no keyboard route to a game's actions.
    #[test]
    fn the_row_draws_the_more_actions_control_too() {
        use crate::view::a11y::harness;

        let game = Game::new_named("Half-Life 2");
        let labels = RowLabels {
            runner: "System Wine",
            last_played: "Never played",
        };
        let mut el: Element<'_, Msg> = row(&cache(), &game, &labels, Msg::Play, Some(Msg::More));
        assert_eq!(
            harness::focusables(&mut el).len(),
            2,
            "the row must offer Play and More actions, as the card does"
        );
    }

    /// **With no actions message the tile draws Play alone** — the tile a caller
    /// that has nowhere to open the actions gets.
    ///
    /// This is the `Option`'s whole purpose, and it is asserted because the
    /// alternative is worse than it looks: a control whose press publishes a
    /// message nothing answers is a button that does nothing, and it would still
    /// be a Tab stop with an accessible name promising actions that never open.
    #[test]
    fn a_tile_with_no_actions_message_draws_play_alone() {
        use crate::view::a11y::harness;

        let game = Game::new_named("Half-Life 2");
        let mut el: Element<'_, Msg> = card(&cache(), &game, "System Wine", Msg::Play, None);
        assert_eq!(
            harness::focusables(&mut el).len(),
            1,
            "the tile without an actions message must be Play and nothing else"
        );
        assert_eq!(
            texts(&traversal(&mut el)),
            ["HL", "Half-Life 2", "System Wine", "Play"],
            "and it draws no label the actions control would have added"
        );
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
        let cache = CoverCache::new();

        let mut tile: Element<'_, ()> = cover_box(&cache, &game, row_cover_spec());
        assert_eq!(
            texts(&traversal(&mut tile)),
            ["HL"],
            "a library tile draws the game's initials and nothing else"
        );

        let plan = cover_plan(&cache, &game);
        let mut picker: Element<'_, ()> = cover_preview(&cache, &game, &plan);
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

        let mut card: Element<'_, ()> = card(&cache(), &game, &label, (), None);
        assert_eq!(
            texts(&traversal(&mut card)),
            ["HL", "Half-Life 2", "System Wine", "Play"]
        );

        let played = format_last_played(0.0, FROZEN_NOW);
        let labels = RowLabels {
            runner: &label,
            last_played: &played,
        };
        let mut row: Element<'_, ()> = row(&cache(), &game, &labels, (), None);
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
        let mut row: Element<'_, ()> = row(&cache(), &game, &labels, (), None);
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

        let mut card: Element<'_, ()> = card(&cache(), &game, &label, (), None);
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
        let mut row: Element<'_, ()> = row(&cache(), &game, &labels, (), None);

        assert_eq!(
            texts(&traversal(&mut row)),
            [
                "MY",
                "Mystery",
                "Shooter · System Wine · Never played",
                "Play"
            ]
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

        let mut card: Element<'_, ()> = card(&cache(), &game, &label, (), None);
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

        let mut row: Element<'_, ()> = row(&cache(), &game, &labels, (), None);
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

        let mut card: Element<'_, &str> = card(&cache(), &game, &label, "launch", None);
        let (cell_w, cell_h) = metrics::GRID_CELL;
        let card_hit = Point::new(cell_w / 2.0, cell_h / 2.0);
        assert_eq!(
            published_by_double_click(&mut card, card_hit),
            ["launch"],
            "a double click on the card must publish the caller's message \
             exactly once"
        );

        let mut row: Element<'_, &str> = row(&cache(), &game, &labels, "launch", None);
        let row_hit = Point::new(20.0, metrics::LIST_ROW_HEIGHT / 2.0);
        assert_eq!(
            published_by_double_click(&mut row, row_hit),
            ["launch"],
            "a double click on the row must publish the caller's message \
             exactly once"
        );
    }

    /// The boxes a card's column holds, in order: the cover, the name and
    /// subtitle, and the Play control.
    ///
    /// `mouse_area` delegates its layout to its content and adds no node of its
    /// own (`iced/widget/src/mouse_area.rs:234-238`), so the root here is the
    /// card's container and its one child is the column.
    ///
    /// Takes the element rather than building one, so a caller can read the
    /// column's boxes and the traversal's reported bounds **from the same
    /// element**. That is what makes the control's box and the label drawn inside
    /// it two readings of one layout rather than two cards that happen to agree.
    fn column_children_of<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<Size> {
        let renderer = renderer();
        let mut tree = Tree::new(el.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let column = node
            .children()
            .first()
            .expect("the card's container holds its column");
        column.children().iter().map(|child| child.size()).collect()
    }

    /// **A name too long for the card must not squeeze the Play control — #96.**
    ///
    /// The card's cell is exactly filled: cover 207 + gap 6 + text block 37 +
    /// gap 6 + control 32 = 288, the whole interior. iced's `Column` gives a
    /// deficit to its last child, so a text block that grows by one line comes
    /// out of the control. Before `name_and_subtitle` clamped its two lines,
    /// this name laid the Play control out at **1.4000015 px** — with a 1.4-px
    /// icon and a 1.4-px label inside it — and every test in this file passed.
    /// (That number was measured when the chrome was `70.0` and the cover 218;
    /// with the chrome corrected to 81.0 the same wrap takes the control to
    /// **12.4 px** instead. Both are squeezes, and both fail the assertions
    /// below — see [`metrics::PLAY_BUTTON_HEIGHT`] for why the chrome moved.)
    ///
    /// # Why this is a layout test and not an arithmetic one
    ///
    /// Because the defect is a relationship between a drawn box and the space it
    /// was given. `metrics`' const checks bound the reserve; only laying the
    /// real element out says whether the drawing fits inside it. Note also that
    /// this had to be [`traversal_at_width`] and not [`traversal`]: at infinite
    /// width nothing wraps, so the original assertions would have passed against
    /// the broken card.
    ///
    /// # The fixture is proven, not assumed
    ///
    /// A short name would make all of this vacuous, so the last block measures
    /// the same name with iced's *default* wrapping, at the same width, and
    /// asserts it takes more than one line. That is what says the name is a real
    /// stress case — and it is the mutation in miniature: turn the clamp off and
    /// this is the layout the card gets.
    #[test]
    fn a_long_name_does_not_squeeze_the_play_control() {
        let short = Game::new_named("Celeste");
        let long = Game::new_named("The Elder Scrolls V Skyrim Special Edition");
        let available = metrics::GRID_CELL.0 - 2.0 * metrics::CARD_MARGIN;
        let one_line = metrics::CARD_NAME_SIZE * metrics::LINE_HEIGHT_RATIO;

        let mut short_card: Element<'_, ()> = card(&cache(), &short, "", (), None);
        let short_parts = column_children_of(&mut short_card);
        let mut long_card: Element<'_, ()> = card(&cache(), &long, "", (), None);
        let long_parts = column_children_of(&mut long_card);
        assert_eq!(
            long_parts.len(),
            3,
            "the card's column is the cover, the name-and-subtitle block and the Play control"
        );
        // The cover is the part that absorbs, so it is the part that would hide a
        // mistake: the column's own `spacing` is a second literal, and raising it
        // shrinks the cover without moving the chrome the const checks bound. This
        // is what ties the drawn cover to the box `metrics` derives for it.
        assert_eq!(
            long_parts[0],
            Size::new(card_cover_spec().width, card_cover_spec().height),
            "the cover must be drawn in the box the metrics module derives from the cell, \
             the chrome and the margin — not in whatever the column has left. The card's \
             parts were {long_parts:?}"
        );

        assert!(
            long_parts[2].height >= metrics::PLAY_BUTTON_HEIGHT,
            "the Play control on a card with a long name laid out {:.4} px high; it must be \
             at least metrics::PLAY_BUTTON_HEIGHT ({}). A control squeezed below that is \
             #96, and the name is what took the space. The card's parts were {:?}",
            long_parts[2].height,
            metrics::PLAY_BUTTON_HEIGHT,
            long_parts
        );
        // And the other end of the same squeeze: the text block must take exactly
        // the room the chrome reserves for it, so the deficit cannot be hidden by
        // the block genuinely needing a second line. This is the assertion that
        // covers the *subtitle* as well as the name — both are clamped, and only
        // this one measures the block as a whole.
        assert_eq!(
            long_parts[1].height,
            metrics::CARD_TEXT_BLOCK_HEIGHT,
            "a card's name-and-subtitle block must be exactly the height the chrome \
             reserves, however long the name is. A taller block is a wrapped line, and \
             the cell has no slack to give it — it comes out of the Play control. The \
             card's parts were {long_parts:?}"
        );
        assert_eq!(
            long_parts[2].height, short_parts[2].height,
            "the Play control must be the same height whatever the game is called — a name \
             must not come out of the control. Short: {short_parts:?}, long: {long_parts:?}"
        );

        let content: f32 =
            long_parts.iter().map(|size| size.height).sum::<f32>() + 2.0 * metrics::CARD_MARGIN;
        let interior = metrics::GRID_CELL.1 - 2.0 * metrics::CARD_MARGIN;
        assert!(
            content <= interior,
            "a card's content must fit its cell's interior: {content} > {interior}, so \
             something is being clipped rather than drawn"
        );

        // The subtitle is the block's *other* line and needs its own fixture: a
        // real game's subtitle is short enough to fit, so the name alone cannot
        // test it. This is what the card's `label` parameter is for — a long
        // runner label makes a long subtitle without inventing a game whose
        // metadata is stranger than any real one's. Without this block, removing
        // the clamp from the subtitle alone survives the whole suite.
        let long_label = "Proton-GE-Proton9-20-x86_64 ".repeat(8);
        let subtitle = subtitle_of(&long, &long_label);
        let subtitle_line = metrics::CARD_SUBTITLE_SIZE * metrics::LINE_HEIGHT_RATIO;
        let mut labelled_card: Element<'_, ()> = card(&cache(), &long, &long_label, (), None);
        let labelled_parts = column_children_of(&mut labelled_card);
        assert_eq!(
            labelled_parts[1].height,
            metrics::CARD_TEXT_BLOCK_HEIGHT,
            "the same block, with a subtitle long enough to wrap. Its parts were \
             {labelled_parts:?}"
        );
        let labelled_seen = traversal_at_width(&mut labelled_card, metrics::GRID_CELL.0);
        assert_eq!(
            drawn(&labelled_seen, &subtitle).height,
            subtitle_line,
            "the card's subtitle must be a single line whatever its length"
        );
        // The same anti-vacuity as the name's, for the same reason.
        let mut unclamped_subtitle: Element<'_, ()> =
            text(&subtitle).size(metrics::CARD_SUBTITLE_SIZE).into();
        let wrapped_subtitle = traversal_at_width(&mut unclamped_subtitle, available);
        assert!(
            drawn(&wrapped_subtitle, &subtitle).height > subtitle_line,
            "the fixture subtitle must be long enough to wrap at {available} px, or the \
             assertion above cannot fail on it"
        );

        // The fix itself, stated as what it is: one line, in both delegates.
        let mut card_el: Element<'_, ()> = card(&cache(), &long, "", (), None);
        let card_seen = traversal_at_width(&mut card_el, metrics::GRID_CELL.0);
        assert_eq!(
            drawn(&card_seen, &title_of(&long)).height,
            one_line,
            "the card's name must be a single line whatever its length"
        );

        let played = format_last_played(0.0, FROZEN_NOW);
        let labels = RowLabels {
            runner: "",
            last_played: &played,
        };
        // Narrow on purpose: the row is a `Length::Fill` line inside the page, so
        // a wide window would never wrap its text and the row's half of this
        // would be untested.
        let mut row_el: Element<'_, ()> = row(&cache(), &long, &labels, (), None);
        let row_seen = traversal_at_width(&mut row_el, 400.0);
        assert_eq!(
            drawn(&row_seen, &title_of(&long)).height,
            one_line,
            "the row's name must be a single line too — it is the same builder"
        );

        // Anti-vacuity, and the sharpest form of it available: with iced's own
        // default wrapping this name *does* take more than one line at this
        // width, so the clamp is what the assertions above are measuring.
        let mut unclamped: Element<'_, ()> =
            text(title_of(&long)).size(metrics::CARD_NAME_SIZE).into();
        let wrapped = traversal_at_width(&mut unclamped, available);
        assert!(
            drawn(&wrapped, &title_of(&long)).height > one_line,
            "the fixture name {:?} must be long enough to wrap at {available} px, or this \
             test cannot fail on a wrapping name and should be given a longer one",
            title_of(&long)
        );

        // The width half, which is what the recorded "long titles overflow into
        // the neighbour tile" finding was about (`../audit/BASELINE.md` open item
        // 4, `../migration/REPORT.md` residual 4). Measured rather than argued,
        // because that finding and its retraction were both made from captures:
        // the name's *drawn* box must not exceed the width the cell gives it.
        let mut card_for_width: Element<'_, ()> = card(&cache(), &long, "", (), None);
        let wide = traversal_at_width(&mut card_for_width, available);
        let name_box = drawn(&wide, &title_of(&long));
        assert!(
            name_box.width <= available,
            "a {} character name drew {:.4} px wide in a {available} px cell — it spills into \
             the neighbour tile, which is the finding this assertion exists to keep dead. \
             `Wrapping::None` clamps the run to the limits it is given, so a name wider than \
             its box means the box was not what bounded it.",
            title_of(&long).chars().count(),
            name_box.width,
        );
        // Anti-vacuity for the assertion above: `available` must be a width this
        // name actually fills, or `<=` would hold for a box that measured zero.
        assert!(
            name_box.width > available * 0.5,
            "the name drew only {:.4} px of its {available} px cell, so the bound above is \
             not being tested by this fixture — it would pass on any short string",
            name_box.width
        );
    }

    /// **The names are ellipsized the way the reference ellipsizes them.**
    ///
    /// See [`name_ellipsize`] for why this asserts the strategy and not the
    /// rendered marker: `Text`'s `format` is private, iced has no downcast, and
    /// the traversal above reports a widget's *fragment* (the full string) rather
    /// than the shaped run, so **no test in this suite can observe a `…`**. A
    /// test that claimed to would be this project's dominant defect class. The
    /// marker is verified by eye against the running app; what is pinned here is
    /// the input that decides it, so a change to the strategy is a failing test
    /// rather than a silent return of the clipped-at-the-edge behaviour this
    /// replaced.
    ///
    /// The layout half — that an ellipsized name is still one line and still
    /// leaves the Play control its full height — is not asserted here because
    /// `a_long_name_does_not_squeeze_the_play_control` already drives the real
    /// card through a real layout, and that card now carries this strategy. The
    /// two together are the whole check: this one pins the value, that one
    /// proves the value does not break the cell.
    #[test]
    fn the_card_and_row_names_are_ellipsized_at_the_end() {
        use cosmic::iced::core::text::EllipsizeHeightLimit;
        use cosmic::iced::widget::text::Ellipsize;

        assert_eq!(
            name_ellipsize(),
            Ellipsize::End(EllipsizeHeightLimit::Lines(1)),
            "`Text.ElideRight` under `Text.NoWrap` (`LibraryPage.qml:186`, `:195`, `:265`, \
             `:272`) is one line cut at the end with the marker. `End` rather than `Start` or \
             `Middle` is the reference's choice, not a preference: a game's name is \
             identified by its first words."
        );
        // `Lines(1)` and not `Height(..)`: the limit is the reference's own unit.
        // `ElideRight` with no `wrapMode` is a *line* count, and a height limit
        // would silently stop eliding if the font size ever changed
        // (`metrics::CARD_NAME_SIZE` is a literal that is free to move).
        assert!(
            !matches!(name_ellipsize(), Ellipsize::None),
            "the divergence this replaced was `Ellipsize::None` in all but name — a name too \
             long for its tile was cut at the tile's edge with no marker. `None` here would \
             reinstate it without the comment that used to explain it."
        );
        // Deliberately *not* asserted here: that the two builders call this, and
        // that `Wrapping::None` accompanies it. The only ways to see either from a
        // test are to grep this file's source text or to assert a constant
        // against itself, and a source-text check standing in for a behaviour
        // check is `BUGS.md` BUG-12 — the project already has one of those and
        // does not need a second. The wiring is two calls in one function, three
        // lines above this module's own card builder, and the layout test named
        // above fails loudly if either is dropped.
    }

    /// **The Play control is exactly the height the card reserves for it.**
    ///
    /// [`metrics::PLAY_BUTTON_HEIGHT`] is the only number in the metrics module
    /// that is another crate's layout rather than arithmetic of ours, and this is
    /// what keeps it honest: libcosmic's button is its padding plus its label, so
    /// a libcosmic change that moves its height fails here, naming the constant —
    /// instead of silently changing how much of the cell is left for the cover,
    /// which is the direction that leads back to #96.
    ///
    /// # The second half is the one #96 needed
    ///
    /// "The button lays out to 32" and "the card gives the button 32" are two
    /// different claims, and the card's was the false one before this: the
    /// constant said 21, the button's own layout said 32, and the card drew it at
    /// 21 — a Play label of 20 pixels inside a 21-pixel box, 0.5 pixels of
    /// padding where libcosmic's button asks for 5. Nothing failed. So the second
    /// assertion lays out a real card and reads the control's box out of the
    /// column, and **that** is the one the old code fails: in the pre-#96 chrome
    /// the leftover was 27 pixels, and with a wrapped name it was 1.4000015.
    ///
    /// # The third assertion, and what it is for
    ///
    /// The third reads the same element as the second and states the invariant
    /// `metrics::PLAY_BUTTON_HEIGHT` exists to hold: **the control's content fits
    /// in the box the card reserves.** It is not an independent witness for any
    /// mutation reachable from this file — the second assertion fires first on
    /// all of them, which is measured, not assumed. It is kept because it is the
    /// claim itself rather than its arithmetic, and because it is the only thing
    /// here that would notice a libcosmic whose button reported a box through
    /// `layout` that its own label overflowed: the second reads the layout node,
    /// the third reads what the text operation was handed for the same element.
    ///
    /// # The measurement this test cannot make
    ///
    /// The app's theme may set a different text size from `renderer`'s
    /// `Pixels(16.0)`, and then the control asks for a different height. Nothing
    /// here can see that — the renderer is the test's, not the app's — which is
    /// why the number is documented as measured at a written-down text size
    /// rather than as derived.
    #[test]
    fn the_play_control_is_the_height_the_card_reserves() {
        let game = Game::new_named("Celeste");

        let mut control: Element<'_, ()> = play_button(&game.id, ());
        let renderer = renderer();
        let mut tree = Tree::new(control.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = control
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        assert_eq!(
            node.size().height,
            metrics::PLAY_BUTTON_HEIGHT,
            "the Play control's own height, which metrics::PLAY_BUTTON_HEIGHT is the \
             card's reservation for"
        );

        // One element, read two ways: the layout node's sizes, and the bounds the
        // traversal reports for the strings inside it.
        let mut card_el: Element<'_, ()> = card(&cache(), &game, "Shooter", (), None);
        let parts = column_children_of(&mut card_el);
        assert_eq!(
            parts[2].height,
            metrics::PLAY_BUTTON_HEIGHT,
            "the Play control inside a card, which is the box the reservation is for. \
             The card's parts were {parts:?}"
        );

        // The box the card gives, against the content that has to go in it.
        let seen = traversal(&mut card_el);
        let label = drawn(&seen, PLAY_LABEL);
        assert!(
            label.height <= parts[2].height,
            "the Play label is {} px tall and the card gives its control {} px — the \
             control is being squeezed by whatever grew above it, which is #96. The \
             strings drawn were {:?}",
            label.height,
            parts[2].height,
            texts(&seen)
        );
        assert!(
            label.height > 0.5 * metrics::PLAY_BUTTON_HEIGHT,
            "the label's {} px must be a real label and not a collapsed one; the box is \
             {} px",
            label.height,
            parts[2].height
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

        let mut card: Element<'_, ()> = card(&cache(), &one, "", (), None);
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
        assert_eq!((w, h), (188.0, 207.0));
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
    /// points at the card's 188×207 cover box — and not from the compact rule's
    /// 87.
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
    ///
    /// Both numbers moved with the cover box when #96 shortened it from 218 to
    /// 207: the full-size rule takes `min(width, height)`, which is still the
    /// width, so 64 is unmoved; the compact rule takes the height, so 92 became
    /// 87. The assertion that reads the *card* is the quotient below, and it is
    /// the one that would have caught the widget changing rules.
    #[test]
    fn a_cards_initials_are_the_full_size_rule_and_not_the_compact_one() {
        assert_eq!(metrics::initials_size(188.0, 207.0, false), 64.0);
        assert_eq!(metrics::initials_size(188.0, 207.0, true), 87.0);

        // "Halo" so that the initials "HA" cannot be confused with the name.
        let game = Game::new_named("Halo");
        let mut card: Element<'_, ()> = card(&cache(), &game, "", (), None);
        let seen = traversal(&mut card);
        let ratio = drawn(&seen, "HA").height / drawn(&seen, "Halo").height;

        // `name_and_subtitle` sets the name at 14.0.
        let expected = 64.0 / 14.0;
        assert!(
            (ratio - expected).abs() < 0.05,
            "the initials/name height quotient should be 64/14 = {expected:.3} \
             for the full-size rule; got {ratio:.3}, and the compact rule would \
             give 87/14 = {:.3}",
            87.0 / 14.0
        );
    }

    /// **The row's initials are the compact ones**, which is how the row's
    /// drawing is told apart from the tile's.
    ///
    /// This is also what catches a `row` that stopped asking for
    /// [`row_cover_spec`] and took the tile's spec instead: the drawn height of
    /// a row's cover is clamped to the line, so the *box* difference (50.4
    /// against 54) never reaches the layout — but a tile spec carries
    /// `compact: false`, and the initials fall from 21 points to 12.
    #[test]
    fn a_rows_initials_are_the_compact_rule_and_not_the_tiles() {
        assert_eq!(metrics::initials_size(36.0, 50.4, true), 21.0);
        assert_eq!(
            metrics::initials_size(36.0, 54.0, false),
            12.0,
            "the tile's rule at the row's width, which is what a row drawn \
             through a tile spec would use"
        );

        let game = Game::new_named("Halo");
        // This test is about the text *sizes*, so the two labels are empty —
        // the row still draws its last-played line, but as an empty string it
        // contributes no glyphs to measure.
        let labels = RowLabels {
            runner: "",
            last_played: "",
        };
        let mut row: Element<'_, ()> = row(&cache(), &game, &labels, (), None);
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
    /// differ, and a refactor that gave `row` the tile's spec would draw
    /// 36×54 instead of 36×50.4 — close enough to look right in a screenshot.
    ///
    /// Pinned on the spec rather than on the drawn box because the row clamps
    /// its cover to the line height, so both boxes are drawn 36×43.2 and the
    /// *visible* difference is carried by the initials instead — see
    /// `a_rows_initials_are_the_compact_rule_and_not_the_tiles`, which calls
    /// the real `row`.
    ///
    /// The tile aspect itself is `metrics::PORTRAIT_RATIO`, now `#[cfg(test)]`
    /// (ARCH-25): the library grid draws a card, so `GRID_CELL.1` is the cell
    /// **height** rather than a number anything multiplies a width by. That is
    /// the opposite direction from what this test needs, and confusing the two
    /// is what the failure above exists to catch: the tile's spec is 200×300,
    /// three times the row's 50.4.
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

        let tile = card_cover_spec();
        assert!(
            tile.height > row.height * 4.0,
            "the card's cover should dwarf the row's strip, got {} against {}",
            tile.height,
            row.height
        );
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
        let mut photo: Element<'_, ()> = cover_box(&cache(), &with_photo_game, spec);
        assert_eq!(
            ids(&traversal(&mut photo)),
            [Id::from(PICTURE_ID)],
            "a photograph fills the box; there is no plate behind it"
        );

        let with_icon_game = with_icon("Half-Life 2");
        let mut icon: Element<'_, ()> = cover_box(&cache(), &with_icon_game, spec);
        assert_eq!(
            ids(&traversal(&mut icon)),
            [Id::from(PLATE_ID), Id::from(PICTURE_ID)],
            "an icon sits on the plate, the way the QML's `showPlate` says"
        );

        let bare = Game::new_named("Half-Life 2");
        let mut placeholder: Element<'_, ()> = cover_box(&cache(), &bare, spec);
        assert_eq!(
            ids(&traversal(&mut placeholder)),
            [Id::from(PLATE_ID), Id::from(INITIALS_ID)],
            "a game with no artwork is the plate and its initials"
        );
    }

    /// **PERF-01's measurement, at the level the audit measured it.**
    ///
    /// A real tile, built the way the page builds it, over many frames: the
    /// filesystem is read once per cover and the pixels are decoded once per
    /// cover — not once per tile per frame.
    ///
    /// # What the counter is a counter of
    ///
    /// [`CoverCache::classify_calls`] counts calls that reached
    /// `CoverSource::classify`, which is `Path::new(..).is_file()` (a `statx`)
    /// plus, when the file exists, a `File::open` and a four-byte read. It is
    /// therefore the number of cover-file syscalls this frame *issued*, and the
    /// audit's 6,624-syscall measurement is 200 of these for 100 frames of one
    /// tile — `cover_plan` and `framed` each classified, so **twice per tile per
    /// frame**. The pre-fix body of `cover_plan` is therefore
    /// `CoverSource::classify(&game.cover_path)`, and restoring it makes this
    /// test fail with `left: 0, right: 3` rather than with `left: 150`: the
    /// uncached call does not touch the counter at all. That direction is
    /// deliberate — a builder that bypassed the cache would otherwise pass a
    /// count-based assertion by not being counted.
    ///
    /// # The control, which is the half that makes it a measurement
    ///
    /// `decode_calls() == 3` and `image_calls() == 75` are asserted together, so
    /// "no filesystem work" cannot be reached by a builder that stopped asking
    /// for covers. And the *drawn* result is asserted too: each tile really is
    /// the picture and not the plate, which is what `PICTURE_ID` alone in the
    /// traversal means — see
    /// `a_photograph_is_drawn_alone_while_an_icon_and_a_placeholders_are_on_the_plate`.
    #[test]
    fn a_frame_of_tiles_classifies_each_cover_once() {
        let cache = CoverCache::new();
        let spec = card_cover_spec();
        // Three games with three *different* cover files, so three is the answer
        // only if the key really is the path.
        let games: Vec<Game> = (0..3)
            .map(|n| with_photo(&format!("Windowed {n}")))
            .collect();

        for _ in 0..25 {
            for game in &games {
                let mut tile: Element<'_, ()> = cover_box(&cache, game, spec);
                assert_eq!(
                    ids(&traversal(&mut tile)),
                    [Id::from(PICTURE_ID)],
                    "the tile draws its cover, so the cache was actually asked"
                );
            }
        }

        assert_eq!(
            cache.classify_calls(),
            3,
            "25 frames of 3 tiles = 75 tiles: one stat and one open per cover, \
             not per tile and not per frame"
        );
        assert_eq!(cache.decode_calls(), 3);
        assert_eq!(
            cache.image_calls(),
            75,
            "the cache was asked 75 times, so the count above is not low because \
             nothing asked"
        );
        assert_eq!(
            cache.resident_images(),
            3,
            "and all three are resident, so the 75 asks were hits"
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
    /// That a photograph's box is the full 188x207 and not the inset one. What
    /// it **cannot** show is that a picture is there at all: [`framed`] pins the
    /// container with `width(Fixed(..))`/`height(Fixed(..))`,
    /// `Limits::width(Length::Fixed(x))` sets `min == max == x`
    /// (`iced/core/src/layout/limits.rs:60-66`), and `Image::layout` resolves
    /// `intrinsic.min(max).max(min)` (`iced/widget/src/image.rs:257-268`) — so
    /// the leaf is 188x207 for *any* intrinsic, including the `Size::ZERO` a
    /// failed decode produces. Measured, all three in this binary:
    ///
    /// | cover | `measure_image` | `leaves` |
    /// |---|---|---|
    /// | `WEBP_COVER` | `Some(24x16)` | 188x207 |
    /// | twelve bytes of PNG header | `None` | 188x207 |
    /// | no file at all (`Prey`) | `None` | 78.528x89.6 |
    ///
    /// The plate row's *height* is the initials' line height and nothing to do
    /// with the cover box: `initials_only` sizes the text at
    /// `initials_size(188, h, false)` — 64, because the full-size rule takes
    /// `min(width, height)` and the width is the smaller — and iced's line
    /// height is 1.4 of it, so 64 × 1.4 = 89.6 for every name. Its width is the
    /// drawn initials and so moves with the name (`Celeste` 76.032, `Bare`
    /// 82.496, `""` 27.776 — all measured, all at this spec). It is named here
    /// because a bare 78.528 would be a number no reader could reproduce: it is
    /// what `Prey`'s two initials measure.
    ///
    /// **That height was 89.6 before #96 shortened the cover too, and this
    /// paragraph used to say it was `spec.height - 2 * 64.2`.** Those two
    /// expressions agree at 218 and nowhere else — 218 − 128.4 = 89.6, and so
    /// does 64 × 1.4 — so the wrong derivation read as correct until the box
    /// moved to 207 and the leaf stayed at 89.6. It is corrected here, and it is
    /// the reason the numbers in this table are re-measured rather than
    /// recomputed.
    ///
    /// The third row is why this test is not worthless — it does separate a
    /// plate from a photograph. The first two are why it needed
    /// [`a_webp_cover_is_decoded`] beside it: before that, the photograph here
    /// could not decode at all and this assertion still passed.
    #[test]
    fn an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset() {
        let spec = card_cover_spec();
        assert_eq!((spec.width, spec.height), (188.0, 207.0));
        assert_eq!(metrics::ICON_INSET, 18.0);

        let with_icon_game = with_icon("Half-Life 2");
        let mut icon: Element<'_, ()> = cover_box(&cache(), &with_icon_game, spec);
        assert_eq!(
            leaves(&mut icon),
            [Size::new(188.0 - 2.0 * 18.0, 207.0 - 2.0 * 18.0)],
            "the icon's box should be the plate's, less the inset on each side"
        );

        // A photograph is cropped to the box instead, so its inset is zero.
        let with_photo_game = with_photo("Half-Life 2");
        assert_decodes(&with_photo_game, "the photograph this branch draws");
        let mut photo: Element<'_, ()> = cover_box(&cache(), &with_photo_game, spec);
        assert_eq!(
            leaves(&mut photo),
            [Size::new(188.0, 207.0)],
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
    /// laid-out box is the pinned 188x207 for a decoded image and for an
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
            measured, None,
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

    /// No page builds the card surface itself; they all call the one above.
    ///
    /// The radius test above proves the *constant* reaches the renderer. It
    /// cannot notice a page that stops calling this function, and neither can
    /// any runtime assertion: a fourth `card_style` with `14.0` typed into it
    /// satisfies every behavioural test in this file on the day it is written
    /// and keeps satisfying them right up until someone moves the constant. That
    /// is not hypothetical — it is exactly what happened to the second and third
    /// copies (ARCH-17), and both carried a comment claiming they were in sync.
    ///
    /// So this reads the source. It is a text check, and it is honest about what
    /// that costs: it cannot tell a real copy from one written inside a comment
    /// or a string, and it would need widening if the surface ever gains a
    /// second legitimate definition. The alternative — the check that shipped —
    /// was no check at all, which is how the duplication survived two audits.
    ///
    /// Each page is named with its own path so a failure says which one left.
    #[test]
    fn only_one_page_defines_the_card_surface() {
        const PAGES: [(&str, &str); 3] = [
            ("widgets.rs", include_str!("widgets.rs")),
            ("installers.rs", include_str!("installers.rs")),
            ("runners.rs", include_str!("runners.rs")),
        ];

        // The definition text itself, not the name: `card_style` is also a
        // value at the call sites, and counting those would fail on the fix.
        const DEFINITION: &str = "fn card_style(theme: &cosmic::Theme) -> container::Style {";

        let mut pages_defining = PAGES
            .iter()
            .filter(|(_, source)| source.contains(DEFINITION))
            .map(|(name, _)| *name);

        assert_eq!(
            pages_defining.next(),
            Some("widgets.rs"),
            "the card surface's single definition should be in widgets.rs"
        );
        assert_eq!(
            pages_defining.collect::<Vec<_>>(),
            Vec::<&str>::new(),
            "a second page defines the card surface again — call \
             `super::widgets::card_style` instead; a copy hardcodes the radius \
             and goes stale the moment `metrics::CARD_RADIUS` moves (ARCH-17)"
        );

        // And the call sites that must exist, so deleting the surface outright
        // fails here rather than silently removing the background from a page.
        for (name, source) in PAGES {
            assert!(
                name == "widgets.rs" || source.contains("\nuse super::widgets::card_style;"),
                "{name} no longer imports the shared card surface"
            );
        }
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

    // ---- UX-17: the ink the initials are drawn in -------------------------
    //
    // The tests below compute the ratio themselves, from the shade constants,
    // rather than calling [`super::contrast_ratio`] back. That function is what
    // `plate_text_color` *chooses* with, so a test that asked it would agree
    // with itself — including if the gamma were wrong on both sides. Same for
    // the band: it is measured off a real laid-out plate, and the widget's own
    // [`plate_text_band`] is checked against the measurement rather than
    // trusted.

    /// WCAG 2.x relative luminance, written out here so the numbers below are
    /// the test's own.
    fn luminance([r, g, b]: [f32; 3]) -> f32 {
        fn channel(value: f32) -> f32 {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }

        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
    const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

    /// WCAG 2.x contrast ratio between two opaque colours, 1.0 to 21.0.
    fn contrast(ink: [f32; 3], behind: [f32; 3]) -> f32 {
        let (first, second) = (luminance(ink), luminance(behind));
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    /// The colour a plate shows at `offset` into its gradient.
    ///
    /// The same lerp `plate_colour_at` performs, deliberately re-derived from
    /// [`plate_stops`] rather than reached through it: the stops are the
    /// widget's, the interpolation is the test's reading of the renderer.
    fn behind(accent: usize, offset: f32) -> [f32; 3] {
        let [(_, top), (_, bottom)] = plate_stops(accent);
        [
            top.r + (bottom.r - top.r) * offset,
            top.g + (bottom.g - top.g) * offset,
            top.b + (bottom.b - top.b) * offset,
        ]
    }

    /// The pair UX-17 was made at — `CoverArt.qml:43-44`'s `#ffffff` at
    /// `opacity: 0.92`, composited over whatever the plate shows there.
    fn the_pair_before(accent: usize, offset: f32) -> [f32; 3] {
        let alpha = 0.92;
        let under = behind(accent, offset);
        [0, 1, 2].map(|channel| alpha + (1.0 - alpha) * under[channel])
    }

    /// An ink as it lands on the plate: `colour` composited over what is behind
    /// it, which is the colour a reader's eye actually receives.
    ///
    /// The alpha is load-bearing and was got wrong here first. Reading `r`, `g`
    /// and `b` off a `Color` and calling that "what is drawn" silently discards
    /// the alpha, so `white at 0.92` measured as opaque white — and the contrast
    /// test below then passed against the very pair it exists to reject. It was
    /// the mutation proof that found it: reverting `plate_text_color` to the
    /// audit's constant left the test green.
    fn landed(colour: Color, under: [f32; 3]) -> [f32; 3] {
        let alpha = colour.a.clamp(0.0, 1.0);
        let ink = [colour.r, colour.g, colour.b];
        [0, 1, 2].map(|channel| ink[channel] * alpha + (1.0 - alpha) * under[channel])
    }

    /// A real plate and the initials drawn on it, from one laid-out `cover_box`.
    fn plate_and_initials(spec: CoverSpec) -> (Rectangle, Rectangle) {
        let game = Game::new_named("Celeste");
        let mut el: Element<'_, ()> = cover_box(&cache(), &game, spec);
        let seen = traversal(&mut el);

        let plate = seen
            .iter()
            .find(|seen| seen.id == Some(Id::from(PLATE_ID)))
            .expect("the plate container is on the tree")
            .bounds;
        let initials = *drawn(&seen, &cover::initials(&game.name));

        (plate, initials)
    }

    /// The gradient offsets a real plate's initials span, measured.
    fn measured_band(spec: CoverSpec) -> (f32, f32) {
        let (plate, initials) = plate_and_initials(spec);
        (
            (initials.y - plate.y) / plate.height,
            (initials.y + initials.height - plate.y) / plate.height,
        )
    }

    /// The two boxes the app draws initials in.
    ///
    /// Two of the four specs are absent on purpose, and ARCH-25 turned the
    /// first of these from a caveat into a deletion:
    ///
    /// - the **grid tile**'s box was here until ARCH-25, which found that the
    ///   library draws a *card* (`card_cover_spec`) and nothing ever asked for a
    ///   `tile_cover_spec`. It was reachable only from this list, so the test
    ///   was asserting about a plate the application does not draw — the same
    ///   objection the doc already made to the picker's box below, one entry up.
    /// - the **picker**'s fourth box: `cover_preview` draws
    ///   [`preview_label_widget`] — words — for every game without artwork, so
    ///   `initials_only` is never built at a preview spec.
    fn initials_specs() -> [(&'static str, CoverSpec); 2] {
        [("row", row_cover_spec()), ("card", card_cover_spec())]
    }

    /// The font each text node in a built element hands the renderer, with the
    /// content it was built for.
    ///
    /// # Why this reads the tree rather than measuring
    ///
    /// Weight is not visible from outside a `Text` widget: `Widget` has no
    /// accessor for it, `operation::text` reports only `(id, bounds, text)`
    /// (`iced/core/src/widget/operation.rs:63`) with no font on it, and the
    /// renderer's `measure` is the shaped *advance*, which in this environment
    /// is the same for every weight — `"CE"` at 21, 64 and 68 px measures
    /// 24.948002, 76.032005 and 80.784004 px in `default`, `bold` **and**
    /// `semibold`, to six decimals, while a different *family* does differ
    /// (`mono` → 76.800003 at 64 px). The cause is the font stack, not the
    /// widget: [`cosmic::font::default`] names `Open Sans` (`src/font.rs:10-13`)
    /// and `bold` raises only `weight`, but Open Sans is not installed here, so
    /// fontconfig resolves both to `NotoSans[wght].ttf` — a variable font that
    /// the shaper registers as one face and does not vary by axis. A weight
    /// asserted through width on this machine would pass against any weight at
    /// all, which is this repository's own defect class.
    ///
    /// `Widget::layout` caches a `text::State<P>` in the node's `Tree` state,
    /// and its paragraph carries the font the text was shaped with —
    /// `Paragraph::font` (`iced/core/src/text/paragraph.rs:35`), set from the
    /// `Text` the widget built (`:17`). Reading that is a real assertion about
    /// what the plate asks the renderer to draw; it is the *input* to shaping,
    /// so it shows the request and not that this machine's face honours it.
    fn text_fonts<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<(String, Font)> {
        // The trait, for `Paragraph::font`; the associated type, for the state
        // the text widget caches it in.
        use cosmic::iced::advanced::text::Paragraph as _;

        type Paragraph = <cosmic::Renderer as cosmic::iced::advanced::text::Renderer>::Paragraph;
        type State = cosmic::iced::widget::text::State<Paragraph>;

        fn visit(tree: &Tree, fonts: &mut Vec<(String, Font)>) {
            // Matched rather than `downcast_ref`ed: `Tree::state`'s accessor
            // *panics* on a stateless node (`iced/core/src/widget/tree.rs:480`),
            // and most nodes in a plate are stateless.
            if let cosmic::iced::advanced::widget::tree::State::Some(state) = &tree.state
                && let Some(state) = state.downcast_ref::<State>()
            {
                fonts.push((state.content().to_string(), state.raw().font()));
            }
            for child in &tree.children {
                visit(child, fonts);
            }
        }

        let (tree, _) = crate::view::a11y::harness::built(el);
        let mut fonts = Vec::new();
        visit(&tree, &mut fonts);
        fonts
    }
    /// **UX-17.** Every plate's initials are drawn at the contrast their ink was
    /// chosen for, over the region the glyphs cover — and better than the fixed
    /// white-at-0.92 pair the finding was made at.
    ///
    /// The bar is 3:1 rather than 4.5:1 because these are large text: the row
    /// plate draws at 21 px = 15.75 pt and [`initials_only`] draws it bold, as
    /// `CoverArt.qml:45` does, and WCAG 2.2 §1.4.3 counts 14 pt **bold** as
    /// large. The weight is pinned by
    /// [`a_coverless_tiles_initials_are_drawn_bold_like_the_reference`], so this
    /// bar is the right one only while that test passes.
    ///
    /// The numbers, on the row plate (before → after): shade 4 **3.516 →
    /// 3.845**, shade 2 **4.246 → 4.705**, the other six 4.655-6.092 →
    /// 5.173-6.881. Shade 4 is still under 4.5:1 and no ink reaches it there —
    /// the candidates are 3.845 white and 3.226 black — which is stated rather
    /// than closed: what closes it is the plate's own gradient or a bigger
    /// drawing, not the ink. Every failure prints the whole table.
    #[test]
    fn the_plates_initials_clear_the_contrast_bar_and_beat_the_pair_they_replaced() {
        let mut report = String::new();

        for (plate, spec) in initials_specs() {
            let (top, bottom) = measured_band(spec);

            for accent in 0..cover::COVER_ACCENTS {
                let ink = plate_text_color(accent, spec);
                let after = contrast(landed(ink, behind(accent, top)), behind(accent, top)).min(
                    contrast(landed(ink, behind(accent, bottom)), behind(accent, bottom)),
                );
                let before = contrast(the_pair_before(accent, top), behind(accent, top)).min(
                    contrast(the_pair_before(accent, bottom), behind(accent, bottom)),
                );
                // The candidate that was not chosen, composited the same way the
                // chosen one is, so the two numbers are the same quantity. Both
                // candidates `plate_text_color` can return are opaque; the test
                // would not be comparing like with like if one were not.
                let other_ink = if ink.r + ink.g + ink.b > 1.5 {
                    BLACK
                } else {
                    WHITE
                };
                let other = contrast(other_ink, behind(accent, top))
                    .min(contrast(other_ink, behind(accent, bottom)));

                report.push_str(&format!(
                    "{plate:>9} shade {accent}: was {before:.3} (white at 0.92), now {after:.3} \
                     ({}), the other ink would be {other:.3}\n",
                    if other_ink == BLACK { "white" } else { "black" },
                ));

                assert!(
                    after >= 3.0,
                    "shade {accent} on the {plate} plate is drawn at {after:.3}:1, under the 3:1 \
                     WCAG 1.4.3 asks of large text. Pairs:\n{report}"
                );
                assert!(
                    after > before,
                    "shade {accent} on the {plate} plate is drawn at {after:.3}:1, which is no \
                     better than the fixed white-at-0.92 pair this replaced ({before:.3}:1). The \
                     ink is meant to be chosen for the plate it lands on, not to be that constant \
                     with the alpha shuffled. Pairs:\n{report}"
                );
                assert!(
                    after >= other,
                    "shade {accent} on the {plate} plate is drawn at {after:.3}:1 when the other \
                     candidate ink would have given {other:.3}:1 — `plate_text_color` picked the \
                     worse of the two. Pairs:\n{report}"
                );
            }
        }
    }

    /// The band the ink is chosen over is the band the glyphs occupy.
    ///
    /// This is the assumption the whole derivation rests on and the one part of
    /// it no layout can show on its own: the ink is picked for a *region*, and a
    /// region that is not where the text is picks a colour for a plate nobody
    /// sees. The right-hand side is a measurement, not the same expression
    /// again — a test that recomputed `plate_text_band` would agree with it even
    /// with the line-height factor wrong.
    #[test]
    fn the_band_the_ink_is_chosen_over_is_the_band_the_glyphs_occupy() {
        for (plate, spec) in initials_specs() {
            let (top, bottom) = measured_band(spec);
            let (claimed_top, claimed_bottom) = plate_text_band(spec);

            assert!(
                (top - claimed_top).abs() < 1e-4 && (bottom - claimed_bottom).abs() < 1e-4,
                "on the {plate} plate the initials' box spans {top:.5}..{bottom:.5} of the \
                 gradient, and plate_text_band says {claimed_top:.5}..{claimed_bottom:.5}. The \
                 ink is chosen for the band the function claims, so a band that is not the drawn \
                 one picks a colour for a region no glyph is in."
            );
        }
    }

    /// The initials are drawn **bold**, which is `CoverArt.qml:45`'s
    /// `font.bold: true`.
    ///
    /// The weight is not decoration. The row plate draws its initials at 21 px
    /// = 15.75 pt, which WCAG 2.2 §1.4.3 counts as large text only through its
    /// 14 pt **bold** clause; as regular text the same 21 px owes 4.5:1, and
    /// shade 4 of the plate gradient cannot reach 4.5:1 from any ink (3.845
    /// white, 3.226 black). So this test is what makes the 3:1 bar that
    /// [`the_plates_initials_clear_the_contrast_bar_and_beat_the_pair_they_replaced`]
    /// asserts the correct bar — which is why it is asserted here rather than
    /// left as a comment.
    ///
    /// The font is read off the built tree by [`text_fonts`], not measured;
    /// the doc there records why width cannot answer this question in this
    /// environment, and what that costs. The anti-vacuity assertion is the
    /// first one: if `cosmic::font::bold()` were itself the default weight, the
    /// loop below would pass against any weight at all.
    #[test]
    fn a_coverless_tiles_initials_are_drawn_bold_like_the_reference() {
        assert_ne!(
            cosmic::font::bold(),
            cosmic::font::default(),
            "`bold` and `default` are the same font, so asserting a plate draws one rather than \
             the other would pass against either and this test would inspect nothing"
        );

        let game = Game::new_named("Celeste");
        let initials = cover::initials(&game.name);

        for (plate, spec) in initials_specs() {
            let mut el: Element<'_, ()> = cover_box(&cache(), &game, spec);
            let fonts = text_fonts(&mut el);

            let (content, font) = fonts
                .iter()
                .find(|(content, _)| *content == initials)
                .unwrap_or_else(|| {
                    panic!(
                        "the {plate} plate built no text node reading {initials:?}; it built \
                         {fonts:?}. There is no weight to read, and a plate whose initials went \
                         missing is not a plate whose initials are bold."
                    )
                });

            assert_eq!(
                *font,
                cosmic::font::bold(),
                "the {plate} plate draws {content:?} in {font:?}. The reference draws it bold \
                 (`CoverArt.qml:45`), and at 21 px on the row plate the weight is what puts the \
                 initials in WCAG 1.4.3's large-text class."
            );
        }
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
    fn published_by_double_click<M: Clone + 'static>(el: &mut Element<'_, M>, at: Point) -> Vec<M> {
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

    /// The box a drawn string was laid out in.
    fn drawn<'a>(seen: &'a [Seen], text: &str) -> &'a Rectangle {
        seen.iter()
            .find(|seen| seen.text.as_deref() == Some(text))
            .map(|seen| &seen.bounds)
            .unwrap_or_else(|| {
                panic!(
                    "nothing drew {text:?}; the traversal drew {:?}",
                    texts(seen)
                )
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
    ///
    /// The bytes themselves are [`WEBP_COVER`], at module scope because
    /// `super::cover_cache`'s tests decode them too.
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
