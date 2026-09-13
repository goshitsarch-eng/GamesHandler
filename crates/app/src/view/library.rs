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
    Column, Row, button, container, context_menu, menu, scrollable, text, text_input,
};
use gamehandler_core::models::{Game, Library, format_last_played};
use gamehandler_core::runners::RunnerManager;

use crate::Message;
use crate::state::PrefixTool;

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
// The page
// ---------------------------------------------------------------------------

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
    /// called from a widget, because [`RunnerManager::label`] is uncached and
    /// walks the filesystem. [`view`] runs every frame, so taking the manager
    /// here reintroduces exactly that — once per shown game per frame.
    ///
    /// It is taken anyway, and the reason is that the alternative is worse: the
    /// label has to come from somewhere, and the only other source today is the
    /// empty string, which is `#30` — every Windows game rendered no runner at
    /// all. Between a page that says nothing true and a page that walks a
    /// directory, the walk is the lesser defect and the one that is visible in
    /// a profile. The fix is a `State` field holding the resolved rows, updated
    /// where the library is re-read (install and uninstall both already are);
    /// it is not in this slice, and it is named here rather than left for a
    /// reader to discover.
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
}

/// The page.
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
            .push(
                text_input(SEARCH_PLACEHOLDER, page.search.to_string())
                    .id(SEARCH_INPUT_ID.into())
                    .on_input(Message::SetSearchText)
                    .width(Length::Fill),
            )
            .push(cosmic::widget::dropdown(
                categories.clone(),
                category_index(&categories, page.category),
                // The selector's index, mapped to the category it stands for —
                // never the index itself. See [`category_selection`].
                {
                    let options = categories.clone();
                    move |index| category_selection(&options, index)
                },
            ))
            .push(cosmic::widget::dropdown(
                sort_labels(),
                sort_index(page.sort_mode),
                |index| Message::SetSortMode(SORT_OPTIONS[index].0.to_string()),
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
                list_body(&shown, page.runners, page.now)
            } else {
                grid_body(&shown, page.runners)
            });
        }
    }

    container(scrollable(body)).padding(18).into()
}

/// The list: one row per game, each with its last-played label.
///
/// `now` is the page's single instant, passed down rather than read here; see
/// [`LibraryPage::now`]. The last-played half is why this is not
/// [`grid_body`]'s twin: `LibraryPage.qml` draws the two delegates with
/// different text, and only the row carries the timestamp (`:269` against the
/// card's `:192`).
fn list_body<'a>(games: &[&'a Game], runners: &'a RunnerManager, now: f64) -> Element<'a, Message> {
    let mut body = Column::new().spacing(6).width(Length::Fill);
    for game in games {
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
        body = body.push(context_menu(
            widgets::row(game, &labels, Message::LaunchGame(game.id.clone())),
            Some(game_menu_trees(game)),
        ));
    }
    body.into()
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

/// The grid: the cards, wrapped.
///
/// `Row::wrap` rather than a fixed number of columns: the reference's grid is a
/// `GridView` whose `cellWidth` is 200 (`LibraryPage.qml:139`), so the column
/// count is whatever fits — which is what wrapping gives and what a hard-coded
/// count would get wrong on every window width but one.
fn grid_body<'a>(games: &[&'a Game], runners: &'a RunnerManager) -> Element<'a, Message> {
    // Children go on the `Row` and the whole row wraps: `Wrapping` itself has
    // no `push`, only the spacing and alignment of the wrapped lines.
    let mut row = Row::new().spacing(6);
    for game in games {
        let label = widgets::resolved_runner_label(runners, game);
        // Built here rather than in the builder; see `list_body`.
        row = row.push(context_menu(
            widgets::card(game, &label, Message::LaunchGame(game.id.clone())),
            Some(game_menu_trees(game)),
        ));
    }
    row.wrap().into()
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
    fn page_strings(library: &Library, view_mode: &str) -> Vec<String> {
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        drawn_strings(view(LibraryPage {
            library,
            search: "",
            category: ALL_CATEGORIES,
            sort_mode: "name",
            view_mode,
            runners: &runners,
            now: 0.0,
        }))
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
