//! The Library page — `LibraryPage.qml`, `ux.md` §3, PLAN P-01…P-14.
//!
//! Same split as [`super::installers`] and [`super::runners`]: every decision is
//! a pure function over data in the first half of this file, and [`view`] is
//! arrangement. Nothing here is asserted through a renderer.
//!
//! ```text
//! category_options → the filter's list, "All" first
//! sort_index       → which sort the selector shows
//! empty_state      → which of the two empty states, if either
//! ```
//!
//! # The filtering is not here, and that is deliberate
//!
//! Which games are *shown* is `Library::search` (`crates/core/src/models.rs:672`),
//! already a documented port of `models.py:188-199` with its own tests. The page
//! draws the list it is handed, exactly as [`super::installers`] draws the
//! catalog it is handed. Re-deriving the filter here would give the port two
//! implementations of one rule, which is the defect class this project keeps
//! finding.
//!
//! # The two empty states, and why there are two
//!
//! `LibraryPage.qml:82-121` draws **"No games yet"** when `librarySize === 0`
//! and **"No matching games"** — with a Clear-filters button — when
//! `librarySize > 0 && games.length === 0`. The two are keyed on the **total**
//! and on the **filtered** count respectively, and that distinction is the whole
//! point of having two: the first means *there is nothing*, the second means
//! *your search hid everything*.
//!
//! Collapsing them is not a cosmetic slip. A page that keys both on the filtered
//! list tells a user with three hundred games that their library is empty the
//! moment a search matches nothing — and it does so in the one situation where
//! the user most needs to be told the opposite. It is also the collapse
//! `runners::status_line` had, where `Ready | Idle` shared the arm *"No builds
//! found for this family."* and the page stated a negative result it had never
//! obtained.
//!
//! The failure is invisible to the obvious test, which builds an empty library
//! and asserts the empty state: that test passes for both the correct and the
//! collapsed implementation. [`empty_state`] therefore takes the total as its
//! own argument, and the tests drive all three cases, including a **non-empty**
//! library whose filter matches nothing.

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget::{
    Column, Id, Row, Space, button, container, context_menu, menu, scrollable, text, text_input,
};
use gamehandler_core::models::{Game, Library, format_last_played};
use gamehandler_core::runners::RunnerManager;

use crate::Message;
use crate::state::PrefixTool;

use super::a11y;
use super::cover_cache::CoverCache;
use super::metrics;
use super::widgets;

/// The filter value that means "do not filter", as [`super::installers`] spells
/// it (`installers.py:244`, `bridge.py:806`).
pub const ALL_CATEGORIES: &str = "All";

/// The search box's placeholder. `LibraryPage.qml:32`, verbatim.
pub const SEARCH_PLACEHOLDER: &str = "Search games…";

/// The search box's widget id, so a keyboard shortcut can focus it.
///
/// `Ctrl+F` is two actions in the reference — show the Library *and* call
/// `focusSearch()` (`Main.qml:126-134`) — and the second is a widget operation
/// naming this id (`Shell::focus_library_search`). The constant lives here
/// rather than in
/// `main.rs` because this is where the widget that wears it is built: an id
/// declared beside the task that focuses it, but not beside the input that
/// carries it, is a focus that silently finds nothing the day the input is
/// renamed.
///
/// The spelling follows the ids in [`super::widgets`] — `gamehandler.<area>.<name>`
/// — so an id in a log line says which part of the app it belongs to.
pub const SEARCH_INPUT_ID: &str = "gamehandler.library.search";

/// The accessible names of the toolbar's two selectors.
///
/// Neither is a string this port draws, and neither is invented here: they are
/// the reference's own words for these two controls, which are a
/// `QQC2.ToolTip.text` on each ComboBox (`LibraryPage.qml:41`, `:61`) — the QML
/// equivalent of a label, and the only name either control has. Until UX-01
/// wrapped them, neither was announced at all.
///
/// The sort selector's name is the tooltip as written rather than a rewording of
/// it, because the same words already appear in this file's vocabulary for the
/// control ([`sort_labels`], [`sort_index`]) and a second phrasing would be a
/// second thing to keep in step.
pub const CATEGORY_FILTER_LABEL: &str = "Filter by category";
pub const SORT_FILTER_LABEL: &str = "Sort library";

/// The sort options, `(key, label)`. `bridge.py:67-71`, verbatim.
///
/// The keys are validated against [`SORT_MODES`](gamehandler_core::models::SORT_MODES)
/// by a test below, because the
/// settings file's own validation and this selector's list have to agree: a
/// label here whose key the loader rejects would save a mode that silently
/// reverts to `"name"` on the next start.
pub const SORT_OPTIONS: [(&str, &str); 3] = [
    ("name", "Name"),
    ("recent", "Recently played"),
    ("added", "Recently added"),
];

/// The view-mode toggle's two values, matching `VIEW_MODES` in the settings.
pub const GRID: &str = "grid";
pub const LIST: &str = "list";

/// "No games yet" — the state a library with nothing in it draws.
/// `LibraryPage.qml:85-86`.
pub const NO_GAMES_TITLE: &str = "No games yet";
pub const NO_GAMES_EXPLANATION: &str = "Add a Windows or Linux game, install a store launcher in one click, or download a Proton build to get started.";

/// "No matching games" — the state a *non-empty* library draws when the filter
/// matches nothing. `LibraryPage.qml:107-108`. See the module docs: this is not
/// a synonym for [`NO_GAMES_TITLE`].
pub const NO_MATCHES_TITLE: &str = "No matching games";
pub const NO_MATCHES_EXPLANATION: &str = "Try a different search, or clear the category filter.";

/// The three buttons under "No games yet", and the one under "No matching
/// games". `LibraryPage.qml:91,97,102,114`.
pub const ADD_FIRST_GAME: &str = "Add your first game";
pub const EASY_INSTALL: &str = "Easy install";
pub const DOWNLOAD_A_RUNNER: &str = "Download a runner";
pub const CLEAR_FILTERS: &str = "Clear filters";

/// The toolbar's create action. `LibraryPage.qml:18-23`, the page's `actions:`
/// block.
///
/// Its **own** string, not [`ADD_FIRST_GAME`], because the two are not the same
/// control even though they emit the same message: the reference draws this one
/// in the page header whether or not the library is empty, and the first-game
/// wording only under the empty state. Reusing the longer label in the toolbar
/// would make the row grow with it for no reason. Finding #13.
pub const ADD_GAME: &str = "Add game";

// ---------------------------------------------------------------------------
// The decisions, as data
// ---------------------------------------------------------------------------

/// Which empty state the body draws, if any.
///
/// The first arm is the whole point: a library with nothing in it is "No games
/// yet" *whatever* the filter says, and the two counts are separate arguments
/// precisely so that collapsing them is not expressible. Taking one `shown`
/// count and inferring the other is how the distinction gets lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// The library has no games at all.
    NoGames,
    /// The library has games, but the filter matches none of them.
    NoMatches,
    /// There is something to draw.
    None,
}

/// Decide from the **two** counts. See the module docs on why not one.
pub fn empty_state(total: usize, shown: usize) -> EmptyState {
    if total == 0 {
        EmptyState::NoGames
    } else if shown == 0 {
        EmptyState::NoMatches
    } else {
        EmptyState::None
    }
}

/// The category filter's options, `"All"` first.
///
/// `Library.categories()` already sorts and already puts `Uncategorized` last;
/// this only prepends the sentinel, which is what makes the first entry of the
/// selector "all of them" (`bridge.py:339-340`).
pub fn category_options(categories: &[String]) -> Vec<String> {
    let mut options = Vec::with_capacity(categories.len() + 1);
    options.push(ALL_CATEGORIES.to_string());
    options.extend(categories.iter().cloned());
    options
}

/// The labels the sort selector shows, in [`SORT_OPTIONS`] order.
pub fn sort_labels() -> Vec<String> {
    SORT_OPTIONS
        .iter()
        .map(|(_, label)| label.to_string())
        .collect()
}

/// Which sort entry the selector shows.
///
/// `None` when the stored mode is not in the list, which is the same shape as
/// `installers::runner_index`: a dropdown with no selection rather than a panic
/// or a silently-wrong highlight. The loader has already folded an unrecognised
/// value to `"name"` (`Settings::from_dict`'s sort-mode fold,
/// `crates/core/src/settings.rs:161-163`), so this is reachable only if the
/// two lists disagree — which `sort_keys_are_the_ones_the_settings_accept`
/// is what prevents. (Backticks rather than a link: it is a `cfg(test)`
/// function, so it has no documented item to point at.)
pub fn sort_index(mode: &str) -> Option<usize> {
    SORT_OPTIONS.iter().position(|(key, _)| *key == mode)
}

/// The index of `category` in `options`, for the selector.
pub fn category_index(options: &[String], category: &str) -> Option<usize> {
    options.iter().position(|option| option == category)
}

/// The `Message` a category selection carries, from the selector's index.
///
/// **Named rather than left as a closure in [`view`], because the callback's
/// parameter is an index and not a name.** `dropdown`'s `on_selected` is
/// `impl Fn(usize) -> Message` (`libcosmic
/// src/widget/dropdown/mod.rs:30`), so a closure written as
/// `|category| SetCategoryFilter(category.to_string())` — which is what this
/// file originally had, with the parameter named as though it were the category
/// — stores `"1"` where the model wants `"Puzzle"`. Nothing catches that: the
/// string is a valid `String`, the page draws, and the filter matches no game.
/// It is the #46 shape in the message path — a wrong value that no assertion in
/// this file could see, because the closure is inside a builder and a builder
/// needs a renderer.
///
/// So the mapping is a function, and `a_selection_carries_the_name_and_not_the_index`
/// is the assertion that fails for the stringified-index version.
pub fn category_selection(options: &[String], index: usize) -> Message {
    Message::SetCategoryFilter(
        options
            .get(index)
            .cloned()
            .unwrap_or_else(|| ALL_CATEGORIES.to_string()),
    )
}

/// The `Message` a "Clear filters" press carries.
///
/// One message rather than two, because the reference's button does two writes
/// (`LibraryPage.qml:118-121`: the search box is emptied *and* the category
/// reset) and a libcosmic button carries one. Splitting it across two messages
/// would need the button to fire twice; collapsing it into a handler is what
/// makes the two writes atomic, so a partial clear cannot be observed.
pub fn clear_filters() -> Message {
    Message::ClearFilters
}

/// What the view-mode toggle switches *to*, given what is showing.
///
/// The toggle is `checkable` in the reference and reads as "switch to the other
/// one", so the message names the destination and this function is where that
/// is decided — rather than in the button, where it would be untestable.
pub fn toggled_mode(view_mode: &str) -> &'static str {
    if view_mode == LIST { GRID } else { LIST }
}

// ---------------------------------------------------------------------------
// The scrollable, and the window of rows it makes
// ---------------------------------------------------------------------------

/// The Library page's scrollable, so its offset can be read back and (in a
/// future slice) written.
///
/// The page is one scrollable for both view modes — the toolbar and the body
/// scroll together, which is what the reference does: `LibraryPage.qml` puts a
/// single `ScrollView`/`ListView` around the grid and the list alike, and its
/// header stays with the content. A second scrollable per mode would be two
/// widgets to keep in step for no visible difference.
pub const LIBRARY_SCROLL_ID: &str = "gamehandler.library.scroll";

/// How many screenfuls of rows the Library page builds beyond what is on
/// screen.
///
/// **A viewport-height margin, not a row count, and that is the load-bearing
/// choice.** A fixed count would be wrong at both ends: 10 rows is most of a
/// 1200×800 window and a seventh of a 4K one, where 10 cards is not even the
/// rows the grid draws at once. A margin expressed in *viewports* is the same
/// amount of work relative to what the user can see, whatever the window and
/// whatever the mode.
///
/// **Measured, not chosen:** the audit's covers are 200×300 in the grid
/// (`metrics::GRID_CELL`) and 61.2 in the list ([`metrics::LIST_ROW_HEIGHT`]),
/// so a 1200×800 window draws about 3 grid rows (900 px of covers) or 13 list
/// rows (796 px), and two viewports of margin is ~40 grid rows / 26 list rows
/// of *slack in pixels* — far more than a scroll of any speed outruns, because
/// the scroll event that carries the offset is published in the frame the
/// offset changes, before `view()` reads it. One viewport would be enough for
/// correctness; two is what makes a fling not show a blank edge.
pub const WINDOW_VIEWPORTS: f32 = 2.0;

/// The viewport height assumed before the scrollable has published one.
///
/// `State::library_scroll_viewport` starts at `0.0` (see
/// [`crate::Message::SetLibraryScroll`]), and a window of `0.0` rows would draw
/// an empty Library page on the frames before the first publish. The fallback
/// is deliberately close to the smallest window this app is usable in rather
/// than to a large one: the first publish follows the first layout, so this
/// covers one or two frames, and a number that is too small costs one extra
/// frame before the rest appears, where a number that is too large costs work
/// on every frame the publish is missing.
pub const ASSUMED_VIEWPORT_HEIGHT: f32 = 800.0;

/// A half-open range of indices into the filtered list that the page builds.
///
/// `end` is exclusive, so `end - start` is the number of rows and
/// `games[start..end]` is exactly what [`list_body`] and [`grid_body`] draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowWindow {
    /// First built row.
    pub start: usize,
    /// One past the last built row.
    pub end: usize,
}

impl RowWindow {
    /// How many rows are built.
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether `index` is built.
    pub fn contains(self, index: usize) -> bool {
        index >= self.start && index < self.end
    }
}

/// How many rows the grid puts on one line at `viewport_width`.
///
/// # This is a decision and not a measurement, and it has to match iced's
///
/// `grid_body` draws a `Row::wrap()`, and iced decides where to wrap by laying
/// the children out and breaking when `x + child.width > max_width`
/// (`iced/widget/src/row.rs:534-541`). This function is the same rule done in
/// arithmetic, because the window has to be computed *before* any of those
/// children exist. So the numbers have to agree with iced's:
///
/// - a card is `metrics::GRID_CELL.0` wide — `card` pins that with
///   `width(Fixed(..))`, and `Limits` makes a fixed width `min == max`
///   (`iced/core/src/layout/limits.rs:60-66`), so the child's size does not
///   depend on anything;
/// - `Row::new().spacing(6)` puts 6 px between children, and the trailing
///   spacing of a line is trimmed rather than counted
///   (`iced/widget/src/row.rs:563-565`).
///
/// # The two ways this can be wrong, and what happens
///
/// Too **few** columns means the real grid wraps earlier than the window
/// assumed, so the window covers more rows than it needs to and the built set
/// is a superset of what is drawn. Too **many** would leave a gap: the window
/// would believe rows it never built are on screen. The `max(1)` is the
/// degenerate case — a viewport narrower than one card — where iced still draws
/// one card per line and a zero here would divide by zero below.
///
/// `viewport_width` is `0.0` on the pre-layout frame, which is why the
/// fallback is spelled out rather than left to the division: `0 / 206` is 0
/// columns and `max(1)` catches it, but only by accident of the clamp. The
/// named constant makes it a decision.
pub fn grid_columns(viewport_width: f32) -> usize {
    let width = if viewport_width.is_finite() && viewport_width > 0.0 {
        viewport_width
    } else {
        GRID_VIEWPORT_WIDTH_FALLBACK
    };
    let (card, spacing) = (metrics::GRID_CELL.0, GRID_SPACING);
    // `floor((w + spacing) / (card + spacing))` is iced's rule read the other
    // way round: `k` cards and the `k-1` gaps between them take
    // `k * card + (k - 1) * spacing` and must not exceed `width`.
    let columns = ((width + spacing) / (card + spacing)).floor() as usize;
    columns.max(1)
}

/// The viewport width assumed before the scrollable has published one.
///
/// The same one-frame case as [`ASSUMED_VIEWPORT_HEIGHT`], and the same
/// reasoning: the number only has to be sane for the frame before the first
/// layout, and the layout is where the truth comes from. 1200 is the width the
/// audit measured against (`docs/audit/PERFORMANCE.md`, PERF-01: "a 1200×800
/// window").
pub const GRID_VIEWPORT_WIDTH_FALLBACK: f32 = 1200.0;

/// The gap [`grid_body`] puts between cards, and between the rows of cards.
///
/// One constant read by the builder and by [`grid_columns`], because two copies
/// is two chances for the window's arithmetic and the grid's to disagree about
/// where a line ends. `Row::new().spacing(6)` is the builder's half and this is
/// the window's; the test below pins them together.
pub const GRID_SPACING: f32 = 6.0;

/// The gap [`list_body`] puts between rows — `Column::new().spacing(6)`.
pub const LIST_SPACING: f32 = 6.0;

/// Which rows of a filtered list a page of `viewport_height` has to build.
///
/// # The whole of PERF-03's fix, as arithmetic
///
/// The audit measured that every game in the library was given a built
/// `Element` on every frame — `list_body` and `grid_body` looped over all
/// `shown` games (`docs/audit/PERFORMANCE.md`, PERF-03, citing the loops that
/// are now bounded) — which is what made the cover `stat`/`open` and the
/// decoded-cover residency apply to the whole library rather than to the page.
/// This function is the bound: the caller builds `window.len()` rows and pads
/// the rest of the scroll height with two spacers, so the *content* is still
/// the size the user expects and only the *construction* is bounded.
///
/// # Why a row height and not a measured height
///
/// Because both widgets are fixed-height by construction: a list row is
/// `height(Fixed(LIST_ROW_HEIGHT))` and a card is pinned to
/// `metrics::GRID_CELL.1`. A window derived from measured layout would be a
/// feedback loop — the number of rows built would depend on how tall the rows
/// built last frame turned out to be — and this project has already paid for
/// one number defined by the thing it was supposed to be an input to (#96, see
/// [`metrics::PLAY_BUTTON_HEIGHT`]). The heights are constants; the arithmetic
/// over them is testable without a renderer, which is the property the rest of
/// this module is built on.
///
/// # What is built
///
/// `ceil(viewport_rows)` below the first visible row, plus
/// [`WINDOW_VIEWPORTS`] viewports of margin above and below. The margin above
/// matters as much as the one below: scrolling *up* is the case where a window
/// that only looked forward would show unbuilt rows, and it is the one a
/// "start at the last visible row" implementation gets wrong.
///
/// # The index arithmetic is in `f64`, and that is not decoration
///
/// The offsets are `f32` logical pixels and the row height is 61.2 — a value
/// `f32` cannot hold exactly (`metrics.rs`'s own test measures
/// `18.0f32 * 2.8f32` as `50.399998`). Dividing a 6-digit offset by that in
/// `f32` loses about a unit in the last place of the *quotient*, which at the
/// end of a 500-row list is the difference between the last row and beyond it.
/// The inputs stay `f32` — that is what iced hands over — and the division that
/// turns pixels into row indices is done once, in `f64`.
///
/// # Degenerate inputs, and why each has an answer rather than a panic
///
/// Every one of these is external state as far as this function is concerned
/// (a window resize, a first frame before layout, a list emptied by a search
/// while the offset still points past it), so none of them is a reason to take
/// the app down:
///
/// - `total == 0` — nothing to build; the empty range.
/// - `total == 1` — exactly one row whatever the geometry; every list has to
///   be able to show its first entry.
/// - `viewport_height <= 0.0` — the pre-layout frame; [`ASSUMED_VIEWPORT_HEIGHT`].
/// - `row_height <= 0.0` — cannot happen for a caller using `metrics`, and an
///   infinite row count would be worse than a wrong one, so one row.
/// - `offset` past the end — the window lands on the last rows rather than
///   beyond them, because the alternative is an empty page under a scrollbar
///   the user has dragged to the bottom.
/// - a non-finite `offset` or height (`NaN`, `±∞`) — the comparison below is
///   the guard: `first` is only computed when the offset is finite and positive
///   and the row height is positive, so a `NaN` offset takes the `0` arm.
///   *Not* clamped arithmetically, which for `NaN` would be another `NaN`.
pub fn visible_range(
    offset: f32,
    viewport_height: f32,
    row_height: f32,
    total: usize,
) -> RowWindow {
    if total == 0 {
        return RowWindow { start: 0, end: 0 };
    }
    if total == 1 {
        // One row, always built. This is the case a "first visible row" window
        // gets wrong on a list that is shorter than the viewport: the row is
        // on screen, and a window computed from the offset alone would build
        // it only if the offset happened to be in its range.
        return RowWindow { start: 0, end: 1 };
    }

    let row_height = if row_height.is_finite() && row_height > 0.0 {
        row_height
    } else {
        // One row rather than an infinite window: `total` rows of zero height
        // would otherwise all be "visible".
        return RowWindow {
            start: 0,
            end: total,
        };
    };
    let viewport_height = if viewport_height.is_finite() && viewport_height > 0.0 {
        viewport_height
    } else {
        ASSUMED_VIEWPORT_HEIGHT
    };

    // See the doc note: the division is `f64`, the inputs are not.
    let first = if offset.is_finite() && offset > 0.0 {
        (f64::from(offset) / f64::from(row_height)).floor()
    } else {
        0.0
    };
    let rows_on_screen = (f64::from(viewport_height) / f64::from(row_height)).ceil();
    // A non-finite `rows_on_screen` cannot reach here (`viewport_height` and
    // `row_height` are both finite and positive above), but the margin is
    // computed from it, so it is bounded rather than assumed.
    let margin = if rows_on_screen.is_finite() {
        rows_on_screen * f64::from(WINDOW_VIEWPORTS)
    } else {
        0.0
    };

    let last = first + rows_on_screen + margin;
    let start = (first - margin).max(0.0);

    // `total` is a `usize` and every bound above is finite by now, so the
    // conversion is the saturating one and the min is what makes the window a
    // subset of the list — which is the property the spacers depend on.
    let start = (start as usize).min(total - 1);
    let end = (last.ceil().max(0.0) as usize).clamp(start + 1, total);

    RowWindow { start, end }
}

/// The page's inputs, all borrowed from [`crate::State`].
///
/// A bundle rather than a `&State` for the same reason as
/// [`super::runners::RunnersView`]: it is what makes [`view`] callable from a
/// test with no `State` at all, and it makes the page's dependencies readable
/// from its signature instead of from its body.
pub struct LibraryPage<'a> {
    /// The whole library, because both counts are needed and neither is the
    /// other. See [`empty_state`].
    pub library: &'a Library,
    /// [`crate::State::search_text`].
    pub search: &'a str,
    /// [`crate::State::category_filter`].
    pub category: &'a str,
    /// [`gamehandler_core::settings::Settings::sort_mode`].
    pub sort_mode: &'a str,
    /// [`gamehandler_core::settings::Settings::view_mode`].
    pub view_mode: &'a str,
    /// The runner manager, used only to resolve each shown game's label.
    ///
    /// # This is called from the render path, and that is a known cost
    ///
    /// [`super::widgets::resolved_runner_label`] is documented as *not* being
    /// called from a widget, because [`RunnerManager::label`] was uncached and
    /// walked the filesystem. [`view`] runs every frame, so taking the manager
    /// here reintroduced exactly that — once per shown game per frame.
    ///
    /// **PERF-06 memoised `label`**, against the runner directory's mtime, so
    /// what that costs now is one `stat` per shown game per frame rather than a
    /// directory walk, a file open and a JSON parse. That is a bounded cost, and
    /// it is why the paragraph that used to stand here — promising a `State`
    /// field holding the resolved rows instead — no longer applies: 20 `stat`s
    /// per frame do not need caching *above* a cache to be affordable, and a
    /// second layer would only be a second thing to invalidate.
    ///
    /// The manager is taken rather than the labels because the alternative was
    /// worse: the label has to come from somewhere, and the only other source
    /// was the empty string, which is `#30` — every Windows game rendered no
    /// runner at all.
    pub runners: &'a RunnerManager,
    /// The instant every row's last-played label is computed against.
    ///
    /// **One value for the whole page, not one per row.** `format_last_played`
    /// takes `now` as a parameter precisely so this is possible: a row reading
    /// the clock itself would make two rows of the same age render
    /// `"Played 1 day ago"` and `"Played 2 days ago"` depending on which side of
    /// midnight the loop reached first, and a test could not pin either without
    /// also freezing the clock it cannot see. Taking it here makes the whole
    /// page's timestamps a function of `(library, now)`, which is what the
    /// oracle's frozen-clock cases assume.
    ///
    /// The caller supplies [`gamehandler_core::models::now`].
    pub now: f64,
    /// [`crate::State::cover_cache`] — the classify and decode memo.
    ///
    /// Taken here rather than reached for globally, for the reason the rest of
    /// this struct is a bundle: the page's dependencies are readable from its
    /// signature. It is the `&`-shared cache the widget builders ask through,
    /// so a test can hand the page a cache of its own and read back how many
    /// filesystem reads the frame it just built actually performed — which is
    /// [`CoverCache::classify_calls`], the counter
    /// `widgets::a_frame_of_tiles_classifies_each_cover_once` asserts on.
    pub covers: &'a CoverCache,
    /// How far the page is scrolled, and how tall the box it is drawn in is.
    ///
    /// A struct rather than three floats on this bundle because the three move
    /// together: they are one reading of one widget, published together and
    /// consumed together, and a caller that passed a fresh offset with a stale
    /// height would build the window for a frame that never existed. See
    /// [`ScrollGeometry`].
    pub scroll: ScrollGeometry,
}

/// What the window computation needs to know about the scrollable.
///
/// The three numbers of [`crate::Message::SetLibraryScroll`], grouped so the
/// page takes them as one value, plus the convention that every one of them is
/// `0.0` until the first publish — which [`visible_range`] and [`grid_columns`]
/// each read as "not known yet" rather than as a measurement of zero.
///
/// `viewport_width` is here rather than derived from `viewport_height` because
/// the two are genuinely independent: the grid's column count is a function of
/// the width, and the list's row count of the height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollGeometry {
    /// [`crate::State::library_scroll_offset`].
    pub offset: f32,
    /// [`crate::State::library_scroll_viewport`] — the **height** of the
    /// viewport. `0.0` before the first layout.
    pub viewport_height: f32,
    /// [`crate::State::library_scroll_width`] — the **width** of the viewport,
    /// from the same publish as the other three
    /// (`Viewport::bounds().width`). `0.0` before the first layout, which
    /// [`grid_columns`] reads as "use [`GRID_VIEWPORT_WIDTH_FALLBACK`]".
    ///
    /// The offset and the width come from the same `Viewport` in the same
    /// message rather than one of them being derived here, because mixing a
    /// measured width with a published offset would describe two different
    /// frames — the one the page is about to be drawn in and the one the
    /// scrollable last measured.
    pub viewport_width: f32,
    /// [`crate::State::library_scroll_content`].
    pub content_height: f32,
}

/// The page.
///
/// # The whole page is wrapped in one scrollable, with an id and an `on_scroll`
///
/// That is PERF-03's other half, and the two halves are one decision. The body
/// is virtualized by [`visible_range`], which needs the offset and the viewport
/// size; those come from the scrollable the body is *inside*, and
/// `Scrollable::on_scroll` (`iced/widget/src/scrollable.rs:183`) is the only
/// route from that widget to this state. Wrapping is what lets iced hand the
/// viewport back; the `Id` is not required by `on_scroll` (a scrollable needs
/// an id only to be scrolled *to* from elsewhere), but it is set here for the
/// next slice — a "go to top" affordance or a keyboard shortcut — and because
/// an unaddressed scrollable is one nothing can ever reach, which is the same
/// reason [`SEARCH_INPUT_ID`] exists.
///
/// iced publishes the viewport from the scrollable's own `update`, in the frame
/// that changed it (`:1285`) and from the `RedrawRequested` arm that drives
/// auto-scrolling (`:1262`) — long before `view()` for that frame runs, so the
/// offset the window is computed from is the one the user's scroll produced,
/// not last frame's. And it publishes *only* on a change: the redundant case
/// returns before `shell.publish` (`:2058-2076`), so an idle app sends no
/// messages at all — which is the property `docs/audit/PERFORMANCE.md`'s
/// verified-sound item 1 is about, and the one this must not break.
pub fn view<'a>(page: LibraryPage<'a>) -> Element<'a, Message> {
    let categories = category_options(&page.library.categories());
    let shown = page
        .library
        .search(page.search, page.category, page.sort_mode);
    let empty = empty_state(page.library.len(), shown.len());

    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- The toolbar -------------------------------------------------------
    let view_toggle = if page.view_mode == LIST {
        "Switch to grid"
    } else {
        "Switch to list"
    };
    body = body.push(
        Row::new()
            // `input_with_id`, not `input`: this field's widget id is
            // [`SEARCH_INPUT_ID`], which `Ctrl+F` focuses by name
            // (`Shell::focus_library_search`), so it cannot take the id its
            // placeholder would derive — see that constant and
            // `a11y::input_with_id`. The name is still the placeholder, which is
            // what the field shows when it is empty.
            .push(a11y::input_with_id(
                text_input(SEARCH_PLACEHOLDER, page.search.to_string())
                    .on_input(Message::SetSearchText)
                    .width(Length::Fill),
                SEARCH_PLACEHOLDER,
                page.search.to_string(),
                SEARCH_INPUT_ID.into(),
            ))
            .push(a11y::dropdown(
                cosmic::widget::dropdown(
                    categories.clone(),
                    category_index(&categories, page.category),
                    // The selector's index, mapped to the category it stands for —
                    // never the index itself. See [`category_selection`].
                    {
                        let options = categories.clone();
                        move |index| category_selection(&options, index)
                    },
                ),
                CATEGORY_FILTER_LABEL,
                // What the selector is *showing*, which is also what
                // `on_selected` would be handed next: `category_index` is
                // `None` for a stored category that is not in the list, and
                // the step below is `None` there too. See [`category_index`].
                category_index(&categories, page.category)
                    .and_then(|index| categories.get(index).cloned()),
                {
                    let options = categories.clone();
                    let count = categories.len();
                    let shown = category_index(&categories, page.category);
                    move |delta| {
                        let next = shown?.checked_add_signed(delta as isize)?;
                        (next < count).then(|| category_selection(&options, next))
                    }
                },
            ))
            .push(a11y::dropdown(
                cosmic::widget::dropdown(sort_labels(), sort_index(page.sort_mode), |index| {
                    Message::SetSortMode(SORT_OPTIONS[index].0.to_string())
                }),
                SORT_FILTER_LABEL,
                sort_index(page.sort_mode).map(|index| SORT_OPTIONS[index].1.to_string()),
                {
                    let shown = sort_index(page.sort_mode);
                    let count = SORT_OPTIONS.len();
                    move |delta| {
                        let next = shown?.checked_add_signed(delta as isize)?;
                        // The **key**, not the index — the same mapping the
                        // pointer's `on_selected` above makes. A step that
                        // stringified the index would store `"1"`, which is
                        // not in `SORT_OPTIONS`, and the loader would fold it
                        // back to `"name"` on the next start
                        // (`a_selection_carries_the_name_and_not_the_index`
                        // is the same defect at the sibling selector).
                        (next < count)
                            .then(|| Message::SetSortMode(SORT_OPTIONS[next].0.to_string()))
                    }
                },
            ))
            .push(button::standard(view_toggle).on_press(Message::SetViewMode(
                toggled_mode(page.view_mode).to_string(),
            )))
            // The page's create action, which the reference keeps in the header
            // and therefore shows **whenever there is a library at all**. It was
            // ported into the `NoGames` empty state only, so once a user had one
            // game the sole route to the form was `Ctrl+N` — invisible in the UI,
            // and (measured at T-19) swallowed while a text field has focus.
            // `BUG-07`.
            .push(button::standard(ADD_GAME).on_press(Message::OpenNewGameForm))
            .spacing(8)
            .align_y(cosmic::iced::Alignment::Center)
            .width(Length::Fill),
    );

    // ---- The body ----------------------------------------------------------
    match empty {
        EmptyState::NoGames => {
            body = body
                .push(text::title3(NO_GAMES_TITLE))
                .push(text::body(NO_GAMES_EXPLANATION))
                .push(
                    Row::new()
                        .push(button::standard(ADD_FIRST_GAME).on_press(Message::OpenNewGameForm))
                        .push(
                            button::standard(EASY_INSTALL)
                                .on_press(Message::NavigateTo(crate::Page::Installers)),
                        )
                        .push(
                            button::standard(DOWNLOAD_A_RUNNER)
                                .on_press(Message::NavigateTo(crate::Page::Runners)),
                        )
                        .spacing(8),
                );
        }
        EmptyState::NoMatches => {
            body = body
                .push(text::title3(NO_MATCHES_TITLE))
                .push(text::body(NO_MATCHES_EXPLANATION))
                .push(button::standard(CLEAR_FILTERS).on_press(clear_filters()));
        }
        EmptyState::None => {
            body = body.push(if page.view_mode == LIST {
                list_body(&shown, page.runners, page.now, page.covers, page.scroll)
            } else {
                grid_body(&shown, page.runners, page.covers, page.scroll)
            });
        }
    }

    // `on_scroll` is what makes the window possible; see this function's docs.
    // The four numbers are read off the `Viewport` iced hands over, and
    // `bounds()` is the viewport's box where `content_bounds()` is the
    // content's — swapping the two would make the window grow as the user
    // scrolled, which is the feedback loop `visible_range`'s doc refuses.
    container(
        scrollable(body)
            .id(Id::from(LIBRARY_SCROLL_ID))
            .on_scroll(|viewport| Message::SetLibraryScroll {
                offset: viewport.absolute_offset().y,
                viewport_width: viewport.bounds().width,
                viewport_height: viewport.bounds().height,
                content_height: viewport.content_bounds().height,
            }),
    )
    .padding(18)
    .into()
}

/// One list body row, wrapped in the game's context menu.
///
/// Split out of [`list_body`] so that the row builder and the menu builder are
/// written once, at the loop that is now a window. Extracting it is also what
/// makes the window's effect visible to a test: the count of these inside a
/// built body is the count of rows the frame constructed, which is the quantity
/// PERF-03 is about.
fn list_row<'a>(
    game: &'a Game,
    runners: &'a RunnerManager,
    now: f64,
    covers: &CoverCache,
) -> Element<'a, Message> {
    // Both strings are resolved here, where the row data is assembled, and
    // carried as data — `resolved_runner_label`'s doc explains why the
    // manager must not be reached from inside the builder.
    let (runner, last_played) = row_labels(game, runners, now);
    let labels = widgets::RowLabels {
        runner: &runner,
        last_played: &last_played,
    };
    // The launch is built here, where the game is in hand, and handed to the
    // row as a value: `view/widgets.rs` names no `Message` of its own, which
    // is the property that keeps its tests message-free. Constructing it
    // inside the builder would also move the emission off this page, where
    // `tests/dispatch_coverage.rs` can see it.
    Element::from(context_menu(
        widgets::row(
            covers,
            game,
            &labels,
            Message::LaunchGame(game.id.clone()),
            Some(Message::OpenGameMenu(game.id.clone())),
        ),
        Some(game_menu_trees(game)),
    ))
}

/// A spacer that stands in for a **run of `rows` unbuilt rows**, gaps included.
///
/// The window's other half, and it is what keeps the *content* the size the user
/// expects while the *construction* is bounded: the scrollable's content bounds
/// are the sum of the built rows and these, so the scrollbar's thumb is the
/// right size and the offset the user asks for is the one they get.
///
/// # The `(rows - 1)` is the whole of the arithmetic, and its absence is a bug
///
/// A run of `rows` rows occupies `rows * row_height` plus the `rows - 1` gaps
/// *between* them. A spacer sized to `rows * row_height` alone is short by those
/// gaps, and the shortfall is not a rounding error: it is `(rows - 1) *
/// LIST_SPACING`, which for a 500-row library is 2,994 px of drift between one
/// scroll position and another — the content height would change as the user
/// scrolled, and the window that is computed *from* the offset would be computed
/// against bounds that the window itself had moved.
///
/// That was this function's first version, and it was wrong. Measured by
/// `the_spacers_keep_the_page_the_height_the_full_list_would_have`, which
/// built the same 500-game page at three geometries and got 30,902.4 px for the
/// windowed ones and 31,052.4 px for the run with every row built: 150.002 px,
/// which is 25 × 6.0 — the 25 gaps the two windows differed by. The test is the
/// reason this line has a `- 1` in it.
///
/// The derivation, so a reader can check the `- 1` rather than trust it. With
/// `a` leading and `c` trailing unbuilt rows, `b` built rows (`a + b + c = n`),
/// and `Column`'s spacing between adjacent children:
///
/// | spacer height | children | gaps | total |
/// |---|---|---|---|
/// | `a*h` | `b + 1 + 1` | `b + 1` | `n*h + (a + b + c + 1)*s` — **two gaps too many** |
/// | `a*h + (a-1)*s` | `b + 1 + 1` | `b + 1` | `n*h + (n - 1)*s` |
///
/// The second row is the all-rows-built body's own height,
/// `n * row_height + (n - 1) * LIST_SPACING` — which is the number the offset
/// is measured against in the first place.
///
/// `rows` is never 0: the caller checks before pushing, because a `Space` of
/// height `-6.0` is a negative layout contribution rather than a no-op.
fn rows_spacer(rows: usize) -> Element<'static, Message> {
    debug_assert!(rows > 0, "an empty run has no height to stand in for");
    let height = rows as f32 * metrics::LIST_ROW_HEIGHT + (rows as f32 - 1.0) * LIST_SPACING;
    Space::new()
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .into()
}

/// The list: one row per game, each with its last-played label.
///
/// `now` is the page's single instant, passed down rather than read here; see
/// [`LibraryPage::now`]. The last-played half is why this is not
/// [`grid_body`]'s twin: `LibraryPage.qml` draws the two delegates with
/// different text, and only the row carries the timestamp (`:269` against the
/// card's `:192`).
///
/// # The loop is over a window, and that is PERF-03
///
/// The audit measured this loop building a row for **every** game in the
/// filtered library on every frame (`docs/audit/PERFORMANCE.md`, PERF-03, which
/// cites the `for game in games` at the line this replaced). It now builds
/// [`visible_range`]'s window and pads the rest with [`rows_spacer`]s, so the
/// work is a function of the window's height rather than of the library's size.
///
/// The spacers are why the two modes are not one loop: the list is a single
/// column of fixed-height rows, so one spacer above and one below is exact,
/// where the grid needs a spacer per *line* to preserve its wrap points. See
/// [`grid_body`].
fn list_body<'a>(
    games: &[&'a Game],
    runners: &'a RunnerManager,
    now: f64,
    covers: &CoverCache,
    scroll: ScrollGeometry,
) -> Element<'a, Message> {
    let window = visible_range(
        scroll.offset,
        scroll.viewport_height,
        list_row_pitch(),
        games.len(),
    );

    let mut body = Column::new().spacing(LIST_SPACING).width(Length::Fill);
    if window.start > 0 {
        body = body.push(rows_spacer(window.start));
    }
    for game in &games[window.start..window.end] {
        body = body.push(list_row(game, runners, now, covers));
    }
    let tail = games.len() - window.end;
    if tail > 0 {
        body = body.push(rows_spacer(tail));
    }
    body.into()
}

/// The height one list row takes in the column: the row and the gap under it.
///
/// **The pitch and not the row height**, because the offset the window is
/// computed from is a distance down a column of `spacing`-separated rows, so
/// row `k` starts at `k * (height + spacing)` and not at `k * height`. Using
/// the bare height would put the window one row further down for every ten
/// rows scrolled, which is a blank edge at the bottom of a long list — the
/// exact failure this window exists to avoid, and invisible in a short one.
///
/// A gap after the *last* row is not part of the content
/// ([`rows_spacer`]'s doc), and that is why this is used for the division and
/// not for the spacer heights.
fn list_row_pitch() -> f32 {
    metrics::LIST_ROW_HEIGHT + LIST_SPACING
}

/// The grid: the cards, wrapped.
///
/// `Row::wrap` rather than a fixed number of columns: the reference's grid is a
/// `GridView` whose `cellWidth` is 200 (`LibraryPage.qml:139`), so the column
/// count is whatever fits — which is what wrapping gives and what a hard-coded
/// count would get wrong on every window width but one.
///
/// # The window, and why its spacers are per line
///
/// PERF-03 applies here as it does to [`list_body`], but the spacer arithmetic
/// is not the same, because the content is not one column. The **wrap points**
/// are what a naive window would move: iced breaks a line when the next card
/// would pass the right edge (`iced/widget/src/row.rs:534-541`), so a body that
/// started at card 40 with nothing before it would re-flow from the left margin
/// and draw a different grid. So the rows *before* the window are laid down as
/// whole lines — `start_row` of them, each a full row of columns — which puts
/// the first built card back at the column the full grid would have given it.
///
/// Each spacer is `columns` card-widths wide, which is what makes it exactly one
/// line tall: `grid_columns` is the same rule iced wraps by, so a spacer that
/// occupies `columns` children takes exactly one line. A single spacer spanning
/// the whole height would be *taller* by the vertical spacing between lines
/// (`Row::wrap` adds `vertical_spacing` per line, `:543`), which is why this is
/// a count of lines and not a count of pixels.
///
/// # Where the grid can be short, stated
///
/// The columns here are computed from [`ScrollGeometry::viewport_width`], which
/// the caller supplies because `on_scroll` publishes a viewport *rectangle* and
/// this crate keeps only the three numbers the window needs
/// ([`crate::Message::SetLibraryScroll`]). A width that is wrong means the
/// spacers hold the wrong number of columns, so a line may hold one card fewer
/// or more than the built rows beside it. The consequence is a scrollbar whose
/// range is off by up to a line at the very bottom — the cards themselves are
/// always built, because the window's *row* range comes from the height, and
/// each row's cards are contiguous. Recorded rather than papered over.
fn grid_body<'a>(
    games: &[&'a Game],
    runners: &'a RunnerManager,
    covers: &CoverCache,
    scroll: ScrollGeometry,
) -> Element<'a, Message> {
    let columns = grid_columns(scroll.viewport_width);
    let line_pitch = metrics::GRID_CELL.1 + GRID_SPACING;

    // The window is computed over *lines*, so the index arithmetic in
    // `visible_range` — which is about a list — is reused by asking it for the
    // lines of a list of `ceil(games / columns)` entries, then widening the
    // answer back to cards. Doing it the other way round (windows over cards,
    // divided by columns) would need the division to round *out* at both ends
    // to keep every card of a partly-visible line, which is the same answer
    // with one more place to get it wrong.
    let lines = games.len().div_ceil(columns);
    let line_window = visible_range(scroll.offset, scroll.viewport_height, line_pitch, lines);

    let first_card = line_window.start * columns;
    let end_card = (line_window.end * columns).min(games.len());
    let start_line = line_window.start;
    let tail_lines = lines - line_window.end;

    // Children go on the `Row` and the whole row wraps: `Wrapping` itself has
    // no `push`, only the spacing and alignment of the wrapped lines.
    let mut row = Row::new().spacing(GRID_SPACING);
    if start_line > 0 {
        row = row.push(grid_lines_spacer(start_line));
    }
    for game in &games[first_card..end_card] {
        let label = widgets::resolved_runner_label(runners, game);
        // Built here rather than in the builder; see `list_body`.
        row = row.push(context_menu(
            widgets::card(
                covers,
                game,
                &label,
                Message::LaunchGame(game.id.clone()),
                Some(Message::OpenGameMenu(game.id.clone())),
            ),
            Some(game_menu_trees(game)),
        ));
    }
    if tail_lines > 0 {
        row = row.push(grid_lines_spacer(tail_lines));
    }
    row.wrap().into()
}

/// A spacer standing in for a **run of `lines` unbuilt grid lines**, gaps
/// included.
///
/// **Full width, which is what makes it a line of its own.** A `Row` breaks when
/// the running `x` plus the next child's width would pass the right edge
/// (`iced/widget/src/row.rs:534-541`), so a child as wide as the row's own
/// maximum cannot share a line with anything: the card that follows it is
/// placed at column 0 of the next line — which is exactly the column card
/// `start_line * columns` has in the fully built grid, so the windowed grid's
/// cards land on the columns they belong to. That is the property the whole
/// window depends on: without it a windowed grid would re-flow from the left
/// margin and draw a different picture.
///
/// **One spacer for the whole run, not one per line.** A run of `lines` lines
/// occupies `lines * GRID_CELL.1` plus the `lines - 1` vertical gaps *between*
/// them, and `Row::wrap` adds only the gaps *between children* (`:543`) — so a
/// spacer of `lines * GRID_CELL.1` alone is short by `(lines - 1) *
/// GRID_SPACING` exactly as [`rows_spacer`]'s first version was short by its
/// own. The derivation is that function's, at one row per line.
///
/// The empty `Space` inside is what gives the `container` something to be: a
/// `Container` with no content would still take the fixed height, but it would
/// report no child, and the grid's shape is read through child counts.
///
/// `lines` is never 0: the caller checks before pushing.
fn grid_lines_spacer(lines: usize) -> Element<'static, Message> {
    debug_assert!(lines > 0, "an empty run has no height to stand in for");
    let height = lines as f32 * metrics::GRID_CELL.1 + (lines as f32 - 1.0) * GRID_SPACING;
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .into()
}

/// The two labels one list row is built from: the runner, and the last-played
/// string.
///
/// # Why this is a function and not two lines in the loop
///
/// Because the loop builds `Element`s, and an assertion cannot read a built
/// row's text back out — the wall #46, #51 and #57 each hit, and the reason
/// `view/widgets.rs` has a `traversal` helper at all and this module has none.
/// Extracting the pair makes *the values* checkable even though the *call site*
/// is not, which is the honest split `view/runners.rs` and
/// `view/installers.rs` already record for their own extractions.
///
/// **Measured, not assumed:** with this function inlined back into the loop,
/// replacing `game.last_played` with `0.0` leaves the whole suite green — every
/// row would read `"Never played"` and nothing would notice. With it extracted,
/// `tests::a_rows_labels_carry_the_games_own_timestamp` fails on that
/// mutation. The call-site mutation that remains is `list_body` passing the
/// wrong `now`, which is unobservable for the reason above.
///
/// That test's name is a plain code span: it lives in this module's private
/// `mod tests`, which rustdoc does not document, so the bracketed form is a
/// broken intra-doc link rather than a pointer.
fn row_labels(game: &Game, runners: &RunnerManager, now: f64) -> (String, String) {
    (
        widgets::resolved_runner_label(runners, game),
        format_last_played(game.last_played, now),
    )
}

/// One entry of a game's context menu: an action, or a divider.
///
/// Pure data, so the menu's shape — the eight labels, their order, the two
/// dividers, which items carry icons, which a Linux game disables — is
/// asserted without a renderer. [`game_menu_trees`] turns this into widgets;
/// the mapping between the two is pinned by `tests::every_spec_entry_reaches_a_tree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        label: &'static str,
        icon: Option<crate::icons::Icon>,
        enabled: bool,
        action: GameMenuKind,
    },
    Divider,
}

/// The eight actions a game's context menu offers, without the game.
///
/// Split from [`GameMenuAction`] so the menu's *shape* is plain data: the
/// spec says what the menu holds, the action says what a pressed item sends.
/// `Copy` because the action wraps it and
/// [`MenuAction`](cosmic::widget::menu::Action) requires `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMenuKind {
    Play,
    Edit,
    FindCover,
    Winecfg,
    Winetricks,
    OpenPrefix,
    Shortcut,
    Remove,
}

/// A menu action bound to its game: what a pressed item sends.
///
/// Borrowed, because [`MenuAction`](cosmic::widget::menu::Action) requires
/// `Copy` and a `GameId` is a `String`: the `&str` borrows the game the menu
/// was built for, and
/// [`message`](cosmic::widget::menu::Action::message) clones it into the
/// outgoing message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameMenuAction<'a> {
    kind: GameMenuKind,
    game_id: &'a str,
}

impl<'a> menu::Action for GameMenuAction<'a> {
    type Message = Message;

    fn message(&self) -> Message {
        let id = self.game_id.to_string();
        match self.kind {
            GameMenuKind::Play => Message::LaunchGame(id),
            GameMenuKind::Edit => Message::OpenEditGameForm(id),
            GameMenuKind::FindCover => Message::FetchCover(id),
            GameMenuKind::Winecfg => Message::RunPrefixTool {
                game_id: id,
                tool: PrefixTool::WineCfg,
            },
            GameMenuKind::Winetricks => Message::RunPrefixTool {
                game_id: id,
                tool: PrefixTool::Winetricks,
            },
            GameMenuKind::OpenPrefix => Message::OpenPrefixFolder(id),
            GameMenuKind::Shortcut => Message::CreateDesktopShortcut(id),
            GameMenuKind::Remove => Message::ConfirmDeleteGame(id),
        }
    }
}

/// The game menu's eight items and two dividers, in order.
///
/// `gameMenu`, `LibraryPage.qml:298-343`, entry for entry: labels and icon
/// names verbatim, the prefix tools and the prefix folder disabled for Linux
/// games (`:320, :326, :331`). The three prefix items read `!is_linux` rather
/// than the QML's `menuGame !== null &&` guard — there is no null menu game
/// here, because each card builds its own menu with its own id baked into the
/// actions instead of sharing one menu over mutable state.
pub fn game_menu_spec(game: &Game) -> Vec<MenuEntry> {
    let prefix = !game.is_linux();
    let item = |label: &'static str,
                icon: Option<crate::icons::Icon>,
                enabled: bool,
                action: GameMenuKind| {
        MenuEntry::Item {
            label,
            icon,
            enabled,
            action,
        }
    };
    vec![
        item(
            "Play",
            Some(crate::icons::Icon::Play),
            true,
            GameMenuKind::Play,
        ),
        item(
            "Edit",
            Some(crate::icons::Icon::Edit),
            true,
            GameMenuKind::Edit,
        ),
        item(
            "Find cover art",
            Some(crate::icons::Icon::Image),
            true,
            GameMenuKind::FindCover,
        ),
        MenuEntry::Divider,
        item("Winecfg", None, prefix, GameMenuKind::Winecfg),
        item("Winetricks", None, prefix, GameMenuKind::Winetricks),
        item(
            "Open prefix folder",
            Some(crate::icons::Icon::FolderOpen),
            prefix,
            GameMenuKind::OpenPrefix,
        ),
        MenuEntry::Divider,
        item(
            "Create desktop shortcut",
            None,
            true,
            GameMenuKind::Shortcut,
        ),
        item(
            "Remove from library",
            Some(crate::icons::Icon::Delete),
            true,
            GameMenuKind::Remove,
        ),
    ]
}

/// The spec as widgets, one tree per entry, for [`context_menu()`].
///
/// No surface wiring and no `window_id`: without them the menu renders as an
/// in-window overlay anchored at the click on every platform, which is what a
/// menu that must also work outside a Wayland compositor wants. See D-56.
pub fn game_menu_trees<'a>(game: &'a Game) -> Vec<menu::Tree<Message>> {
    let items: Vec<menu::Item<GameMenuAction<'a>, &'static str>> = game_menu_spec(game)
        .into_iter()
        .map(|entry| match entry {
            MenuEntry::Divider => menu::Item::Divider,
            MenuEntry::Item {
                label,
                icon,
                enabled,
                action,
            } => {
                let item = GameMenuAction {
                    kind: action,
                    game_id: game.id.as_str(),
                };
                let handle = icon.map(crate::icons::handle);
                if enabled {
                    menu::Item::Button(label, handle, item)
                } else {
                    menu::Item::ButtonDisabled(label, handle, item)
                }
            }
        })
        .collect();
    menu::items(&std::collections::HashMap::new(), items)
}

/// The id of the actions layer's `index`-th control, for a game (**UX-16**).
///
/// Indexed by position among the **items** of [`game_menu_spec`], counting from
/// zero and skipping the two dividers, so the id a caller focuses and the
/// control a user lands on are the same one. `Shell::update` focuses index 0
/// when the layer opens; without that the keyboard user who opened the layer
/// would be left on the button they pressed it with, with the rest of the page
/// between them and the actions.
///
/// Per game, for [`widgets::play_button_id`]'s reason: the layer is drawn once
/// per game that has one open, and the library can have many cards.
pub fn game_menu_item_id(game_id: &str, index: usize) -> String {
    format!("gamehandler.library.menu.{game_id}.{index}")
}

/// The message a game's menu action sends.
///
/// One function for the three things that must agree about it: the context menu
/// item ([`game_menu_trees`]), the actions layer's control
/// ([`game_menu_actions`]), and any caller that needs to know what choosing an
/// action does. Each used to build the `GameMenuAction` itself, which is three
/// chances to disagree about which message a "Winecfg" press carries.
///
/// The trait is reached through its path rather than imported: `menu::Action` is
/// not in scope at this level, and importing a trait used in one expression
/// would put a name in every reader's way.
pub fn game_menu_message(game_id: &str, kind: GameMenuKind) -> Message {
    menu::Action::message(&GameMenuAction { kind, game_id })
}

/// A game's actions as buttons — the layer the "More actions" control opens
/// (**UX-16**).
///
/// # Why the app draws this rather than opening the toolkit's menu
///
/// The menu the port already renders ([`game_menu_trees`], through
/// `context_menu()`) can only be opened by a pointer: `ContextMenu` acts on
/// `Event::Mouse(ButtonReleased(Right))` and on the two-finger touch lift beside
/// it (`src/widget/context_menu.rs:441-460`, predicate at `:626`) and on nothing
/// else. There is no `Message`, no method and no `LocalState` field a caller
/// outside libcosmic can reach to open it, and there is no way to reach the
/// widget it renders either: `context_menu` builds a
/// `crate::widget::menu::Menu` (`:573-590`) and that struct is `pub(crate)`
/// (`src/widget/menu.rs:81`), declared in a private module (`mod menu_inner`,
/// `:71`). So the app cannot reuse the toolkit's menu as a keyboard-opened one;
/// what it can do is draw the same entries as ordinary buttons, which is what
/// this is.
///
/// # The divergence from the reference, stated
///
/// `LibraryPage.qml:209-213` (card) and `:281-284` (row) draw a
/// `QQC2.ToolButton` beside Play that calls `page.openGameMenu(modelData, this)`,
/// and what opens is the real `QQC2.Menu` — a popup with eight items
/// (`:298-343`). The port reaches the same eight entries through the same
/// control, but the layer behind it is a modal dialog of buttons rather than a
/// popup menu, because the widget that draws the popup is not constructible from
/// outside libcosmic. What is *not* divergent is the content or the effect: the
/// entries come from [`game_menu_spec`] and each control sends the message
/// [`GameMenuAction`] gives for its entry, so the keyboard route and the context
/// menu cannot disagree about what an action is.
///
/// # Disabled entries
///
/// The three prefix items are inert for a Linux game, exactly as the menu's
/// `ButtonDisabled` items are (`:320, :326, :331`). They are drawn — with their
/// labels and their accessible names, so a keyboard user is told the action
/// exists and is unavailable rather than never learning of it — and they carry
/// no `on_press`, which is the same shape `view/installers.rs` uses for the
/// Install button while a download runs.
pub fn game_menu_actions<'a>(game: &'a Game) -> Element<'a, Message> {
    let mut column = Column::new().spacing(metrics::CARD_MARGIN);
    let mut index = 0;
    for entry in game_menu_spec(game) {
        match entry {
            MenuEntry::Divider => {
                column = column.push(cosmic::widget::divider::horizontal::light());
            }
            MenuEntry::Item {
                label,
                enabled,
                action,
                ..
            } => {
                let message = game_menu_message(&game.id, action);
                let control = button::text(label).id(game_menu_item_id(&game.id, index).into());
                column = column.push(if enabled {
                    control.on_press(message)
                } else {
                    control
                });
                index += 1;
            }
        }
    }
    column.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::widget::menu::Action as _;
    use gamehandler_core::models::SORT_MODES;

    fn windows_game() -> Game {
        let mut game = Game::new_named("Hades");
        game.kind = "windows".to_string();
        game.id = "had-es".to_string();
        game
    }

    fn linux_game() -> Game {
        let mut game = Game::new_named("Celeste");
        game.kind = "linux".to_string();
        game.id = "ce-les-te".to_string();
        game
    }

    fn labels(spec: &[MenuEntry]) -> Vec<&str> {
        spec.iter()
            .map(|entry| match entry {
                MenuEntry::Divider => "---",
                MenuEntry::Item { label, .. } => label,
            })
            .collect()
    }

    /// The menu is the reference's `gameMenu`, entry for entry.
    ///
    /// `LibraryPage.qml:298-343`: eight labels in order with the two
    /// separators where the QML puts them, and the five icon names verbatim.
    /// A label reworded, an icon renamed, or an entry reordered fails here —
    /// the menu's text is user-visible and the reference's is the authority.
    #[test]
    fn menu_spec_matches_the_reference_entry_for_entry() {
        let spec = game_menu_spec(&windows_game());
        assert_eq!(
            labels(&spec),
            [
                "Play",
                "Edit",
                "Find cover art",
                "---",
                "Winecfg",
                "Winetricks",
                "Open prefix folder",
                "---",
                "Create desktop shortcut",
                "Remove from library",
            ]
        );
        let icons: Vec<Option<&str>> = spec
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Divider => None,
                MenuEntry::Item { icon, .. } => Some(icon.map(crate::icons::Icon::legacy_name)),
            })
            .collect();
        assert_eq!(
            icons,
            [
                Some("media-playback-start"),
                Some("edit-entry"),
                Some("viewimage"),
                None,
                None,
                Some("folder-open"),
                None,
                Some("delete"),
            ]
        );
        assert!(
            spec.iter().all(|entry| match entry {
                MenuEntry::Divider => true,
                MenuEntry::Item { enabled, .. } => *enabled,
            }),
            "a Windows game disables nothing"
        );
    }

    /// Linux games disable the three prefix items and nothing else.
    ///
    /// `LibraryPage.qml:320, :326, :331` — and the disabled items keep their
    /// icons, because `ButtonDisabled` carries one: "Open prefix folder" is
    /// dimmed with its folder, not dimmed and stripped.
    #[test]
    fn linux_games_disable_the_prefix_items() {
        let spec = game_menu_spec(&linux_game());
        let states: Vec<(&str, bool)> = spec
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Divider => None,
                MenuEntry::Item { label, enabled, .. } => Some((*label, *enabled)),
            })
            .collect();
        assert_eq!(
            states,
            [
                ("Play", true),
                ("Edit", true),
                ("Find cover art", true),
                ("Winecfg", false),
                ("Winetricks", false),
                ("Open prefix folder", false),
                ("Create desktop shortcut", true),
                ("Remove from library", true),
            ]
        );
        let prefix = spec
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item {
                    label: "Open prefix folder",
                    icon,
                    enabled,
                    ..
                } => Some((*icon, *enabled)),
                _ => None,
            })
            .expect("the prefix item");
        assert_eq!(prefix, (Some(crate::icons::Icon::FolderOpen), false));
    }

    /// Every menu action sends its message with the menu's game id.
    ///
    /// The spec says what the menu holds; this says what a press does. The
    /// id is the assertion's point: each card bakes in its own, which is what
    /// replaces the QML's shared `menuGame` without sharing anything.
    #[test]
    fn every_menu_action_sends_its_message_with_the_games_id() {
        use GameMenuKind::*;
        // `matches!`, because `Message` is not `PartialEq` — the guards carry
        // the payload assertions an `assert_eq!` would.
        let message = |kind| {
            GameMenuAction {
                kind,
                game_id: "had-es",
            }
            .message()
        };
        assert!(matches!(message(Play), Message::LaunchGame(id) if id == "had-es"));
        assert!(matches!(message(Edit), Message::OpenEditGameForm(id) if id == "had-es"));
        assert!(matches!(message(FindCover), Message::FetchCover(id) if id == "had-es"));
        assert!(
            matches!(message(Winecfg), Message::RunPrefixTool { game_id, tool }
                if game_id == "had-es" && matches!(tool, PrefixTool::WineCfg))
        );
        assert!(
            matches!(message(Winetricks), Message::RunPrefixTool { game_id, tool }
                if game_id == "had-es" && matches!(tool, PrefixTool::Winetricks))
        );
        assert!(matches!(message(OpenPrefix), Message::OpenPrefixFolder(id) if id == "had-es"));
        assert!(matches!(message(Shortcut), Message::CreateDesktopShortcut(id) if id == "had-es"));
        assert!(matches!(message(Remove), Message::ConfirmDeleteGame(id) if id == "had-es"));
    }

    /// Every spec entry reaches a tree: no silent drops in the mapping.
    ///
    /// `menu::items` drops a *trailing* divider; ours are interior, so the
    /// tree count equals the spec length for both game kinds — and a future
    /// entry added to the spec without reaching the widgets fails here.
    #[test]
    fn every_spec_entry_reaches_a_tree() {
        for game in [windows_game(), linux_game()] {
            let spec = game_menu_spec(&game);
            let trees = game_menu_trees(&game);
            assert_eq!(
                trees.len(),
                spec.len(),
                "a spec entry never became a widget"
            );
        }
    }

    /// Every action in the layer is a **named, focusable, reachable control**,
    /// one per menu entry, in the menu's order — and the three prefix entries a
    /// Linux game disables are drawn, announced, and inert.
    ///
    /// UX-16, from the assistive-technology side. The keyboard half is
    /// [`every_action_in_the_layer_sends_what_its_menu_item_sends`]; this half is
    /// what a screen reader is told before any key is pressed, and the two are
    /// asserted separately because a layer of anonymous buttons would satisfy the
    /// second and not this one.
    ///
    /// The labels are compared against [`game_menu_spec`]'s own, so a layer that
    /// reworded an entry — or drew seven of the eight — fails here rather than
    /// matching itself.
    #[test]
    fn every_action_in_the_layer_is_a_named_control_and_the_disabled_ones_say_so() {
        use super::a11y::harness;

        for (game, disabled_are) in [(windows_game(), 0usize), (linux_game(), 3)] {
            let mut layer = game_menu_actions(&game);
            let nodes = harness::published(&mut layer);

            let items: Vec<(&str, bool, GameMenuKind)> = game_menu_spec(&game)
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Divider => None,
                    MenuEntry::Item {
                        label,
                        enabled,
                        action,
                        ..
                    } => Some((label, enabled, action)),
                })
                .collect();
            assert_eq!(
                items.len(),
                8,
                "the fixture is the reference's eight entries"
            );

            // Selected by id rather than by position: each of these buttons
            // publishes its label as a child node of its own, so the tree is
            // twice as long as the layer and a positional walk would compare a
            // button against a paragraph.
            let controls: Vec<&harness::NodeFacts> = (0..items.len())
                .map(|index| {
                    let id = iced_accessibility::A11yId::from(Id::from(game_menu_item_id(
                        &game.id, index,
                    )));
                    nodes.iter().find(|node| node.id == id).unwrap_or_else(|| {
                        panic!(
                            "nothing in the layer carries the id `Shell::update` focuses \
                                 when it opens, so that focus lands nowhere. Published: {nodes:#?}"
                        )
                    })
                })
                .collect();
            assert_eq!(
                controls.len(),
                items.len(),
                "the layer drew {} controls for {} menu entries",
                controls.len(),
                items.len()
            );
            for (index, (node, (label, enabled, _))) in controls.iter().zip(&items).enumerate() {
                assert_eq!(
                    node.role,
                    iced_accessibility::accesskit::Role::Button,
                    "control {index} ({label})"
                );
                assert_eq!(
                    node.label.as_deref(),
                    Some(*label),
                    "control {index} is not named with its menu entry's label"
                );
                assert!(
                    node.focus,
                    "control {index} ({label}) cannot be focused, so it is not a \
                     keyboard route to anything"
                );
                assert_eq!(
                    node.disabled, !*enabled,
                    "control {index} ({label}) reports the wrong enabled state"
                );
            }

            let disabled = controls.iter().filter(|node| node.disabled).count();
            assert_eq!(
                disabled, disabled_are,
                "a {} game has {disabled} disabled entries, not {disabled_are}",
                game.kind
            );
        }
    }

    /// **Pressing a control in the layer sends exactly what pressing the menu
    /// item sends** — for all eight, on both a Windows and a Linux game.
    ///
    /// This is UX-16's load-bearing assertion: the layer is a *duplicate* of the
    /// context menu in everything but the widget that draws it, and the only
    /// thing that makes that duplication safe is that its messages are not
    /// written twice. The expectations here are built from
    /// [`game_menu_message`] — the same function the layer itself calls — so the
    /// test cannot pass by agreeing with a second transcription of the mapping.
    ///
    /// The key is driven through the real widget tree: `tab_to` carries the
    /// framework's own focus forward, and `dispatch` hands the widget the event
    /// the runtime would, so a control that is drawn but not in the Tab ring
    /// fails here rather than being counted.
    #[test]
    fn every_action_in_the_layer_sends_what_its_menu_item_sends() {
        use super::a11y::harness;
        use cosmic::iced::keyboard::{Key, key::Named};

        for game in [windows_game(), linux_game()] {
            let mut layer = game_menu_actions(&game);
            let expected: Vec<(&str, bool, Message)> = game_menu_spec(&game)
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Divider => None,
                    MenuEntry::Item {
                        label,
                        enabled,
                        action,
                        ..
                    } => Some((label, enabled, game_menu_message(&game.id, action))),
                })
                .collect();

            let stops = harness::focusables(&mut layer).len();
            assert_eq!(
                stops,
                expected.len(),
                "the layer reports {stops} Tab stops for {} entries — every entry, \
                 disabled or not, is drawn as a button and libcosmic's button reports \
                 itself focusable without consulting `on_press` \
                 (`src/widget/button/widget.rs:357`)",
                expected.len()
            );

            let (mut tree, node) = harness::built(&mut layer);
            for (index, (label, enabled, want)) in expected.iter().enumerate() {
                harness::tab_to(&mut layer, &mut tree, &node);
                let mut messages = Vec::new();
                let _ = harness::dispatch(
                    &mut layer,
                    &mut tree,
                    &node,
                    &harness::pressed(Key::Named(Named::Enter)),
                    &mut messages,
                );
                if *enabled {
                    assert_eq!(
                        messages.len(),
                        1,
                        "Enter on control {index} ({label}) published {messages:?}"
                    );
                    // `Message` is not `PartialEq` — it carries a `ToastId`, a
                    // `Task` and a `GameForm` — so the comparison is the derived
                    // `Debug` shape, which is structural for every variant in it.
                    assert_eq!(
                        format!("{:?}", messages[0]),
                        format!("{want:?}"),
                        "control {index} ({label}) sends a different message from the \
                         menu item it duplicates"
                    );
                } else {
                    assert!(
                        messages.is_empty(),
                        "control {index} ({label}) is one the menu disables, and Enter on \
                         it published {messages:?}"
                    );
                }
            }
        }
    }

    /// **Enter at each Tab stop of the real Library page reaches each game's
    /// own actions, and carries that card's id** (UX-16).
    ///
    /// The end of the route as a user meets it: the card's control is built by
    /// the *page's* call site, which is where the game's id is in hand. Every
    /// other test of this finding starts from a control with the message already
    /// attached, so a call site that passed one game's id to every card — or
    /// passed `LaunchGame`'s id into the actions control — would leave them all
    /// green and give every card the first card's actions. That failure is the
    /// one this walks the page for.
    ///
    /// The stops are walked rather than named, because the ring is the claim: a
    /// control drawn but not focusable is not a keyboard route.
    #[test]
    fn enter_at_each_stop_of_the_real_page_reaches_each_games_actions() {
        use super::a11y::harness;
        use cosmic::iced::keyboard::{Key, key::Named};

        let library = two_games_with_known_ids();
        let covers = CoverCache::new();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        let mut element =
            page_element(&library, LIST, ScrollGeometry::default(), &covers, &runners);

        let stops = harness::focusables(&mut element).len();
        assert!(
            stops >= 4,
            "this page draws two cards of two controls each, so a ring of {stops} \
             cannot contain them"
        );
        let (mut tree, node) = harness::built(&mut element);
        let mut messages = Vec::new();
        for _ in 0..stops {
            harness::tab_to(&mut element, &mut tree, &node);
            let _ = harness::dispatch(
                &mut element,
                &mut tree,
                &node,
                &harness::pressed(Key::Named(Named::Enter)),
                &mut messages,
            );
        }

        let mut opened: Vec<String> = messages
            .iter()
            .filter_map(|message| match message {
                Message::OpenGameMenu(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        opened.sort_unstable();
        assert_eq!(
            opened,
            ["alpha", "beta"],
            "Enter at each Tab stop of the real page must reach each game's actions \
             once, carrying that card's own id. Messages: {messages:?}"
        );
    }

    /// A library of two games whose **ids are known**, for the route test above.
    ///
    /// [`categorised_library`] names its games and lets `new_id()` name the ids,
    /// which is right for a test that reads names and wrong for one that has to
    /// say which game a message is about.
    fn two_games_with_known_ids() -> Library {
        let root = std::env::temp_dir().join(format!(
            "gh-lib-route-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
        for id in ["alpha", "beta"] {
            let mut game = Game::new_named(id.to_string());
            game.id = id.to_string();
            library.add(game).expect("the temp library is writable");
        }
        library
    }

    /// The layer's ids are per game and per position, so two cards open two
    /// layers and neither focus request can land on the other's control.
    ///
    /// Written because the alternative — one id derived from the entry alone —
    /// compiles, draws, and reads correctly, and would make
    /// `Shell::update`'s focus request a coin toss between two visible cards.
    #[test]
    fn the_layers_ids_name_the_game_and_the_position() {
        assert_eq!(
            game_menu_item_id("had-es", 0),
            "gamehandler.library.menu.had-es.0"
        );
        assert_eq!(
            game_menu_item_id("had-es", 7),
            "gamehandler.library.menu.had-es.7"
        );
        assert_ne!(
            game_menu_item_id("had-es", 0),
            game_menu_item_id("ce-les-te", 0)
        );
        assert_ne!(
            game_menu_item_id("had-es", 0),
            game_menu_item_id("had-es", 1)
        );
    }

    /// The keys the selector offers are the ones the settings loader accepts.
    ///
    /// Two lists in two crates have to agree: a label here whose key
    /// `Settings::from_dict` rejects would save a mode that reverts to `"name"`
    /// on the next start, with nothing on screen saying so.
    #[test]
    fn sort_keys_are_the_ones_the_settings_accept() {
        let keys: Vec<&str> = SORT_OPTIONS.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, SORT_MODES.to_vec());
    }

    /// A row's labels carry the game's **own** timestamp and the page's **one**
    /// instant. Parity item P-02/P-16.
    ///
    /// This test exists because of a measured gap, not a hypothetical one: with
    /// [`row_labels`] inlined back into [`list_body`]'s loop, replacing
    /// `game.last_played` with `0.0` — so every row in the library reads
    /// "Never played" — leaves the entire suite green. Nothing else asserts
    /// what a row's timestamp is, because `list_body` builds `Element`s and
    /// this module has no traversal helper to read them back out.
    ///
    /// Two games with deliberately different timestamps, so a `row_labels` that
    /// ignored its argument and returned one constant string for both fails
    /// here rather than passing on a single-game fixture.
    #[test]
    fn a_rows_labels_carry_the_games_own_timestamp() {
        let runners = RunnerManager::at("/nonexistent");
        let now = 1_700_000_000.0;

        let mut recent = Game::new_named("Recent");
        recent.kind = "windows".into();
        recent.last_played = now - 3.0 * 3600.0;

        let mut old = Game::new_named("Old");
        old.kind = "windows".into();
        old.last_played = now - 40.0 * 86_400.0;

        let (recent_runner, recent_played) = row_labels(&recent, &runners, now);
        let (old_runner, old_played) = row_labels(&old, &runners, now);

        assert_eq!(recent_played, "Played 3 hours ago");
        assert_eq!(old_played, "Played 1 month ago");
        assert_ne!(
            recent_played, old_played,
            "two games of different ages rendered the same last-played label — \
             the timestamp is not reaching the row"
        );
        // The runner half is resolved here too, and for a Windows game with no
        // runner directory it is the fallback rather than an empty string.
        assert_eq!(recent_runner, "System Wine");
        assert_eq!(old_runner, "System Wine");
    }

    /// A game that has never been played still gets a label, and the page's
    /// `now` is what makes it deterministic.
    ///
    /// `format_last_played(0.0, _)` is `"Never played"` whatever the instant,
    /// which is the branch a zero timestamp takes rather than the arithmetic —
    /// asserted beside the arithmetic case above so that "no timestamp" and
    /// "an old timestamp" are not confused for one another.
    #[test]
    fn an_unplayed_games_labels_say_never_played() {
        let runners = RunnerManager::at("/nonexistent");
        let game = Game::new_named("Mystery");
        assert_eq!(game.last_played, 0.0);

        let (_, played) = row_labels(&game, &runners, 1_700_000_000.0);
        assert_eq!(played, "Never played");
    }

    /// **The two empty states are not the same state.**
    ///
    /// The case that matters is the third: a library with games in it whose
    /// filter matches none. An implementation that keyed both states on the
    /// filtered count answers `NoGames` there, and every test that only ever
    /// builds an **empty** library passes for it — which is every test anyone
    /// would naturally write.
    #[test]
    fn a_filter_that_matches_nothing_is_not_an_empty_library() {
        assert_eq!(empty_state(0, 0), EmptyState::NoGames);
        assert_eq!(empty_state(3, 0), EmptyState::NoMatches);
        assert_eq!(empty_state(3, 2), EmptyState::None);
        assert_ne!(
            empty_state(3, 0),
            empty_state(0, 0),
            "a library with games and a filter that hides them all is not \
             'No games yet'. Keying this on the filtered count alone tells a \
             user their library is empty exactly when their search matched \
             nothing"
        );
    }

    /// The strings are the reference's, and the two titles differ.
    #[test]
    fn the_two_empty_states_say_different_things() {
        assert_eq!(NO_GAMES_TITLE, "No games yet");
        assert_eq!(NO_MATCHES_TITLE, "No matching games");
        assert_ne!(NO_GAMES_TITLE, NO_MATCHES_TITLE);
        assert!(NO_GAMES_EXPLANATION.starts_with("Add a Windows or Linux game"));
        assert!(NO_MATCHES_EXPLANATION.starts_with("Try a different search"));
        assert_eq!(SEARCH_PLACEHOLDER, "Search games…");
    }

    /// The three buttons under "No games yet" are the reference's, and the
    /// third goes to the Runners page rather than nowhere.
    #[test]
    fn the_empty_library_offers_the_reference_three_ways_out() {
        assert_eq!(ADD_FIRST_GAME, "Add your first game");
        assert_eq!(EASY_INSTALL, "Easy install");
        assert_eq!(DOWNLOAD_A_RUNNER, "Download a runner");
        assert_eq!(CLEAR_FILTERS, "Clear filters");
    }

    /// Every string the page actually draws, by walking the built widget tree.
    ///
    /// Copied in shape from `view::settings`'s `drawn_strings`, and here for the
    /// reason `BUG-07` is a finding at all: no test in this file had ever built
    /// [`view`], so a control could leave the page — it did — without a single
    /// assertion noticing. Constant-level tests cannot see *where* a widget is.
    fn drawn_strings(mut element: Element<'_, Message>) -> Vec<String> {
        use cosmic::iced::advanced::widget::{Operation, Tree};
        use cosmic::iced::advanced::{Layout, layout::Limits};
        use cosmic::iced::{Font, Pixels, Rectangle, Size};

        #[derive(Default)]
        struct Texts(Vec<String>);
        impl Operation for Texts {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn text(&mut self, _id: Option<&cosmic::widget::Id>, _bounds: Rectangle, text: &str) {
                self.0.push(text.to_string());
            }
        }

        let renderer = cosmic::Renderer::new(Font::default(), Pixels(16.0));
        let mut tree = Tree::new(element.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        let mut texts = Texts::default();
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut texts);
        texts.0
    }

    /// The page, built the way the shell builds it.
    ///
    /// The cache is fresh and the geometry is the zero one. Zero is the
    /// pre-layout publish — no scrollable has reported itself yet — and
    /// `visible_range` treats it as one assumed window rather than as "nothing
    /// is visible", so a page built here draws the first screenful exactly as
    /// the shell's first frame does. See
    /// `a_page_with_no_published_geometry_still_draws_its_first_rows`.
    fn page_strings(library: &Library, view_mode: &str) -> Vec<String> {
        page_strings_scrolled(library, view_mode, ScrollGeometry::default())
    }

    /// The same page, with the geometry the shell publishes stamped on it.
    ///
    /// The window tests need both halves of the same run: the *names the page
    /// built* (read off the traversal, which is what a user would see) and the
    /// *range the arithmetic says it should have built* (from
    /// [`visible_range`]). Handing the same [`ScrollGeometry`] to both is what
    /// makes them comparable.
    fn page_strings_scrolled(
        library: &Library,
        view_mode: &str,
        scroll: ScrollGeometry,
    ) -> Vec<String> {
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        let covers = CoverCache::new();
        page_strings_with(library, view_mode, scroll, &covers, &runners)
    }

    /// The page with a cache the caller keeps, so a test can read the counters
    /// back after building it.
    fn page_strings_with(
        library: &Library,
        view_mode: &str,
        scroll: ScrollGeometry,
        covers: &CoverCache,
        runners: &RunnerManager,
    ) -> Vec<String> {
        drawn_strings(page_element(library, view_mode, scroll, covers, runners))
    }

    /// The page as an `Element`, built the way the shell builds it.
    fn page_element<'a>(
        library: &'a Library,
        view_mode: &'a str,
        scroll: ScrollGeometry,
        covers: &'a CoverCache,
        runners: &'a RunnerManager,
    ) -> Element<'a, Message> {
        view(LibraryPage {
            library,
            search: "",
            category: ALL_CATEGORIES,
            sort_mode: "name",
            view_mode,
            runners,
            now: 0.0,
            covers,
            scroll,
        })
    }

    /// The height the page lays out to, under unbounded limits.
    ///
    /// The same layout pass [`drawn_strings`] runs, stopped one step earlier:
    /// the node's size instead of the strings the operation collected. It is
    /// what makes the spacer test a measurement of the widget tree rather than
    /// of the expression that built it.
    fn page_height(mut element: Element<'_, Message>) -> f32 {
        use cosmic::iced::advanced::Layout;
        use cosmic::iced::advanced::layout::Limits;
        use cosmic::iced::advanced::widget::Tree;
        use cosmic::iced::{Font, Pixels, Size};

        let renderer = cosmic::Renderer::new(Font::default(), Pixels(16.0));
        let mut tree = Tree::new(element.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        let _ = Layout::new(&node);
        node.size().height
    }

    /// A library with two categorised games, so the toolbar's two selectors have
    /// more than one entry each (`category_options` prepends [`ALL_CATEGORIES`])
    /// and the category filter can be set to something that is not the default.
    fn categorised_library() -> Library {
        let root = std::env::temp_dir().join(format!(
            "gh-lib-a11y-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
        for (name, category) in [("Alpha", "Puzzle"), ("Beta", "Action")] {
            let mut game = Game::new_named(name.to_string());
            game.category = category.to_string();
            library.add(game).expect("the temp library is writable");
        }
        library
    }

    /// The page with every one of its own inputs set to a **non-default** value.
    ///
    /// That is the point of the fixture rather than a convenience: the state
    /// assertions below compare each published node against a value the page was
    /// handed, and a page built from defaults would pass just as well against a
    /// builder that published the defaults. `"alpha"` is not `""`, `"Puzzle"` is
    /// not [`ALL_CATEGORIES`] and `"recent"` is not `"name"`.
    fn a11y_page<'a>(
        library: &'a Library,
        covers: &'a CoverCache,
        runners: &'a RunnerManager,
    ) -> Element<'a, Message> {
        a11y_page_showing(library, covers, runners, "Puzzle")
    }

    /// The same page with the category filter set to `category`.
    ///
    /// Split out for the step walk, which needs the filter somewhere in the
    /// *middle* of its option list: [`category_options`] is `["All", …the
    /// library's own…]`, so a filter on the first or last entry has an arrow
    /// that is `None` by design and a walk from it would measure one direction.
    fn a11y_page_showing<'a>(
        library: &'a Library,
        covers: &'a CoverCache,
        runners: &'a RunnerManager,
        category: &'a str,
    ) -> Element<'a, Message> {
        view(LibraryPage {
            library,
            search: "alpha",
            category,
            sort_mode: "recent",
            view_mode: LIST,
            runners,
            now: 0.0,
            covers,
            scroll: ScrollGeometry::default(),
        })
    }

    /// **Every control on the real Library page is a Tab stop, announces the role
    /// it is, and publishes the state or value it is showing** — UX-01 and UX-03
    /// measured where the user meets them.
    ///
    /// # Why this is a page test and not another wrapper test
    ///
    /// `view/a11y.rs` proves the wrapper reports a focusable state, builds a node
    /// and puts the node's id in the ring. All of that stays true if this page
    /// goes back to the bare `cosmic::widget::dropdown` / `text_input`, because a
    /// wrapper nobody calls still works perfectly — so what this test is *for* is
    /// the call sites. Reverting any one of the three in [`view`] takes its name
    /// out of both lists and this fails on that name.
    ///
    /// # Why each control is asserted by name and not by a count
    ///
    /// The toolbar also carries libcosmic's own buttons — the view-mode toggle
    /// and Add game — and those report a focusable state of their own
    /// (`src/widget/button/widget.rs:358-359`), so a count over this page would
    /// be a number that moves whenever the toolkit's button changes. The
    /// Settings page can assert an exact count because every control on it is one
    /// this port built; this page cannot, for the reason `view/form.rs`'s
    /// equivalent gives at more length.
    ///
    /// # Why the values are asserted and not just the names
    ///
    /// A node's *label* and its *role* are the two halves that say a control is
    /// announced; they say nothing about whether it is announced **correctly**.
    /// A call site that passed a literal `"All"` as the category value, or
    /// `SORT_OPTIONS[0].1` where the shown entry is index 1, would satisfy every
    /// name and role assertion above and tell a screen-reader user that the
    /// filter is showing something it is not. The value is the only field that
    /// catches that, and this page is where the mutation was measured: replacing
    /// the sort selector's `sort_index(page.sort_mode).map(…)` with the constant
    /// `Some(SORT_OPTIONS[0].1.to_string())` fails the assertion below with
    /// `left: Some("Name"), right: Some("Recently played")` and nothing else in
    /// this module notices.
    #[test]
    fn every_control_on_the_real_library_page_is_a_tab_stop_and_a_named_node() {
        use super::a11y::harness;
        use cosmic::iced::core::id::IdEq;
        use iced_accessibility::accesskit::Role;

        let library = categorised_library();
        let covers = CoverCache::new();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        // **One** element, read twice. The toolkit's own widgets take
        // `Id::unique()` at construction, so the focus reports and the nodes have
        // to come out of the same build to be comparable at all — see
        // `a11y::harness::unreachable_controls`.
        let mut element = a11y_page(&library, &covers, &runners);

        // ---- the Tab ring ---------------------------------------------------
        let stops = harness::focusables(&mut element);
        let ids: Vec<cosmic::widget::Id> = stops
            .iter()
            .map(|stop| {
                stop.clone().unwrap_or_else(|| {
                    panic!(
                        "a control is focusable but reports no id, so Tab can \
                         reach it and nothing can address it: {stops:?}"
                    )
                })
            })
            .collect();

        // ---- the nodes ------------------------------------------------------
        let nodes = harness::published(&mut element);
        let listing = || {
            nodes
                .iter()
                .map(|node| (&node.role, node.label.as_deref(), node.value.as_deref()))
                .collect::<Vec<_>>()
        };
        let node = |role: Role, label: &str| {
            nodes
                .iter()
                .find(|node| node.role == role && node.label.as_deref() == Some(label))
                .unwrap_or_else(|| {
                    panic!(
                        "the {label:?} control publishes no {role:?} node. The \
                         toolbar's search box and two selectors are the three \
                         controls this page wraps, and a bare \
                         `cosmic::widget::dropdown` publishes nothing at all \
                         (`view/a11y.rs`'s \
                         `the_toolkit_controls_the_app_used_to_build_are_invisible` \
                         measures that directly). Nodes: {:?}",
                        listing()
                    )
                })
        };

        // The three wrapped controls, each with the role it must announce as and
        // the value [`a11y_page`] handed the page.
        let search = node(Role::TextInput, SEARCH_PLACEHOLDER);
        assert_eq!(
            search.value.as_deref(),
            Some("alpha"),
            "the search box must publish the text it is holding, not a default \
             and not the placeholder. A constant here is the defect this \
             assertion exists for: it announces an empty box while the user is \
             reading a filtered list. Nodes: {:?}",
            listing()
        );

        let category = node(Role::ComboBox, CATEGORY_FILTER_LABEL);
        assert_eq!(
            category.value.as_deref(),
            Some("Puzzle"),
            "the category filter must announce the category it is showing — the \
             same string its own `on_selected` would carry for this index (see \
             [`category_selection`]). Nodes: {:?}",
            listing()
        );

        let sort = node(Role::ComboBox, SORT_FILTER_LABEL);
        assert_eq!(
            sort.value.as_deref(),
            Some("Recently played"),
            "the sort selector must announce the *label* of the entry it is \
             showing (`sort_index` is 1 for \"recent\"), not the first entry and \
             not the key. Nodes: {:?}",
            listing()
        );

        // ---- no two controls share an id ------------------------------------
        //
        // The id is the accessible name (`a11y::stable_id`), so two wrappers built
        // with the same name are one node to assistive technology: the second
        // overwrites the first, the Tab ring reports the id twice, and a user
        // reaches one control where the page draws two. `view/settings.rs` and
        // `view/form.rs` have had this loop since the wrapper landed; this page did
        // not, which made the invariant a property of the pages someone remembered
        // rather than of the widget.
        let mut seen: Vec<cosmic::widget::Id> = Vec::new();
        for id in &ids {
            assert!(
                !seen.contains(id),
                "two controls on this page report the same id {id:?}. The id is \
                 derived from the control's name (`a11y::stable_id`), so this is \
                 two controls sharing a name: a screen reader would see one where \
                 the user sees two, and one of the two would be unreachable by \
                 name. Ids reported: {ids:?}"
            );
            seen.push(id.clone());
        }

        // ---- the id identity, per control ----------------------------------
        //
        // A node and a focus report are two halves of one control, and each half
        // can work while the two name different things — which is what `input`
        // shipped with before the page test in `view/form.rs` caught it. The
        // search box is the sharper case here: its id is the page's own constant
        // (`SEARCH_INPUT_ID`), not one derived from its label, so the two halves
        // agreeing is a property of `a11y::input_with_id` rather than of the
        // name table.
        let in_ring = |id: &iced_accessibility::A11yId| {
            ids.iter()
                .any(|stop| IdEq::eq(&iced_accessibility::A11yId::from(stop.clone()), id))
        };
        for (what, node) in [
            ("search box", &search),
            ("category filter", &category),
            ("sort selector", &sort),
        ] {
            assert!(
                in_ring(&node.id),
                "the {what} is published as {:?}, which no focus report carries: \
                 assistive technology can see this control and cannot reach it. \
                 Reported ids: {ids:?}",
                node.id
            );
        }

        // The four step closures on this page — the two selectors' — are asserted
        // in `tab_then_arrow_steps_every_selector_on_the_real_library_page`
        // below, which needs a Tab walk of its own and so gets its own element.

        // ---- the invariant over the whole page ------------------------------
        //
        // The per-control loop above says the three controls this page wraps are
        // reachable. This says no *control of any of the three kinds* is
        // published that the Tab ring has never heard of — including one this
        // page did not build. `Switch`, `ComboBox` and `TextInput` are the three
        // roles UX-01/UX-02/UX-03 are about and the toolkit publishes none of
        // them, so a node with one of those roles is a control the app drew; the
        // `Paragraph` nodes the drawn labels produce are not controls and are
        // filtered out by `unreachable_controls`, which says why.
        let unreachable = harness::unreachable_controls(&mut element);
        assert!(
            unreachable.is_empty(),
            "these controls are published as nodes whose ids no focus report \
             carries, so a screen reader can read them and a keyboard cannot \
             reach them: {unreachable:?}. Reported ids: {ids:?}"
        );
    }

    /// **Every selector on the real Library page changes hands from the
    /// keyboard, to the value the pointer route would have chosen** — the
    /// pointer-free half of UX-01.
    ///
    /// # Why the step closures need a test of their own
    ///
    /// There are eleven `checked_add_signed` step closures in the view tree and
    /// two tests that read one: `view/settings.rs`'s colour-scheme walk and a
    /// `view/a11y.rs` unit test over its own local fixture. The other nine are
    /// reachable only by driving a built widget — so this one test walks the
    /// whole ring of this page, dispatching a real key event at every stop
    /// through the framework's own `focus_next` and `Widget::update`, and reads
    /// the messages that come out.
    ///
    /// It matters here more than in most places because of how the selector
    /// fails: `Dropdown::operate`'s body is commented out at the pinned rev
    /// (`src/widget/dropdown/widget.rs:336-338`) so the popup cannot be opened
    /// from the keyboard at all, which makes Up and Down the *only* way these
    /// two controls ever change hands without a pointer (see `view/a11y.rs`'s
    /// [`dropdown`](super::a11y::dropdown)). A step that published the wrong
    /// variant, or a stringified index, would be a control that announces a
    /// selection it can never make.
    ///
    /// # The two payloads, which are the two defects this file has already had
    ///
    /// The category filter's payload is a **category name**: `dropdown`'s
    /// `on_selected` takes a `usize`
    /// (`src/widget/dropdown/mod.rs:30`), so a closure that stringifies its
    /// parameter publishes `\"1\"`, which is a valid `String`, so the page
    /// draws and the filter matches nothing. [`category_selection`]'s own doc
    /// records that this is the shape the file originally shipped with, and
    /// that "no assertion in this file could see it, because the closure is
    /// inside a builder and a builder needs a renderer". There is a renderer
    /// here. The sort selector's payload is a **key**
    /// ([`SORT_OPTIONS`]'s first field, `\"name\"`/`\"recent\"`/`\"added\"`),
    /// and the loader folds an unrecognised value back to `\"name\"` on the next
    /// start — so a stringified index there is a selection that silently does
    /// nothing at all.
    ///
    /// Neither expectation is written as a constant: both are derived from the
    /// same mapping functions the pointer path uses, through the option list the
    /// page itself was handed, so what is asserted is that the two routes agree.
    #[test]
    fn tab_then_arrow_steps_every_selector_on_the_real_library_page() {
        use super::a11y::harness;
        use cosmic::iced::keyboard::{Key, key::Named};

        let library = categorised_library();
        let covers = CoverCache::new();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);

        // The options this page will build its category selector from, and the
        // index it is showing — the same readers the call site uses.
        let categories = category_options(&library.categories());
        let shown = category_index(&categories, "Action").expect(
            "the fixture's category must be one of its own options, or the step \
             below would start from `None` and publish nothing",
        );
        assert!(
            shown + 1 < categories.len() && shown >= 1,
            "this fixture is picked so that Down and Up are *both* defined from \
             the entry it shows; from index {shown} of {} one of them is an edge \
             and publishes nothing, which would make this test measure half of \
             what it says it does. Categories: {categories:?}",
            categories.len()
        );

        let mut element = a11y_page_showing(&library, &covers, &runners, "Action");
        let stops = harness::focusables(&mut element).len();
        assert!(
            stops >= 3,
            "the page reports {stops} Tab stops, so this walk cannot reach the \
             three wrapped controls it exists for"
        );
        // One element and one tree for the whole walk: `tab_to` carries the
        // focus forward *in* that tree, which is what makes stop N the N-th
        // control rather than the first one N times.
        let (mut tree, node) = harness::built(&mut element);

        let mut stepped: Vec<Message> = Vec::new();
        for _ in 0..stops {
            harness::tab_to(&mut element, &mut tree, &node);
            for key in [Named::ArrowDown, Named::ArrowUp] {
                let mut messages = Vec::new();
                let _ = harness::dispatch(
                    &mut element,
                    &mut tree,
                    &node,
                    &harness::pressed(Key::Named(key)),
                    &mut messages,
                );
                stepped.extend(messages);
            }
        }

        // ---- the category filter, whose payload is a category --------------
        let expected_categories: Vec<String> = [shown + 1, shown - 1]
            .iter()
            .map(|index| match category_selection(&categories, *index) {
                Message::SetCategoryFilter(name) => name,
                other => panic!("the category step's own mapping is not a filter: {other:?}"),
            })
            .collect();
        let stepped_categories: Vec<String> = stepped
            .iter()
            .filter_map(|message| match message {
                Message::SetCategoryFilter(name) => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            stepped_categories, expected_categories,
            "the category filter must step to the neighbouring *category*, through \
             the same [`category_selection`] the pointer route uses — Down to the \
             next option, Up to the previous one, and nothing at either end. A \
             step publishing `SetCategoryFilter(\"1\")` is the stringified-index \
             defect this module's doc records, and it appears here as a name that \
             is not in the option list. Every message the arrow keys published: \
             {stepped:?}"
        );

        // ---- the sort selector, whose payload is a key ----------------------
        let sort_shown = sort_index("recent").expect("this fixture's mode is in the list");
        assert!(
            sort_shown + 1 < SORT_OPTIONS.len() && sort_shown >= 1,
            "this fixture is picked so both arrows are defined from the sort entry \
             it shows; it is at index {sort_shown} of {}",
            SORT_OPTIONS.len()
        );
        let expected_sorts: Vec<String> = [sort_shown + 1, sort_shown - 1]
            .iter()
            .map(|index| SORT_OPTIONS[*index].0.to_string())
            .collect();
        let stepped_sorts: Vec<String> = stepped
            .iter()
            .filter_map(|message| match message {
                Message::SetSortMode(mode) => Some(mode.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            stepped_sorts, expected_sorts,
            "the sort selector must step to the neighbouring sort *mode* — \
             [`SORT_OPTIONS`]'s key, which is what the model stores and what \
             [`sort_index`] reads back. A stringified index is not in the list, \
             so the loader folds it to `\"name\"` on the next start and the \
             selection silently does nothing. Expected {expected_sorts:?} from \
             index {sort_shown}; every message the arrow keys published: \
             {stepped:?}"
        );

        // ---- and nothing else moved -----------------------------------------
        //
        // The walk pressed an arrow at every stop, including the two buttons and
        // any control that is not a selector. A page where an arrow scrolled the
        // list would still satisfy both assertions above, so this is what says
        // the two messages counted are the whole of what the arrows did.
        assert_eq!(
            stepped.len(),
            expected_categories.len() + expected_sorts.len(),
            "the arrow keys published {} messages, but a selector that steps \
             publishes exactly one and nothing else on this page is steppable: \
             {stepped:?}",
            stepped.len()
        );
    }

    /// A library of `count` named games in a temp directory of its own.
    ///
    /// Named `Windowed N`, and the names are the whole point: every test below
    /// reads *which rows the page built* by looking for those strings in the
    /// traversal, so the numbers in the assertions and the numbers a reader can
    /// reproduce are the same numbers the page was handed.
    fn windowed_library(count: usize) -> Library {
        let root = std::env::temp_dir().join(format!(
            "gh-lib-window-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
        for n in 0..count {
            // **Zero-padded**, because the page sorts by name (`sort_mode:
            // "name"`) and `Library`'s comparison is the string's. `Windowed
            // 10` precedes `Windowed 2` lexicographically, so an unpadded
            // fixture would make "row 44" a name and not a position, and every
            // index assertion below would be reading a different list from the
            // one the window was computed over. Measured: the first version of
            // these tests was unpadded and failed with `left: 10, right: 2`.
            library
                .add(Game::new_named(format!("Windowed {n:03}")))
                .unwrap();
        }
        library
    }

    /// The indices of the `Windowed N` rows a traversal actually built.
    fn built_rows(strings: &[String]) -> Vec<usize> {
        strings
            .iter()
            .filter_map(|s| s.strip_prefix("Windowed "))
            .filter_map(|n| n.parse::<usize>().ok())
            .collect()
    }

    /// A 1200×800 window at `offset`, which is the geometry the audit measured
    /// against (`docs/audit/PERFORMANCE.md`, PERF-01: "a 1200×800 window").
    fn geometry_at(offset: f32) -> ScrollGeometry {
        ScrollGeometry {
            offset,
            viewport_height: 800.0,
            viewport_width: 1200.0,
            content_height: 0.0,
        }
    }

    /// **PERF-03's measurement, as a count of the rows the page built.**
    ///
    /// The audit's finding is that every game in the filtered library was given
    /// a built `Element` on every frame — `list_body` and `grid_body` looped
    /// over all of `shown` (`docs/audit/PERFORMANCE.md`, PERF-03). So the
    /// assertion is on the number of rows a *frame* constructs, read from the
    /// strings the built tree actually draws rather than from the arithmetic
    /// that decided them: a `visible_range` that returned the right window and a
    /// `list_body` that ignored it would pass an assertion made on the range and
    /// fail this one.
    ///
    /// Pre-fix this is 500 names; post-fix it is one window. The two numbers are
    /// three orders of magnitude apart, which is the point — this is not a
    /// marginal change to assert.
    #[test]
    fn the_list_builds_a_window_of_rows_and_not_the_whole_library() {
        let library = windowed_library(500);
        let strings = page_strings_scrolled(&library, "list", geometry_at(0.0));
        let built = built_rows(&strings);

        assert!(
            built.len() < 100,
            "100 frames of the whole library is the defect; this frame built {}",
            built.len()
        );
        assert!(
            built.len() >= 14,
            "and the window has to cover the 800 px on screen (13 rows at \
             61.2 + 6) plus margin, or the page shows blank rows: {}",
            built.len()
        );
        assert_eq!(
            built,
            (0..built.len()).collect::<Vec<_>>(),
            "the window is contiguous and starts at the top"
        );
    }

    /// The window follows the offset: scrolled to the middle, the rows on
    /// screen are built and the first row is not.
    ///
    /// This is the half a "build the first N rows" implementation fails — the
    /// cheapest way to make the count above small is to always build row 0, and
    /// that page is blank from the second screenful down.
    #[test]
    fn the_window_follows_the_offset() {
        let library = windowed_library(500);
        // Row 44 of 67.2 px is at 2956.8–3024.0; the viewport is 800 tall, so
        // rows 44..=56 are on screen.
        let strings = page_strings_scrolled(&library, "list", geometry_at(3000.0));
        let built = built_rows(&strings);

        assert!(
            !built.contains(&0),
            "row 0 is 3000 px above the fold and must not be built: {:?}",
            &built[..built.len().min(4)]
        );
        assert!(
            built.contains(&44),
            "row 44 is the first row the offset puts on screen, and it is not \
             built: {:?}",
            &built[..built.len().min(4)]
        );
        assert!(
            built.contains(&56),
            "row 56 is the last row on screen, and it is not built: built {}..{}",
            built.first().copied().unwrap_or(0),
            built.last().copied().unwrap_or(0)
        );
        assert_eq!(
            built,
            (built[0]..built[0] + built.len()).collect::<Vec<_>>(),
            "contiguous"
        );
    }

    /// And scrolling *up* is covered: the margin above the first visible row is
    /// as large as the one below it.
    ///
    /// A window that only looked forward would show unbuilt rows the moment the
    /// user scrolled up, which is the failure mode a "start at the first visible
    /// row" implementation has and the reason [`WINDOW_VIEWPORTS`] is applied to
    /// `start` as well as to `end`.
    #[test]
    fn the_window_has_margin_above_the_first_visible_row_too() {
        let library = windowed_library(500);
        let strings = page_strings_scrolled(&library, "list", geometry_at(3000.0));
        let built = built_rows(&strings);
        let first_on_screen = 44;

        assert!(
            built[0] < first_on_screen,
            "the window starts at {} and the first row on screen is {first_on_screen}, \
             so scrolling up by even one row draws a row that was never built",
            built[0]
        );
    }

    /// The grid is windowed the same way, and its window is in **lines**: a
    /// 1200 px viewport is 5 columns and 15 lines of 306 px, so a frame builds
    /// 5 cards per line and not 500.
    #[test]
    fn the_grid_builds_a_window_of_lines_and_not_the_whole_library() {
        let library = windowed_library(500);
        let strings = page_strings_scrolled(&library, "grid", geometry_at(0.0));
        let built = built_rows(&strings);

        assert_eq!(grid_columns(1200.0), 5, "1200 / 206 = 5 lines of 5");
        assert!(built.len() < 100, "the grid built {} cards", built.len());
        assert!(
            built.len() >= 15,
            "and it has to cover the 800 px on screen (3 lines of 306) plus \
             margin: {}",
            built.len()
        );
        // Lines are contiguous and start at column 0 — the property the
        // full-width line spacer exists to preserve, because a window that
        // started mid-line would put the first card at the wrong column.
        assert_eq!(built[0], 0);
        for (n, row) in built.iter().enumerate() {
            assert_eq!(*row, n, "the grid's window is the first whole lines");
        }
    }

    /// **The spacers keep the page the height it would have been if every row
    /// were built.**
    ///
    /// The window bounds what is *constructed* and nothing else: the two `Space`
    /// spacers stand in for the rows that are not, so the content the scrollable
    /// scrolls is the same size, the thumb is the same height, and the bottom of
    /// the list is still reachable.
    ///
    /// # Why this is a laid-out height and not arithmetic
    ///
    /// An arithmetic version — "the leading spacer covers `start` rows of
    /// `LIST_ROW_HEIGHT` and the trailing one the rest" — is a restatement of the
    /// builder's own expression and passes whatever the builder does, which is
    /// this project's named defect class. The laid-out height is a different
    /// quantity: it is the sum of what the widgets actually resolved to, so a
    /// spacer of the wrong height, a missing spacer, or a `Space` that collapsed
    /// to zero all move it.
    ///
    /// The three geometries are the same library seen three ways, and **the
    /// third is the control**: a viewport taller than the whole list forces every
    /// row to be built, so if it agrees with the two windowed runs then the
    /// spacers really are substituting for the rows.
    ///
    /// # Measured, and the tolerance is a decision rather than a habit
    ///
    /// With the spacers correct this test measured **33,680.4 px at offset 0 and
    /// at offset 3,000, and 33,680.234 px for the run with every row built** —
    /// agreement to 0.166 px on a 33,680 px page, which is `f32` rounding of two
    /// different summations of the same total (500 children added up by the
    /// layout, against one `Fixed` height). The tolerance below is 1.0 px.
    ///
    /// The two errors it has to catch, with their sizes at this library, so the
    /// tolerance is not a number chosen to make the test pass:
    ///
    /// - a spacer that forgot the gaps inside its own run — the bug this
    ///   function's first version had — which is `(rows - 1) * LIST_SPACING`:
    ///   150.002 px between the two windowed runs here, and 2,994 px at the
    ///   bottom of the list;
    /// - a spacer built from the grid's cell height instead of the row's:
    ///   `500 * (300 - 61.2)` = 119,400 px.
    ///
    /// So the tolerance is 150× under the smallest failure it must see.
    #[test]
    fn the_spacers_keep_the_page_the_height_the_full_list_would_have() {
        let library = windowed_library(500);
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        let covers = CoverCache::new();

        let height = |scroll: ScrollGeometry| {
            page_height(page_element(&library, "list", scroll, &covers, &runners))
        };

        // `f32` rounding, see the doc above: 1.0 px against the 150.002 px the
        // smallest real error here costs.
        const TOLERANCE: f32 = 1.0;

        let top = height(geometry_at(0.0));
        let middle = height(geometry_at(3000.0));
        assert!(
            (top - middle).abs() <= TOLERANCE,
            "the page is one height however far it is scrolled, or the offset \
             the window is computed from is measured against bounds the window \
             moved: {top} at the top, {middle} in the middle"
        );

        let everything = height(ScrollGeometry {
            offset: 0.0,
            viewport_height: 100_000.0,
            viewport_width: 1200.0,
            content_height: 0.0,
        });
        assert!(
            (top - everything).abs() <= TOLERANCE,
            "and it is the height of the whole list: the window replaces rows, \
             it does not remove them — {top} windowed against {everything} with \
             every row built"
        );
    }

    /// `BUG-07`: the toolbar carries the create action **even when the library
    /// already has games**.
    ///
    /// The reference keeps "Add Game" in the page's `actions:` block
    /// (`LibraryPage.qml:18-23`), so it is present whether or not the library is
    /// empty. The port drew the equivalent button only inside the `NoGames`
    /// state, which meant a user with one game had no visible way to add a
    /// second — `Ctrl+N` is invisible in the UI and is swallowed while a text
    /// field has focus.
    ///
    /// Built and traversed rather than asserted on the constant, because the
    /// defect was *where the button is*, not what it says: a test that checked
    /// `ADD_GAME == "Add game"` would have passed on the broken tree.
    #[test]
    fn the_toolbar_offers_add_game_once_the_library_is_not_empty() {
        let root = std::env::temp_dir().join(format!("gh-lib-addbtn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
        library.add(windows_game()).unwrap();

        let strings = page_strings(&library, GRID);
        assert!(
            strings.iter().any(|s| s == ADD_GAME),
            "the toolbar must offer {ADD_GAME:?} with a non-empty library; got {strings:?}"
        );
        // And the empty state still offers its own, longer-wording button —
        // the two are different controls, so this is not a duplicate assertion.
        let empty = page_strings(&Library::new_at(Some(root.join("empty.json")), 0.0), GRID);
        assert!(
            empty.iter().any(|s| s == ADD_FIRST_GAME),
            "the empty state keeps {ADD_FIRST_GAME:?}; got {empty:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// "All" leads the filter, and the library's own categories follow in its
    /// order rather than being re-sorted here.
    #[test]
    fn the_category_filter_puts_all_first_and_changes_nothing_else() {
        let categories = vec!["action".to_string(), "puzzle".to_string()];
        assert_eq!(
            category_options(&categories),
            vec!["All", "action", "puzzle"]
        );
        assert_eq!(category_options(&[]), vec!["All"]);
    }

    /// `"All"` is what the selector shows when nothing is filtered, and a
    /// category the library no longer has selects nothing rather than the
    /// wrong row.
    #[test]
    fn the_category_selector_finds_its_row_or_none() {
        let options = category_options(&["action".to_string()]);
        assert_eq!(category_index(&options, "All"), Some(0));
        assert_eq!(category_index(&options, "action"), Some(1));
        assert_eq!(category_index(&options, "gone"), None);
    }

    /// **A category selection carries the category, not the row it sat at.**
    ///
    /// The selector's callback receives an index (`libcosmic
    /// src/widget/dropdown/mod.rs:30`), and this file's original closure —
    /// `|category| SetCategoryFilter(category.to_string())`, with the parameter
    /// named as though it were the category — stored `"1"` instead of
    /// `"puzzle"`. Every test in this module passed. The page drew, the filter
    /// matched nothing, and the only way to see it was to read the closure and
    /// know the callback's type.
    ///
    /// So this asserts the defect explicitly, in both directions: the message
    /// carries the name, and it is *not* the stringified index. The second is
    /// the one that fails for the original code.
    /// The category a selection carries, read out of the real `Message`.
    ///
    /// `Message` has no `PartialEq` (it holds a `ToastId`, a `Task` and a
    /// `GameForm`), so the payload is destructured rather than compared — which
    /// is stricter, not looser: a selection that produced the *wrong variant*
    /// panics here instead of failing an equality it might have passed.
    fn category_of(message: Message) -> String {
        match message {
            Message::SetCategoryFilter(value) => value,
            other => panic!("expected SetCategoryFilter, got {other:?}"),
        }
    }

    #[test]
    fn a_selection_carries_the_name_and_not_the_index() {
        let options = category_options(&["action".to_string(), "puzzle".to_string()]);

        assert_eq!(
            category_of(category_selection(&options, 2)),
            "puzzle",
            "index 2 is the third entry, `puzzle`"
        );
        assert_eq!(
            category_of(category_selection(&options, 0)),
            ALL_CATEGORIES,
            "index 0 is the `All` sentinel"
        );

        // The defect, spelled out.
        assert_ne!(
            category_of(category_selection(&options, 1)),
            "1",
            "index 1 is `action`, and `\"1\"` is a filter that matches nothing"
        );

        // An index no option provides cannot invent a category: it folds to
        // `All` rather than to a string that matches nothing.
        assert_eq!(
            category_of(category_selection(&options, 99)),
            ALL_CATEGORIES
        );
    }

    /// The sort selector reads its row out of the same list it offers.
    #[test]
    fn the_sort_selector_finds_its_row_or_none() {
        assert_eq!(sort_index("name"), Some(0));
        assert_eq!(sort_index("recent"), Some(1));
        assert_eq!(sort_index("added"), Some(2));
        assert_eq!(sort_index("nonsense"), None);
        assert_eq!(sort_labels().len(), SORT_OPTIONS.len());
        assert_eq!(sort_labels()[1], "Recently played");
    }

    /// The toggle names the mode that is *not* showing, both ways.
    #[test]
    fn the_view_toggle_switches_to_the_other_mode() {
        assert_eq!(toggled_mode(GRID), LIST);
        assert_eq!(toggled_mode(LIST), GRID);
    }

    /// Clearing the filters is one message, not two.
    ///
    /// The reference's button empties the search box *and* resets the category
    /// (`LibraryPage.qml:118-121`). One message is what makes those two writes
    /// happen together, so a half-cleared filter cannot be drawn.
    #[test]
    fn clearing_the_filters_is_a_single_message() {
        assert!(matches!(clear_filters(), Message::ClearFilters));
    }
}
