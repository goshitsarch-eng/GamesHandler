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
use cosmic::widget::{Column, Row, container, scrollable, text, toggler};
use gamehandler_core::runners::RunnerManager;
use gamehandler_core::settings::{COLOR_SCHEMES, Settings, VIEW_MODES};

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
/// printed but that they are *accounted for*: every row must be either
/// implemented or named in [`IMPLEMENTED_SHORTCUTS`]' complement, and
/// `the_page_does_not_claim_a_shortcut_the_shell_does_not_implement` is what
/// holds the two together.
/// The sentence the page prints under the shortcut table.
///
/// It exists because three of the four accelerators **do not fire while a text
/// field has focus** (`BUG-12`), which is where a user reaching for one is most
/// often standing — an Add-game form with the name half-typed, or the library
/// search. The divergence is measured and recorded at
/// `crates/app/src/shortcuts.rs:86-102`; a focused `text_input` swallows these
/// keys despite not binding them, so Qt's `Qt.ApplicationShortcut` semantics —
/// active whenever any window of the application is active, whatever holds
/// focus — are not reproduced.
///
/// The page a user visits *to learn the shortcuts* was the wrong place to stay
/// silent about that, and this constant is the row's own recommended first
/// remedy: print the caveat rather than leave the list reading as
/// unconditional. The second remedy — the guard asserting the caveat is
/// **rendered** rather than that a word appears in `main.rs` — is
/// `the_shortcut_caveat_is_drawn_with_the_rows_it_qualifies`.
pub const SHORTCUT_FOCUS_CAVEAT: &str = "These work anywhere in the window except while you are typing in a field — leave the field first if a key does nothing.";

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
/// # Two things this still does not claim
///
/// * `Ctrl+Shift+F` reaches the `on_search` hook (`main.rs`), because libcosmic's
///   match tests Control and does not reject Shift, where Qt's `Shortcut` would
///   not match it. It is the framework's binding; the divergence is recorded
///   here because this is the page that advertises the key.
/// * That a key press actually arrives is a claim about a running window.
///   Nothing here is observable without one, so it is T-19's to walk in the
///   Flatpak — the same bound P-68's own acceptance criteria name.
///
/// **What "implemented" means here is narrower than it reads, and the page now
/// says so.** Membership means the shell answers the key *when it reaches the
/// shell's subscription* — not that it reaches it from wherever the user is
/// standing. Three of the four do not arrive while a text field has focus
/// (`BUG-12`), which is a limitation of this list rather than of the
/// accelerators: the list is keyed on the *accelerator*, and the condition is
/// about focus, so neither this list nor [`UNWIRED_SHORTCUTS`] can express it.
/// Rather than leave the page printing a list that reads as unconditional,
/// [`SHORTCUT_FOCUS_CAVEAT`] is rendered under the rows and
/// `the_shortcut_caveat_is_drawn_with_the_rows_it_qualifies` holds it there.
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

/// The close-on-launch switch's label and explanation,
/// `SettingsPage.qml:120-125`.
///
/// The two are the reference's two different strings on one control — its
/// `FormData.label` and its `text` — and they are drawn in two different ways,
/// which matters for what can check them:
///
/// * [`CLOSE_ON_LAUNCH_LABEL`] goes to [`row`], so it becomes a `text::body`
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
/// `RunnerManager::choices()` (`runners/mod.rs:1119`) is the reference's
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

/// A labelled row: the reference's `FormLayout` label on the left, the control
/// on the right.
///
/// Hand-rolled rather than `iced`'s `form`, because the reference's form labels
/// are part of the visible page (P-66 names each one) and inheriting a layout's
/// own idea of where a label goes would make that a property of the toolkit.
fn row<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    Row::new()
        .push(text::body(label))
        .push(control)
        .spacing(12)
        .align_y(cosmic::iced::Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// A section heading, as the reference's `Kirigami.Separator` with a label is.
fn section<'a>(heading: &'a str) -> Element<'a, Message> {
    text::title4(heading).into()
}

pub fn view<'a>(page: SettingsPage<'a>) -> Element<'a, Message> {
    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- Appearance --------------------------------------------------------
    body = body
        .push(section(SECTION_APPEARANCE))
        .push(row(
            "Color scheme:",
            cosmic::widget::dropdown(
                color_scheme_labels(),
                color_scheme_index(&page.settings.color_scheme),
                color_scheme_selection,
            )
            .into(),
        ))
        .push(row(
            "Library layout:",
            cosmic::widget::dropdown(
                view_mode_labels(),
                view_mode_index(&page.settings.view_mode),
                view_mode_selection,
            )
            .into(),
        ));

    // ---- New games ---------------------------------------------------------
    let choices = page.runners.choices();
    body = body.push(section(SECTION_NEW_GAMES)).push(row(
        "Default runner:",
        cosmic::widget::dropdown(
            runner_labels(&choices),
            Some(default_runner_index(
                &choices,
                &page.settings.default_runner,
            )),
            {
                let choices = choices.clone();
                move |index| default_runner_selection(&choices, index)
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
        body = body.push(
            toggler(checked)
                .label(toggle_label(label, subtitle))
                .on_toggle(move |value| toggle_selection(key, value))
                .width(Length::Fill),
        );
    }

    // ---- Behavior ----------------------------------------------------------
    //
    // The reference's close-on-launch control is a `QQC2.Switch` with **two**
    // strings on it (`SettingsPage.qml:120-125`): `FormData.label` — the form
    // label, "Hide window when launching:" — and `text`, which is what the
    // switch itself renders. This port had the `text` half only, so the label
    // was declared, passed nothing, and drawn nowhere; `row()` is what the
    // three appearance rows already use for a form label, so this is the same
    // shape rather than a new one.
    //
    // It also puts one of this page's fourteen previously-unobservable strings
    // back inside the render instrument: a `Toggler`'s own label is painted with
    // a direct `text::draw` and reaches no `Text` widget (see the note on the
    // drawn-strings test), but a label passed to `row()` is a real `text::body`
    // child and `drawn_strings` sees it. The explanation stays on the toggler,
    // where the reference puts it.
    body = body
        .push(section(SECTION_BEHAVIOR))
        .push(row(
            CLOSE_ON_LAUNCH_LABEL,
            toggler(page.settings.close_on_launch)
                .label(CLOSE_ON_LAUNCH_EXPLANATION.to_string())
                .on_toggle(Message::SetCloseOnLaunch)
                .width(Length::Fill)
                .into(),
        ))
        .push(section(SECTION_SHORTCUTS));

    for (keys, what) in SHORTCUTS {
        body = body.push(row(keys, text::body(what).into()));
    }
    // Directly under the rows it qualifies, so it is read as part of them
    // rather than as a general note about the page.
    body = body.push(text::caption(SHORTCUT_FOCUS_CAVEAT));

    container(scrollable(body)).padding(18).into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let qml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../gamehandler/qml/SettingsPage.qml");
        let text = std::fs::read_to_string(&qml).unwrap_or_else(|err| {
            panic!(
                "{} should be readable: {err}\n\
                 It is the reference this page was ported from. If it has been \
                 moved, this check needs a new path.",
                qml.display()
            )
        });

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
        let qml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../gamehandler/qml/SettingsPage.qml");
        let text = std::fs::read_to_string(&qml)
            .expect("SettingsPage.qml is the reference and should be readable");

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
    /// The "does the shell handle keys at all" half reads `main.rs`, because
    /// that is where the answer lives and there is no runtime observable for
    /// "a key would do something" without a window.
    #[test]
    fn the_page_does_not_claim_a_shortcut_the_shell_does_not_implement() {
        let main_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let text = std::fs::read_to_string(&main_rs).expect("main.rs should be readable");

        // A `subscription()` that listens for keyboard events is how a shortcut
        // would arrive. Neither string alone is enough — `subscription` is a
        // trait method that appears in comments, `keyboard` appears in prose —
        // so the signal is their co-occurrence in a `fn subscription` body.
        let handles_keys = text.contains("fn subscription")
            && text
                .split("fn subscription")
                .skip(1)
                .any(|body| body[..body.len().min(600)].contains("keyboard"));

        assert_eq!(
            handles_keys,
            !IMPLEMENTED_SHORTCUTS.is_empty(),
            "`main.rs` {} a keyboard subscription while IMPLEMENTED_SHORTCUTS lists \
             {}. These must agree: if the shell now handles keys, fill in the \
             record (and delete the rows it still does not honour from it); if it \
             does not, the record is right and the page is printing four \
             shortcuts the shell ignores — which is P-68 unmet and must be \
             reported, not hidden",
            if handles_keys { "has" } else { "has no" },
            IMPLEMENTED_SHORTCUTS.len()
        );

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

    /// **The caveat about focus is drawn, with the rows it qualifies.**
    ///
    /// `BUG-12`: three of the four accelerators do not fire while a text field
    /// has focus, and this page — the one a user visits *to learn the
    /// shortcuts* — printed the four rows with no mention of it. The row's own
    /// recommended remedy was to render the caveat rather than leave the list
    /// reading as unconditional, and then to make the guard assert the caveat is
    /// **rendered** rather than that some word appears in `main.rs`.
    ///
    /// This asserts three things, and the middle one is why it walks the built
    /// tree instead of checking a constant:
    ///
    /// 1. the caveat's text is among the strings the page actually draws;
    /// 2. it is drawn **after the last shortcut row**, so it reads as a note on
    ///    those rows rather than as a general remark about the page — a caveat
    ///    rendered above the section header, or on a different page, would
    ///    satisfy (1) alone;
    /// 3. no shortcut row is drawn *after* it, which is the same claim from the
    ///    other side.
    ///
    /// The constant is compared against the *drawn* string rather than the
    /// assertion being written on the literal, so a page that stopped rendering
    /// it fails here even though the constant still exists.
    #[test]
    fn the_shortcut_caveat_is_drawn_with_the_rows_it_qualifies() {
        let drawn = page_strings();

        let caveat_at = drawn
            .iter()
            .position(|text| text == SHORTCUT_FOCUS_CAVEAT)
            .unwrap_or_else(|| {
                panic!(
                    "the Settings page draws four shortcuts and no caveat about \
                     focus. Three of them do not fire while a text field has \
                     focus (BUG-12), and this is the page a user reads to learn \
                     them; drawn: {drawn:?}"
                )
            });

        // The description column of each row, which is what `SHORTCUTS` holds.
        let last_row = SHORTCUTS
            .iter()
            .filter_map(|(_, what)| drawn.iter().position(|text| text == what))
            .max()
            .expect("the shortcut rows must be drawn at all, or this test is vacuous");
        assert!(
            caveat_at > last_row,
            "the caveat is drawn before the rows it qualifies (row at {last_row}, \
             caveat at {caveat_at}), so it does not read as a note on them; \
             drawn: {drawn:?}"
        );
        assert!(
            !drawn[last_row + 1..caveat_at]
                .iter()
                .any(|text| SHORTCUTS.iter().any(|(_, what)| what == text)),
            "a shortcut row is drawn between the last one and the caveat"
        );
    }

    /// **The old guard read `main.rs` for a word; this one reads the page for
    /// the property.**
    ///
    /// The predecessor to the caveat test asserted
    /// `main.rs.contains("fn subscription") && …contains("keyboard")` — a
    /// source-text check standing in for a behaviour check, over a divergence
    /// that is conditional on focus rather than on existence. `BUG-12`'s note is
    /// that the guard "cannot" catch a mismatch, because a word appearing in a
    /// file is not a key being delivered.
    ///
    /// What replaces it is not a stronger grep. The focus divergence is recorded
    /// in exactly one place — [`SHORTCUT_FOCUS_CAVEAT`]'s doc, citing
    /// `crates/app/src/shortcuts.rs:86-102` — and the thing worth guarding is
    /// that the page **renders** it, which
    /// `the_shortcut_caveat_is_drawn_with_the_rows_it_qualifies` does by walking
    /// the built tree. This test holds the remaining half: that the divergence
    /// has not been silently dropped from `IMPLEMENTED_SHORTCUTS`' own doc, so a
    /// reader of the constant cannot come away thinking the four are
    /// unconditional.
    ///
    /// It reads the source because the property *is* about the documentation,
    /// which has no runtime representation — the one case where a source read is
    /// the honest instrument rather than a substitute for one.
    #[test]
    fn the_focus_divergence_is_recorded_where_a_reader_of_the_list_will_find_it() {
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
            doc.contains("BUG-12") || doc.contains("focus"),
            "`IMPLEMENTED_SHORTCUTS`' doc no longer records that three of the \
             four do not fire while a text field has focus. A reader of the list \
             takes it as unconditional, which is the defect BUG-12 describes"
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

    /// The strings **this page**, as actually built, hands the operation
    /// traversal.
    ///
    /// A deliberate second copy of `main.rs`'s `drawn_strings` in the test module
    /// beside the claim it serves, and not a shared helper: the point of the two
    /// tests below is to measure *this* page rather than a hand-built widget, and
    /// `main.rs`'s copy is private to its own test module — which T-27 is
    /// forbidden from editing. If a third caller ever appears, the right move is
    /// a shared `#[cfg(test)]` helper, not a third copy.
    ///
    /// The renderer is `iced_tiny_skia`, pure software, so this needs no display
    /// and draws nothing; it is asked only to lay the tree out.
    fn drawn_strings(element: cosmic::Element<'_, Message>) -> Vec<String> {
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

        let mut element = element;
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

    /// This page, built the way the shell builds it.
    fn page_strings() -> Vec<String> {
        let settings = Settings::default();
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);
        drawn_strings(view(SettingsPage {
            settings: &settings,
            runners: &runners,
        }))
    }

    /// `SettingsPage.qml`, as text — the independent source three of the tests
    /// below read their expectations out of.
    fn settings_qml() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../gamehandler/qml/SettingsPage.qml");
        std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "{} should be readable: {err}\n\
                 It is the reference this page was ported from. If it has been \
                 moved, this check needs a new path.",
                path.display()
            )
        })
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
    /// that, and it is why the constant is now passed to `row()`.
    ///
    /// The two are asserted separately rather than as a pair, because they are
    /// drawn by different mechanisms and can fail independently: the explanation
    /// reaches a `Toggler` label and no tree walk sees it, while the label goes
    /// to `row()` and becomes a `text::body` child that does. Asserting them
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
    /// * the label goes through `row()` → `text::body`, which is a real child
    ///   widget, so `drawn_strings` sees it. That is what makes routing it
    ///   through `row()` a fix and not merely a rearrangement — and it is the
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
             `row()`, so its absence means it stopped being drawn at all — which \
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
