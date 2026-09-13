//! The Settings page: appearance, defaults for new games, and behaviour.
//!
//! Port of `SettingsPage.qml` (138 lines) and its setters, `bridge.py:188-270`.
//!
//! # What is landing here, and what is not
//!
//! This is the reference's **whole visible page**: the two appearance selectors,
//! the default runner, the thirteen default toggles, close-on-launch, and the
//! four shortcut rows. What is *not* here is the effect half of the appearance
//! selectors. `_set_color_scheme` (`bridge.py:197-203`) applies the theme as
//! well as storing it, and this port applies no theme at all yet — the shell
//! has no `theme()` override, so `ColorScheme` is written to the model and
//! nothing reads it. That is D-13 (`PLAN.md:479`), an open decision that names
//! T-13 as where it is made, and it is left open on purpose rather than closed
//! by defaulting: see [`COLOR_SCHEME_OPTIONS`] for what the page can and cannot
//! claim while it is open.
//!
//! # The split
//!
//! As everywhere in this module, every decision is a pure function over data and
//! the widget builder only calls it. The thirteen toggles are a table plus a
//! `key`-addressed read/write pair rather than thirteen fields, because the
//! reference addresses them by name (`setDefaultToggle(name, value)`,
//! `bridge.py:264-269`) and a name-addressed accessor is the only shape that
//! makes the table the single place the set is written down.

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget::{Column, container, scrollable, text, toggler};
use gamehandler_core::runners::RunnerManager;
use gamehandler_core::settings::{COLOR_SCHEMES, Settings, VIEW_MODES};

use super::a11y;
use super::{settings_row, settings_section};
use crate::Message;

/// The colour-scheme selector's `(key, label)` pairs, in `SettingsPage.qml:44-48`
/// order.
///
/// The keys are [`COLOR_SCHEMES`]' own — the three the loader and the message
/// path both accept — so the selector cannot offer a value the model would
/// reject. The labels are the QML's.
///
/// **`"system"` is the first entry and `"dark"` is the fourth line of it, which
/// is the whole of what this port can honour today.** The reference applies the
/// scheme to a live theme (`theme.py:18-89`); here the choice is stored and
/// nothing more, so "Match system" is not *more* honest than "Dark" — both are
/// inert. It is listed first because the QML lists it first, not because it does
/// anything.
pub const COLOR_SCHEME_OPTIONS: [(&str, &str); 3] = [
    ("system", "Match system"),
    ("light", "Light"),
    ("dark", "Dark"),
];

/// The library-layout selector's pairs, in `SettingsPage.qml:58-61` order.
pub const VIEW_MODE_OPTIONS: [(&str, &str); 2] = [("grid", "Grid"), ("list", "List")];

/// The thirteen default toggles as `(key, label, subtitle)`, in
/// `SettingsPage.qml:12-26` order.
///
/// The keys are the model's own: each one addresses a `default_<key>` field on
/// [`Settings`], which is exactly how the reference builds its field name
/// (`bridge.py:266`). The order is user-visible — it is the order the switches
/// appear in — so it is data here rather than a property of how the loop happens
/// to run.
///
/// Thirteen, not fifteen: `_TOGGLE_FIELDS` (`bridge.py:74-78`) also names
/// `wayland` and `hdr`, and the reference's own filter drops them because
/// `Settings` has no `default_wayland` and no `default_hdr`
/// (`bridge.py:79-82`). This table is the filtered set, which is what the page
/// draws.
pub const DEFAULT_TOGGLES: [(&str, &str, &str); 13] = [
    ("mangohud", "Enable MangoHud by default", ""),
    ("gamemode", "Enable GameMode by default", ""),
    ("prefer_sdl", "Prefer SDL by default", ""),
    (
        "esync",
        "Enable Esync by default",
        "Eventfd-based Wine sync. Usually leave this on.",
    ),
    (
        "fsync",
        "Enable Fsync by default",
        "Futex-based Wine sync. Preferred when the kernel supports it.",
    ),
    (
        "dxvk",
        "Enable DXVK by default",
        "Direct3D 8–11 through Vulkan.",
    ),
    (
        "vkd3d",
        "Enable VKD3D by default",
        "Direct3D 12 through Vulkan.",
    ),
    (
        "nvapi",
        "Enable DXVK-NVAPI / DLSS by default",
        "Only needed for some NVIDIA / DLSS titles.",
    ),
    ("fsr", "Enable AMD FSR by default", ""),
    ("battleye", "Enable BattlEye runtime by default", ""),
    ("eac", "Enable Easy Anti-Cheat runtime by default", ""),
    ("gamescope", "Enable Gamescope by default", ""),
    ("virtual_desktop", "Enable virtual desktop by default", ""),
];

/// The four shortcut rows, `(keys, what it does)`, in `SettingsPage.qml:132-135`
/// order.
///
/// **These are a list, not a claim.** `P-68` is the requirement that each
/// shortcut actually works, and this page only prints them — the accelerators
/// are the shell's. A row here that the shell does not honour is the page lying
/// about the shell, which is why what this page asserts is not that the rows are
/// printed but that they are *answered*:
/// `every_row_this_page_prints_is_a_key_the_window_answers` drives each row's own
/// key through the subscriptions that run in a window — the shell's and
/// libcosmic's — and requires exactly one of them to answer it in each focus
/// state. That test is what makes the rows a claim rather than a list.
///
/// It used to be printed under a caveat about focus, and the caveat is gone
/// with the divergence that made it true: see [`IMPLEMENTED_SHORTCUTS`].
pub const SHORTCUTS: [(&str, &str); 4] = [
    ("Ctrl+N:", "Add a game"),
    ("Ctrl+F:", "Search the library"),
    ("Ctrl+,:", "Settings"),
    ("Ctrl+Q:", "Quit"),
];

/// The shortcuts the shell actually implements — all four, as of T-25 (P-68).
///
/// # This constant was empty, and how it emptied is the useful part
///
/// It previously read `[&str; 0] = []` under a heading that said **"None of
/// the four work"**, with the reason paragraphs that survive in the git history:
/// *"`Ctrl+F` must focus the Library search field (a widget `Id` and a focus
/// `Task`), and `Ctrl+N`/`Ctrl+F` must not fire while the user is typing in a
/// text field, which is a focus-visibility question the shell does not yet
/// answer for any widget."* Two of those three claims turned out to be false,
/// and both were false in the same direction — the framework already answered
/// the question:
///
/// * **`Ctrl+F` is bound by libcosmic, not by the port.** Its
///   `keyboard_nav::subscription()` matches `Character("f")` with Control
///   (`src/keyboard_nav.rs:50-55`) and `Cosmic::update` routes that to
///   `Application::on_search()` (`src/app/cosmic.rs:850`). `main.rs` replies to
///   that hook; it does not add a binding of its own, because a second one would
///   fire the same key twice.
/// * **"must not fire while the user is typing" needed no code.** The keyboard
///   subscriptions deliver only events whose status is `Ignored` — the status
///   the widget tree reported — so a focused `text_input` keeps its own editing
///   keys and these three pass through. `Main.qml:122` sets
///   `context: Qt.ApplicationShortcut` precisely so that the shortcut is active
///   regardless of which widget holds focus, so passing through *is* the
///   reference's behaviour rather than a hole.
///
/// So this is not a constant that was filled in by wishful thinking: it is one
/// whose emptiness was measured, whose stated reasons were then falsified
/// against the vendored source, and which the suite refused to let change until
/// the list was updated — `the_page_does_not_claim_a_shortcut_the_shell_does_not_implement`
/// is what failed when the subscription landed.
///
/// # The second of those two bullets was false, and this is where it was fixed
///
/// The paragraph above is kept because it is *almost* right and the way it is
/// wrong is the whole of `BUG-12`. "A focused `text_input` keeps its own editing
/// keys" is true; "and these three pass through" does not follow from it. A
/// focused field captures **every** key, including the ones it does nothing with
/// — `shell.capture_event()` is the last line of its focused branch
/// (`src/widget/text_input/input.rs:2261-2262`) — so the editing-bindings list
/// was the list of keys it *acts* on, and reading that as the list of keys it
/// *claims* made a swallowed accelerator look like a design decision.
///
/// Measured, not read: with the Library's search box focused, `Ctrl+N`, `Ctrl+,`
/// and `Ctrl+Q` all left `Captured`, which is the status `keyboard::listen()`
/// filters out — so all three were dead in the state a user reaching for a
/// shortcut is most often standing in, and `Ctrl+F` was dead too, behind
/// libcosmic's identical `Ignored` gate (`src/keyboard_nav.rs:20-23`). The
/// subscription is now `shortcuts::subscription()`, which answers all four in
/// every status, and the *measurement* is what
/// `every_row_this_page_prints_is_a_key_the_window_answers` drives. See
/// [`crate::shortcuts`] for the plumbing and the argument that nothing typed is
/// displaced.
///
/// # Two things this still does not claim
///
/// * `Ctrl+Shift+F` reaches the search navigation, because libcosmic's match
///   tests Control and does not reject Shift, where Qt's `Shortcut` would not
///   match it. It is the framework's binding and the shell's arm mirrors its
///   predicate deliberately, so that the key does not change meaning with the
///   focus. It is a superset of the advertised key rather than a failure of it,
///   and it is recorded here because this is the page that advertises the key.
/// * That a key press actually arrives is a claim about a running window.
///   Nothing here is observable without one, so it is T-19's to walk in the
///   Flatpak — the same bound P-68's own acceptance criteria name.
///
/// # What "implemented" means here, now that the caveat is gone
///
/// Membership means the shell answers the key **from anywhere in the window**,
/// which is the property the guard measures rather than a reading of this list.
/// This constant used to be qualified by a printed caveat about focus
/// (`SHORTCUT_FOCUS_CAVEAT`), because membership meant only "the shell answers
/// it if it arrives". With the subscription answering every status, that caveat
/// would have been false for three of the four rows, so it went with the
/// divergence: a page that apologises for a defect it no longer has is the same
/// lie as one that stays silent about a defect it does.
///
/// The list is kept as data rather than deleted, and so is
/// [`UNWIRED_SHORTCUTS`], for the reason the complement's own doc gives.
pub const IMPLEMENTED_SHORTCUTS: [&str; 4] = ["Ctrl+N:", "Ctrl+F:", "Ctrl+,:", "Ctrl+Q:"];

/// Every row [`SHORTCUTS`] prints that [`IMPLEMENTED_SHORTCUTS`] does not.
///
/// **Empty, and that is now the good case rather than the gap.** The constant
/// and its test exist for the state this file was in before T-25: a page
/// advertising four shortcuts while the shell honoured none, with nothing in the
/// suite saying so. The complement is spelled out rather than computed so a
/// reviewer can read the status without running anything, and it stays declared
/// at zero length rather than deleted so that the *next* shortcut to be
/// advertised-but-unwired has an obvious home and the test that guards it
/// remains a two-way check rather than one-way.
pub const UNWIRED_SHORTCUTS: [&str; 0] = [];

/// A section heading, `(heading, is_a_form_section)`.
///
/// The reference splits its two `Kirigami.FormLayout`s across four labelled
/// sections (`SettingsPage.qml:34-37`, `72-75`, `115-118`, `127-130`); the
/// headings are user-visible text, so they are data.
pub const SECTION_APPEARANCE: &str = "Appearance";
pub const SECTION_NEW_GAMES: &str = "New games";
pub const SECTION_BEHAVIOR: &str = "Behavior";
pub const SECTION_SHORTCUTS: &str = "Keyboard shortcuts";

/// The three selector rows' form labels, `SettingsPage.qml:41`, `:55`, `:79`.
///
/// Declared rather than written inline at each call site because each one is
/// used **twice** on the row it belongs to: once as the text [`settings_row`] draws
/// beside the control, and once as the accessible name
/// [`a11y::dropdown`] publishes for it. A screen reader should announce the
/// label the user can see, and two literals at two call sites are two strings
/// that can drift — so the constant is what makes them the same string rather
/// than merely equal ones.
pub const LABEL_COLOR_SCHEME: &str = "Color scheme:";
pub const LABEL_LAYOUT: &str = "Library layout:";
pub const LABEL_DEFAULT_RUNNER: &str = "Default runner:";

/// The close-on-launch switch's label and explanation,
/// `SettingsPage.qml:120-125`.
///
/// The two are the reference's two different strings on one control — its
/// `FormData.label` and its `text` — and they are drawn in two different ways,
/// which matters for what can check them:
///
/// * [`CLOSE_ON_LAUNCH_LABEL`] goes to [`settings_row`], so it becomes a `text::body`
///   child and **is** visible to a render-level assertion;
/// * [`CLOSE_ON_LAUNCH_EXPLANATION`] goes to the `Toggler`'s own label, which is
///   painted with a direct `text::draw` and is visible to no tree walk.
///
/// The label previously reached neither: it was declared here and referenced
/// nowhere, so the reference's form label was absent from the page while this
/// constant sat in the source looking like it was drawn. `the_close_on_launch_
/// switch_carries_the_references_two_strings` reads both out of the QML and is
/// what stops that recurring.
pub const CLOSE_ON_LAUNCH_LABEL: &str = "Hide window when launching:";
pub const CLOSE_ON_LAUNCH_EXPLANATION: &str =
    "Keeps the launcher out of the way while a game starts";

/// The colorscheme selector's labels, in options order.
pub fn color_scheme_labels() -> Vec<String> {
    COLOR_SCHEME_OPTIONS
        .iter()
        .map(|(_, label)| label.to_string())
        .collect()
}

/// The layout selector's labels, in options order.
pub fn view_mode_labels() -> Vec<String> {
    VIEW_MODE_OPTIONS
        .iter()
        .map(|(_, label)| label.to_string())
        .collect()
}

/// The default-runner selector's labels, from the manager's own `(id, label)`
/// pairs.
///
/// `RunnerManager::choices()` (`runners/mod.rs:1322`) is the reference's
/// `runnerChoices`, and it already puts System Wine first — so the selector's
/// labels come from core rather than being rebuilt here.
pub fn runner_labels(choices: &[(String, String)]) -> Vec<String> {
    choices.iter().map(|(_, label)| label.clone()).collect()
}

/// Which entry of the colour-scheme selector the stored value selects.
///
/// `None` when the stored value is not one of the three — the loader folds an
/// unrecognised scheme to `"dark"` (`Settings::from_dict`,
/// `crates/core/src/settings.rs:155-157`, and the fallback is the
/// literal `"dark"`, not `COLOR_SCHEMES[0]` — FINDINGS F-E), so this is
/// reachable only if the two lists disagree. A dropdown with no selection rather
/// than a panic or a silently-wrong highlight, the same shape as
/// [`super::library::sort_index`].
pub fn color_scheme_index(scheme: &str) -> Option<usize> {
    COLOR_SCHEME_OPTIONS
        .iter()
        .position(|(key, _)| *key == scheme)
}

/// Which entry of the layout selector the stored value selects; `None` as above.
pub fn view_mode_index(mode: &str) -> Option<usize> {
    VIEW_MODE_OPTIONS.iter().position(|(key, _)| *key == mode)
}

/// Which entry of the runner selector the stored default selects.
///
/// **Zero when it is not found**, which is the reference's own fallback, twice
/// over: `currentIndex = index >= 0 ? index : 0` on completion
/// (`SettingsPage.qml:83-86`) and again when the runner list changes
/// (`:88-94`). Zero is System Wine because [`RunnerManager::choices`] puts it
/// first, so "the default runner is gone" degrades to the one choice that is
/// always present rather than to a blank selector.
///
/// Unlike the two above this returns a plain `usize`, and the difference is not
/// an oversight: those two have a `None` because a stored value that is not in
/// the list is a *port* bug worth showing as unselected, while a default runner
/// that is not in the list is an ordinary state — the user uninstalled it — that
/// the reference deliberately absorbs.
pub fn default_runner_index(choices: &[(String, String)], runner_id: &str) -> usize {
    choices
        .iter()
        .position(|(id, _)| id == runner_id)
        .unwrap_or(0)
}

/// The switch's text: the label, and the subtitle appended when there is one
/// (`SettingsPage.qml:104-106`).
///
/// The separator is the QML's own `" — "` — an em dash between two spaces — and
/// is reproduced rather than approximated, because it is user-visible text and
/// the reference's exact wording is the specification (P-66).
pub fn toggle_label(label: &str, subtitle: &str) -> String {
    if subtitle.is_empty() {
        label.to_string()
    } else {
        format!("{label} — {subtitle}")
    }
}

/// The stored value of the default toggle named `key`, or `None` for a key this
/// page does not offer.
///
/// Addressed by name because the reference addresses it by name, and because it
/// is what makes [`DEFAULT_TOGGLES`] the only place the set of thirteen is
/// written down: a fourteenth field on [`Settings`] is invisible here until it
/// is added to the table, and a table entry with no field is a compile error
/// rather than a switch that silently reads `false`.
pub fn toggle_value(settings: &Settings, key: &str) -> Option<bool> {
    Some(match key {
        "mangohud" => settings.default_mangohud,
        "gamemode" => settings.default_gamemode,
        "prefer_sdl" => settings.default_prefer_sdl,
        "esync" => settings.default_esync,
        "fsync" => settings.default_fsync,
        "dxvk" => settings.default_dxvk,
        "vkd3d" => settings.default_vkd3d,
        "nvapi" => settings.default_nvapi,
        "fsr" => settings.default_fsr,
        "battleye" => settings.default_battleye,
        "eac" => settings.default_eac,
        "gamescope" => settings.default_gamescope,
        "virtual_desktop" => settings.default_virtual_desktop,
        _ => return None,
    })
}

/// Set the default toggle named `key`, reporting whether anything changed.
///
/// The return value is not decoration. The reference writes only when the value
/// actually differs (`bridge.py:267`), and `SetDefaultToggle` arrives from a
/// switch that fires on every interaction — so "the write happened" and "the
/// value moved" are different questions, and the caller is the one that can tell
/// them apart. Returning `false` for an unknown key is the reference's own
/// behaviour (`name in _DEFAULTED_TOGGLES` is the guard) and is why this returns
/// a `bool` rather than asserting.
pub fn set_toggle(settings: &mut Settings, key: &str, value: bool) -> bool {
    let Some(slot) = toggle_slot(settings, key) else {
        return false;
    };
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

/// The field [`set_toggle`] writes, by name.
///
/// Split out so the read and the write can never disagree about which field a
/// key means: [`toggle_value`] reads through this too if it is ever worth
/// unifying them, and the compiler checks the borrow is sound.
fn toggle_slot<'a>(settings: &'a mut Settings, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "mangohud" => &mut settings.default_mangohud,
        "gamemode" => &mut settings.default_gamemode,
        "prefer_sdl" => &mut settings.default_prefer_sdl,
        "esync" => &mut settings.default_esync,
        "fsync" => &mut settings.default_fsync,
        "dxvk" => &mut settings.default_dxvk,
        "vkd3d" => &mut settings.default_vkd3d,
        "nvapi" => &mut settings.default_nvapi,
        "fsr" => &mut settings.default_fsr,
        "battleye" => &mut settings.default_battleye,
        "eac" => &mut settings.default_eac,
        "gamescope" => &mut settings.default_gamescope,
        "virtual_desktop" => &mut settings.default_virtual_desktop,
        _ => return None,
    })
}

/// The message a colour-scheme selection produces, from the selector's index.
///
/// A named function rather than a closure in [`view`] for the reason
/// [`super::library::category_selection`] gives: the callback's parameter is an
/// **index**, and an index silently converted to a string is a filter that
/// matches nothing — a defect a test can only see if the mapping is reachable
/// without a renderer.
pub fn color_scheme_selection(index: usize) -> Message {
    Message::SetColorScheme(
        COLOR_SCHEME_OPTIONS
            .get(index)
            .map(|(key, _)| (*key).to_string())
            .unwrap_or_else(|| COLOR_SCHEMES[0].to_string()),
    )
}

/// The message a layout selection produces, from the selector's index.
pub fn view_mode_selection(index: usize) -> Message {
    Message::SetViewMode(
        VIEW_MODE_OPTIONS
            .get(index)
            .map(|(key, _)| (*key).to_string())
            .unwrap_or_else(|| VIEW_MODES[0].to_string()),
    )
}

/// The message a runner selection produces, from the selector's index.
///
/// The fallback is `System Wine`'s id rather than an empty string, matching the
/// reference's `index >= 0 ? index : 0` collapse: an out-of-range index selects
/// the same entry [`default_runner_index`] falls back to.
pub fn default_runner_selection(choices: &[(String, String)], index: usize) -> Message {
    Message::SetDefaultRunner(
        choices
            .get(index)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| gamehandler_core::models::SYSTEM_WINE.to_string()),
    )
}

/// The message a default-toggle switch produces.
pub fn toggle_selection(key: &str, value: bool) -> Message {
    Message::SetDefaultToggle {
        name: key.to_string(),
        value,
    }
}

/// The Settings page.
///
/// Borrows rather than owns, and is taken **by value** by [`view`] for the
/// reason `LibraryPage` is: the returned `Element` borrows from the caller's
/// `State`, so the page cannot outlive the state it draws.
pub struct SettingsPage<'a> {
    pub settings: &'a Settings,
    pub runners: &'a RunnerManager,
}

/// The row label and the selector it names, as a pair that cannot drift.
///
/// **The keyboard step and the accessible name both come from here, and that is
/// the point.** [`a11y::dropdown`] needs (a) a name for the node it publishes —
/// which is the visible form label, the same string `row` puts beside the
/// control — and (b) a closure that turns Up/Down into the message the selector
/// itself would publish for the neighbouring choice. Taking both from one call
/// is what stops the two paths from disagreeing: a keyboard step that called a
/// different selection function than the pointer's own `on_selected` closure is a
/// control that behaves differently depending on how it is used, and nothing
/// about the widget tree would say so.
///
/// The step is `None` at either end of the list and when the stored value matches
/// no entry (see [`color_scheme_index`]): there is no neighbouring choice to move
/// to, and `None` leaves the arrow uncaptured rather than swallowing it.
///
/// The name is also the widget's *id* (`a11y::stable_id`), so the three labels
/// this is called with must differ. They are three distinct settings, so they do.
fn selector<'a>(
    label: &'a str,
    selected: Option<usize>,
    selections: Vec<String>,
    selection: impl Fn(usize) -> Message + Send + Sync + Clone + 'static,
) -> Element<'a, Message> {
    let count = selections.len();
    let shown = selected.and_then(|index| selections.get(index).cloned());
    // The one function twice: `Clone` on the parameter is what lets the same
    // value be both the toolkit's `on_selected` and the keyboard step, rather
    // than one call site passing a named function and another a closure that
    // quietly differs.
    let pointer = selection.clone();
    a11y::dropdown(
        cosmic::widget::dropdown(selections, selected, pointer),
        label,
        shown,
        move |delta| {
            let next = selected?.checked_add_signed(delta as isize)?;
            (next < count).then(|| selection(next))
        },
    )
    .into()
}

pub fn view<'a>(page: SettingsPage<'a>) -> Element<'a, Message> {
    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- Appearance --------------------------------------------------------
    body = body
        .push(settings_section(SECTION_APPEARANCE))
        .push(settings_row(
            LABEL_COLOR_SCHEME,
            selector(
                LABEL_COLOR_SCHEME,
                color_scheme_index(&page.settings.color_scheme),
                color_scheme_labels(),
                color_scheme_selection,
            ),
        ))
        .push(settings_row(
            LABEL_LAYOUT,
            selector(
                LABEL_LAYOUT,
                view_mode_index(&page.settings.view_mode),
                view_mode_labels(),
                view_mode_selection,
            ),
        ));

    // ---- New games ---------------------------------------------------------
    //
    // The runner selector is the one whose step cannot be `selected ± 1` off the
    // *stored* value, because its fallback is not "no selection": an id that is
    // not in the list falls back to index 0 (see [`default_runner_index`]), so
    // the step has to count from the index the control is actually showing.
    let choices = page.runners.choices();
    let runner_labels = runner_labels(&choices);
    let runner_shown = default_runner_index(&choices, &page.settings.default_runner);
    body = body
        .push(settings_section(SECTION_NEW_GAMES))
        .push(settings_row(
            LABEL_DEFAULT_RUNNER,
            a11y::dropdown(
                cosmic::widget::dropdown(runner_labels.clone(), Some(runner_shown), {
                    let choices = choices.clone();
                    move |index| default_runner_selection(&choices, index)
                }),
                LABEL_DEFAULT_RUNNER,
                runner_labels.get(runner_shown).cloned(),
                {
                    let choices = choices.clone();
                    let count = runner_labels.len();
                    move |delta| {
                        let next = runner_shown.checked_add_signed(delta as isize)?;
                        (next < count).then(|| default_runner_selection(&choices, next))
                    }
                },
            )
            .into(),
        ));

    // ---- The thirteen defaults ---------------------------------------------
    for (key, label, subtitle) in DEFAULT_TOGGLES {
        // `toggle_value` is `Some` for every key in the table, and the `unwrap_or`
        // is the compiler's half of that: the table and the match are checked
        // against each other by `every_toggle_in_the_table_reads_and_writes`.
        let checked = toggle_value(page.settings, key).unwrap_or(false);
        let text = toggle_label(label, subtitle);
        // Bound before `.into()`: an `Accessible` and an `Element` both convert
        // into an `Element`, so the bare `into()` that pushes the other rows is
        // ambiguous here.
        let control: Element<'a, Message> = a11y::toggler(
            toggler(checked)
                .label(text.clone())
                .on_toggle(move |value| toggle_selection(key, value))
                .width(Length::Fill),
            // The switch's accessible name is the same composed string it
            // paints, so what a screen reader announces is what the label
            // beside it says. `text` is cloned here rather than composed twice:
            // two calls to `toggle_label` are two strings that can drift.
            text,
            checked,
            // The keyboard path's message is the pointer path's for the flipped
            // state, built from the same table and the same function the
            // `on_toggle` above uses. `Some`, and not an `Option` chosen here:
            // every row of this table is live on every platform — the
            // `on_toggle` three lines up is applied unconditionally — so there
            // is no inert case for this call site to express. `view::form` is
            // the one that has them.
            Some(toggle_selection(key, !checked)),
        )
        .into();
        body = body.push(control);
    }

    // ---- Behavior ----------------------------------------------------------
    //
    // The reference's close-on-launch control is a `QQC2.Switch` with **two**
    // strings on it (`SettingsPage.qml:120-125`): `FormData.label` — the form
    // label, "Hide window when launching:" — and `text`, which is what the
    // switch itself renders. This port had the `text` half only, so the label
    // was declared, passed nothing, and drawn nowhere; `settings_row()` is what the
    // three appearance rows already use for a form label, so this is the same
    // shape rather than a new one.
    //
    // It also puts one of this page's fourteen previously-unobservable strings
    // back inside the render instrument: a `Toggler`'s own label is painted with
    // a direct `text::draw` and reaches no `Text` widget (see the note on the
    // drawn-strings test), but a label passed to `settings_row()` is a real `text::body`
    // child and `drawn_strings` sees it. The explanation stays on the toggler,
    // where the reference puts it.
    body = body
        .push(settings_section(SECTION_BEHAVIOR))
        .push(settings_row(
            CLOSE_ON_LAUNCH_LABEL,
            a11y::toggler(
                toggler(page.settings.close_on_launch)
                    .label(CLOSE_ON_LAUNCH_EXPLANATION.to_string())
                    .on_toggle(Message::SetCloseOnLaunch)
                    .width(Length::Fill),
                // The name is the form label, not the explanation: the label is
                // what the user reads this control *as*, and it is the string
                // `row` draws beside the switch. The explanation is the switch's
                // own text and stays where the reference puts it.
                CLOSE_ON_LAUNCH_LABEL,
                page.settings.close_on_launch,
                // Live unconditionally, like every switch on this page.
                Some(Message::SetCloseOnLaunch(!page.settings.close_on_launch)),
            )
            .into(),
        ))
        .push(settings_section(SECTION_SHORTCUTS));

    for (keys, what) in SHORTCUTS {
        body = body.push(settings_row(keys, text::body(what).into()));
    }

    container(scrollable(body)).padding(18).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::testkit;

    /// The keys the table offers, in order.
    fn table_keys() -> Vec<&'static str> {
        DEFAULT_TOGGLES.iter().map(|(key, _, _)| *key).collect()
    }

    /// **Every key in the table reads and writes, and the two agree.**
    ///
    /// The table and the two `match`es are three lists of the same thirteen
    /// names, and this is what holds them together: a key added to the table
    /// without a field is an `unwrap_or(false)` in the view (silently off, which
    /// is exactly the shape #46 taught us to refuse), and a field without a
    /// table entry is a setting the user cannot reach.
    #[test]
    fn every_toggle_in_the_table_reads_and_writes() {
        for key in table_keys() {
            let mut settings = Settings::default();
            let before = toggle_value(&settings, key)
                .unwrap_or_else(|| panic!("{key} is in the table but has no field"));

            assert!(
                set_toggle(&mut settings, key, !before),
                "{key} should report a change when it is written to a new value"
            );
            assert_eq!(
                toggle_value(&settings, key),
                Some(!before),
                "{key} should read back what was written"
            );
        }
    }

    /// **An unknown key is refused rather than silently accepted**, and a write
    /// of the stored value reports no change.
    ///
    /// # Which half is a real check
    ///
    /// The **unknown-key** half is: `toggle_slot`'s `_ => return None` arm was
    /// mutated to hand back `default_mangohud` instead, and this test failed —
    /// so a name that is not one of the thirteen provably reaches no field.
    /// `wayland` is the case that matters, because the reference names it in
    /// `_TOGGLE_FIELDS` and deliberately excludes it from the defaults
    /// (`bridge.py:79-82`), which makes it the mistake a reader is most likely
    /// to make.
    ///
    /// The **same-value** half is not evidence of the reference's compare
    /// (`bridge.py:267`): `set_toggle` returning `false` for "already this
    /// value" is unobservable from outside — the setting is unchanged either
    /// way, and `set_toggle` is not the state. Measured: deleting the compare
    /// leaves this test green. The compare is kept for fidelity, and the
    /// assertions below state the value, not the guard.
    #[test]
    fn a_repeated_write_changes_nothing_and_an_unknown_key_is_refused() {
        let mut settings = Settings::default();
        let known = "mangohud";
        let current = toggle_value(&settings, known).expect("in the table");

        // "Reports no change" — not "the compare ran". See the doc above.
        assert!(
            !set_toggle(&mut settings, known, current),
            "writing the stored value must report no change, not a change"
        );
        assert_eq!(toggle_value(&settings, known), Some(current));

        assert!(!set_toggle(&mut settings, "wayland", true));
        assert_eq!(
            toggle_value(&settings, "wayland"),
            None,
            "`wayland` is deliberately not a defaulted toggle (bridge.py:79-82)"
        );
        assert!(!set_toggle(&mut settings, "", true));
        assert_eq!(toggle_value(&settings, ""), None);
    }

    /// **The table is the reference's thirteen, in the reference's order.**
    ///
    /// Read off the QML at test time rather than compared to a second copy, for
    /// the reason this port's other transcription checks give: a list verified
    /// against itself cannot detect a wrong list. `SettingsPage.qml` is the
    /// specification and it is in the tree.
    #[test]
    fn the_toggles_are_the_reference_pages_toggles_in_order() {
        let text = settings_qml();

        let rows = text
            .split_once("readonly property var defaultRows: [")
            .and_then(|(_, rest)| rest.split_once("\n    ]"))
            .map(|(block, _)| block)
            .expect("SettingsPage.qml should have a `defaultRows` block");

        let keys: Vec<&str> = rows
            .lines()
            .filter_map(|line| line.split_once("key: "))
            .filter_map(|(_, rest)| rest.split_once('"').map(|(_, after)| after))
            .filter_map(|rest| rest.split_once('"').map(|(key, _)| key))
            .collect();

        assert_eq!(
            keys,
            table_keys(),
            "the port's default toggles must be the reference's, in the \
             reference's order: the switches are drawn in this order and the \
             user sees it (P-66)"
        );
    }

    /// **The page's shortcut rows are the reference's**, read off the QML.
    ///
    /// Independent source, not a second copy: `SettingsPage.qml:132-135` is the
    /// specification and it is in the tree. A row this page invented, or one it
    /// dropped, shows up here.
    #[test]
    fn the_shortcut_rows_are_the_reference_pages_rows() {
        let text = settings_qml();

        let rows: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("QQC2.Label {"))
            .filter_map(|rest| rest.split_once("FormData.label: \""))
            .filter_map(|(_, rest)| rest.split_once('"').map(|(keys, _)| keys))
            .collect();

        let ours: Vec<&str> = SHORTCUTS.iter().map(|(keys, _)| *keys).collect();
        assert_eq!(
            rows, ours,
            "the shortcut rows must be the reference's, in order (P-66/P-68)"
        );
    }

    /// **Every row the page prints is either implemented or recorded as not
    /// being — and the record cannot go stale in either direction.**
    ///
    /// This is the test that caught the real state: the page lists four
    /// shortcuts, the shell implements none, and nothing said so. It fails in
    /// both directions, which is what makes it a check rather than a comment:
    ///
    /// - if the shell gains a keyboard subscription while
    ///   [`IMPLEMENTED_SHORTCUTS`] is still empty, the first assertion fires and
    ///   says to fill the record in;
    /// - if a row is added to [`SHORTCUTS`] that is in neither list, the second
    ///   fires.
    ///
    /// # The "does the shell handle keys at all" half no longer reads `main.rs`
    ///
    /// It did, and `BUG-12` is why it must not: `main.rs.contains("fn subscription")
    /// && …contains("keyboard")` is a source-text check standing in for a
    /// behaviour check, and it passed for the whole time three of the four
    /// accelerators were dead while a text field had focus — the word `keyboard`
    /// was in the file and the key was not delivered. It is not that the grep was
    /// too weak; it is that no grep can answer this.
    ///
    /// The answer is measured now, in
    /// [`every_row_this_page_prints_is_a_key_the_window_answers`], which drives
    /// each row's own key through the real subscriptions and reads the messages
    /// back. What is left here is the accounting those rows must satisfy — a
    /// property of the two lists rather than of the shell — and the two are
    /// deliberately separate, because a test that both measured the shell and
    /// checked the list could satisfy itself by editing the list.
    #[test]
    fn the_page_does_not_claim_a_shortcut_the_shell_does_not_implement() {
        let mut accounted: Vec<&str> = IMPLEMENTED_SHORTCUTS.to_vec();
        accounted.extend(UNWIRED_SHORTCUTS);
        let mut accounted_sorted = accounted.clone();
        accounted_sorted.sort_unstable();
        accounted_sorted.dedup();
        assert_eq!(
            accounted.len(),
            accounted_sorted.len(),
            "a shortcut is in both IMPLEMENTED_SHORTCUTS and UNWIRED_SHORTCUTS: {accounted:?}"
        );

        let mut page: Vec<&str> = SHORTCUTS.iter().map(|(keys, _)| *keys).collect();
        page.sort_unstable();
        assert_eq!(
            accounted_sorted, page,
            "every row the Settings page prints must be either implemented or \
             recorded as unwired. A row in neither is a claim the port cannot \
             back (P-68)"
        );
    }

    /// **Each of the four rows is a key the window really answers, whichever
    /// widget has the focus.**
    ///
    /// This is the check `BUG-12` asked for, and it replaces the two that could
    /// not give it. The predecessor asserted `main.rs.contains("fn subscription")
    /// && …contains("keyboard")` — a source-text check standing in for a
    /// behaviour check — and it passed for the whole time three of the four
    /// accelerators were dead while a text field had focus. A word appearing in
    /// a file is not a key being delivered, and no grep closes that gap.
    ///
    /// So this measures, three layers down and with no source read anywhere:
    ///
    /// 1. **the row's own key is parsed out of [`SHORTCUTS`]** — the page's own
    ///    data, so a row whose key changed is driven as its new key rather than
    ///    as a restatement of the old one;
    /// 2. **it is handed to the subscriptions that run in a window**, as a real
    ///    runtime event, in *both* statuses a widget traversal can leave: the
    ///    shell's ([`crate::shortcuts::subscription`], which is what
    ///    `App::subscription` returns) and libcosmic's own
    ///    `keyboard_nav::subscription()`, which the framework composes in
    ///    (`src/app/cosmic.rs:674`) and which routes `Ctrl+F` to
    ///    `Application::on_search`. `Ignored` is the unfocused case and
    ///    `Captured` the one a focused text field leaves — the case that was
    ///    broken, and the reason a test that only drove `Ignored` would have
    ///    passed throughout;
    /// 3. **exactly one of the two answers each time**, and when the shell's does
    ///    it is the reference's own message.
    ///
    /// The "exactly one" half is not decoration. It is the double-fire check:
    /// `Ctrl+F` is bound by the framework *and* by the shell's new arm, and the
    /// two are exclusive only because they gate on opposite statuses. A test that
    /// counted the messages from the shell alone would be satisfied by binding
    /// `Ctrl+F` twice and navigating twice for one press.
    ///
    /// What this cannot reach is the window itself: that winit delivers the key
    /// to the runtime, and that `on_search`'s focus operation lands on the search
    /// box. Those stay T-19's to walk in the Flatpak, as P-68's acceptance says.
    #[test]
    fn every_row_this_page_prints_is_a_key_the_window_answers() {
        use crate::Page;
        use crate::shortcuts;
        use cosmic::iced::event::Status;
        use cosmic::iced::keyboard::Modifiers;

        /// What the row's key must produce, and where from.
        ///
        /// `Ctrl+F` is the odd one: the framework answers it when the event is
        /// ignored, and the shell answers it when it is not, so both halves are
        /// named and exactly one must fire.
        struct Answer {
            row: &'static str,
            character: &'static str,
            reference: &'static str,
            is_it: fn(&Message) -> bool,
        }

        let answers = [
            Answer {
                row: "Ctrl+N:",
                character: "n",
                reference: "`root.openGameForm(\"\")` (Main.qml:121-125)",
                is_it: |message| matches!(message, Message::OpenNewGameForm),
            },
            Answer {
                row: "Ctrl+F:",
                character: "f",
                reference: "`root.showPage(\"library\"); focusSearch()` (Main.qml:126-134)",
                is_it: |message| matches!(message, Message::FocusLibrarySearch),
            },
            Answer {
                row: "Ctrl+,:",
                character: ",",
                reference: "`root.showPage(\"settings\")` (Main.qml:135-139)",
                is_it: |message| matches!(message, Message::NavigateTo(Page::Settings)),
            },
            Answer {
                row: "Ctrl+Q:",
                character: "q",
                reference: "`backend.quit()` (Main.qml:140-143)",
                is_it: |message| matches!(message, Message::Quit),
            },
        ];

        // Every row the page prints is driven, and none is invented here: the
        // table's rows must be the page's rows, in the page's order.
        let rows: Vec<&str> = SHORTCUTS.iter().map(|(keys, _)| *keys).collect();
        let driven: Vec<&str> = answers.iter().map(|answer| answer.row).collect();
        assert_eq!(
            driven, rows,
            "this guard must drive the keys the page prints, or it is measuring \
             a list of its own"
        );

        for answer in &answers {
            for status in [Status::Ignored, Status::Captured] {
                // A real press: `text` carries the character a keyboard would
                // type, which is what a focused field reads before deciding
                // whether to insert, and Control is held — as it is for every
                // accelerator here.
                let event = shortcuts::pressed_event(
                    answer.character,
                    Modifiers::CTRL,
                    Some(answer.character),
                    false,
                );

                let shell =
                    shortcuts::messages_for(shortcuts::subscription(), event.clone(), status);
                let framework =
                    shortcuts::messages_for(cosmic::keyboard_nav::subscription(), event, status);

                assert_eq!(
                    shell.len() + framework.len(),
                    1,
                    "`{}` — {} — produced {} message(s) from the shell and {} from \
                     libcosmic when the key arrived {status:?}. Exactly one of the \
                     two must answer: none is the accelerator being dead, and two \
                     is it firing twice for one press. Shell: {shell:?}, \
                     framework: {framework:?}",
                    answer.row,
                    answer.reference,
                    shell.len(),
                    framework.len()
                );

                if !shell.is_empty() {
                    assert!(
                        matches!(shell.as_slice(), [only] if (answer.is_it)(only)),
                        "`{}` — {} — produced {shell:?} rather than the \
                         reference's message",
                        answer.row,
                        answer.reference
                    );
                } else {
                    // The framework's half, which is only ever `Ctrl+F`: pinned
                    // as the `Action` `Cosmic::update` turns into `on_search`, so
                    // that a change in what the framework emits is a failure here
                    // rather than a row that quietly stops navigating.
                    assert!(
                        matches!(framework.as_slice(), [cosmic::keyboard_nav::Action::Search]),
                        "`{}` was answered by libcosmic rather than by the shell, \
                         and the framework must have answered it with \
                         `Action::Search` — the action `Cosmic::update` routes to \
                         `Application::on_search`. Got {framework:?}",
                        answer.row
                    );
                }
            }
        }
    }

    /// **The record beside the list says where the behaviour lives.**
    ///
    /// [`IMPLEMENTED_SHORTCUTS`] used to carry a printed caveat under the rows
    /// claiming three of the four did not fire while a text field had focus. That
    /// stopped being true when the shell's subscription started answering every
    /// status, and a page apologising for a defect it no longer has is the same
    /// lie as one silent about a defect it does — so the caveat is gone and this
    /// holds the record that replaced it: the constant's own doc must name
    /// [`crate::shortcuts::subscription`], so a reader of the list finds where
    /// the four are answered rather than a claim about their limits.
    ///
    /// It reads the source because the property *is* about the documentation,
    /// which has no runtime representation — the one case where a source read is
    /// the honest instrument rather than a substitute for one.
    #[test]
    fn the_record_beside_the_rows_names_where_they_are_answered() {
        let source = include_str!("settings.rs");
        let doc = source
            .split("pub const IMPLEMENTED_SHORTCUTS")
            .next()
            .expect("the constant must exist");
        // The last doc block before the constant, which is the one a reader of
        // it sees.
        let doc = doc
            .rsplit("/// Every row [`SHORTCUTS`] prints")
            .next()
            .unwrap_or(doc);
        assert!(
            doc.contains("shortcuts::subscription"),
            "`IMPLEMENTED_SHORTCUTS`' doc no longer names the subscription that \
             answers the four accelerators, so a reader of the list cannot tell \
             where to look when one of them stops working — and `BUG-12` is what \
             happens when that gap is filled with a reading of the framework \
             instead of a measurement of it"
        );
        assert!(
            !doc.contains("do not fire while a text field has focus"),
            "`IMPLEMENTED_SHORTCUTS`' doc still claims the accelerators do not \
             fire while a text field has focus. They do: the subscription answers \
             every status, and `every_row_this_page_prints_is_a_key_the_window_\
             answers` is what measures it. A stale caveat here is how the page \
             would come to print one again"
        );
    }

    /// The reference's separator, exactly: an em dash between two spaces.
    #[test]
    fn the_toggle_label_is_the_references_wording() {
        assert_eq!(
            toggle_label("Enable DXVK by default", "Direct3D 8–11 through Vulkan."),
            "Enable DXVK by default — Direct3D 8–11 through Vulkan."
        );
        assert_eq!(
            toggle_label("Enable MangoHud by default", ""),
            "Enable MangoHud by default",
            "a subtitle-less row is the label alone, with no trailing separator"
        );
    }

    /// This page, built the way the shell builds it.
    fn page_strings() -> Vec<String> {
        let settings = Settings::default();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        testkit::drawn_strings(view(SettingsPage {
            settings: &settings,
            runners: &runners,
        }))
    }

    /// `SettingsPage.qml`, as text — the independent source three of the tests
    /// below read their expectations out of.
    fn settings_qml() -> String {
        gamehandler_core::oracle_support::repo_file("gamehandler/qml/SettingsPage.qml")
    }

    /// The reference's `defaultRows` as `(key, label, subtitle)` triples.
    ///
    /// Read from the QML rather than restated, for the reason every other
    /// transcription check in this port gives: a list compared against a second
    /// copy of itself cannot detect a wrong list. The block is a line per row —
    /// `{ key: "…", label: "…", subtitle: "…" },` — and neither a label nor a
    /// subtitle contains a `"`, so the three components come out by successive
    /// `split_once`.
    fn reference_toggle_rows() -> Vec<(String, String, String)> {
        let text = settings_qml();
        let block = text
            .split_once("readonly property var defaultRows: [")
            .and_then(|(_, rest)| rest.split_once("\n    ]"))
            .map(|(block, _)| block)
            .expect("SettingsPage.qml should have a `defaultRows` block");

        let rows: Vec<(String, String, String)> = block
            .lines()
            .filter(|line| line.contains("key: \""))
            .filter_map(|line| {
                let (_, after_key) = line.split_once("key: \"")?;
                let (key, after_key) = after_key.split_once('"')?;
                let (_, after_label) = after_key.split_once("label: \"")?;
                let (label, after_label) = after_label.split_once('"')?;
                let (_, after_subtitle) = after_label.split_once("subtitle: \"")?;
                let (subtitle, _) = after_subtitle.split_once('"')?;
                Some((key.to_string(), label.to_string(), subtitle.to_string()))
            })
            .collect();

        assert!(
            !rows.is_empty(),
            "the `defaultRows` block parsed to nothing, so every assertion \
             built on it would pass vacuously"
        );
        rows
    }

    /// **The whole row is the reference's — key, label and subtitle.**
    ///
    /// This is the check T-27 found missing. The test above compares **keys
    /// only**: it parses `key: "…"` out of the QML and stops, so a typo'd label,
    /// a dropped subtitle, or two rows' subtitles swapped were all invisible to
    /// the entire suite. That is not hypothetical bookkeeping — the strings are
    /// what the user reads on every switch, and the page's own drawn-strings
    /// test cannot see them either, because a `Toggler` paints its label without
    /// building a `Text` child (see that test's note). So before this test the
    /// thirteen labels and subtitles rested on *nothing*: the formatting test
    /// above exercises `toggle_label` with two strings of its own invention, and
    /// never asks whether the table holds the reference's words.
    ///
    /// Equality, not containment, and therefore two-way: a row the port invented
    /// fails here as loudly as one it dropped.
    #[test]
    fn the_toggle_rows_carry_the_references_wording() {
        let reference = reference_toggle_rows();
        let ours: Vec<(String, String, String)> = DEFAULT_TOGGLES
            .iter()
            .map(|(key, label, subtitle)| {
                (
                    (*key).to_string(),
                    (*label).to_string(),
                    (*subtitle).to_string(),
                )
            })
            .collect();

        assert_eq!(
            reference, ours,
            "the port's thirteen rows must be the reference's, wording included \
             (P-66): the keys are the model's field names and the label and \
             subtitle are the text the user reads"
        );
        assert_eq!(ours.len(), 13, "the reference ships thirteen default rows");
    }

    /// **Every row's drawn label is the reference's own expression**, not just
    /// the two rows the formatting test happens to name.
    ///
    /// The reference builds a switch's text with a ternary
    /// (`SettingsPage.qml:104-106`):
    ///
    /// ```text
    /// text: modelData.subtitle
    ///     ? modelData.label + " — " + modelData.subtitle
    ///     : modelData.label
    /// ```
    ///
    /// Both branches are asserted here for **all thirteen rows**, which is the
    /// strengthening: the existing test covers one row with a subtitle and one
    /// without, so a `toggle_label` rewritten as an unconditional
    /// `format!("{label} — {subtitle}")` would have passed it while putting a
    /// dangling `" — "` on the **seven** subtitle-less rows. The reference's
    /// ternary is the whole reason that cannot happen, and seven failures is
    /// what it costs to break it.
    ///
    /// The separator is compared as the reference writes it — an em dash between
    /// two spaces — so a hyphen or a bare dash fails on the six rows that carry a
    /// subtitle.
    #[test]
    fn every_toggle_rows_label_is_the_references_own_expression() {
        let mut with_subtitle = 0;
        let mut without_subtitle = 0;

        for (key, label, subtitle) in reference_toggle_rows() {
            let drawn = toggle_label(&label, &subtitle);

            if subtitle.is_empty() {
                without_subtitle += 1;
                assert_eq!(
                    drawn, label,
                    "{key}: the reference's ternary yields the label alone when \
                     there is no subtitle, so a separator here is a dangling \
                     `\" — \"` on a row that has nothing to separate"
                );
            } else {
                with_subtitle += 1;
                assert_eq!(
                    drawn,
                    format!("{label} — {subtitle}"),
                    "{key}: the reference joins label and subtitle with an em \
                     dash between two spaces (SettingsPage.qml:104-106)"
                );
                assert_ne!(drawn, label, "{key}: the subtitle was dropped");
            }
        }

        // Both branches must be exercised, or "the ternary holds" is a claim
        // about whichever branch the table happens to contain — and a table that
        // lost every subtitle would pass the subtitle branch vacuously.
        //
        // The floor is "each branch is non-empty", *not* a hardcoded count of
        // how many rows carry a subtitle: that number is the reference's content
        // and it is already pinned, row by row, by
        // [`the_toggle_rows_carry_the_references_wording`]. Writing it a second
        // time here would be a hand-copied fact that no assertion keeps true —
        // and this test's first draft did exactly that, asserting 7 and 6 from
        // memory where the measured split is 8 and 5.
        assert!(
            with_subtitle > 0,
            "no row carries a subtitle, so the separator branch is untested"
        );
        assert!(
            without_subtitle > 0,
            "every row carries a subtitle, so the reference's bare-label branch \
             is untested — and it is the branch that catches an unconditional \
             separator"
        );
        assert_eq!(
            with_subtitle + without_subtitle,
            13,
            "every row must take exactly one branch"
        );
    }

    /// **The close-on-launch switch carries the reference's two strings**, read
    /// out of the QML.
    ///
    /// The reference puts two different strings on that one control
    /// (`SettingsPage.qml:120-125`): `FormData.label`, which is the form label,
    /// and `text`, which is what the switch itself renders. `CLOSE_ON_LAUNCH_LABEL`
    /// was declared in this module and referenced **nowhere** — not by the view,
    /// not by any test — so the reference's form label was simply absent from
    /// the page while the constant sat in the source. This test is what found
    /// that, and it is why the constant is now passed to `settings_row()`.
    ///
    /// The two are asserted separately rather than as a pair, because they are
    /// drawn by different mechanisms and can fail independently: the explanation
    /// reaches a `Toggler` label and no tree walk sees it, while the label goes
    /// to `settings_row()` and becomes a `text::body` child that does. Asserting them
    /// together would hide which one moved.
    #[test]
    fn the_close_on_launch_switch_carries_the_references_two_strings() {
        let text = settings_qml();
        // Anchored on `backend.closeOnLaunch`, which appears on that switch and
        // nowhere else, rather than on the label text: an anchor that is also the
        // value under test can only agree with itself. The first draft searched
        // for the label it was looking for and then searched again inside the
        // result, which walked past the switch and picked up the *next* form
        // label in the file — `Keyboard shortcuts`.
        let block = text
            .split("QQC2.Switch {")
            .find(|chunk| chunk.contains("backend.closeOnLaunch"))
            .expect(
                "SettingsPage.qml should still have a close-on-launch `QQC2.Switch` \
                 bound to `backend.closeOnLaunch`; if it was renamed, this anchor \
                 and the two constants below must move together",
            );

        let (_, after) = block
            .split_once("Kirigami.FormData.label: \"")
            .expect("the switch's `FormData.label`");
        let (qml_label, after) = after.split_once('"').expect("a closing quote");
        let (_, after) = after
            .split_once("text: \"")
            .expect("the switch's own `text`");
        let (qml_text, _) = after.split_once('"').expect("a closing quote");

        assert_eq!(
            qml_label, CLOSE_ON_LAUNCH_LABEL,
            "the form label is the reference's, verbatim"
        );
        assert_eq!(
            qml_text, CLOSE_ON_LAUNCH_EXPLANATION,
            "the switch's own text is the reference's, verbatim"
        );
        assert_ne!(
            qml_label, qml_text,
            "the two are different strings in the reference; a port that \
             collapsed them would satisfy both assertions above with one value"
        );
    }

    /// **The close-on-launch label is drawn; the explanation is not
    /// observable — measured on the real page, in the same assertion.**
    ///
    /// This is T-27's answer stated as a measurement rather than as prose, and
    /// the two halves are deliberately in one test because the *contrast* is the
    /// finding:
    ///
    /// * the label goes through `settings_row()` → `text::body`, which is a real child
    ///   widget, so `drawn_strings` sees it. That is what makes routing it
    ///   through `settings_row()` a fix and not merely a rearrangement — and it is the
    ///   assertion that fails if someone moves it back onto the toggler;
    /// * the explanation is the `Toggler`'s own `.label(...)`, painted with a
    ///   direct `iced_widget::text::draw` (`libcosmic src/widget/toggler.rs:316`),
    ///   so it reaches no `Text` widget and is absent from the same list.
    ///
    /// A reader who assumes both are covered because they sit on one control is
    /// exactly who this test is for.
    #[test]
    fn the_close_on_launch_label_is_drawn_and_its_explanation_is_not_observable() {
        let drawn = page_strings();

        assert!(
            drawn.iter().any(|text| text == CLOSE_ON_LAUNCH_LABEL),
            "the reference's form label for the close-on-launch switch must be on \
             the page as a real `Text` child. It reaches the traversal through \
             `settings_row()`, so its absence means it stopped being drawn at all — which \
             is the state this test was written to catch. Drawn: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text == CLOSE_ON_LAUNCH_EXPLANATION),
            "if this now lists the explanation, `Toggler` gained a child text \
             widget: update the note on `the_settings_page_draws_the_settings_\
             and_not_the_placeholder` in `main.rs` and this test with it. \
             Drawn: {drawn:?}"
        );
    }

    /// **The thirteen toggle labels are drawn and reach no text operation** —
    /// the bound, measured against the page this time.
    ///
    /// T-27's question was whether the thirteen labels need a second,
    /// non-render assertion. The answer is **yes**, and this test is the reason
    /// that answer is not simply "the render check is missing": a render check
    /// is not *missing*, it is *impossible* for these strings, and this is the
    /// measurement that says so. `the_toggle_rows_carry_the_references_wording`
    /// and `every_toggle_rows_label_is_the_references_own_expression` are that
    /// missing check's replacement — table-level assertions over the data the
    /// widget is built from.
    ///
    /// It is a second measurement rather than a restatement of `main.rs`'s pin:
    /// that one builds a bare `toggler()` and asserts the operation is empty,
    /// which bounds the *widget*. This one renders **the page** and asserts the
    /// thirteen composed labels are absent from it, which bounds the *page* —
    /// and would fail if a future port drew the labels a second time as ordinary
    /// text, which the bare-widget pin cannot see.
    ///
    /// Both directions are asserted: the labels are absent *and* an observable
    /// string is present, so a `view` that returned an empty element — the
    /// cheapest way to satisfy an "is absent" claim — fails here.
    #[test]
    fn the_toggle_labels_reach_no_text_operation_on_the_real_page() {
        let drawn = page_strings();
        let mut composed = 0;

        for (key, label, subtitle) in DEFAULT_TOGGLES {
            let text = toggle_label(label, subtitle);
            composed += 1;
            assert!(
                !drawn.contains(&text),
                "{key}: the composed label reached a `Text` widget. That is not a \
                 failure of the page — it means `Toggler` now builds a child text \
                 and these labels ARE render-observable, so the note in \
                 `main.rs` and the bound on this test are both stale. Drawn: \
                 {drawn:?}"
            );
        }
        assert_eq!(composed, 13, "all thirteen rows were measured");

        assert!(
            drawn.iter().any(|text| text == SECTION_APPEARANCE),
            "the page drew nothing this traversal can see, so the absences above \
             prove nothing — this is the anti-vacuity half. Drawn: {drawn:?}"
        );
    }

    /// This page, built the way the shell builds it, with its tree and layout
    /// kept so an operation and an event can be run against it.
    ///
    /// `page_strings` above builds and drops its element; a `Tree` is where the
    /// framework keeps a widget's state, so a helper that dropped it would leave
    /// the next event looking at an unfocused widget — see
    /// [`super::a11y::harness::tab_to`].
    fn page<'a>(
        settings: &'a Settings,
        runners: &'a RunnerManager,
    ) -> cosmic::Element<'a, Message> {
        view(SettingsPage { settings, runners })
    }

    /// **Every control on the real page is a Tab stop and publishes a named
    /// node** — UX-01, UX-02 and UX-03, measured where the user meets them.
    ///
    /// # Why this is a page test and not another wrapper test
    ///
    /// `view/a11y.rs` proves that the wrapper reports a focusable state and
    /// builds a node. That is a proof about the *wrapper*, and it says nothing
    /// about whether this page uses it: restoring any call site in [`view`] to
    /// the bare `cosmic::widget::toggler` / `cosmic::widget::dropdown` leaves
    /// every one of those ten green, because a wrapper nobody calls still works
    /// perfectly. This is the test that goes red then — and it is the same
    /// division the page already draws against `main.rs`'s bare-`toggler()` pin
    /// in `the_toggle_labels_reach_no_text_operation_on_the_real_page`: that one
    /// bounds the widget, this one bounds the page.
    ///
    /// # What is counted
    ///
    /// Seventeen controls: the three selectors (colour scheme, layout, default
    /// runner), the thirteen default toggles, and close-on-launch. The thirteen
    /// comes from [`DEFAULT_TOGGLES`]`::len()`, so a fourteenth row moves the
    /// expectation with the table and cannot silently pass. The four that are
    /// not in the table are written down here, and that is deliberate: this is
    /// where a *new* control that nobody wrapped would be caught.
    ///
    /// Every stop must report an id, and no two may report the same one. A `None`
    /// is a control Tab can reach and nothing else can address; a repeat is two
    /// controls the framework cannot tell apart. Both are reachable here because
    /// the id is derived from the control's *name* (`a11y::stable_id`), so two
    /// rows sharing a name collapse into one node silently.
    ///
    /// # Why the counts are the assertion and the names are not enough
    ///
    /// A page that wrapped every control and named them all `""` would still
    /// produce seventeen controls' worth of nodes, so the count is what makes
    /// "every control" true rather than "some node exists". The names are
    /// asserted on top of it, each expected name under its expected role exactly
    /// once: the name is the string the row beside the control draws, so this is
    /// what says a screen reader announces the control the way the page labels
    /// it. `count == 1` rather than `>= 1` is what makes a node published twice
    /// — a real hazard, since `A11yTree` is flat and an intermediate widget that
    /// both joined and forwarded its child would list it twice — a failure
    /// rather than a pass.
    ///
    /// The nodes are read through `a11y_nodes`, which is the only entrance the
    /// runtime uses (`iced/runtime/src/user_interface.rs:606-616`), and the whole
    /// tree comes back because `A11yTree` is flat: `root` and `children` are both
    /// `Vec<A11yNode>` (`iced/accessibility/src/a11y_tree.rs:5-10`), so this is
    /// the page and not its first level.
    #[test]
    fn every_control_on_the_real_page_is_a_tab_stop_and_a_named_node() {
        use super::a11y::harness;
        use iced_accessibility::accesskit::Role;

        let settings = Settings::default();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        // **One element, read twice.** The focus reports and the nodes have to
        // come out of the same build for the identity assertion at the end to
        // mean anything: the toolkit's own widgets take `Id::unique()` at
        // construction (`src/widget/button/widget.rs:63`), so two builds of this
        // page disagree about their ids — see `a11y::harness::unreachable_controls`.
        let mut element = page(&settings, &runners);

        // The `selected` assertions further down are only half a test unless the
        // fixture carries both states: a page whose switches were all off would
        // pass against a builder that published a constant `false`, and one whose
        // were all on would pass against a constant `true`. Asserted rather than
        // assumed, so a default that moves says so here instead of quietly
        // weakening those assertions.
        assert!(
            DEFAULT_TOGGLES
                .iter()
                .any(|(key, _, _)| toggle_value(&settings, key) == Some(true))
                && DEFAULT_TOGGLES
                    .iter()
                    .any(|(key, _, _)| toggle_value(&settings, key) == Some(false)),
            "the thirteen defaults must include at least one switch of each state"
        );

        // ---- the Tab ring ---------------------------------------------------
        let stops = harness::focusables(&mut element);
        let expected = DEFAULT_TOGGLES.len() + 4;
        assert_eq!(
            stops.len(),
            expected,
            "the page has {expected} controls — {} default toggles, three \
             appearance selectors, the default runner and close-on-launch — and \
             every one of them must be a Tab stop. A short list means a control \
             is built from the bare toolkit widget, which reports no focusable \
             state at all (`view/a11y.rs`'s \
             `the_toolkit_controls_the_app_used_to_build_are_invisible` measures \
             that directly): {stops:?}",
            DEFAULT_TOGGLES.len()
        );

        let mut ids: Vec<cosmic::widget::Id> = Vec::new();
        for stop in &stops {
            let id = stop.clone().unwrap_or_else(|| {
                panic!(
                    "a control is focusable but reports no id, so Tab can reach \
                     it and nothing can address it: {stops:?}"
                )
            });
            assert!(
                !ids.contains(&id),
                "two controls report the same id {id:?}. The id is derived from \
                 the control's name (`a11y::stable_id`), so this is two rows \
                 sharing a name: a screen reader would see one control where the \
                 user sees two. Ids: {ids:?}"
            );
            ids.push(id);
        }

        // ---- the nodes ------------------------------------------------------
        let nodes = harness::published(&mut element);
        // A control is identified by its name **and** its role, not by its name
        // alone. A row's form label is itself a node: `settings_row()` draws it as a real
        // `text::body` child, and iced's `text` widget publishes a `Paragraph`
        // node carrying that string — visible in the collected `Nodes:` list on
        // any failure below as a `(Paragraph, Some("Color scheme:"))` beside the
        // `(ComboBox, Some("Color scheme:"))` that is the control. Counting by
        // name alone would therefore find two nodes for every control with a
        // form label and one for every control without one, which is a property
        // of the row rather than of the control.
        let named = |label: &str, role: Role| {
            nodes
                .iter()
                .filter(|node| node.label.as_deref() == Some(label) && node.role == role)
                .count()
        };

        // The four controls [`DEFAULT_TOGGLES`] does not name, with the role
        // each must announce as.
        for (label, role, what) in [
            (LABEL_COLOR_SCHEME, Role::ComboBox, "colour-scheme selector"),
            (LABEL_LAYOUT, Role::ComboBox, "layout selector"),
            (
                LABEL_DEFAULT_RUNNER,
                Role::ComboBox,
                "default-runner selector",
            ),
            (
                CLOSE_ON_LAUNCH_LABEL,
                Role::Switch,
                "close-on-launch switch",
            ),
        ] {
            assert_eq!(
                named(label, role),
                1,
                "the {what} must publish exactly one node named {label:?} \
                 announcing as {role:?}. Zero means the control is not wrapped, \
                 or is wrapped but announcing as the wrong thing — a switch that \
                 announces as a button is not a switch to a screen reader; two \
                 means a node is published twice. Nodes: {:?}",
                nodes
                    .iter()
                    .map(|node| (&node.role, node.label.as_deref()))
                    .collect::<Vec<_>>()
            );
        }

        // The thirteen table rows. These are the ones where a name-only count
        // would be right by accident — a `Toggler`'s own text reaches no `Text`
        // widget, which is what `the_toggle_labels_reach_no_text_operation_on_
        // the_real_page` pins — so naming the role here is what makes the
        // assertion about the control rather than about the row.
        for (key, label, subtitle) in DEFAULT_TOGGLES {
            let name = toggle_label(label, subtitle);
            assert_eq!(
                named(&name, Role::Switch),
                1,
                "{key}: the switch must publish exactly one node named {name:?} \
                 — the same composed string it paints (`toggle_label`), so what \
                 is announced is what the row says. Zero means this row is not \
                 wrapped, two means a node is published twice"
            );
        }

        // ---- the state each switch publishes ---------------------------------
        //
        // A name and a role say a switch is *announced*; neither says what it
        // announces is true. A call site that passed a literal `true` for a
        // setting the user has switched off keeps every assertion above green
        // while telling a screen-reader user the opposite of what the page
        // draws, and the failure is silent in both directions. `selected` is the
        // field that carries the state (`view/a11y.rs`'s `a11y_nodes`), and this
        // is the only place it is compared against the setting the page was
        // handed.
        //
        // The expected value comes from `toggle_value` — the same reader the
        // call site uses to build both the toggler and the node — so what this
        // asserts is that the two agree, not that a constant has been copied
        // here correctly. The fixture's mix of states is asserted above.
        let switch = |name: &str| {
            nodes
                .iter()
                .find(|node| node.role == Role::Switch && node.label.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("the {name:?} switch is asserted above"))
        };

        for (key, label, subtitle) in DEFAULT_TOGGLES {
            assert_eq!(
                switch(&toggle_label(label, subtitle)).selected,
                toggle_value(&settings, key),
                "{key}: the switch must publish the state the row draws, read off \
                 the same `Settings` the toggler was built from. A constant here \
                 — `checked(true)` or `checked(false)` — is what this catches"
            );
        }
        assert_eq!(
            switch(CLOSE_ON_LAUNCH_LABEL).selected,
            Some(settings.close_on_launch),
            "close-on-launch, likewise. This is also the one switch whose node is \
             *not* named after the string its toggler paints \
             (`CLOSE_ON_LAUNCH_LABEL` against `CLOSE_ON_LAUNCH_EXPLANATION`), so \
             its state is the only field tying the node to the setting it shows"
        );

        // ---- the value each combo publishes --------------------------------
        //
        // A combo's `selected` is `None` on every node — the field carries a
        // *switch*'s on/off, and a dropdown is not a switch — so the assertion
        // above cannot reach these three controls at all, and nothing else does
        // either: [`selector`] is where "which option is selected" becomes the
        // node's `value`, and a call site that passed `None`, a constant, or the
        // wrong list's entry would leave every assertion above green. A screen
        // reader would then announce three combo boxes with no setting in them,
        // which is worse than not announcing them: the user is told there is a
        // control, reaches it, and is told nothing about what it holds.
        //
        // The expected string is read off the *same* readers the call sites use
        // (`color_scheme_index`/`view_mode_index`/`default_runner_index` plus the
        // list each was handed), so what this asserts is that the two agree
        // rather than that a constant has been copied here correctly. The
        // premise that each index actually resolves to a string is asserted too:
        // an index past the end of its list would make the expected value `None`
        // and the assertion would pass over the very defect it is for.
        let choices = runners.choices();
        let runner_options = runner_labels(&choices);
        let runner_shown = default_runner_index(&choices, &settings.default_runner);
        let combo_expected: Vec<(&str, Option<String>)> = vec![
            (
                LABEL_COLOR_SCHEME,
                color_scheme_index(&settings.color_scheme)
                    .and_then(|index| color_scheme_labels().get(index).cloned()),
            ),
            (
                LABEL_LAYOUT,
                view_mode_index(&settings.view_mode)
                    .and_then(|index| view_mode_labels().get(index).cloned()),
            ),
            (
                LABEL_DEFAULT_RUNNER,
                runner_options.get(runner_shown).cloned(),
            ),
        ];
        for (label, expected) in &combo_expected {
            assert!(
                expected.is_some(),
                "the {label:?} selector resolves to no option on this fixture, so \
                 the value assertion below would compare `None` with `None` and \
                 pass having checked nothing. Fixture: colour scheme {:?}, view \
                 mode {:?}, default runner {:?}",
                settings.color_scheme,
                settings.view_mode,
                settings.default_runner
            );
        }
        let combo = |label: &str| {
            nodes
                .iter()
                .find(|node| node.role == Role::ComboBox && node.label.as_deref() == Some(label))
                .unwrap_or_else(|| panic!("the {label:?} combo is asserted above"))
        };
        for (label, expected) in &combo_expected {
            assert_eq!(
                combo(label).value.as_deref(),
                expected.as_deref(),
                "the {label:?} selector publishes the value {:?}, but the option it \
                 is showing is {expected:?}. This is the assertion a `selected` \
                 argument dropped from the `a11y::dropdown` call site — or \
                 replaced by a constant, or by another list's entry — fails: a \
                 node whose name and role are right and whose value is wrong \
                 tells a screen-reader user the setting is something it is not. \
                 Every combo node this page publishes: {:?}",
                combo(label).value,
                nodes
                    .iter()
                    .filter(|node| node.role == Role::ComboBox)
                    .map(|node| (node.label.as_deref(), node.value.as_deref()))
                    .collect::<Vec<_>>()
            );
        }

        // ---- every node id is an id the Tab ring reports ---------------------
        //
        // What the wrapper first shipped with was neither a missing node nor a
        // missing tab stop: it was the two halves of one control naming
        // *different* things — the field's node under one id and its focus
        // report under another — which leaves assistive technology able to see
        // the control and unable to reach it. Nothing above can catch that,
        // because the count, the names, the roles and the states are all still
        // right. `view/form.rs`'s page test is what found it there; this is the
        // same assertion on this page, and the one control here it can bite is
        // the close-on-launch switch, whose name is not the string it paints.
        let unreachable = harness::unreachable_controls(&mut element);
        assert!(
            unreachable.is_empty(),
            "these controls are published as nodes whose ids no focus report \
             carries, so a screen reader can read them and a keyboard cannot \
             reach them: {unreachable:?}. Reported ids: {ids:?}"
        );
    }

    /// **Tab reaches the page's first control and Up steps it** — UX-01's
    /// keyboard half, on the real page.
    ///
    /// `every_control_on_the_real_page_is_a_tab_stop_and_a_named_node` proves
    /// the page is *in* the Tab ring. This proves the ring is wired to something
    /// the user can act from: the framework's own `focus_next` operation
    /// (libcosmic runs it for Tab — `src/app/cosmic.rs:842-849`) is driven
    /// against the built page, then a real key event is dispatched through
    /// `Widget::update` and the message that comes out is compared.
    ///
    /// # Why `ArrowUp` and not `ArrowDown`
    ///
    /// [`Settings::default`] is `"dark"`, the *last* of the three schemes, so
    /// Down has no neighbouring choice and correctly publishes nothing (the step
    /// is `None` at either end — see [`selector`]). Up moves to `"light"`, index
    /// 1, and the message is checked for the **key** rather than the index, which
    /// is the property `a_selection_carries_the_key_and_not_the_index` pins for
    /// the pointer path. This asserts the keyboard path produces the identical
    /// message, which is the whole reason both come from one call to [`selector`].
    #[test]
    fn tab_then_arrow_steps_the_colour_scheme_selector_on_the_real_page() {
        use super::a11y::harness;
        use cosmic::iced::keyboard::{Key, key::Named};

        let settings = Settings::default();
        assert_eq!(
            color_scheme_index(&settings.color_scheme),
            Some(2),
            "this test's `ArrowUp` step is only well-defined from the last entry; \
             if the default scheme moved, move the key with it"
        );
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);

        let mut element = page(&settings, &runners);
        let (mut tree, node) = harness::built(&mut element);
        harness::tab_to(&mut element, &mut tree, &node);

        let mut messages = Vec::new();
        let out = harness::dispatch(
            &mut element,
            &mut tree,
            &node,
            &harness::pressed(Key::Named(Named::ArrowUp)),
            &mut messages,
        );

        assert!(
            out.captured,
            "the focused selector must capture the arrow it acted on. An \
             uncaptured arrow is the one that scrolls the page instead"
        );
        match out.messages.as_slice() {
            [Message::SetColorScheme(key)] => assert_eq!(
                key, "light",
                "the first Tab stop must be the colour-scheme selector — it is \
                 the page's first control in tree order — and Up from \"dark\" \
                 must select the neighbouring scheme by key"
            ),
            other => panic!(
                "expected exactly one `SetColorScheme(\"light\")`, got {other:?}. \
                 An empty list means Tab did not reach a wrapped control, or the \
                 wrapper did not treat itself as focused"
            ),
        }
    }

    /// A selection reports the **key**, not the index it sat at.
    ///
    /// This is the assertion the Library page's category selector needed and did
    /// not have: `dropdown`'s callback takes a `usize`
    /// (`libcosmic .../widget/dropdown/mod.rs:30`), so a closure that stringifies
    /// its parameter writes `"1"` where the model stores `"light"` — and `"1"` is
    /// not in `COLOR_SCHEMES`, so the guarded setter drops it and the selection
    /// silently does nothing.
    /// A selection's payload, read out of the real `Message`.
    ///
    /// `Message` has no `PartialEq` — it holds a `ToastId`, a `Task` and a
    /// `GameForm` — so the payload is destructured instead of compared. That is
    /// stricter: a selection that produced the wrong *variant* panics here
    /// rather than failing an equality it might have passed.
    fn payload_of(message: Message) -> (&'static str, String) {
        match message {
            Message::SetColorScheme(value) => ("SetColorScheme", value),
            Message::SetViewMode(value) => ("SetViewMode", value),
            Message::SetDefaultRunner(value) => ("SetDefaultRunner", value),
            other => panic!("expected a settings value, got {other:?}"),
        }
    }

    #[test]
    fn a_selection_carries_the_key_and_not_the_index() {
        assert_eq!(
            payload_of(color_scheme_selection(1)),
            ("SetColorScheme", "light".to_string())
        );
        assert_eq!(
            payload_of(view_mode_selection(1)),
            ("SetViewMode", "list".to_string())
        );

        // The defect, spelled: index 1 is not the string "1".
        assert_ne!(
            payload_of(color_scheme_selection(1)).1,
            "1",
            "a stringified index is dropped by the guard and the selection does nothing"
        );
        assert_ne!(payload_of(view_mode_selection(0)).1, "0");

        // And the runner selector, whose payload is an id rather than a key.
        let choices = vec![
            ("system-wine".to_string(), "System Wine".to_string()),
            ("GE-Proton9-1".to_string(), "GE-Proton9-1".to_string()),
        ];
        assert_eq!(
            payload_of(default_runner_selection(&choices, 1)),
            ("SetDefaultRunner", "GE-Proton9-1".to_string())
        );
    }

    /// The runner selector falls back to entry zero — System Wine — for a stored
    /// default that is not in the list, which is the reference's own collapse.
    #[test]
    fn a_missing_default_runner_selects_the_first_entry() {
        let choices = vec![
            ("system-wine".to_string(), "System Wine".to_string()),
            ("GE-Proton9-1".to_string(), "GE-Proton9-1".to_string()),
        ];
        assert_eq!(default_runner_index(&choices, "GE-Proton9-1"), 1);
        assert_eq!(
            default_runner_index(&choices, "uninstalled-runner"),
            0,
            "a default runner the user has removed degrades to System Wine"
        );
        assert_eq!(
            default_runner_index(&[], "anything"),
            0,
            "and does not panic on an empty list"
        );
    }

    /// Every selector index the page can draw maps to a message whose payload is
    /// in the set the guarded setters accept.
    ///
    /// `SetColorScheme`/`SetViewMode` ignore a value outside their set, so a
    /// selector that offered one would be a control that does nothing. This
    /// walks every index the options provide and checks the payload against the
    /// same lists the loader and the message path use.
    #[test]
    fn every_selector_offers_only_values_the_model_accepts() {
        for index in 0..COLOR_SCHEME_OPTIONS.len() {
            let (kind, value) = payload_of(color_scheme_selection(index));
            assert_eq!(kind, "SetColorScheme");
            assert!(
                COLOR_SCHEMES.contains(&value.as_str()),
                "{value:?} is not in COLOR_SCHEMES, so the guarded setter drops it"
            );
        }
        for index in 0..VIEW_MODE_OPTIONS.len() {
            let (kind, value) = payload_of(view_mode_selection(index));
            assert_eq!(kind, "SetViewMode");
            assert!(
                VIEW_MODES.contains(&value.as_str()),
                "{value:?} is not a view mode"
            );
        }
    }
}
