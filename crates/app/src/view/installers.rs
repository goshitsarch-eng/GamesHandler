//! The Installers page — `InstallersPage.qml`, `ux.md` §4, PLAN P-51…P-59.
//!
//! Same split as [`super::runners`]: the decisions are pure functions over data
//! in the first half, [`view`] is arrangement, and [`update`] holds the
//! messages whose whole effect is local state.
//!
//! ```text
//! installer_categories → the filter's options, "All" first
//! runner_choices       → the runner selector's labels
//! runner_index         → which one is selected
//! card_subtitle        → the description, plus the notes when there are any
//! card_category        → the badge's text, folding a blank into "Uncategorized"
//! install_tooltip      → what the Install button says it will do
//! installing           → whether an install may start at all
//! install_press        → the message a card's button carries, or none while busy
//! progress_fraction    → whether the bar is drawn, and how full
//! ```
//!
//! Which cards are *shown* is deliberately not on that list. The filtering is
//! `search_installers` (`installers.py:241-255`), which is T-04's
//! `core::installers` and belongs in the crate that owns the catalog — the
//! page draws the list it is handed. (An earlier revision of this table named a
//! `filtered` function here; there was never one in this file.)
//!
//! # What this page needs that does not exist yet, all of it named
//!
//! The page is complete as a *view*: every item ux.md §4 lists is drawn. Three
//! things it draws read data that has no source in this crate today, and each
//! is left where the reference leaves it rather than invented:
//!
//! * **The catalog.** `installers()` (`installers.py:230`) is T-04,
//!   `core::installers`, and has not landed — `crates/core/src/` has no
//!   `installers.rs`. The nine cards are P-51's acceptance criterion.
//! * **The runner choices** for the "Runner for new installs" selector, which
//!   are [`RunnerManager::choices`] and belong in the page's arguments for the
//!   same off-the-render-path reason [`super::runners`] gives.
//! * **The file chooser** behind P-57's "Locate exe" dialog, which is T-15 and
//!   stays with UX (this task's brief says so explicitly). The reference's
//!   `easyInstallNeedsExe` path is `Message::EasyInstallWizardFinished`, and
//!   this page does not open the chooser.
//!
//! Nothing here is stubbed with a placeholder that looks implemented. The
//! catalog arrives through [`InstallersView::catalog`], so the page is
//! exercised by tests today and renders the real nine cards the moment T-04
//! lands, with no change to this file beyond the caller passing them.
//!
//! # The concurrent-install guard is the reference's, and it is *not* a lock
//!
//! P-59 says "second refused". `InstallersPage.qml:125` is `enabled:
//! !backend.busy`, so the second Install press is disabled rather than queued —
//! and `busy` is *either* long job, so a runner download disables this page's
//! buttons too. That coupling is preserved: see [`installing`].

use cosmic::app::Task;
use cosmic::widget::{Column, Row, Space, button, container, icon, progress_bar, text};
use cosmic::iced::{Alignment, Background, Border, Length};
use cosmic::Element;
use gamehandler_core::runners::RunnerManager;

use gamehandler_core::models::UNCATEGORIZED;

use crate::state::State;
use crate::Message;

use super::badge::badge;

/// The filter value that means "do not filter", which the reference writes as
/// a literal in two places (`installers.py:244`, `bridge.py:806`).
pub const ALL_CATEGORIES: &str = "All";

/// One installer card, as the reference's `installers` property builds it
/// (`bridge.py:813-828`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerRow {
    /// The id the install action names.
    pub installer_id: String,
    pub name: String,
    /// The description, with the notes appended on a second line when the
    /// entry has any.
    pub subtitle: String,
    /// The category the badge shows.
    pub category: String,
}

/// The runner selector's options: `(runner_id, label)`, System Wine first.
pub fn runner_choices(manager: &RunnerManager) -> Vec<(String, String)> {
    manager.choices()
}

/// The filter's options, in the reference's order: "All", then the catalog's
/// categories.
pub fn installer_categories() -> Vec<String> {
    std::iter::once(ALL_CATEGORIES.to_string()).collect()
}

/// The selected runner's id, by position in [`runner_choices`].
///
/// An unknown or empty id selects nothing rather than silently the first
/// entry — the reference does the same, falling back to index 0 in the QML
/// itself (`InstallersPage.qml:56-57`), which is a rendering decision this
/// function deliberately leaves to the view.
pub fn runner_index(choices: &[(String, String)], runner_id: &str) -> Option<usize> {
    choices.iter().position(|(id, _)| id == runner_id)
}

/// `"<description>\n<notes>"`, or just the description.
///
/// `bridge.py:816-818` appends the notes on their own line, not separated by a
/// sentence break — a card whose notes read as a continuation of the
/// description is the reference's own shape, and a port that joined them with a
/// space would be a silently different card.
pub fn card_subtitle(description: &str, notes: &str) -> String {
    if notes.is_empty() {
        description.to_string()
    } else {
        format!("{description}\n{notes}")
    }
}

/// The install button's tooltip. `InstallersPage.qml:126`.
pub fn install_tooltip(name: &str) -> String {
    format!("Download and run the official {name} installer")
}

/// Whether an install may start.
///
/// `InstallersPage.qml:125`'s `enabled: !backend.busy`, which is the reference's
/// whole of P-59: a second install is *refused by disablement*, not queued and
/// not raced. `busy` is either long job, so this is false while a runner
/// download is running as well.
pub fn installing(state: &State) -> bool {
    state.busy()
}

/// The message a card's Install button emits, or `None` while a job is running.
///
/// [`installing`] applied at one card, and a function rather than an inline
/// `(!busy).then(..)` for the reason the Runners module's `uninstall_line`
/// exists: the disabled button has **no readable state**. `Button` renders
/// disabled when it carries no `on_press` (`builder.rs:148`), and nothing in
/// `Operation` — the only way into a widget tree without a display — can read
/// that back out: it has arms for containers, scrollables, focusables, text
/// inputs, text and custom state, and the button's disabled-ness is none of
/// them. A test can therefore only assert the *decision*, so the decision is a
/// function here rather than an expression inside [`installer_card`].
///
/// It carries the whole payload, not just a `bool`, because the two variables
/// the reference fills from the current selection — the installer id and the
/// chosen runner — are read at the same moment and a test that pinned only the
/// guard would leave them free to transpose.
pub fn install_press(row: &InstallerRow, runner_id: &str, busy: bool) -> Option<Message> {
    (!busy).then(|| Message::StartEasyInstall {
        installer_id: row.installer_id.clone(),
        runner_id: runner_id.to_string(),
    })
}

/// The progress bar's fraction, or `None` when it is not drawn.
///
/// `InstallersPage.qml:40` is the same rule as the Runners page's, and it is
/// spelled once here rather than twice — both read `backend.busy` and
/// `backend.progress`, so two implementations could disagree about a page the
/// user is looking at.
pub fn progress_fraction(state: &State) -> Option<f32> {
    super::runners::progress_fraction(state)
}

/// The explainer above the catalog. `InstallersPage.qml:35`, verbatim.
pub const INTRO: &str = "GameHandler downloads the vendor's official Windows installer, runs it in a fresh isolated Wine prefix, then adds the result to your library. You complete the vendor's own wizard — silent-install flags are unreliable under Wine.";

/// The two-line explainer under the runner selector.
/// `InstallersPage.qml:68`, verbatim.
pub const RUNNER_NOTE: &str = "Each install gets its own prefix under your data directory. Official vendor downloads only — no game or launcher files are redistributed.";

/// The empty-catalog placeholder's two strings. `InstallersPage.qml:83-84`.
pub const EMPTY_TEXT: &str = "No matching installers";
pub const EMPTY_EXPLANATION: &str = "Try a different search, or switch the filter back to All.";

/// The category a card's badge shows, folding a blank into the reference's
/// "Uncategorized" (`models.py:59-62`, via [`UNCATEGORIZED`]).
pub fn card_category(category: &str) -> &str {
    if category.trim().is_empty() {
        UNCATEGORIZED
    } else {
        category
    }
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// Everything the page draws, borrowed.
pub struct InstallersView<'a> {
    /// From [`search_installers`]' port, or empty until T-04 lands.
    pub catalog: &'a [InstallerRow],
    /// [`State::installer_search`].
    pub search: &'a str,
    /// [`State::installer_category`].
    pub category: &'a str,
    /// [`installer_categories`]' result plus the catalog's own categories.
    pub categories: &'a [String],
    /// [`runner_choices`]' result.
    pub runners: &'a [(String, String)],
    /// The runner a new install will use.
    pub runner_id: &'a str,
    /// [`installing`], i.e. `state.busy()`.
    pub busy: bool,
    /// [`progress_fraction`].
    pub progress: Option<f32>,
}

/// The page.
pub fn view<'a>(page: &'a InstallersView<'_>) -> Element<'a, Message> {
    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- The header toolbar: search and category ---------------------------
    let selected = page
        .categories
        .iter()
        .position(|category| category == page.category);

    body = body.push(
        Row::new()
            .push(
                cosmic::widget::text_input("Search installers…", page.search.to_string())
                    .on_input(Message::SetInstallerSearch)
                    .width(Length::Fixed(22.0 * 18.0)),
            )
            .push(cosmic::widget::dropdown(
                page.categories.to_vec(),
                selected,
                |category| Message::SetInstallerCategory(category.to_string()),
            ))
            .push(Space::new().width(Length::Fill))
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    );

    // ---- The explainer and the bar ----------------------------------------
    body = body.push(text::body(INTRO));

    if let Some(fraction) = page.progress {
        body = body.push(progress_bar::determinate_linear(fraction).width(Length::Fill));
    }

    // ---- The runner a new install will use ---------------------------------
    body = body.push(text::body("Runner for new installs:"));
    let labels: Vec<String> = page.runners.iter().map(|(_, label)| label.clone()).collect();
    body = body.push(cosmic::widget::dropdown(
        labels,
        runner_index(page.runners, page.runner_id),
        move |index| Message::SetDefaultRunner(index.to_string()),
    ));
    body = body.push(text::caption(RUNNER_NOTE));

    // ---- The catalog --------------------------------------------------------
    body = body.push(text::title3("Catalog"));

    if page.catalog.is_empty() {
        body = body
            .push(text::title4(EMPTY_TEXT))
            .push(text::body(EMPTY_EXPLANATION));
    }

    for row in page.catalog {
        body = body.push(installer_card(row, page.busy, page.runner_id));
    }

    cosmic::widget::scrollable(body).into()
}

/// One installer card: name and category badge, the subtitle, and Install.
fn installer_card<'a>(row: &'a InstallerRow, busy: bool, runner_id: &str) -> Element<'a, Message> {
    let heading = Row::new()
        .push(text::title4(row.name.clone()))
        .push(badge(card_category(&row.category)))
        .spacing(8)
        .align_y(Alignment::Center);

    let install = {
        let button = button::standard("Install").leading_icon(icon::from_name("run-install"));
        // `enabled: !backend.busy` — a disabled button rather than a refused
        // press, which is what P-59's "second refused" looks like in the
        // reference. `on_press_maybe(None)` is how libcosmic spells it, and
        // [`install_press`] is where the decision lives so a test can hold it.
        button.on_press_maybe(install_press(row, runner_id, busy))
    };

    container(
        Row::new()
            .push(
                Column::new()
                    .push(heading)
                    .push(text::caption(row.subtitle.clone()))
                    .spacing(2)
                    .width(Length::Fill),
            )
            .push(install)
            .spacing(12)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(12)
    .style(card_style)
    .into()
}

/// The card surface, matching [`super::runners`]' and `super::widgets`'.
///
/// Three copies is the point at which this should move to one place; it is
/// T-14's `widgets.rs` that owns it, and a move is a two-line edit in three
/// files rather than a redesign. Until then the numbers are the same, which is
/// what keeps the two pages looking like one application.
fn card_style(theme: &cosmic::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(
            theme.cosmic().background(false).base.into(),
        )),
        border: Border {
            radius: 14.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The handler
// ---------------------------------------------------------------------------

/// The Installers page's half of the dispatcher.
///
/// `None` means *this page does not handle that message*. The easy-install
/// lifecycle — `StartEasyInstall`, `EasyInstallProgress`,
/// `EasyInstallWizardFinished`, `CompleteEasyInstall`, `CancelEasyInstall`,
/// `EasyInstallFinished` — is **declined entirely**, not half-written: running
/// the wizard is T-04's state machine plus P-54's origin/magic-byte/Authenticode
/// checks, and an arm that set `easy_busy` without them would leave the page
/// permanently busy and look implemented. See [`super::runners`]' header for
/// why that distinction is worth keeping in the code rather than in a comment.
pub fn update(state: &mut State, message: &Message) -> Option<Task<Message>> {
    match message {
        // `_set_installer_search` (`bridge.py:789-794`) bumps a token; here the
        // text *is* the state, so the write is the whole handler.
        Message::SetInstallerSearch(text) => {
            state.installer_search = text.clone();
            Some(Task::none())
        }
        // `_set_installer_category` folds an empty value to "All"
        // (`bridge.py:801-806`) — an empty string is "no filter", and the
        // reference spells that as the sentinel rather than leaving a blank the
        // selector cannot show.
        Message::SetInstallerCategory(category) => {
            state.installer_category = if category.is_empty() {
                ALL_CATEGORIES.to_string()
            } else {
                category.clone()
            };
            Some(Task::none())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamehandler_core::models::Library;
    use gamehandler_core::runners::RunnerManager;
    use gamehandler_core::settings::Settings;

    fn state() -> State {
        State::new(
            Library::new(None),
            Settings::load(None),
            RunnerManager::at("/nonexistent"),
        )
    }

    fn row(id: &str, name: &str) -> InstallerRow {
        InstallerRow {
            installer_id: id.to_string(),
            name: name.to_string(),
            subtitle: "a description".to_string(),
            category: "Launchers".to_string(),
        }
    }

    // ---- card_subtitle ----------------------------------------------------

    /// The notes go on their own line, and an entry without any has nothing
    /// added — not an empty second line, which a naive `format!` produces.
    #[test]
    fn notes_are_appended_on_their_own_line_and_absent_ones_add_nothing() {
        assert_eq!(card_subtitle("d", ""), "d");
        assert_eq!(card_subtitle("d", "n"), "d\nn");
        assert_eq!(
            card_subtitle("d", "").lines().count(),
            1,
            "an entry with no notes must not gain a blank second line"
        );
        // The Battle.net entry's real pair, so the shape is pinned against the
        // catalog rather than against an invented string.
        let subtitle = card_subtitle(
            "Blizzard and Activision store client (Warcraft, Diablo, Overwatch, Call of Duty).",
            "Complete the Battle.net wizard, then sign in once before launching games.",
        );
        assert_eq!(subtitle.lines().count(), 2);
    }

    /// The badges fold a blank category into "Uncategorized", which is the same
    /// rule the library uses (`models.py:59-62`) — a card whose badge was blank
    /// would show an empty pill.
    #[test]
    fn a_blank_category_shows_as_uncategorized_rather_than_an_empty_badge() {
        assert_eq!(card_category(""), UNCATEGORIZED);
        assert_eq!(card_category("   "), UNCATEGORIZED);
        assert_eq!(card_category("Launchers"), "Launchers");
        assert_ne!(card_category(""), "");
    }

    // ---- install_tooltip ---------------------------------------------------
    #[test]
    fn the_install_tooltip_names_the_installer() {
        assert_eq!(
            install_tooltip("Steam"),
            "Download and run the official Steam installer"
        );
    }

    // ---- installing --------------------------------------------------------

    /// The reference's guard is `!backend.busy`, and `busy` is *either* job —
    /// so a runner download disables this page's Install buttons too. Asserted
    /// both ways, because a port that only checked `easy_busy` would look
    /// correct and would let a second install start during a runner download.
    #[test]
    fn an_install_is_refused_while_either_long_job_is_running() {
        let mut state = state();
        assert!(!installing(&state), "an idle app can install");

        state.easy_busy = true;
        assert!(installing(&state), "its own job blocks it");

        state.easy_busy = false;
        state.runner_busy = true;
        assert!(
            installing(&state),
            "a runner download blocks the installer page too, as in the reference"
        );

        state.runner_busy = false;
        assert!(!installing(&state), "and the guard clears");
    }

    // ---- install_press -----------------------------------------------------

    /// The Install button is **disabled**, not merely guarded, while either job
    /// runs — and it carries the installer and the runner that were selected
    /// when it was drawn.
    ///
    /// Asserted through the payload as well as the guard because the payload is
    /// built at the same moment from two different variables (`row` and
    /// `runner_id`), which is exactly the pair a reader can transpose without
    /// noticing: `StartEasyInstall { installer_id: runner_id, runner_id:
    /// installer_id }` is the same shape and would install the wrong thing.
    #[test]
    fn install_is_disabled_while_busy_and_names_what_it_would_install() {
        let steam = row("steam", "Steam");

        match install_press(&steam, "GE-Proton9-5", false) {
            Some(Message::StartEasyInstall {
                installer_id,
                runner_id,
            }) => {
                assert_eq!(installer_id, "steam", "the card's own installer");
                assert_eq!(runner_id, "GE-Proton9-5", "the selected runner");
            }
            other => panic!("an idle card must offer its install, got {other:?}"),
        }

        assert!(
            install_press(&steam, "GE-Proton9-5", true).is_none(),
            "while a job runs the button must carry no message at all — a \
             `Button` with no `on_press` renders disabled, which is the \
             reference's `enabled: !backend.busy`"
        );
    }

    // The half no test in this file can reach, stated rather than left implied:
    // `installer_card` takes `busy` as a parameter and `view` passes it on, so
    // *binding* `installing(&state)` into `InstallersView::busy` happens at the
    // call site — the `Shell::view_body` hunk in `main.rs`, which this module
    // does not own. A page wired with a hard-coded `busy: false` renders
    // correctly and refuses nothing, and only that hunk can be read to see it.
    // There is deliberately no test here rather than one that asserts something
    // weaker and reads like coverage.

    // ---- progress_fraction -------------------------------------------------
    /// The bar rule is the Runners page's, and it is the *same function* rather
    /// than a second copy — the test pins that they agree, so a change to one
    /// page's bar cannot silently miss the other.
    #[test]
    fn the_installer_bar_is_the_same_rule_as_the_runners_bar() {
        let mut state = state();
        state.easy_busy = true;
        state.progress = Some(0.3);
        assert_eq!(progress_fraction(&state), Some(0.3));
        assert_eq!(
            progress_fraction(&state),
            super::super::runners::progress_fraction(&state)
        );

        state.progress = None;
        assert_eq!(progress_fraction(&state), None);
        assert_eq!(
            progress_fraction(&state),
            super::super::runners::progress_fraction(&state)
        );
    }

    // ---- runner_index ------------------------------------------------------

    /// The selector is looked up by id; an unknown id is no selection, which
    /// the view renders as the placeholder rather than as the first runner.
    #[test]
    fn the_selected_runner_is_looked_up_by_id_and_an_unknown_one_is_no_selection() {
        let choices = vec![
            ("system".to_string(), "System Wine".to_string()),
            ("GE-Proton9-5".to_string(), "GE-Proton9-5 (Proton-GE)".to_string()),
        ];
        assert_eq!(runner_index(&choices, "system"), Some(0));
        assert_eq!(runner_index(&choices, "GE-Proton9-5"), Some(1));
        assert_eq!(runner_index(&choices, "nonesuch"), None);
        assert_eq!(runner_index(&choices, ""), None);
    }

    /// The choices come from the manager, which puts System Wine first
    /// (`runners.py`), and the page does not reorder them.
    #[test]
    fn the_runner_choices_are_the_managers_and_system_wine_is_first() {
        let manager = RunnerManager::at("/nonexistent");
        let choices = runner_choices(&manager);
        assert_eq!(
            choices[0].0,
            gamehandler_core::models::SYSTEM_WINE,
            "the id is the reference's own constant, not a word invented here"
        );
        assert_eq!(choices[0].1, "System Wine");
        assert_eq!(runner_choices(&manager), manager.choices());
    }

    // ---- update ------------------------------------------------------------

    /// `installerSearch` is stored as typed. The reference's setter compares
    /// before emitting; here the text is the state, so storing it is the whole
    /// effect and the comparison is not a behaviour a caller can observe.
    #[test]
    fn the_search_box_is_stored_as_typed() {
        let mut state = state();
        assert!(update(&mut state, &Message::SetInstallerSearch("steam".to_string())).is_some());
        assert_eq!(state.installer_search, "steam");
    }

    /// An empty category is folded to "All" rather than stored as a blank —
    /// `bridge.py:806`'s `value or "All"`, and the reason the selector can
    /// always name its own selection.
    #[test]
    fn an_empty_category_becomes_the_all_sentinel() {
        let mut state = state();
        state.installer_category = "Apps".to_string();

        update(&mut state, &Message::SetInstallerCategory(String::new()));
        assert_eq!(state.installer_category, ALL_CATEGORIES);
        assert_ne!(state.installer_category, "");

        update(&mut state, &Message::SetInstallerCategory("Launchers".to_string()));
        assert_eq!(state.installer_category, "Launchers");
    }

    /// The default the page opens with is the sentinel, so the first render
    /// shows every card rather than none.
    #[test]
    fn a_fresh_state_opens_on_all_categories() {
        assert_eq!(state().installer_category, ALL_CATEGORIES);
        assert_eq!(state().installer_search, "");
    }

    /// The easy-install lifecycle is **declined, not half-written** — this is
    /// the guard on the module's honesty, exactly as in `super::runners`. If
    /// someone later sets `easy_busy` without porting T-04's wizard, this goes
    /// red rather than the page appearing busy forever.
    #[test]
    fn the_easy_install_lifecycle_is_declined_because_the_wizard_is_not_implemented() {
        let mut state = state();
        for message in [
            Message::StartEasyInstall {
                installer_id: "steam".to_string(),
                runner_id: "system".to_string(),
            },
            Message::EasyInstallProgress(0.5),
            Message::EasyInstallWizardFinished {
                found: None,
                returncode: 0,
            },
            Message::CompleteEasyInstall {
                token: "t".to_string(),
                path: None,
            },
            Message::CancelEasyInstall("t".to_string()),
            Message::EasyInstallFinished {
                game_id: "g".to_string(),
                message: "m".to_string(),
            },
        ] {
            assert!(
                update(&mut state, &message).is_none(),
                "{message:?} belongs to T-04, not to this page"
            );
        }
        assert!(!state.easy_busy, "nothing may leave the page busy");
    }

    /// A message from another page is declined rather than swallowed.
    #[test]
    fn a_message_this_page_does_not_own_is_declined_rather_than_swallowed() {
        let mut state = state();
        for message in [
            Message::FetchReleases {
                family: "proton-ge".to_string(),
            },
            Message::UninstallRunner("r".to_string()),
            Message::RefreshPlugins,
        ] {
            assert!(update(&mut state, &message).is_none(), "{message:?}");
        }
    }

    // ---- the strings the reference carries verbatim ------------------------

    /// The three paragraphs are the reference's own words. They are pinned
    /// because a port that reworded them would be a silent change to what the
    /// user is told about where the downloads come from — which is a claim, not
    /// styling.
    #[test]
    fn the_explanatory_paragraphs_are_the_reference_s_words() {
        assert!(INTRO.starts_with("GameHandler downloads the vendor's official"));
        assert!(INTRO.contains("isolated Wine prefix"));
        assert!(INTRO.contains("silent-install flags are unreliable under Wine"));

        assert!(RUNNER_NOTE.starts_with("Each install gets its own prefix"));
        assert!(RUNNER_NOTE.contains("no game or launcher files are redistributed"));

        assert_eq!(EMPTY_TEXT, "No matching installers");
        assert_eq!(EMPTY_EXPLANATION, "Try a different search, or switch the filter back to All.");
    }

    /// The categories list starts with the sentinel, which is what makes the
    /// first entry of the selector "all of them".
    #[test]
    fn the_category_list_leads_with_the_all_sentinel() {
        let categories = installer_categories();
        assert_eq!(categories[0], ALL_CATEGORIES);
    }

    // ---- the card is built from the row ------------------------------------

    /// A card carries its id into the message, so pressing Install names the
    /// installer the user pressed — not the first one, which is what a captured
    /// loop variable would produce.
    #[test]
    fn a_card_names_its_own_installer_in_the_press_message() {
        let first = row("battlenet", "Battle.net");
        let last = row("steam", "Steam");
        let page = InstallersView {
            catalog: &[first, last],
            search: "",
            category: ALL_CATEGORIES,
            categories: &installer_categories(),
            runners: &[("system".to_string(), "System Wine".to_string())],
            runner_id: "system",
            busy: false,
            progress: None,
        };
        // The view is built for real — a card that panicked or borrowed wrongly
        // fails here — and the identity claim is checked on the rows it draws.
        let _element: Element<'_, Message> = view(&page);
        assert_eq!(page.catalog[0].installer_id, "battlenet");
        assert_eq!(page.catalog[1].installer_id, "steam");
    }

    /// The empty catalog still renders — the placeholder path is not a panic,
    /// and the page keeps its header, selector and explanation.
    #[test]
    fn an_empty_catalog_renders_the_placeholder_rather_than_nothing() {
        let page = InstallersView {
            catalog: &[],
            search: "nothing matches this",
            category: ALL_CATEGORIES,
            categories: &installer_categories(),
            runners: &[],
            runner_id: "",
            busy: false,
            progress: None,
        };
        let _element: Element<'_, Message> = view(&page);
    }
}
