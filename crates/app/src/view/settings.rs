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

use cosmic::iced::Length;
use cosmic::widget::{Column, Row, container, scrollable, text, toggler};
use cosmic::Element;
use gamehandler_core::settings::{COLOR_SCHEMES, Settings, VIEW_MODES};
use gamehandler_core::runners::RunnerManager;

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
pub const COLOR_SCHEME_OPTIONS: [(&str, &str); 3] =
    [("system", "Match system"), ("light", "Light"), ("dark", "Dark")];

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
    ("dxvk", "Enable DXVK by default", "Direct3D 8–11 through Vulkan."),
    ("vkd3d", "Enable VKD3D by default", "Direct3D 12 through Vulkan."),
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
pub const SHORTCUTS: [(&str, &str); 4] = [
    ("Ctrl+N:", "Add a game"),
    ("Ctrl+F:", "Search the library"),
    ("Ctrl+,:", "Settings"),
    ("Ctrl+Q:", "Quit"),
];

/// The shortcuts the shell actually implements.
///
/// # Empty, and that is a recorded gap rather than an oversight
///
/// **None of the four work.** `main.rs` has no `subscription()` override, no
/// `on_key_press` and no keyboard handling of any kind — only `.desktop`
/// entry *files* the app writes for games, which are a different thing with the
/// same word in them. So the reference's four accelerators
/// (`Main.qml:121-143`) are unported and **P-68 is unmet**.
///
/// This is written down instead of left implicit because the page prints the
/// rows either way: without it, this page would show four shortcuts beside
/// controls that do nothing, and nothing in the suite would say so. The constant
/// exists so that "the shell implements none of these" is a value a test can
/// read, and so that wiring one is a one-line change here rather than a
/// rediscovery.
///
/// # Why this is not wired in this task
///
/// It is not page work, and it is not small. Each shortcut needs a
/// `Subscription` on the application (`cosmic::Application::subscription`,
/// `libcosmic src/app/mod.rs:460`) listening through
/// `iced_futures::keyboard::listen()` — and the two that look easiest are the
/// hard ones: `Ctrl+F` must *focus* the Library search field (a widget `Id` and
/// a focus `Task`), and `Ctrl+N`/`Ctrl+F` must not fire while the user is typing
/// in a text field, which is a focus-visibility question the shell does not yet
/// answer for any widget. Landing that inside a three-page task, untested,
/// would be the same class of mistake as the T-11/T-12 wiring: a control that
/// renders and does nothing.
pub const IMPLEMENTED_SHORTCUTS: [&str; 0] = [];

/// Every row [`SHORTCUTS`] prints that [`IMPLEMENTED_SHORTCUTS`] does not.
///
/// The complement, spelled out rather than computed, because the two constants
/// exist to be read by a human deciding what to do next: "these four are
/// advertised and none of them works" is the P-68 status, and a reviewer should
/// get it without running anything. `the_page_does_not_claim_a_shortcut_the_shell_does_not_implement`
/// checks the complement is exact — a row in neither list, or in both, fails.
///
/// When a shortcut is wired, move it from here to [`IMPLEMENTED_SHORTCUTS`].
pub const UNWIRED_SHORTCUTS: [&str; 4] = ["Ctrl+N:", "Ctrl+F:", "Ctrl+,:", "Ctrl+Q:"];

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
/// unrecognised scheme to `"dark"` (`settings.rs:89-91`, and the fallback is the
/// literal `"dark"`, not `COLOR_SCHEMES[0]` — FINDINGS F-E), so this is
/// reachable only if the two lists disagree. A dropdown with no selection rather
/// than a panic or a silently-wrong highlight, the same shape as
/// [`super::library::sort_index`].
pub fn color_scheme_index(scheme: &str) -> Option<usize> {
    COLOR_SCHEME_OPTIONS.iter().position(|(key, _)| *key == scheme)
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
            Some(default_runner_index(&choices, &page.settings.default_runner)),
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
    body = body
        .push(section(SECTION_BEHAVIOR))
        .push(
            toggler(page.settings.close_on_launch)
                .label(CLOSE_ON_LAUNCH_EXPLANATION.to_string())
                .on_toggle(Message::SetCloseOnLaunch)
                .width(Length::Fill),
        )
        .push(section(SECTION_SHORTCUTS));

    for (keys, what) in SHORTCUTS {
        body = body.push(row(keys, text::body(what).into()));
    }

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
        assert_eq!(default_runner_index(&[], "anything"), 0, "and does not panic on an empty list");
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
            assert!(VIEW_MODES.contains(&value.as_str()), "{value:?} is not a view mode");
        }
    }
}
