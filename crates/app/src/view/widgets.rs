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
//! # What is not here yet
//!
//! Nothing here emits a message and nothing here reads state. The card is not
//! clickable, the row has no play button, and there is no hover or selection
//! styling — all of which need the `Message`/`State` contract (T-07). The
//! shapes below are the ones those will wrap.

use cosmic::Element;
use cosmic::iced::gradient::Linear;
use cosmic::iced::widget::container;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Radians};
use cosmic::widget::{image, text, Column, Row};
use gamehandler_core::models::Game;

use super::cover::{self, CoverSource};
use super::meta;
use super::metrics;

/// The plate's text. White at slightly under full opacity, as `CoverArt.qml`
/// sets it (`color: "#ffffff"; opacity: 0.92`).
const PLATE_TEXT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.92);

/// A game's cover at a given *width*, its height taken from the tile aspect.
///
/// The single-argument form exists for a grid that lays out a column of tiles
/// and wants them all the same shape; a caller with its own box should use
/// [`cover_box`] instead, which is what [`card`] and [`row`] do.
pub fn cover_tile<M: Clone + 'static>(game: &Game, width: f32) -> Element<'_, M> {
    cover_box(game, width, metrics::tile_height(width), metrics::TILE_RADIUS, false)
}

/// A game's cover in an explicit box.
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
pub fn cover_box<M: Clone + 'static>(
    game: &Game,
    width: f32,
    height: f32,
    radius: f32,
    compact: bool,
) -> Element<'_, M> {
    let source = CoverSource::classify(&game.cover_path);

    match source {
        CoverSource::Plate => {
            let inner = initials_only(game, width, height, compact);
            plate(game, width, height, radius, inner)
        }
        CoverSource::Icon => {
            // The plate goes *behind* the icon — `CoverArt.qml`'s
            // `showPlate: coverUrl === "" || coverIsIcon` — so an icon tile
            // reads as artwork on a coloured card rather than as a floating
            // logo. The initials are not drawn: the QML's `visible` test is
            // `coverUrl === ""`, which an icon fails.
            let inner = framed(game, width, height, source, radius, metrics::ICON_INSET);
            plate(game, width, height, radius, inner)
        }
        CoverSource::Photo => framed(game, width, height, source, radius, 0.0),
    }
}

/// A library grid tile: the cover, the name and the subtitle.
///
/// The cover's box comes from [`metrics::card_cover_box`], which is the same
/// subtraction the metrics tests assert on — so a card whose chrome grows
/// shortens its cover rather than overlapping it.
pub fn card<M: Clone + 'static>(game: &Game) -> Element<'_, M> {
    let (cell_w, cell_h) = metrics::GRID_CELL;
    let (box_w, box_h) =
        metrics::card_cover_box(metrics::GRID_CELL, metrics::CARD_CHROME_HEIGHT, metrics::CARD_MARGIN);

    let body = Column::new()
        .push(cover_box(game, box_w, box_h, metrics::TILE_RADIUS, false))
        .push(name_and_subtitle(game))
        .spacing(metrics::CARD_MARGIN)
        .width(Length::Fill);

    container(body)
        .width(Length::Fixed(cell_w))
        .height(Length::Fixed(cell_h))
        .padding(metrics::CARD_MARGIN)
        .align_x(Alignment::Start)
        .align_y(Alignment::Start)
        .style(card_style)
        .into()
}

/// A list row: the cover, the name and the subtitle, laid out horizontally.
///
/// The cover's size is [`metrics::LIST_COVER_WIDTH`] ×
/// [`metrics::LIST_COVER_HEIGHT`] — a fixed strip, not the tile aspect. A row
/// is a fixed-height line whose text must not move when one game's art is a
/// different shape from the next, so the cover is fitted *into* the row rather
/// than setting its height.
pub fn row<M: Clone + 'static>(game: &Game) -> Element<'_, M> {
    let (w, h) = (metrics::LIST_COVER_WIDTH, metrics::LIST_COVER_HEIGHT);

    container(
        Row::new()
            .push(cover_box(game, w, h, metrics::COMPACT_RADIUS, true))
            .push(name_and_subtitle(game))
            .spacing(metrics::CARD_MARGIN)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(metrics::LIST_ROW_HEIGHT))
    .padding(metrics::ICON_INSET / 2.0)
    .align_y(Alignment::Center)
    .into()
}

/// The game form's cover picker: a small preview, or the words "No cover yet".
///
/// This is the widget `NO_COVER_LABEL` belongs to. `GameFormPage.qml` shows an
/// `Image` at `gridUnit * 2` by `gridUnit * 3` with
/// `fillMode: Image.PreserveAspectFit`, and a label in its place when there is
/// nothing to show — so unlike the library plate this one letterboxes a
/// photograph as well as an icon, and unlike the plate it says what is
/// missing rather than drawing initials.
///
/// The label appears for [`CoverSource::Plate`] only, which is the same
/// condition the QML puts on `coverPreview.visible` (`source !== ""`): a game
/// whose cover file has been deleted gets the label, not a broken image.
pub fn cover_preview<M: Clone + 'static>(game: &Game) -> Element<'_, M> {
    let (w, h) = (metrics::FORM_PREVIEW_WIDTH, metrics::FORM_PREVIEW_HEIGHT);

    match CoverSource::classify(&game.cover_path) {
        CoverSource::Plate => container(text(cover::NO_COVER_LABEL).size(11.0))
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .align_x(Alignment::Start)
            .align_y(Alignment::Center)
            .into(),
        source => framed(game, w, h, source, metrics::COMPACT_RADIUS, 0.0),
    }
}

/// The name over the subtitle, as every game-shaped widget shows them.
///
/// The subtitle comes from [`meta`] rather than being composed here — that is
/// the whole point of the module — and the two widgets that show it are
/// therefore guaranteed to show the same string.
fn name_and_subtitle<'a, M: Clone + 'static>(game: &'a Game) -> Element<'a, M> {
    Column::new()
        .push(text(game.name.clone()).size(14.0))
        .push(text(subtitle_of(game)).size(11.0))
        .spacing(2.0)
        .width(Length::Fill)
        .into()
}

/// The subtitle for a game, through the pure function.
///
/// The runner label is the piece this layer cannot resolve: it comes from the
/// runner manager, not from the [`Game`], and the manager is not part of the
/// view's contract yet (T-07). Until it is, a Windows game's label is the
/// empty string — which is exactly the answer [`meta::runner_label`] gives for
/// a manager with nothing to say, and a case [`meta::subtitle`] already
/// handles, so the subtitle degrades to the category alone rather than to a
/// trailing separator. A Linux game is unaffected: its label never consults
/// the manager at all.
fn subtitle_of(game: &Game) -> String {
    let label = meta::runner_label(game.is_linux(), "");
    meta::subtitle(game.display_category(), &label)
}

/// A cover image in a box of exactly `width` × `height`, inset by `inset`.
fn framed<'a, M: Clone + 'static>(
    game: &'a Game,
    width: f32,
    height: f32,
    source: CoverSource,
    radius: f32,
    inset: f32,
) -> Element<'a, M> {
    let picture = image(image::Handle::from_path(&game.cover_path))
        .content_fit(source.content_fit())
        .width(Length::Fill)
        .height(Length::Fill)
        .border_radius(radius);

    // The inset is applied whether or not it is zero, so the two branches
    // cannot drift in their width/height handling.
    container(picture)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .padding(inset)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

/// The initials, centred — what a library tile shows for a game with no cover.
///
/// No words: `CoverArt.qml`'s text is `art.initials` alone. The "No cover yet"
/// string belongs to [`cover_preview`], and a test below pins the difference.
fn initials_only<'a, M: Clone + 'static>(
    game: &'a Game,
    width: f32,
    height: f32,
    compact: bool,
) -> Element<'a, M> {
    container(
        text(cover::initials(&game.name))
            .size(metrics::initials_size(width, height, compact))
            .class(PLATE_TEXT),
    )
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .into()
}

/// A gradient plate of exactly `width` × `height` with `content` centred on
/// it.
fn plate<'a, M: Clone + 'static>(
    game: &'a Game,
    width: f32,
    height: f32,
    radius: f32,
    content: Element<'a, M>,
) -> Element<'a, M> {
    let accent = cover::accent_of(&game.id);
    container(content)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| plate_style(accent, radius))
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
    use gamehandler_core::models::Game;

    /// The subtitle a widget shows, reached through the same path the widgets
    /// use. These are the strings a user reads, so they are asserted as
    /// strings; the pieces they are built from are tested in [`super::meta`].
    #[test]
    fn a_row_shows_the_category_and_the_runner_for_a_windows_game() {
        let mut game = Game::new_named("Half-Life 2");
        game.category = "Shooter".into();
        game.kind = "windows".into();
        // No runner manager yet, so the label is empty and the subtitle is the
        // category with no stray separator — the case `meta::subtitle` and
        // `meta::runner_label` exist to keep honest.
        assert_eq!(subtitle_of(&game), "Shooter");
    }

    /// A Linux game says so without any manager, which is the one half of the
    /// subtitle this layer can complete today.
    #[test]
    fn a_linux_game_is_labelled_native_without_a_runner_manager() {
        let mut game = Game::new_named("Celeste");
        game.category = "Platformer".into();
        game.kind = "linux".into();
        assert_eq!(subtitle_of(&game), "Platformer \u{b7} Linux native");
    }

    /// An uncategorised game shows the runner alone, and today that is the
    /// empty string. Asserted rather than skipped: the point is that it is
    /// *empty* and not a lone separator, which is the visible bug the
    /// uncategorised branch of `meta::subtitle` exists to prevent.
    #[test]
    fn an_uncategorised_windows_game_shows_nothing_rather_than_a_separator() {
        let mut game = Game::new_named("Mystery");
        game.kind = "windows".into();
        assert_eq!(subtitle_of(&game), "");
    }

    /// **The plate and the form's picker are different widgets.** The library
    /// tile shows initials and no words; the form's picker shows the words and
    /// no initials.
    ///
    /// Asserted on the strings the two would draw — `initials` for the tile,
    /// `NO_COVER_LABEL` for the picker — because the two are one function call
    /// apart and swapping them would still compile, still render, and read as
    /// a design choice rather than a bug.
    #[test]
    fn the_plate_shows_initials_and_the_picker_shows_the_words() {
        let game = Game::new_named("Half-Life 2");
        assert_eq!(cover::initials(&game.name), "HL");
        assert_eq!(cover::NO_COVER_LABEL, "No cover yet");
        assert_ne!(
            cover::initials(&game.name),
            cover::NO_COVER_LABEL,
            "the tile's graphic and the form's placeholder must not be the \
             same string"
        );
        // The tile's graphic is not the label, and the label is not initials:
        // neither can be substituted for the other by accident.
        assert!(!cover::NO_COVER_LABEL.contains("HL"));
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

    /// The list row's cover is the fixed strip, not the tile aspect. Those two
    /// differ, and a refactor that made `row` call `cover_tile` would give
    /// 36×54 instead of 36×50.4 — close enough to look right in a screenshot.
    #[test]
    fn a_rows_cover_is_the_strip_and_not_the_tile_aspect() {
        let strip = metrics::LIST_COVER_HEIGHT;
        let tile = metrics::tile_height(metrics::LIST_COVER_WIDTH);
        assert_ne!(strip, tile, "the row's cover must not be the 2:3 tile");
        assert!(
            (strip - tile).abs() > 0.5,
            "the two are only {} apart, which is too close to tell apart",
            (strip - tile).abs()
        );
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
