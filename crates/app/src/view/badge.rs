//! The pill ux.md asks for by hand, because libcosmic has nothing like it.
//!
//! `ux.md` I7 names this as a `[custom]` row and says why: "**Chip has NO
//! libcosmic equivalent** (no chip/badge/pill found anywhere under
//! `src/widget/`). Build a pill: `container` with rounded style +
//! `text::caption`. Reused for R5 category/release chips." The reference is
//! `Kirigami.Chip` with `closable: false; checkable: false` — a **label** that
//! looks like a chip, never a control: it has no handler in the QML and must
//! not grow one here, because a chip that looks pressable and is not is worse
//! than a heading.
//!
//! # Why this is its own module rather than a function in a page
//!
//! Two pages draw it — the "Installed" badge on a release card
//! (`RunnersPage.qml:190-195`) and the category badge on an installer card
//! (`InstallersPage.qml:107-111`) — and a second copy of the pill would be the
//! second visual idiom the rebalance warned about. It sits beside
//! [`super::cover`], [`super::meta`] and [`super::metrics`] for the same
//! reason they do: it is one visible decision, in one place, with its own test.
//!
//! # The numbers are written down rather than chosen
//!
//! `metrics.rs` holds the geometry of the game-shaped widgets and is T-14's;
//! this module holds the one shape it needs. The radius is a **pill** — half
//! the height or more, so the ends are fully round at any label length, which
//! is what `Kirigami.Chip`'s own `radius` computes to — and the padding is
//! `gridUnit / 2` by `gridUnit / 8`, the pair a chip in the reference theme
//! gets. They are constants with names rather than literals at the call site so
//! that a test can hold them and a second caller cannot quietly disagree.

use cosmic::Element;
use cosmic::iced::{Background, Border, Color, Length};
use cosmic::widget::{container, text};

use super::metrics;

/// The corner radius. Larger than any pill this app draws is tall, which makes
/// the ends semicircular — the shape `Kirigami.Chip` has.
pub const BADGE_RADIUS: f32 = 999.0;

/// Horizontal padding: half a grid unit.
pub const BADGE_PADDING_X: f32 = metrics::GRID_UNIT / 2.0;

/// Vertical padding: an eighth of a grid unit.
pub const BADGE_PADDING_Y: f32 = metrics::GRID_UNIT / 8.0;

/// A label in a pill.
///
/// Generic over the message type and emits none, exactly like everything in
/// [`super::widgets`]: a badge has no behaviour in the reference and this
/// function gives it none.
pub fn badge<M: Clone + 'static>(label: &str) -> Element<'_, M> {
    container(text::caption(label.to_string()))
        .padding([BADGE_PADDING_Y, BADGE_PADDING_X])
        .style(badge_style)
        .into()
}

/// The surface a badge sits on.
///
/// `component.base` is the theme's own colour for a small raised element, which
/// is what a chip is — the same lookup the reference gets through
/// `Kirigami.Theme.alternateBackgroundColor`, without borrowing the card's
/// surface colour (a badge on a card would otherwise be invisible).
fn badge_style(theme: &cosmic::Theme) -> container::Style {
    let cosmic = theme.cosmic();
    container::Style {
        background: Some(Background::Color(
            cosmic.background(false).component.base.into(),
        )),
        border: Border {
            radius: BADGE_RADIUS.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The colour a badge's text is drawn in, for a caller that needs to match it.
///
/// Kept beside [`badge_style`] so the two cannot drift: a badge whose label was
/// left at the default colour would be unreadable on a dark theme, and no test
/// of the widget tree can see a colour.
pub fn badge_text_color(theme: &cosmic::Theme) -> Color {
    theme.cosmic().background(false).on.into()
}

/// `Length::Shrink` for a caller that wants to say so explicitly beside a
/// `Fill` sibling — a row of a badge and a stretched label is the common case,
/// and writing it here keeps the intent legible at the call site.
pub const BADGE_WIDTH: Length = Length::Shrink;

#[cfg(test)]
mod tests {
    use super::*;

    /// The radius is large enough to round the ends of anything this app draws
    /// in a pill. A value that merely looked rounded would pass a render and
    /// fail nothing, so the property is asserted rather than the number.
    #[test]
    fn the_radius_rounds_the_ends_rather_than_only_softening_them() {
        // The tallest badge in the reference is a caption line plus both
        // paddings; anything at or above that radius is a pill.
        let tallest = 24.0 + 2.0 * BADGE_PADDING_Y;
        assert!(
            BADGE_RADIUS >= tallest,
            "a radius below half the height is a rounded rectangle, not a pill"
        );
    }

    /// The paddings are the reference's `gridUnit` fractions, and they are
    /// derived rather than transcribed so a change to `GRID_UNIT` moves them.
    #[test]
    fn the_padding_is_the_reference_s_grid_unit_fractions() {
        assert_eq!(BADGE_PADDING_X, metrics::GRID_UNIT / 2.0);
        assert_eq!(BADGE_PADDING_Y, metrics::GRID_UNIT / 8.0);
    }
}
