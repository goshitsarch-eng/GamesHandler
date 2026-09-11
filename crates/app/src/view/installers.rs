//! The Installers page — `InstallersPage.qml`, `ux.md` §4, PLAN P-51…P-59.
//!
//! Same split as [`super::runners`]: the decisions are pure functions over data
//! in the first half, [`view`] is arrangement, and [`update`] holds the
//! messages whose whole effect is local state.
//!
//! ```text
//! installer_categories → the filter's options, "All" first
//! selected_category    → which of those options the filter is showing
//! installer_rows       → the catalog, filtered, as the cards the page draws
//! runner_choices       → the runner selector's labels
//! runner_index         → which one is selected
//! seeded_install_runner → what a fresh page starts out selecting
//! install_runner_selection → the per-install runner the selector names
//! category_selection   → the category the filter's selector names
//! card_subtitle        → the description, plus the notes when there are any
//! card_category        → the badge's text, folding a blank into "Uncategorized"
//! install_tooltip      → what the Install button says it will do
//! installing           → whether an install may start at all
//! install_press        → one card's install decision: the message, or none while busy
//! progress_fraction    → whether the bar is drawn, and how full
//! ```
//!
//! Which cards are *shown* is [`installer_rows`], and the *rule* is not this
//! file's: matching is `search_installers` (`installers.py:241-255`), in
//! `core::installers`, the crate that owns the catalog. [`installer_rows`]
//! calls it and reshapes the result into the cards the page draws
//! (`bridge.py:813-828`); it never re-implements the match, so there is still
//! exactly one place where a query decides what matches. (An earlier revision of
//! this table named a `filtered` function here; there was never one in this
//! file, and the one added later is named for the catalog rather than for the
//! act of filtering.)
//!
//! # What this page still needs, and what it no longer does
//!
//! The page is complete as a *view*, and since T-04/T-34 the catalog is real
//! data rather than a promise: [`installer_rows`] builds it from
//! `core::installers`, so P-51 (the nine cards) and P-52 (the search box and the
//! Launchers/Apps filter) are met here and tested.
//!
//! **T-38 wired it**: `Page::Installers` is no longer routed to `pending_page`,
//! the page is the body `Shell::view_body` draws for it, and the install flow
//! behind the Install button lives in `main.rs` (`start_easy_install` and the
//! five replies it answers). `PENDING_PAGES` is empty and
//! `crates/app/tests/pending_pages.rs`'s `PINNED_PENDING` with it. That paragraph
//! used to say the page "is the last entry in `PENDING_PAGES` (`main.rs`) and the
//! only entry in `PINNED_PENDING`", and it stayed true for as long as it took to
//! land the flow — a sentence about the code that outlives the line it describes
//! is the defect this file's own header warns about, so it moved with the entry
//! rather than after it.
//!
//! What is still missing is named rather than left to look finished:
//!
//! * **The runner choices** for the "Runner for new installs" selector are
//!   [`runner_choices`] over [`RunnerManager::choices`], and they reach the page
//!   through `State::installer_runners`, primed by `State::refresh_installers`
//!   — the same off-the-render-path reason [`super::runners`] gives for its own
//!   two bundles. The **selection** is [`State::installer_runner`], and it is
//!   the install's rather than the app's (P-53's first clause, D-55): see
//!   [`seeded_install_runner`] and [`install_runner_selection`].
//! * **The file chooser** behind P-57's "Locate exe" dialog, which is T-15 and
//!   stays with UX (this task's brief says so explicitly). The reference's
//!   `easyInstallNeedsExe` path is `Message::EasyInstallWizardFinished`, and
//!   this page does not open the chooser. As of T-38 the shell *stores* the
//!   pending install and toasts the reference's sentence, so what is missing is
//!   the picker and the `CompleteEasyInstall` token it would feed — not the
//!   state the token names.
//!
//! Nothing here is stubbed with a placeholder that looks implemented: the
//! catalog arrives through [`InstallersView::catalog`], which is
//! [`installer_rows`]' result, so the cards are exercised by tests today and the
//! page renders the real nine.
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
use gamehandler_core::installers::{Installer, INSTALLER_CATEGORIES, search_installers};
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
/// categories — `["All", *INSTALLER_CATEGORIES]` (`bridge.py:787`).
///
/// # #74: this returned the sentinel alone, and the list was the defect
///
/// Until T-12 the body was `["All"]`, with a comment naming the missing catalog
/// as the reason. The catalog then landed (`core::installers`, `60ee689`) and
/// the list did not — the ordinary way a placeholder becomes a defect, because
/// nothing looks wrong: the selector draws correctly and offers the one option
/// that was never in doubt, while a card can read "Launchers" next to a filter
/// that cannot select "Launchers". It is the same shape as the module doc's own
/// warning and as #65: a control that renders and does nothing.
///
/// The list is **derived** from [`INSTALLER_CATEGORIES`] rather than spelled
/// out beside it, so it cannot drift from the catalog again — and
/// `every_category_the_catalog_can_produce_is_one_the_filter_offers` checks the
/// other direction, against the real nine recipes.
pub fn installer_categories() -> Vec<String> {
    std::iter::once(ALL_CATEGORIES.to_string())
        .chain(INSTALLER_CATEGORIES.iter().map(|item| (*item).to_string()))
        .collect()
}

/// Which of [`installer_categories`]' options the filter is showing, by
/// position — what the category dropdown needs to draw its own selection.
///
/// `None` when the current value is not one of the options, which the dropdown
/// renders as no selection rather than silently as the first entry: a filter set
/// to something the list does not offer would otherwise draw "All" over a
/// filtered catalog, which is #74's defect one level down.
///
/// # What this extraction does and does not close — measured, not argued
///
/// The lookup is covered: `.position(|item| item == category)` mutated to `!=`,
/// or to `.map(|_| 0)`, each fail
/// `the_selected_filter_is_the_position_of_the_current_value`.
///
/// The **call site** survives — in [`view`], replacing
/// `selected_category(page.categories, page.category)` with `Some(0)` leaves the
/// whole suite green. A `Dropdown`'s selected index is not one of `Operation`'s
/// arms, the same wall `install_press`'s call site hits, so no test can read
/// which option a built dropdown is showing. Recorded rather than papered over:
/// that wiring is checked by reading [`view`], not by a test that cannot see it.
pub fn selected_category(categories: &[String], category: &str) -> Option<usize> {
    categories.iter().position(|item| item == category)
}

/// The selected runner's id, by position in [`runner_choices`].
///
/// An unknown or empty id selects nothing rather than silently the first
/// entry — the reference does the same, falling back to index 0 in the QML
/// itself (`InstallersPage.qml:56-57`), which is a rendering decision this
/// function deliberately leaves to the view. The value it is handed comes from
/// [`seeded_install_runner`], which performs that collapse on the *state*
/// instead, so the case is unreachable in practice rather than unhandled.
pub fn runner_index(choices: &[(String, String)], runner_id: &str) -> Option<usize> {
    choices.iter().position(|(id, _)| id == runner_id)
}

/// The `Message` a runner selection carries, from the selector's index.
///
/// # This is `#97`/D-55, and the reason it is a function rather than a closure
///
/// `dropdown`'s `on_selected` is `impl Fn(usize) -> Message` (`libcosmic
/// src/widget/dropdown/mod.rs:30`), so the callback's parameter is a **position
/// in the model**, never the value the model holds. The line this replaced was
/// `move |index| Message::SetDefaultRunner(index.to_string())`, which is wrong
/// twice over:
///
/// * **the value** — it stored `"1"` where a runner id belongs, and
///   `RunnerManager::get` answers an unknown id with System Wine, so the user
///   picked Proton GE and installed under System Wine, silently, every time;
/// * **the destination** — it wrote `settings.default_runner`, the Settings
///   page's control. The reference's combo has **no write-back at all**
///   (`InstallersPage.qml:49-64` reads `defaultRunner` only, at `:55` and
///   `:61`); its `valueRole: "runnerId"` reaches `installEasy` as an argument
///   (`:128-130`) and the global default is only the *fallback*
///   (`bridge.py:840`, and the resolved id is carried onto the created game at
///   `:855`, `:890`, `:906`). See [`State::installer_runner`].
///
/// The named function is what makes the mapping testable at all: a closure
/// lives inside a builder and a builder needs a renderer, so no assertion could
/// read what it produced — the same wall [`super::library::category_selection`]
/// hit with the same defect, and the same repair. `form.rs`'s
/// `no_dropdown_callback_turns_its_index_into_the_payload` is the guard that now
/// reads the call sites themselves.
///
/// [`State::installer_runner`]: crate::state::State::installer_runner
pub fn install_runner_selection(choices: &[(String, String)], index: usize) -> Message {
    Message::SetInstallRunner(
        choices
            .get(index)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| gamehandler_core::models::SYSTEM_WINE.to_string()),
    )
}

/// The runner a new install starts out selecting.
///
/// This is the QML combo's `currentIndex`, in the port's state, and it is the
/// reference's own two moments folded into one rule:
///
/// * `Component.onCompleted` (`InstallersPage.qml:56-57`) — a fresh page selects
///   [`State::settings`]'s `default_runner` when the list offers it, and entry
///   zero (System Wine) when it does not;
/// * the `onRunnersChanged` handler (`:60-63`) — the same lookup again, so a
///   runner that has been uninstalled cannot leave the selector pointing at a
///   choice that is gone.
///
/// The difference is that `current` — the port's stand-in for the widget's own
/// `currentIndex`, which the reference never stores — is kept when it is still
/// one of `choices`. Without that, every keystroke in the search box (which
/// calls [`State::refresh_installers`]) would silently discard a choice the user
/// had made. The reference has no such write to make: its combo is not rebuilt
/// by `installersChanged`, only by `runnersChanged`.
///
/// [`State::settings`]: crate::state::State::settings
/// [`State::refresh_installers`]: crate::state::State::refresh_installers
pub fn seeded_install_runner(
    choices: &[(String, String)],
    current: &str,
    default_runner: &str,
) -> String {
    for candidate in [current, default_runner] {
        if runner_index(choices, candidate).is_some() {
            return candidate.to_string();
        }
    }
    choices
        .first()
        .map(|(id, _)| id.clone())
        .unwrap_or_default()
}

/// The `Message` a category selection carries, from the selector's index.
///
/// The same mapping as [`super::library::category_selection`], for this page's
/// own message and its own sentinel-carrying list. It exists for the reason
/// [`install_runner_selection`] does, and it was needed for the same reason: the
/// closure this replaced named its parameter `category` and was
/// `|category| Message::SetInstallerCategory(category.to_string())`, so choosing
/// "Launchers" stored `"2"` and the catalog came back empty. `library.rs` fixed
/// that shape once already and its doc says so; a rule that reads every call
/// site is what stops it being fixed a third time by hand.
pub fn category_selection(options: &[String], index: usize) -> Message {
    Message::SetInstallerCategory(
        options
            .get(index)
            .cloned()
            .unwrap_or_else(|| ALL_CATEGORIES.to_string()),
    )
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
///
/// # What this extraction does and does not close — measured, not argued
///
/// **The guard is now readable; the call site is not.** The extracted body is
/// covered: `(!busy).then(..)` mutated to `true.then(..)` fails
/// `install_is_disabled_while_busy_and_names_what_it_would_install` (a code
/// span, not a link — rustdoc does not document `#[cfg(test)]` items, so a
/// bracket form here would be the #75 defect and would still read as a link),
/// as does transposing the installer and the runner in the payload and
/// `installer_id`/`runner_id` the other way round.
///
/// Two call-site mutations **survive**, and are recorded rather than papered
/// over:
///
/// ```text
/// B   installer_card ignores this function and always enables  SURVIVES
/// B2  the call site hardcodes `busy = false`                    SURVIVES
/// ```
///
/// Both are unobservable for the reason above: a `Button`'s pressed-message is
/// not one of `Operation`'s arms, so no test can read the message a built card
/// actually carries. Closing them needs a button reader libcosmic does not
/// expose. A test double would be satisfied by the very thing it cannot
/// distinguish, so there is deliberately none — the wiring is checked by
/// reading the handoff, not by a test that cannot see it.
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
// The catalog
// ---------------------------------------------------------------------------

/// The cards the page draws: every recipe the search box and the category filter
/// leave, in the catalog's order (`bridge.py:813-828`).
///
/// The *matching* is [`search_installers`]' — the reference's every rule about
/// what a query hits and how the category is compared lives there and in one
/// place, and this function adds none of its own. What it does is the bridge's
/// half of the same property: turn a recipe into the four fields a card has. The
/// one thing worth naming is that `query` and `category` reach the catalog
/// **as the page holds them**; there is no second fold and no second default, so
/// `""` means "no filter" here only because the reference's `ALL_CATEGORIES` and
/// `""` both do there (`installers.py:245`).
pub fn installer_rows(query: &str, category: &str) -> Vec<InstallerRow> {
    search_installers(query, category)
        .into_iter()
        .map(installer_row)
        .collect()
}

/// One recipe as one card. `bridge.py:815-820`'s four fields, and the subtitle
/// is [`card_subtitle`]'s rule rather than a second spelling of it.
fn installer_row(installer: &Installer) -> InstallerRow {
    InstallerRow {
        installer_id: installer.id.to_string(),
        name: installer.name.to_string(),
        subtitle: card_subtitle(installer.description, installer.notes),
        category: installer.category.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// Everything the page draws, borrowed.
pub struct InstallersView<'a> {
    /// [`installer_rows`]' result for [`Self::search`] and [`Self::category`] —
    /// i.e. already filtered. The page draws the list it is handed.
    pub catalog: &'a [InstallerRow],
    /// [`State::installer_search`].
    pub search: &'a str,
    /// [`State::installer_category`].
    pub category: &'a str,
    /// [`installer_categories`]' result plus the catalog's own categories.
    pub categories: &'a [String],
    /// [`runner_choices`]' result.
    pub runners: &'a [(String, String)],
    /// The runner a new install will use — **this install's**, not the global
    /// default.
    ///
    /// [`State::installer_runner`], which [`seeded_install_runner`] fills from
    /// `settings.default_runner` and the selector's own choice then overrides.
    /// The distinction is the whole of D-55: binding this to
    /// `state.settings.default_runner` made the page's selector write the
    /// Settings page's control, which the reference's combo never does.
    ///
    /// [`State::installer_runner`]: crate::state::State::installer_runner
    pub runner_id: &'a str,
    /// [`installing`], i.e. `state.busy()`.
    pub busy: bool,
    /// [`progress_fraction`].
    pub progress: Option<f32>,
}

/// The page.
///
/// # The argument is taken by value, and that was T-38's one change here
///
/// This took `&'a InstallersView<'_>` until T-38 wired the page into
/// [`crate::Shell::view_body`], and the reference form is **uncallable from a
/// dispatch arm**: the returned [`Element`]'s lifetime is the borrow of the
/// `InstallersView`, so a dispatcher that builds one as a local gets
/// `E0515: cannot return value referencing local variable` — the element
/// outlives the struct it was handed. Every sibling page's `view` already takes
/// its arguments by value (`view::library::view(page)`,
/// `view::runners::view(page)`, `view::plugins::view(page)`,
/// `view::settings::view(page)`), so this is the odd one joining the
/// convention rather than a new shape: the render path is byte-for-byte
/// unchanged, and only the two test call sites moved from `view(&page)` to
/// `view(page)`.
pub fn view<'a>(page: InstallersView<'a>) -> Element<'a, Message> {
    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- The header toolbar: search and category ---------------------------
    let selected = selected_category(page.categories, page.category);
    // The callback is `Send + Sync + 'static` (`libcosmic
    // src/widget/dropdown/mod.rs:30`), so it cannot borrow the page it is built
    // from: each selector takes its own copy of the model, exactly as
    // `view::settings`'s runner row does. The copies are of a nine-entry list
    // and a handful of runners, once per frame.
    let category_options = page.categories.to_vec();
    let category_choices = category_options.clone();

    body = body.push(
        Row::new()
            .push(
                cosmic::widget::text_input("Search installers…", page.search.to_string())
                    .on_input(Message::SetInstallerSearch)
                    .width(Length::Fixed(22.0 * 18.0)),
            )
            .push(cosmic::widget::dropdown(
                category_options,
                selected,
                move |index| category_selection(&category_choices, index),
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
    let runner_choices = page.runners.to_vec();
    body = body.push(cosmic::widget::dropdown(
        labels,
        runner_index(page.runners, page.runner_id),
        move |index| install_runner_selection(&runner_choices, index),
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
        let button = button.on_press_maybe(install_press(row, runner_id, busy));
        // `QQC2.ToolTip.text` on that same button (`InstallersPage.qml:126`).
        // [`install_tooltip`] carried the sentence with no caller until T-38,
        // which is a tested function that reads as coverage; the wrap is
        // `widget::tooltip`, because a libcosmic `Button` has no `.tooltip`
        // method and this free function is the only tooltip the toolkit
        // exposes. `Position::Bottom` is QQC2's own default placement.
        //
        // **This call site cannot be asserted, and that is measured.** iced's
        // `Tooltip::operate` traverses `self.content` and nothing else
        // (`iced_widget/src/tooltip.rs:372-383`), so the page's
        // `drawn_strings` returns the button's "Install" and never the
        // sentence — probed, and the probe failed. It is the same wall as
        // [`install_press`]'s call site one level up: `Operator` cannot read
        // what a wrapped widget carries. What *is* covered is the sentence
        // itself, in `install_tooltip`'s own test.
        cosmic::widget::tooltip(
            button,
            text::caption(install_tooltip(&row.name)),
            cosmic::widget::tooltip::Position::Bottom,
        )
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
        // The per-install runner (D-55). Stored as it arrives, with no
        // validation and no fallback of its own: the reference's combo can only
        // yield a value from `backend.runnerChoices` (`InstallersPage.qml:50`),
        // and the one place a value that is not a runner has to survive is
        // `start_easy_install`, which already resolves an empty id to the global
        // default exactly as `bridge.py:840` does. A guard here would be a
        // second rule about the same value, free to disagree with the first.
        //
        // It deliberately does **not** write `settings.default_runner`: that is
        // the Settings page's control and the reference never writes it from
        // here. The value travels to the install as an argument — through
        // `InstallersView::runner_id`, `install_press` and
        // `Message::StartEasyInstall` — and lands on the created game.
        Message::SetInstallRunner(runner_id) => {
            state.installer_runner = runner_id.clone();
            Some(Task::none())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamehandler_core::installers::{APPS, LAUNCHERS};
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

    // ---- the two selectors' mappings (#97 / D-55) --------------------------

    /// The fixture both mapping tests use: no id equals any label, so a
    /// mutation that read the wrong half could not pass by coincidence — the
    /// property `form.rs`'s runner test spells out one module over.
    fn fixture_choices() -> Vec<(String, String)> {
        let choices = vec![
            (
                gamehandler_core::models::SYSTEM_WINE.to_string(),
                "System Wine".to_string(),
            ),
            ("GE-Proton9-5".to_string(), "GE-Proton9-5 (Proton-GE)".to_string()),
        ];
        let mut halves: Vec<&String> = choices.iter().flat_map(|(id, label)| [id, label]).collect();
        halves.sort();
        let all = halves.len();
        halves.dedup();
        assert_eq!(
            halves.len(),
            all,
            "the fixture has an id equal to a label, so it cannot tell the id \
             from the label: {choices:?}"
        );
        choices
    }

    /// The selector carries the runner's **id**, not its position — #97.
    ///
    /// The defect, spelled out: the line this replaced was
    /// `move |index| Message::SetDefaultRunner(index.to_string())`, so index 1
    /// became the id `"1"`, which `RunnerManager::get` answers with System Wine.
    /// The assertion that would have caught it is the first one below, and the
    /// second is its destination: this message must not be
    /// [`Message::SetDefaultRunner`], because the reference never writes the
    /// global default from this page (D-55).
    #[test]
    fn the_runner_selection_carries_the_id_and_not_the_position() {
        let choices = fixture_choices();

        assert!(matches!(
            install_runner_selection(&choices, 1),
            Message::SetInstallRunner(ref value) if value == "GE-Proton9-5"
        ));
        assert!(matches!(
            install_runner_selection(&choices, 0),
            Message::SetInstallRunner(ref value)
                if value == gamehandler_core::models::SYSTEM_WINE
        ));

        // The defect, spelled out: index 1 is Proton GE, and `"1"` is an id no
        // manager holds.
        assert_ne!(
            match install_runner_selection(&choices, 1) {
                Message::SetInstallRunner(value) => value,
                other => panic!("the selector must carry the per-install message, got {other:?}"),
            },
            "1",
            "the index is a position, and `\"1\"` resolves to System Wine"
        );

        // An index past the end collapses to System Wine — the same entry
        // `runner_index` falls back to — rather than to an empty id.
        for index in [2usize, 9] {
            assert!(matches!(
                install_runner_selection(&choices, index),
                Message::SetInstallRunner(ref value)
                    if value == gamehandler_core::models::SYSTEM_WINE
            ));
        }
        assert!(matches!(
            install_runner_selection(&[], 0),
            Message::SetInstallRunner(ref value)
                if value == gamehandler_core::models::SYSTEM_WINE
        ));
    }

    /// The category selector carries the category, not its position.
    ///
    /// #97's twin, live in this file until the D-55 repair: the closure's
    /// parameter was named `category` and was a `usize`, so
    /// `category.to_string()` stored `"2"` and the page drew "No matching
    /// installers" over a full catalog. `library.rs` records fixing exactly this
    /// by hand once before; this is the same assertion for this page's list.
    #[test]
    fn the_category_selection_carries_the_name_and_not_the_position() {
        let options = installer_categories();
        let launchers = options
            .iter()
            .position(|option| option == "Launchers")
            .expect("the catalog has a Launchers category");

        assert!(matches!(
            category_selection(&options, launchers),
            Message::SetInstallerCategory(ref value) if value == "Launchers"
        ));
        assert!(matches!(
            category_selection(&options, 0),
            Message::SetInstallerCategory(ref value) if value == ALL_CATEGORIES
        ));
        // The defect, spelled out.
        assert_ne!(
            match category_selection(&options, launchers) {
                Message::SetInstallerCategory(value) => value,
                other => panic!("the selector must carry the category message, got {other:?}"),
            },
            launchers.to_string(),
            "the position is not the category, and a filter set to `\"{launchers}\"` \
             matches no card"
        );
        // An index no option provides cannot invent a category: it folds to the
        // sentinel rather than to a string that matches nothing.
        assert!(matches!(
            category_selection(&[], 99),
            Message::SetInstallerCategory(ref value) if value == ALL_CATEGORIES
        ));
    }

    /// A fresh page selects the stored default when the list offers it, and
    /// System Wine when it does not — `InstallersPage.qml:56-57`'s two-way
    /// lookup, and the same one the `onRunnersChanged` handler repeats (`:60-63`).
    #[test]
    fn a_fresh_page_selects_the_default_when_the_list_offers_it() {
        let choices = fixture_choices();
        let system = gamehandler_core::models::SYSTEM_WINE;

        assert_eq!(
            seeded_install_runner(&choices, "", "GE-Proton9-5"),
            "GE-Proton9-5",
            "the default is what a fresh page selects when it is installed"
        );
        assert_eq!(
            seeded_install_runner(&choices, "", "uninstalled-runner"),
            system,
            "a default that is not in the list degrades to entry zero, which is \
             System Wine — the reference's `index >= 0 ? index : 0`"
        );
        assert_eq!(
            seeded_install_runner(&[], "", "GE-Proton9-5"),
            "",
            "an empty list has nothing to select, and `runner_index` renders that \
             as no selection rather than as a panic"
        );

        // A choice that is still in the list survives the refresh: this is the
        // port's stand-in for the combo's own `currentIndex`, and without it a
        // keystroke in the search box would silently discard the user's pick.
        assert_eq!(
            seeded_install_runner(&choices, "GE-Proton9-5", "uninstalled-runner"),
            "GE-Proton9-5",
            "the current choice wins over the default while it is still offered"
        );
        // ...and one that is gone does not, which is the `onRunnersChanged` half.
        assert_eq!(
            seeded_install_runner(&choices, "uninstalled-runner", "GE-Proton9-5"),
            "GE-Proton9-5",
            "a choice whose runner was uninstalled falls back to the default"
        );
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

    /// The per-install choice is stored on the install's field and **not** on
    /// the app's default — the first half of D-55.
    ///
    /// This is the assertion whose absence let #97 ship: the arm it replaces
    /// wrote `settings.default_runner`, so the page's selector moved a control
    /// on another page while the value it wrote there was the index.
    #[test]
    fn choosing_a_runner_for_the_next_install_leaves_the_global_default_alone() {
        let mut state = state();
        state.settings.default_runner = "system".to_string();
        state.installer_runner = "system".to_string();

        assert!(update(
            &mut state,
            &Message::SetInstallRunner("GE-Proton9-5".to_string())
        )
        .is_some());
        assert_eq!(state.installer_runner, "GE-Proton9-5");
        assert_eq!(
            state.settings.default_runner, "system",
            "the Installers selector is not the Settings page's control; the \
             reference's combo has no write-back at all"
        );

        // And the other direction, so the two fields cannot be one field with
        // two names: the Settings page's message does not move the install's
        // choice either. (`SetDefaultRunner` is declined here — it is T-13's —
        // so this asserts the *state* is untouched rather than the return.)
        assert!(update(&mut state, &Message::SetDefaultRunner("system".to_string())).is_none());
        assert_eq!(state.installer_runner, "GE-Proton9-5");
    }

    /// The chain D-55 is about, walked end to end without a runner: the choice
    /// reaches the install press as the runner the install will use.
    ///
    /// Three hops — `State::installer_runner` → `InstallersView::runner_id` →
    /// [`install_press`]'s payload — and the middle one is a binding in
    /// `main.rs`'s `view_body` that no test can read. What this holds is the two
    /// ends: the message writes the field the page is handed, and the press
    /// carries what the field says. The binding between them is checked by
    /// reading it, which is what the comment on it is for.
    #[test]
    fn the_chosen_runner_is_the_one_the_install_press_names() {
        let mut state = state();
        state.easy_busy = false;

        update(
            &mut state,
            &Message::SetInstallRunner("GE-Proton9-5".to_string()),
        );

        let steam = row("steam", "Steam");
        // `false` for `busy`: pressing Install while a job runs carries no
        // message at all, which is P-59 and is `install_press`'s own test.
        match install_press(&steam, &state.installer_runner, false) {
            Some(Message::StartEasyInstall {
                installer_id,
                runner_id,
            }) => {
                assert_eq!(installer_id, "steam");
                assert_eq!(
                    runner_id, "GE-Proton9-5",
                    "the runner the user picked for this install, by id"
                );
            }
            other => panic!("an idle card must offer its install, got {other:?}"),
        }
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

    /// The categories list is the reference's `["All", *INSTALLER_CATEGORIES]`
    /// (`bridge.py:787`) — **the whole list**, derived from the constant rather
    /// than re-spelled beside it.
    ///
    /// The test this replaces asserted only `categories[0] == ALL_CATEGORIES`.
    /// That passed while the function returned `["All"]` and would have passed
    /// after: it pinned the single element that was never in doubt, which is
    /// #74's defect surviving its own test.
    #[test]
    fn the_category_list_is_the_sentinel_then_the_catalogs_own_categories() {
        let categories = installer_categories();
        let expected: Vec<String> = std::iter::once(ALL_CATEGORIES.to_string())
            .chain(INSTALLER_CATEGORIES.iter().map(|item| (*item).to_string()))
            .collect();

        assert_eq!(categories, expected);
        assert_eq!(categories[0], ALL_CATEGORIES, "the sentinel is first");
        assert_eq!(categories.len(), 1 + INSTALLER_CATEGORIES.len());
        assert_ne!(categories.len(), 1, "the list is not the sentinel alone (#74)");
    }

    /// Every category the catalog can put on a card is one the filter can
    /// select — #74's trap, checked against the real nine recipes rather than
    /// argued from the constants.
    ///
    /// The defect the missing list produced is invisible in the view: the badge
    /// renders from the row and the dropdown renders from the list, and nothing
    /// compared the two. This is that comparison, and it fails if a recipe is
    /// ever given a third category without `INSTALLER_CATEGORIES` growing with
    /// it.
    #[test]
    fn every_category_the_catalog_can_produce_is_one_the_filter_offers() {
        let categories = installer_categories();
        let rows = installer_rows("", ALL_CATEGORIES);
        assert_eq!(rows.len(), 9, "P-51's acceptance is the nine cards");

        let mut seen: Vec<&str> = Vec::new();
        for row in &rows {
            if row.category.trim().is_empty() {
                continue;
            }
            assert!(
                categories.iter().any(|category| category == &row.category),
                "a card can read {:?} while the filter cannot select it",
                row.category
            );
            if !seen.contains(&row.category.as_str()) {
                seen.push(&row.category);
            }
        }

        // The other half: both options are actually used by the catalog, so the
        // loop above is not vacuous and neither constant is dead.
        for category in INSTALLER_CATEGORIES {
            assert!(
                seen.contains(&category),
                "{category} is offered by the filter and used by no recipe"
            );
        }
    }

    /// The dropdown's selection is the position of the page's current value, and
    /// a value the list does not offer is no selection — not the first entry,
    /// which is what would draw "All" over a filtered catalog.
    #[test]
    fn the_selected_filter_is_the_position_of_the_current_value() {
        let categories = installer_categories();
        assert_eq!(selected_category(&categories, ALL_CATEGORIES), Some(0));
        assert_eq!(
            selected_category(&categories, LAUNCHERS),
            Some(1),
            "the sentinel holds index 0, so the catalog's categories start at 1"
        );
        assert_eq!(selected_category(&categories, APPS), Some(2));
        assert_eq!(selected_category(&categories, "uncategorized"), None);
        assert_eq!(selected_category(&categories, ""), None);
        assert_ne!(
            selected_category(&categories, APPS),
            selected_category(&categories, ALL_CATEGORIES),
            "the last option must not draw as the first"
        );
    }

    // ---- installer_rows: the catalog ---------------------------------------

    /// The catalog is `core::installers`' own, in its order, field for field —
    /// compared against the core array rather than against a list written here,
    /// so a tenth recipe or a reordering is a change to the test's *source*
    /// rather than to its numbers.
    #[test]
    fn the_catalog_is_the_core_catalogs_recipes_in_its_order() {
        let rows = installer_rows("", ALL_CATEGORIES);
        let catalog = gamehandler_core::installers::installers();
        assert_eq!(rows.len(), catalog.len(), "no recipe is dropped or invented");

        for (row, installer) in rows.iter().zip(catalog) {
            assert_eq!(row.installer_id, installer.id);
            assert_eq!(row.name, installer.name);
            assert_eq!(row.category, installer.category);
        }

        // The order is load-bearing: `search_installers` filters and never
        // sorts, so the cards appear in exactly this order and `INSTALLERS[0]`
        // is the first card drawn.
        assert_eq!(rows[0].installer_id, catalog[0].id);
        assert_eq!(
            rows[rows.len() - 1].installer_id,
            catalog[catalog.len() - 1].id
        );
    }

    /// A card's second line is the recipe's own notes when it has any, and the
    /// description alone when it has none — over **every** recipe in the
    /// catalog, with both branches asserted to be non-empty so neither is a
    /// vacuous loop.
    ///
    /// Written against the data rather than against a named recipe because the
    /// first version of this test named `steam` and went red: Steam does have no
    /// notes, but the query `"steam"` also matches EA's description
    /// ("Steam-unlisted EA titles"), so it returned two cards and
    /// `assert_eq!(rows.len(), 1)` failed. The catalog is the authority, so the
    /// catalog is what this test reads.
    #[test]
    fn a_card_carries_the_recipes_notes_on_a_second_line_when_it_has_any() {
        let mut with_notes = 0;
        let mut without_notes = 0;

        for installer in gamehandler_core::installers::installers() {
            let row = installer_rows(installer.id, ALL_CATEGORIES)
                .into_iter()
                .find(|row| row.installer_id == installer.id)
                .expect("a recipe's own id must find its own card");

            if installer.notes.is_empty() {
                without_notes += 1;
                assert_eq!(
                    row.subtitle, installer.description,
                    "an entry with no notes must gain nothing, not a blank line"
                );
            } else {
                with_notes += 1;
                assert_eq!(
                    row.subtitle,
                    format!("{}\n{}", installer.description, installer.notes),
                    "the notes go on their own line, not joined to the description"
                );
                assert_eq!(
                    row.subtitle.lines().count(),
                    installer.description.lines().count() + installer.notes.lines().count()
                );
            }
        }

        assert!(with_notes > 0, "no recipe has notes, so the branch is untested");
        assert!(
            without_notes > 0,
            "every recipe has notes, so the other branch is untested"
        );

        // Named once concretely, so the shape is legible without the loop:
        // Battle.net is the entry the reference explains on a second line.
        let battle_net = installer_rows("battlenet", ALL_CATEGORIES)
            .into_iter()
            .find(|row| row.installer_id == "battlenet")
            .expect("battlenet is a recipe id");
        assert_eq!(battle_net.subtitle.lines().count(), 2);
    }

    /// The search box and the category filter reach the catalog, and they reach
    /// it with the reference's rules — matching is `search_installers`' and this
    /// test holds the page's half: that the query and the category the page
    /// holds are the ones the catalog is filtered by.
    #[test]
    fn the_search_box_and_the_category_filter_reach_the_catalog() {
        let all = installer_rows("", ALL_CATEGORIES);
        assert_eq!(all.len(), 9);
        assert_eq!(
            installer_rows("", "").len(),
            all.len(),
            "\"\" is also no filter (installers.py:245)"
        );

        // A description-only match: "blizzard" is in no recipe's name or id.
        let blizzard = installer_rows("blizzard", ALL_CATEGORIES);
        assert!(
            blizzard.iter().any(|row| row.installer_id == "battlenet"),
            "the query must reach the description"
        );
        assert!(blizzard.len() < all.len(), "and it must actually filter");

        let launchers = installer_rows("", LAUNCHERS);
        assert!(!launchers.is_empty());
        assert!(launchers.iter().all(|row| row.category == LAUNCHERS));

        let apps = installer_rows("", APPS);
        assert!(apps.iter().all(|row| row.category == APPS));
        assert_eq!(apps.len() + launchers.len(), all.len(), "the filter partitions");

        // The category is compared **exactly**, with no case folding
        // (`installers.py:245`): a lowercased value is neither the sentinel nor
        // any recipe's category, so it selects nothing. That reads like a bug
        // and is the reference's behaviour, which is why it is pinned.
        assert!(
            installer_rows("", "launchers").is_empty(),
            "the category is matched exactly — a lowercased one matches nothing"
        );

        // And the two compose: the query is applied to what the filter left.
        let composed = installer_rows("blizzard", LAUNCHERS);
        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].installer_id, "battlenet");
        assert!(
            installer_rows("blizzard", APPS).is_empty(),
            "the category still filters what the query left"
        );
        assert!(installer_rows("nonesuchatall", ALL_CATEGORIES).is_empty());
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
        // The identity claim is checked on the rows the page hands the view, and
        // *before* the view is built: `view` takes its argument by value now, so
        // the build consumes `page`. The order is the only thing that moved.
        assert_eq!(page.catalog[0].installer_id, "battlenet");
        assert_eq!(page.catalog[1].installer_id, "steam");
        // The view is built for real — a card that panicked or borrowed wrongly
        // fails here.
        let _element: Element<'_, Message> = view(page);
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
        let _element: Element<'_, Message> = view(page);
    }
}
