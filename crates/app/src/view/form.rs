//! The add/edit game form (P-18..P-31).
//!
//! Port of `GameFormPage.qml` (362 lines) and the form half of `bridge.py` —
//! `getGame` (`:356-379`), `newGameTemplate` (`:382-399`) and `saveGame`
//! (`:404-446`). The reference pushes the form as a **layer above the pages**,
//! and that is what it is here too: it is drawn over whatever page is showing,
//! so it is [`crate::Shell::view_body`]'s sibling rather than a
//! [`crate::state::Page`].
//!
//! # Every decision is a function, and why it matters more here than elsewhere
//!
//! The reference's form is a `QVariantMap` the QML mutates by key and hands to
//! `saveGame` whole (`GameFormPage.qml:40-51`). Two of its controls index into a
//! list and then read a *value* back out of that index: `runnerBox.currentValue`
//! (`:45`, `valueRole: "runnerId"` at `:177`) and `kindBox.currentIndex === 1`
//! (`:16`). Written as closures inside a widget builder, that mapping is
//! invisible to a test — finding #57, and the reason this module is a table of
//! pure functions with a thin builder over it, exactly as [`super::settings`] is.
//!
//! So [`TEXT_ROWS`]/[`LAUNCH_TOGGLES`]/[`COMPAT_TOGGLES`] are data,
//! [`kind_index`]/[`kind_is_linux`] and [`runner_index`]/[`runner_selection`] are
//! a function and its inverse, and [`category_index`] is the mapping the editable
//! combo needs. The widget builder only calls them.
//!
//! # What this module is, and what it is not
//!
//! It draws the form and returns the messages its controls produce. It does not
//! save: `saveGame`'s validation and normalisation are
//! [`crate::state::GameForm::apply`], tested without a display, and
//! [`crate::Message::SaveGameForm`] is what carries the values there.
//!
//! One of the reference's controls is **not drawn**, because drawing it would
//! put a control on screen that cannot do what it appears to do: the runner
//! selector, for a Linux game only (`:172-183`) — not a missing message but a
//! missing *capability*, recorded by [`RUNNER_ROW_HIDDEN_FOR_LINUX`].
//!
//! The executable and cover browse buttons (`:101-106`, `:159-164`, and the two
//! `FileDialog`s at `:334-361`) used to be on this list with Find cover. U5
//! drew Find cover; U6 drew both browse buttons behind
//! [`crate::Message::PickExeFile`] and [`crate::Message::PickCoverFile`].
//!
//! **Find cover** (`:151-158`) used to be the fourth: its arm was empty, so the
//! button was gated behind a `COVER_FETCH_MISSING` constant the suite read.
//! U5 wrote the arm and deleted the gate with the constant — the same device
//! the credits page used for its dead links, whose `LINKS_OPEN` was deleted
//! when P-65 wired them.
//!
//! Two of the reference's values are still drawn either way, because those are
//! reads rather than writes: the cover row draws the *preview* — the picture
//! itself, or the words the widget owns when there is no artwork, which is why
//! this module has no string of its own for that state (UX-11) — and the
//! runner's *value* is visible in the Library row's subtitle even when the
//! selector is not drawn here.
//!
//! [`crate::Message::PickExeFile`]: crate::Message::PickExeFile
//! [`crate::Message::PickCoverFile`]: crate::Message::PickCoverFile

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget::{
    Column, Row, button, container, divider, dropdown, scrollable, text, text_input, toggler,
};
use gamehandler_core::covers::DEFAULT_CATEGORIES;
use gamehandler_core::models::Library;
use gamehandler_core::runners::RunnerManager;

use crate::Message;
use crate::state::{FormField, GameForm};
use crate::view::cover_cache::CoverCache;

use super::a11y;
use super::{settings_row, settings_section};

// ---------------------------------------------------------------------------
// The reference's own words. Every one of these is a string
// `GameFormPage.qml` draws, and each is pinned against that file in the tests
// below rather than trusted.
// ---------------------------------------------------------------------------

/// `title: isNew ? "Add Game" : "Edit Game"` (`:18`).
pub const TITLE_ADD: &str = "Add Game";
pub const TITLE_EDIT: &str = "Edit Game";

/// The two actions (`:25-37`). The second's label is
/// `form.isNew ? "Add" : "Save"` (`:32`).
pub const ACTION_CANCEL: &str = "Cancel";
pub const ACTION_ADD: &str = "Add";
pub const ACTION_SAVE: &str = "Save";

/// The six `Kirigami.FormData` section labels, in the order the form shows them
/// (`:77`, `:122`, `:168`, `:194`, `:252`, `:315`).
pub const SECTION_GAME: &str = "Game";
pub const SECTION_LIBRARY: &str = "Library";
pub const SECTION_RUNNER: &str = "Compatibility tool";
pub const SECTION_LAUNCH: &str = "Launch options";
pub const SECTION_COMPAT: &str = "Compatibility";
pub const SECTION_ADVANCED: &str = "Advanced";

/// The Type selector's two entries (`:90`), in order.
///
/// The index *is* the mapping — `isLinux` is `currentIndex === 1` (`:16`) — so
/// [`kind_index`] and [`kind_is_linux`] are its two directions and neither is
/// written as a comparison against the label's text.
pub const KIND_OPTIONS: [&str; 2] = ["Windows (Wine / Proton)", "Linux native"];

/// The row labels, including the QML's trailing colons.
pub const LABEL_TYPE: &str = "Type:";
pub const LABEL_CATEGORY: &str = "Category:";
pub const LABEL_COVER: &str = "Cover art:";
pub const LABEL_RUNNER: &str = "Runner:";

/// The category field's placeholder.
///
/// The reference's combo is one widget, `editable: true`, whose `editText` is
/// seeded to the game's category or to `"Uncategorized"` (`:131`). The port draws
/// the editable half as a text field of its own, so this is the *shared* value
/// rather than a second spelling of it: the placeholder is
/// [`models::UNCATEGORIZED`], which is the same string the blank-category fold in
/// [`crate::state::GameForm::apply`] writes.
///
/// [`models::UNCATEGORIZED`]: gamehandler_core::models::UNCATEGORIZED
pub const CATEGORY_PLACEHOLDER: &str = gamehandler_core::models::UNCATEGORIZED;

/// The Find-cover button's word (`:149`).
///
/// The reference gives two cover-art states — `:140` is the "No cover yet" label
/// and this is the button — but the label is no longer a string of this module's.
/// The cover row draws the preview widget, and the widget is where that word
/// lives ([`crate::view::widgets::preview_label`]); a second copy here would be
/// a constant the module does not draw, kept alive only by the test that checks
/// it against the reference. That is the shape this audit files as a defect, so
/// the copy is deleted rather than left in place (UX-11).
pub const FIND_COVER: &str = "Find cover";

/// The two browse buttons' hover hints, which are also their accessible names —
/// UX-12.
///
/// The strings are the reference's own `QQC2.ToolTip.text` on those buttons
/// (`GameFormPage.qml:103` for the executable, `:161` for the cover), not
/// sentences invented here. One string serves both roles at each call site: a
/// hint that differs from the name is two things to keep in step, and the
/// toolkit cannot carry both from one builder (see [`a11y::tooltipped_button`]).
pub const BROWSE_EXE_HINT: &str = "Browse for an executable";
pub const BROWSE_COVER_HINT: &str = "Choose a custom cover image";

/// The Prefix row's placeholder (`:189`).
pub const PREFIX_PLACEHOLDER: &str = "Leave empty for an isolated prefix per game";

/// The Desktop size row's placeholder (`:310`), which is the same string
/// `saveGame` falls back to — [`crate::state::DEFAULT_DESKTOP_SIZE`].
pub const DESKTOP_SIZE_PLACEHOLDER: &str = "1920x1080";

/// The Advanced section's two placeholders (`:322`, `:328`).
pub const ADDITIONAL_APP_PLACEHOLDER: &str = "Optional helper launched in the same prefix";
pub const ENVIRONMENT_PLACEHOLDER: &str = "KEY=value pairs. Overrides the toggles above";

/// The toggle the desktop-size row is gated on — `desktopSwitch` (`:299-303`),
/// which the row's `enabled:` reads (`:309`).
pub const VIRTUAL_DESKTOP_TOGGLE: &str = "virtual_desktop";

/// Whether the runner row is **omitted** for a Linux game.
///
/// `true` because the reference's runner combo is `enabled: !form.isLinux`
/// (`:175`) and this port cannot draw a disabled one: the pinned libcosmic
/// revision's [`Dropdown`] has no `enabled`/`disabled` builder — it exposes `id`
/// and `with_positioner` and nothing else, and `disabled` does not appear anywhere
/// under `src/widget/dropdown/`. A [`TextInput`] draws inert when its callback is
/// dropped; a `Dropdown` always opens.
///
/// So the row is dropped rather than drawn live, which is the lesser of the two
/// departures: [`crate::state::GameForm::apply`] forces System Wine onto a Linux
/// game, so a selector the user could move would display a runner the save then
/// discards — a control lying about what it will store.
///
/// [`Dropdown`]: cosmic::widget::dropdown::Dropdown
/// [`TextInput`]: cosmic::widget::text_input::TextInput
///
/// `the_runner_row_is_hidden_exactly_when_it_cannot_be_disabled` fails the day
/// this becomes `false`. It is a value rather than a comment so the suite reads
/// the gap instead of trusting prose — the device `COVER_FETCH_MISSING` was
/// before U5 deleted it with the gap it recorded.
pub const RUNNER_ROW_HIDDEN_FOR_LINUX: bool = true;

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// One text row of the reference's `FormLayout`.
pub struct TextRow {
    pub field: FormField,
    /// The `id:` of the control the QML gives that row — the handle
    /// `the_text_rows_are_the_reference_pages_fields` uses to read the row out of
    /// the reference and compare it field by field. Every control these rows name
    /// has one; the category combo's is `categoryBox` and the cover row has no
    /// control of its own, which is why neither is a [`TextRow`].
    ///
    /// **`#[cfg(test)]` since ARCH-25**, and the field is not one that should be
    /// read elsewhere: it names a QML object by a string, so a reachability
    /// check on it would be a check against the reference rather than against
    /// the running application. Production addresses these rows through
    /// [`TextRow::field`], which [`GameForm`] can act on.
    #[cfg(test)]
    pub id: &'static str,
    /// The label, and the accessible name of the control this row builds.
    ///
    /// Not `#[cfg(test)]`, unlike the `id` field beside it: the label is the
    /// text the control draws and the name a screen reader announces, so it is
    /// read by the page rather than only by the reference check.
    pub label: &'static str,
    pub placeholder: &'static str,
    /// The QML's `enabled: !form.isLinux` on that row.
    pub windows_only: bool,
    /// The row's browse button, when it has one. Only the executable row does
    /// (F4): the reference's only other `FileDialog` is the cover picker's, and
    /// the cover row is hand-built rather than a [`TextRow`].
    pub browse: Option<BrowseButton>,
}

/// A row's browse button: the reference's `document-open` `ToolButton`.
///
/// The message and the hint are one value rather than two fields, because a row
/// that carried one without the other would build a button that either does
/// nothing or says nothing — and the hint is a sentence about *this* row's
/// subject ("Browse for an executable"), so a second browse row that inherited
/// it silently is exactly the failure two independent `Option` fields would
/// allow.
#[derive(Clone)]
pub struct BrowseButton {
    /// What the button sends, and what an assistive technology's activation
    /// sends: one binding, so the two paths cannot disagree.
    pub press: crate::Message,
    /// Its hover hint (`QQC2.ToolTip.text`, `GameFormPage.qml:103`), which is
    /// also its accessible name. UX-12.
    pub hint: &'static str,
}

/// The reference's text rows that are *not* gated on a second condition, in the
/// order it lays them out.
///
/// The order is user-visible, and it is **not** [`FormField::ALL`]'s. Three form
/// rows are deliberately absent because they are built by hand:
/// [`LABEL_CATEGORY`] and [`LABEL_COVER`] (two controls and a preview), and the
/// desktop-size row, which is gated on the virtual-desktop switch as well as on
/// [`TextRow::windows_only`] (`:309`). Those are asserted in
/// `the_hand_built_rows_are_the_ones_the_table_leaves_out` rather than being
/// silently missing here.
pub const TEXT_ROWS: [TextRow; 6] = [
    TextRow {
        field: FormField::Name,
        #[cfg(test)]
        id: "nameField",
        label: "Name:",
        placeholder: "",
        windows_only: false,
        browse: None,
    },
    TextRow {
        field: FormField::ExePath,
        #[cfg(test)]
        id: "exeField",
        label: "Executable:",
        placeholder: "",
        windows_only: false,
        browse: Some(BrowseButton {
            press: crate::Message::PickExeFile,
            hint: BROWSE_EXE_HINT,
        }),
    },
    TextRow {
        field: FormField::Arguments,
        #[cfg(test)]
        id: "argsField",
        label: "Launch arguments:",
        placeholder: "",
        windows_only: false,
        browse: None,
    },
    TextRow {
        field: FormField::WorkingDirectory,
        #[cfg(test)]
        id: "cwdField",
        label: "Working directory:",
        placeholder: "",
        windows_only: false,
        browse: None,
    },
    TextRow {
        field: FormField::PrefixPath,
        #[cfg(test)]
        id: "prefixField",
        label: "Wine prefix (optional):",
        placeholder: PREFIX_PLACEHOLDER,
        windows_only: true,
        browse: None,
    },
    TextRow {
        field: FormField::AdditionalApp,
        #[cfg(test)]
        id: "extraField",
        label: "Additional application:",
        placeholder: ADDITIONAL_APP_PLACEHOLDER,
        windows_only: false,
        browse: None,
    },
];

/// The Environment row, which is the last of the Advanced section (`:325-330`).
pub const ENVIRONMENT_ROW: TextRow = TextRow {
    field: FormField::Environment,
    #[cfg(test)]
    id: "envField",
    label: "Environment variables:",
    placeholder: ENVIRONMENT_PLACEHOLDER,
    windows_only: false,
    browse: None,
};

/// The desktop-size row (`:306-312`). See [`TEXT_ROWS`] for why it is separate.
pub const DESKTOP_SIZE_ROW: TextRow = TextRow {
    field: FormField::VirtualDesktopSize,
    #[cfg(test)]
    id: "desktopSizeField",
    label: "Desktop size:",
    placeholder: DESKTOP_SIZE_PLACEHOLDER,
    windows_only: true,
    browse: None,
};

/// One switch of the reference's `FormLayout`.
pub struct ToggleRow {
    /// The `_TOGGLE_FIELDS` name: the key of [`GameForm::toggles`], and the name
    /// `saveGame` writes `Game`'s field by.
    pub name: &'static str,
    /// The `FormData.label` — the QML's own, including its trailing colon.
    pub label: &'static str,
    /// The switch's `text:`, shown beside it.
    pub subtitle: &'static str,
    /// `enabled: !form.isLinux`.
    pub windows_only: bool,
}

/// **Launch options** — `GameFormPage.qml:198-249`, in that order.
///
/// Eight switches, and this is the *QML's* order rather than `TOGGLE_NAMES`':
/// `gamescope` is fourteenth in `_TOGGLE_FIELDS` (`bridge.py:74-78`) and eighth
/// here, because the QML groups it with the launch options.
/// `the_toggle_tables_are_the_reference_pages_switches` pins both directions —
/// the two tables are `TOGGLE_NAMES` as a set, and these are the QML's lines in
/// the QML's order.
pub const LAUNCH_TOGGLES: [ToggleRow; 8] = [
    ToggleRow {
        name: "mangohud",
        label: "MangoHud:",
        subtitle: "Performance overlay when MangoHud is installed",
        windows_only: false,
    },
    ToggleRow {
        name: "gamemode",
        label: "Feral GameMode:",
        subtitle: "Ask the system to boost performance while the game runs",
        windows_only: false,
    },
    ToggleRow {
        name: "prefer_sdl",
        label: "Prefer SDL:",
        subtitle: "Can fix controller issues in some games",
        windows_only: false,
    },
    ToggleRow {
        name: "wayland",
        label: "Wine Wayland driver:",
        subtitle: "Experimental. Works best on Proton-EM and recent GE-Proton",
        windows_only: true,
    },
    ToggleRow {
        name: "hdr",
        label: "HDR:",
        subtitle: "Experimental. Requires a compatible Proton build and display",
        windows_only: true,
    },
    ToggleRow {
        name: "esync",
        label: "Esync:",
        subtitle: "Eventfd-based Wine sync. Disable if you hit file-descriptor limits",
        windows_only: true,
    },
    ToggleRow {
        name: "fsync",
        label: "Fsync:",
        subtitle: "Futex-based Wine sync. Preferred when the kernel supports it",
        windows_only: true,
    },
    ToggleRow {
        name: "gamescope",
        label: "Gamescope:",
        subtitle: "Nested compositor for scaling, a stable session, and optional HDR",
        windows_only: false,
    },
];

/// **Compatibility** — `GameFormPage.qml:256-305`, in that order.
///
/// Seven switches. The eighth control of that section is the desktop *size*
/// field, which is [`DESKTOP_SIZE_ROW`].
pub const COMPAT_TOGGLES: [ToggleRow; 7] = [
    ToggleRow {
        name: "dxvk",
        label: "DXVK:",
        subtitle: "Direct3D 8–11 through Vulkan. Turn off to use WineD3D instead",
        windows_only: true,
    },
    ToggleRow {
        name: "vkd3d",
        label: "VKD3D:",
        subtitle: "Direct3D 12 through Vulkan",
        windows_only: true,
    },
    ToggleRow {
        name: "nvapi",
        label: "DXVK-NVAPI / DLSS:",
        subtitle: "NVIDIA NVAPI and DLSS. Needs a Proton runner through UMU",
        windows_only: true,
    },
    ToggleRow {
        name: "fsr",
        label: "AMD FSR:",
        subtitle: "Wine fullscreen FidelityFX Super Resolution",
        windows_only: true,
    },
    ToggleRow {
        name: "battleye",
        label: "BattlEye runtime:",
        subtitle: "Proton BattlEye helper for supported online games",
        windows_only: true,
    },
    ToggleRow {
        name: "eac",
        label: "Easy Anti-Cheat runtime:",
        subtitle: "Proton EAC helper for supported online games",
        windows_only: true,
    },
    ToggleRow {
        name: "virtual_desktop",
        label: "Virtual desktop:",
        subtitle: "Run the game inside a Wine desktop window",
        windows_only: true,
    },
];

// ---------------------------------------------------------------------------
// The decisions, as functions
// ---------------------------------------------------------------------------

/// The category combo's entries: `formCategories` (`bridge.py:344-350`).
///
/// The reference declares it `constant=True`, so it goes stale the moment a game
/// is added with a new category; `architecture.md` §2.5 item 2 records deriving
/// it as a deliberate behaviour improvement, and this is the derived version.
/// The reference's own loop, in the reference's own order: the built-ins first,
/// then the library's extras **in `Library::categories` order**, which puts
/// `Uncategorized` last. Deliberately not sorted — sorting would move
/// `Uncategorized` out of the position the user has seen it in.
pub fn form_categories(library: &Library) -> Vec<String> {
    let mut names: Vec<String> = DEFAULT_CATEGORIES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    for name in library.categories() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Which Type entry `is_linux` selects — the inverse of [`kind_is_linux`].
pub fn kind_index(is_linux: bool) -> usize {
    usize::from(is_linux)
}

/// Whether the Type selector's `index` means a Linux game (`:16`).
pub fn kind_is_linux(index: usize) -> bool {
    index == 1
}

/// The message the Type selector produces, from its index.
///
/// The QML reads `kindBox.currentIndex === 1` *at save time* rather than storing
/// `isLinux` on every change. The port writes it as it changes, which is the same
/// value at the same moment and one fewer thing to keep in step — and it is what
/// makes the two rows gated on `!isLinux` follow the selector immediately.
pub fn kind_selection(index: usize) -> Message {
    Message::SetFormLinux(kind_is_linux(index))
}

/// Which entry of the runner selector `runner_id` selects.
///
/// The same arithmetic as [`crate::view::settings::default_runner_index`] — the
/// reference writes `index >= 0 ? index : 0` in both places
/// (`SettingsPage.qml:83-86`, `GameFormPage.qml:179-182`) — and for the same
/// reason: index 0 is System Wine, because [`RunnerManager::choices`] puts it
/// first, so a runner the user has uninstalled degrades to the one that is always
/// present rather than to a blank selector.
pub fn runner_index(choices: &[(String, String)], runner_id: &str) -> usize {
    crate::view::settings::default_runner_index(choices, runner_id)
}

/// The message a runner selection produces, from the selector's index.
///
/// `valueRole: "runnerId"` (`:177`) — the combo carries the *id*, not the label
/// the dropdown was handed — so this reads the id out of `choices`. The fallback
/// is System Wine's id, matching [`runner_index`]'s collapse.
pub fn runner_selection(choices: &[(String, String)], index: usize) -> Message {
    Message::FormFieldChanged {
        field: FormField::Runner,
        value: choices
            .get(index)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| gamehandler_core::models::SYSTEM_WINE.to_string()),
    }
}

/// Which entry of `categories` the form's current category selects.
///
/// The reference's combo is `editable: true` (`:129`), so this is only what the
/// dropdown shows; the text field beside it holds the value, and a category the
/// user typed that is not in the list leaves the dropdown unselected. `None`
/// rather than a fallback, unlike [`runner_index`]: there is no index here that
/// is guaranteed to mean anything, and highlighting "Uncategorized" for a
/// category the user typed would be a lie about their own form.
pub fn category_index(categories: &[String], category: &str) -> Option<usize> {
    let trimmed = category.trim();
    categories.iter().position(|name| name == trimmed)
}

/// The message choosing entry `index` of `categories` produces.
///
/// A separate function rather than a closure over the list, for the reason the
/// module note gives: `dropdown`'s callback must be `'static`, so a closure has
/// to clone the list in and the index→name mapping then lives inside a builder
/// where no test can reach it. Here it is a function of the slice, and the
/// builder calls it.
pub fn category_selection(categories: &[String], index: usize) -> Message {
    Message::FormFieldChanged {
        field: FormField::Category,
        value: categories.get(index).cloned().unwrap_or_default(),
    }
}

/// The title the layer draws.
pub fn title(is_new: bool) -> &'static str {
    if is_new { TITLE_ADD } else { TITLE_EDIT }
}

/// The confirming action's label.
pub fn action_label(is_new: bool) -> &'static str {
    if is_new { ACTION_ADD } else { ACTION_SAVE }
}

/// Whether the confirming action is available:
/// `enabled: nameField.text.trim().length > 0` (`:34`).
///
/// **This is the only gate on saving**, and it is the reference's own. It is why
/// the empty-name branch of `saveGame` is unreachable from the button in either
/// implementation — which is what [`crate::Shell::update`]'s `SaveGameForm` arm
/// cites for the one place this port deviates from the QML's sequence.
pub fn can_save(form: &GameForm) -> bool {
    !form.name.trim().is_empty()
}

/// The whole form, as the message that saves it.
///
/// `None` when the reference's gate says no, so the button is drawn inert rather
/// than sending a save the model will refuse — the same shape as
/// [`crate::view::plugins::card_action`], and for the same reason: the gate is
/// consulted by the thing that acts, not only by the thing that is drawn.
pub fn save_message(form: &GameForm) -> Option<Message> {
    can_save(form).then(|| Message::SaveGameForm(form.clone()))
}

/// The message one of the text rows produces.
pub fn field_message(field: FormField, value: String) -> Message {
    Message::FormFieldChanged { field, value }
}

/// The message one of the switches produces.
///
/// Takes the row's name rather than a string the caller picked, so the toggle a
/// switch writes is the toggle of the row it was drawn from — the alias defect
/// [`crate::view::plugins::install_message`] was corrected for.
pub fn toggle_message(row: &ToggleRow, value: bool) -> Message {
    Message::FormToggleChanged {
        name: row.name.to_string(),
        value,
    }
}

/// The value a switch is drawn at: the form's, or `false` for a name the form
/// does not hold.
///
/// `unwrap_or(false)` is the compiler's half of the table being right; the table
/// is checked against [`GameForm::TOGGLE_NAMES`] by
/// `the_toggle_tables_are_the_reference_pages_switches`.
pub fn toggle_value(form: &GameForm, row: &ToggleRow) -> bool {
    form.toggle(row.name).unwrap_or(false)
}

/// The desktop-size row's availability (`:309`): a Windows game **and** the
/// virtual-desktop switch on.
///
/// Reads the toggle by *name* rather than by position in [`COMPAT_TOGGLES`]: an
/// index here would be a second place the table's order is written down, and the
/// first version of this function had one. `the_toggle_tables_…` asserts the name
/// below is one of the table's.
pub fn desktop_size_enabled(form: &GameForm) -> bool {
    !form.is_linux && form.toggle(VIRTUAL_DESKTOP_TOGGLE).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// The form's values, as the builder needs them.
///
/// Borrowed rather than owned, and taken **by value** by [`view`], for the reason
/// [`super::settings::SettingsPage`] is: the returned `Element` borrows from the
/// caller's `State`, so a layer cannot outlive the form it draws.
///
/// The two *lists* this draws are derived in [`view`] from the objects that own
/// them rather than held here as slices, which is the one place this differs from
/// what the first version of the struct did. Holding them meant the caller built
/// a `Vec` per list and kept both alive for as long as the returned `Element` — and
/// it cannot: `view` takes this struct by value and returns an `Element` that
/// borrows from it, so a `Vec` owned by the caller's frame would be dropped while
/// the `Element` still referred to it. Owning the `Vec`s *in* the struct would
/// compile and would move the derivation out of [`view`], where it is a call to a
/// tested function, into whichever caller had to remember to fill two fields.
///
/// This is [`super::library::LibraryPage`]'s shape, and it keeps
/// [`form_categories`] a pure function of a [`Library`].
pub struct GameFormView<'a> {
    pub form: &'a GameForm,
    /// `formCategories` (`bridge.py:344-350`) — see [`form_categories`].
    pub library: &'a Library,
    /// `runnerChoices` (`bridge.py:616-619`), for the selector's labels and ids.
    pub runners: &'a RunnerManager,
    /// The cover row's preview (`UX-11`).
    ///
    /// The reference's row draws an `Image` (`GameFormPage.qml:140-146`) and the
    /// port drew only a caption, which left the whole preview stack in
    /// [`super::widgets`] constructed by nothing but its own tests — the state
    /// the row calls "the state that misleads". The cache is what
    /// [`super::widgets::cover_preview`] resolves the picture through, and it is
    /// the same cache the library's tiles use, so the two cannot disagree about
    /// what counts as artwork.
    pub covers: &'a CoverCache,
}

/// A text row's control.
///
/// The `enabled` flag is **not** applied to the input itself: `TextInput` in the
/// pinned libcosmic revision draws an input the same whether or not anything will
/// act on its messages, so `enabled: !form.isLinux` on the reference's runner and
/// prefix fields is drawn by *omitting the row's callback* instead — see [`view`].
/// The value is still shown, which is what the reference does: an
/// `enabled: false` `TextField` still displays its text.
///
/// What an input that cannot be edited also cannot do is be *selected* or
/// scrolled, which a real `enabled: false` would allow — a difference this port
/// takes rather than invents a second way to draw an inert field. It is bounded:
/// the strings are still legible, which is the property the reference's disabled
/// state has too.
///
/// The accessible name is [`TextRow::label`] and the value is the field's current
/// text (UX-03), both read off the row the same way the placeholder and the
/// initial value are. [`a11y::input`] adds the node and **not** a second focus
/// report — the input already makes one (`libcosmic
/// src/widget/text_input/input.rs:853`), and the pair would be two Tab stops for
/// one field — so the Tab order over this form is unchanged by this call.
fn text_control<'a>(row: &TextRow, form: &'a GameForm, live: bool) -> Element<'a, Message> {
    let field = row.field;
    let input = text_input(row.placeholder, form.field(field)).width(Length::Fill);
    // `on_input` and the wrapper are both applied to the `TextInput` value and
    // only then converted, because `a11y::input` wraps the toolkit's own widget
    // — not an `Element`, which would have already erased its type.
    let input = if live {
        input.on_input(move |value| field_message(field, value))
    } else {
        input
    };
    // The reference's browse `ToolButton` (`document-open`, F4): an icon
    // button with no text, which is why its edge — that this button sends
    // this row's press — is read, not tested. An icon publishes no string
    // for `drawn_strings`, and driving a click needs `Widget::update` over a
    // laid-out element, which the credits page measured and abandoned; the
    // press value itself is a constant, so a press-fn would pin nothing a
    // copy of the constant does not.
    //
    // **UX-12: it is now a named node and a hover hint, and the name comes from
    // outside the tooltip.** `browse.hint` is one string serving both, and
    // `a11y::tooltipped_button` is why the button is still announced at all: the
    // toolkit's own `.tooltip(..)` wraps the button in `iced`'s `Tooltip`,
    // which implements no `a11y_nodes`, so the name the builder sets never
    // reaches assistive technology. The wrapper publishes it from outside.
    let input: Element<'a, Message> = a11y::input(input, row.label, form.field(field)).into();
    match row.browse.clone() {
        Some(browse) => Row::new()
            .push(input)
            .push(a11y::tooltipped_button(
                button::icon(crate::icons::handle(crate::icons::Icon::Open))
                    .tooltip(browse.hint)
                    .on_press(browse.press.clone()),
                browse.hint,
                Some(browse.press),
            ))
            .spacing(6)
            .width(Length::Fill)
            .into(),
        None => input,
    }
}

/// One switch.
///
/// The accessible name is the row's subtitle — the string the switch paints —
/// and the activation is the row's own [`toggle_message`] for the flipped state,
/// which is what its `on_toggle` closure publishes off the same function (UX-02).
/// The name is prefixed with the row's toggle *name* rather than used bare: the
/// name is also the widget's id (`a11y::stable_id`), and two rows sharing a
/// subtitle would share an accessibility node, which is possible here because
/// subtitles are prose rather than identifiers.
///
/// `live` gates **both** paths, and it did not always. The pointer's
/// `on_toggle` was dropped on an inert row while the wrapper's activation was
/// applied unconditionally, so the eleven rows the reference disables on Linux
/// could still be flipped with Enter — a regression the wrapper introduced,
/// caught by a red-team pass rather than by a test, because no test asked
/// whether an inert row's node still advertised `Action::Click`. `live` is now
/// passed down rather than consulted once.
fn toggle_control<'a>(row: &'a ToggleRow, form: &'a GameForm, live: bool) -> Element<'a, Message> {
    // `label` takes `impl Into<Option<String>>`, not `Into<Cow<str>>` — the one
    // place in this file where a `&str` does not go straight in.
    let checked = toggle_value(form, row);
    let switch = toggler(checked)
        .label(Some(row.subtitle.to_string()))
        .width(Length::Fill);
    let switch = if live {
        switch.on_toggle(move |value| toggle_message(row, value))
    } else {
        switch
    };
    a11y::toggler(
        switch,
        format!("{}: {}", row.name, row.subtitle),
        checked,
        live.then(|| toggle_message(row, !checked)),
    )
    .into()
}

/// Whether a row with `windows_only` is editable on this form.
///
/// `enabled: !form.isLinux` on the reference's rows, and the one place a table's
/// flag is turned into a live control. Written as a function rather than inlined
/// because the two loops that use it are the only callers and a `windows || !x`
/// in each of them is the shape that gets one of the two wrong — which the first
/// version of [`view`] did, by passing the form-wide `windows` for every toggle
/// and so disabling MangoHud and GameMode on a Linux game, where the reference
/// leaves both on (`:198-215`).
pub fn row_enabled(windows_only: bool, is_linux: bool) -> bool {
    !(windows_only && is_linux)
}

/// The form.
pub fn view<'a>(page: GameFormView<'a>) -> Element<'a, Message> {
    let form = page.form;
    let is_new = form.is_new;
    let is_linux = form.is_linux;
    let windows = !is_linux;

    let categories = form_categories(page.library);
    let choices = page.runners.choices();

    let mut body = Column::new().spacing(12).width(Length::Fill);

    // ---- Game --------------------------------------------------------------
    body = body
        .push(text::title3(title(is_new)))
        .push(settings_section(SECTION_GAME));
    for row in &TEXT_ROWS[..4] {
        let live = row_enabled(row.windows_only, is_linux);
        body = body.push(settings_row(row.label, text_control(row, form, live)));
    }
    body = body.push(settings_row(
        LABEL_TYPE,
        a11y::dropdown(
            dropdown(
                KIND_OPTIONS.to_vec(),
                Some(kind_index(form.is_linux)),
                kind_selection,
            ),
            LABEL_TYPE,
            KIND_OPTIONS
                .get(kind_index(form.is_linux))
                .map(|l| l.to_string()),
            |delta| {
                let next = kind_index(form.is_linux).checked_add_signed(delta as isize)?;
                (next < KIND_OPTIONS.len()).then(|| kind_selection(next))
            },
        )
        .into(),
    ));

    // ---- Library -----------------------------------------------------------
    // The category combo is `editable: true` (`:129`): a dropdown of the known
    // categories *and* a text field, so a category the list does not have can
    // still be typed. The reference has one widget that is both; these are the
    // two controls that make up the same behaviour, and the text field is the one
    // that holds the value.
    //
    // **The two controls are named differently on purpose**, and this row is the
    // one place in the form where that needs saying: `field_row` draws a single
    // [`LABEL_CATEGORY`] for the pair, and the accessible name is also the
    // widget's id (`a11y::stable_id`), so naming both controls after that one
    // label would give them one node between them. The combo takes the form
    // label; the text field takes its own placeholder, which is the string a
    // pointer user reads inside the empty field (`models::UNCATEGORIZED`) and is
    // a real string rather than one invented to disambiguate.
    let category_row = {
        let owned = categories.clone();
        let shown = category_index(&categories, form.field(FormField::Category));
        Row::new()
            .push({
                let combo: Element<'a, Message> = a11y::dropdown(
                    dropdown(owned.clone(), shown, move |index| {
                        category_selection(&owned, index)
                    }),
                    LABEL_CATEGORY,
                    shown.and_then(|index| categories.get(index).cloned()),
                    {
                        let owned = categories.clone();
                        let count = categories.len();
                        move |delta| {
                            let next = shown?.checked_add_signed(delta as isize)?;
                            (next < count).then(|| category_selection(&owned, next))
                        }
                    },
                )
                .into();
                combo
            })
            .push({
                let field: Element<'a, Message> = a11y::input(
                    text_input(CATEGORY_PLACEHOLDER, form.field(FormField::Category))
                        .on_input(|value| field_message(FormField::Category, value))
                        .width(Length::Fill),
                    CATEGORY_PLACEHOLDER,
                    form.field(FormField::Category),
                )
                .into();
                field
            })
            .spacing(6)
            .width(Length::Fill)
    };
    // The cover preview, decided once, here rather than inside the row's element
    // expression.
    //
    // The `Game` is built from the **form**, not looked up in the library: a new
    // game has no entry yet, and the preview has to work while the user is still
    // filling the form in. It is a whole `Game` because that is what the preview
    // stack reads — `cover_plan` names it for the initials a game with no
    // artwork would draw — and because the form's own two fields are the only
    // ones it consults ([`crate::state::GameForm::as_preview_game`]).
    //
    // The plan is hoisted for a second reason: the words the row shows when
    // there is no artwork are *the plan's* (`CoverPlan::InitialsOnPlate`), so
    // the same value has to reach both the drawing and the words. Deciding it
    // inside the element expression would let the drawn cover and the declared
    // state be two separate `classify` calls that could disagree.
    let preview_game = form.as_preview_game();
    let preview_plan = super::widgets::cover_plan(page.covers, &preview_game);

    body = body
        .push(settings_section(SECTION_LIBRARY))
        .push(settings_row(LABEL_CATEGORY, category_row.into()))
        .push(settings_row(LABEL_COVER, {
            // The reference's row, in its own order (`GameFormPage.qml:134-163`):
            // `coverPreview`, then the "No cover yet" label **in its place**,
            // then the two buttons. The preview is portrait and *fixed* — the
            // QML's own `gridUnit * 2` × `gridUnit * 3` — and both of the first
            // two come from one widget ([`super::widgets::cover_preview`]),
            // because they are one decision: `visible: source !== ""` picks
            // between them and never shows both.
            //
            // **The port used to draw neither of them.** The row drew
            // `text::caption(form.cover_path)` — the *path*, as a string — where
            // the reference draws a picture, and the preview widget that existed
            // for it was reached from no page at all. That is UX-11, and the
            // caption was the port's invention: there is no path label anywhere
            // in the reference's row, which is what the test below reads the
            // file to establish rather than taking on trust.
            //
            // `cover_preview` resolves the picture through `CoverCache`, so a
            // cover just chosen shows before the save — which is the point the
            // row makes ("the user picks a cover with no confirmation of what
            // they picked").
            let row = Row::new()
                .push(super::widgets::cover_preview::<Message>(
                    page.covers,
                    &preview_game,
                    &preview_plan,
                ))
                .spacing(6);
            // `token: 0`: the view cannot mint the lookup token (it holds no
            // `&mut State`), so the arm does — see `FetchCoverForForm`.
            row.push(
                button::standard(FIND_COVER).on_press(Message::FetchCoverForForm {
                    token: 0,
                    game_id: form.game_id.clone().unwrap_or_default(),
                    name: form.name.clone(),
                    // `form.isLinux ? "" : exeField.text` (`:157`).
                    exe: if form.is_linux {
                        String::new()
                    } else {
                        form.exe_path.clone()
                    },
                }),
            )
            // The custom-cover browse `ToolButton` (`:159-164`, F8): same
            // read-not-tested edge as the exe row's — see `text_control` — and
            // the same UX-12 treatment: one string for the hint and the name,
            // published from outside the tooltip that would otherwise erase it.
            .push(a11y::tooltipped_button(
                button::icon(crate::icons::handle(crate::icons::Icon::Open))
                    .tooltip(BROWSE_COVER_HINT)
                    .on_press(Message::PickCoverFile),
                BROWSE_COVER_HINT,
                Some(Message::PickCoverFile),
            ))
            .width(Length::Fill)
            .into()
        }));

    // ---- Compatibility tool ------------------------------------------------
    // The runner selector is the one control the reference *disables* rather than
    // gates on the game's kind — `enabled: !form.isLinux` (`:175`) — and a
    // disabled `Dropdown` cannot be drawn; see
    // [`RUNNER_ROW_HIDDEN_FOR_LINUX`] for why the row is dropped instead.
    //
    // The prefix row below it is a text field, so it *is* drawn disabled, by the
    // device [`text_control`] describes.
    body = body.push(settings_section(SECTION_RUNNER));
    if windows || !RUNNER_ROW_HIDDEN_FOR_LINUX {
        let owned = choices.clone();
        let labels = crate::view::settings::runner_labels(&choices);
        let shown = runner_index(&choices, form.field(FormField::Runner));
        let count = labels.len();
        body = body.push(settings_row(
            LABEL_RUNNER,
            a11y::dropdown(
                dropdown(labels.clone(), Some(shown), move |index| {
                    runner_selection(&owned, index)
                }),
                LABEL_RUNNER,
                labels.get(shown).cloned(),
                {
                    let owned = choices.clone();
                    // `shown` rather than a `?` off the stored id: this
                    // selector's fallback is index 0 for an id it does not
                    // know (see [`runner_index`]), so the neighbour of what is
                    // displayed is `shown ± 1`, not "no selection".
                    move |delta| {
                        let next = shown.checked_add_signed(delta as isize)?;
                        (next < count).then(|| runner_selection(&owned, next))
                    }
                },
            )
            .into(),
        ));
    }
    body = body.push(settings_row(
        TEXT_ROWS[4].label,
        text_control(
            &TEXT_ROWS[4],
            form,
            row_enabled(TEXT_ROWS[4].windows_only, is_linux),
        ),
    ));

    // ---- Launch options ----------------------------------------------------
    body = body.push(settings_section(SECTION_LAUNCH));
    for row in &LAUNCH_TOGGLES {
        let live = row_enabled(row.windows_only, is_linux);
        body = body.push(settings_row(row.label, toggle_control(row, form, live)));
    }

    // ---- Compatibility -----------------------------------------------------
    body = body.push(settings_section(SECTION_COMPAT));
    for row in &COMPAT_TOGGLES {
        let live = row_enabled(row.windows_only, is_linux);
        body = body.push(settings_row(row.label, toggle_control(row, form, live)));
    }
    body = body.push(settings_row(
        DESKTOP_SIZE_ROW.label,
        text_control(&DESKTOP_SIZE_ROW, form, desktop_size_enabled(form)),
    ));

    // ---- Advanced ----------------------------------------------------------
    body = body
        .push(settings_section(SECTION_ADVANCED))
        .push(settings_row(
            TEXT_ROWS[5].label,
            text_control(&TEXT_ROWS[5], form, true),
        ))
        .push(settings_row(
            ENVIRONMENT_ROW.label,
            text_control(&ENVIRONMENT_ROW, form, true),
        ));

    // The two actions. Cancel closes whatever overlay is open, which is the
    // reference's `dismiss()` (`:15`, `:20-23`) and the same message the
    // confirm-delete dialog closes with.
    body = body.push(divider::horizontal::default()).push(
        Row::new()
            .push(button::standard(ACTION_CANCEL).on_press(Message::CloseDialog))
            .push(button::suggested(action_label(is_new)).on_press_maybe(save_message(form)))
            .spacing(12),
    );

    container(scrollable(super::bounded_body(body)))
        .padding(super::gutter())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamehandler_core::models::{Game, UNCATEGORIZED};
    use gamehandler_core::settings::Settings;

    /// `gamehandler/qml/GameFormPage.qml`, read from the repository rather than
    /// transcribed.
    ///
    /// Every string and every `enabled:` flag below is pinned against this file,
    /// so a port that drifts from the reference fails here rather than looking
    /// self-consistent. The read goes through
    /// [`gamehandler_core::oracle_support::repo_file`] like every other
    /// reference read in the workspace (`ARCH-15`), so the climb to the
    /// repository root is written once instead of once per crate.
    ///
    /// This doc used to say the path was "derived from `CARGO_MANIFEST_DIR` for
    /// the reason `main.rs`'s tests derive theirs" — which was false twice over.
    /// `main.rs`'s tests used a bare `join("../..")` rather than this function's
    /// `.parent().and_then(...)` chain, so the precedent named was not a
    /// precedent; and there were four `.parent()`-style derivations in the tree
    /// by then, so the reason given was the reason each of them gave. `ARCH-15`
    /// records it because a citation that reads as deliberate is exactly what
    /// keeps a divergence from being looked at.
    fn qml() -> String {
        gamehandler_core::oracle_support::repo_file("gamehandler/qml/GameFormPage.qml")
    }

    /// The text from the `{` at `open` to the `}` that closes it.
    ///
    /// Brace counting over a QML file is a heuristic — a brace inside a string
    /// literal would fool it — so it is never trusted on its own: every caller
    /// asserts on the block's *contents*, which means a wrong block fails the
    /// assertion rather than quietly satisfying it, and
    /// [`a_block_is_the_widget_it_was_asked_for`] checks the two shapes this file
    /// relies on against the real reference.
    fn balanced(source: &str, open: usize) -> &str {
        assert_eq!(
            &source[open..open + 1],
            "{",
            "the block does not open with a brace"
        );
        let mut depth = 0usize;
        for (offset, character) in source[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[open..open + offset + 1];
                    }
                }
                _ => {}
            }
        }
        panic!("the braces from byte {open} never close");
    }

    /// The brace-matched body of the widget whose `id:` is `id`.
    fn qml_widget<'a>(source: &'a str, id: &str) -> &'a str {
        let needle = format!("id: {id}");
        let at = source
            .find(&needle)
            .unwrap_or_else(|| panic!("the reference has no `{needle}`"));
        let open = source[..at]
            .rfind('{')
            .unwrap_or_else(|| panic!("`{needle}` is not inside a block"));
        let block = balanced(source, open);
        assert!(
            block.contains(&needle),
            "the block found for `{needle}` does not contain it: {block}"
        );
        block
    }

    /// The brace-matched body of the `QQC2.Switch` that writes
    /// `form.gameData.<name>`.
    ///
    /// The switches in that section carry no `id:` — the reference writes the
    /// *name* on the `onToggled:` line and that is the only handle any of them
    /// has, which is exactly why the port keys its tables on the same name.
    fn qml_switch<'a>(source: &'a str, name: &str) -> &'a str {
        let needle = format!("form.gameData.{name} = checked");
        let at = source
            .find(&needle)
            .unwrap_or_else(|| panic!("the reference has no `{needle}`"));
        let opens_at = source[..at]
            .rfind("QQC2.Switch")
            .unwrap_or_else(|| panic!("`{needle}` is not inside a QQC2.Switch"));
        let open = opens_at
            + source[opens_at..]
                .find('{')
                .expect("a QQC2.Switch always opens a block");
        let block = balanced(source, open);
        assert!(
            block.contains(&needle),
            "the switch found for `{needle}` does not contain it: {block}"
        );
        block
    }

    /// The `Kirigami.FormData.label` governing the widget whose `id:` is `id`.
    ///
    /// **The label is not reliably before or after the control**, which is the
    /// thing this function exists to get right and the first version of it got
    /// wrong. The reference writes it *inside* the control's own block for a
    /// single-control row (`:81-85`, where `id: nameField` comes first and the
    /// label after it) and on the enclosing `RowLayout` for a row that holds more
    /// than one control (`:94-97`, where the label precedes the control). So the
    /// search is the block's contents first, then the block one level out — never
    /// a scan backwards through the file, which finds the *section separator's*
    /// label ("Game") for the first row of each section and looks entirely
    /// plausible.
    fn governing_label<'a>(source: &'a str, id: &str) -> &'a str {
        let needle = format!("id: {id}");
        let at = source
            .find(&needle)
            .unwrap_or_else(|| panic!("the reference has no `{needle}`"));
        let open = source[..at]
            .rfind('{')
            .unwrap_or_else(|| panic!("`{needle}` is not inside a block"));
        let block = balanced(source, open);
        if let Some(label) = form_label(block) {
            return label;
        }
        // One level out: the enclosing row's block, whose own label is the one
        // that governs this control.
        let outer_open = source[..open]
            .rfind('{')
            .unwrap_or_else(|| panic!("`{needle}` has no enclosing block"));
        let outer = balanced(source, outer_open);
        form_label(outer).unwrap_or_else(|| {
            panic!("neither `{needle}`'s block nor its parent carries a FormData.label")
        })
    }

    /// The first `Kirigami.FormData.label: "…"` in `text`, without its quotes.
    fn form_label(text: &str) -> Option<&str> {
        let marker = "Kirigami.FormData.label:";
        let rest = &text[text.find(marker)? + marker.len()..];
        let start = rest.find('"')? + 1;
        let end = rest[start..].find('"')? + start;
        Some(&rest[start..end])
    }

    /// A library at its own temp path, holding one game per category.
    ///
    /// Built through `Library::add`, which saves, so the categories are read back
    /// out of a file rather than injected — the same reason `main.rs`'s
    /// `library_with` does it that way. The path is keyed on the label and the
    /// process id; `form_categories` only ever reads, so the calls cannot race the
    /// way a saving fixture's do.
    fn library(label: &str, categories: &[&str]) -> Library {
        let root = std::env::temp_dir().join(format!("gh-form-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temp directory");
        let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
        for (index, category) in categories.iter().enumerate() {
            let mut game = Game::new_named(format!("Game {index}"));
            game.category = (*category).to_string();
            library.add(game).expect("the temp library is writable");
        }
        library
    }

    /// A form with a name, so `apply` and `can_save` have something to accept.
    fn form() -> GameForm {
        let mut form = GameForm::new_template(&Settings::default(), "form-game".to_string());
        form.name = "Celeste".to_string();
        form
    }

    /// A form whose every field holds a string of its own.
    ///
    /// [`form`] is a template, so all but one of its text fields are empty — and
    /// an empty string is also what a node whose `value` was never wired up
    /// publishes, so a form full of blanks would let the value assertions in
    /// `every_wrapped_control_on_the_real_form_is_a_tab_stop_and_a_named_node`
    /// pass over a page that tells a screen reader nothing. Distinct strings
    /// remove that coincidence: a node carrying a constant, a row's placeholder,
    /// or its neighbour's text fails, and fails by name.
    fn form_with_distinct_values() -> GameForm {
        let mut form = form();
        for (index, field) in FormField::ALL.iter().enumerate() {
            form.set_field(*field, format!("field-{index}"));
        }
        // The category is the one field put back to a string the combo can
        // actually show, and it is not a convenience: the category row is the
        // one row whose field is *also* a combo option, and `Category:`'s node
        // value is the option the combo has selected. Left as `field-N` the
        // combo has no selection at all — the reference leaves the text in the
        // editable field and the list unselected — so the node publishes no
        // value, and asserting it would compare `None` with `None`. Distinctness
        // is unaffected: a category name is not shaped like `field-N`, and the
        // text-field loop's premise assertion checks it.
        form.set_field(
            FormField::Category,
            DEFAULT_CATEGORIES[DEFAULT_CATEGORIES.len() - 1].to_string(),
        );
        form
    }

    /// The runners the form's selector offers.
    fn runners() -> RunnerManager {
        RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv)
    }

    /// This form, built the way the shell builds it.
    fn page<'a>(
        form: &'a GameForm,
        library: &'a Library,
        runners: &'a RunnerManager,
        covers: &'a CoverCache,
    ) -> Element<'a, Message> {
        view(GameFormView {
            form,
            library,
            runners,
            covers,
        })
    }

    /// **Every control this form wraps is a Tab stop, and the category row's two
    /// controls are two** — UX-01, UX-02 and UX-03 on the real form.
    ///
    /// # Why this form's assertion is per-control and not a count
    ///
    /// The Settings page's equivalent test asserts an exact number of Tab stops,
    /// because every control on that page is one this port built and wrapped. It
    /// cannot be done that way here: this form also carries five buttons — Cancel,
    /// Save, Find cover and the two browse icons — and libcosmic's own button
    /// reports a focusable state (`src/widget/button/widget.rs:358-359`), so they
    /// are Tab stops this module neither built nor wrapped and whose ids it has no
    /// name for. A count would therefore be a number that moves whenever the
    /// toolkit's button changes, and the arithmetic in its failure message would
    /// be a claim about the toolkit rather than about this page.
    ///
    /// So each *wrapped* control is asserted individually, by the name it
    /// publishes. That is the assertion that goes red when a call site in [`view`]
    /// is reverted to the bare toolkit widget: a bare
    /// `cosmic::widget::toggler`/`dropdown` reports no focusable state and
    /// publishes no node at all (`view/a11y.rs`'s
    /// `the_toolkit_controls_the_app_used_to_build_are_invisible` measures that
    /// directly), so its name disappears from both lists and the row below fails
    /// on it by name.
    ///
    /// # The category row is the reason this test is not redundant
    ///
    /// `LABEL_CATEGORY` is drawn once by `field_row` for a row that contains
    /// **two** controls, and the accessible name is also the widget's id
    /// (`a11y::stable_id`) — so naming both of them `LABEL_CATEGORY` would give
    /// the pair one node between them, and the text field would be unreachable by
    /// name. The view avoids that by naming the text field after its placeholder,
    /// a comment says so, and this is the measurement that holds the comment
    /// true. The `distinct` assertions below are that measurement: the pair must
    /// produce two nodes with two different ids, and each of the two names must
    /// be a Tab stop in its own right.
    ///
    /// # What a name and a role do not say
    ///
    /// A node that exists, announces the right role and carries the right name
    /// says nothing about what a reader is told the control *holds*. Every
    /// assertion above would pass for a form whose every field published the same
    /// constant, or its own placeholder, or its neighbour's text. So the value
    /// half below reads each `TextInput` node's `value` back against the field it
    /// is drawn from, through `GameForm::field`, on a fixture that gives every
    /// field a distinct string.
    ///
    /// The value is also where UX-03 is *only* visible: an `input` wrapper whose
    /// `value` argument was dropped is a node a screen reader reads as an empty
    /// text field over a form full of text.
    ///
    /// # The id invariant, which is a property of the whole page
    ///
    /// The category pair's `in_ring` assertions below cover the one row where two
    /// controls compete for a name. The same invariant has to hold for *every*
    /// node the page publishes, including the ones the toolkit built: a node
    /// published under an id no focus report carries is a control assistive
    /// technology can read and a keyboard cannot reach, and the reverse is a Tab
    /// stop that announces nothing. `harness::unreachable_controls` is that
    /// assertion, and the last block is it.
    ///
    /// Both halves are read from **one** element. A second build would mint fresh
    /// `Id::unique()`s for the toolkit's own widgets — the five buttons and the
    /// scrollable — and the invariant would read those as unreachable when they
    /// are perfectly reachable. `harness::unreachable_controls`' doc says so at
    /// length; this is the call site that has to obey it.
    ///
    /// Every row the reference disables stays disabled to the keyboard.
    ///
    /// This is the test for a defect the accessibility wrapper **introduced**
    /// rather than one it failed to fix, which is why it is written from the
    /// inert side. `row_enabled` is false for a `windows_only` row on a Linux
    /// game, and `toggle_control` expresses that by dropping the toolkit's
    /// `on_toggle`: the pointer cannot flip it. The wrapper's activation was
    /// applied unconditionally, so Enter and Space could — eleven rows the
    /// reference forbids, editable by anyone using a keyboard, and the node
    /// advertised `Action::Click` while it did. A red-team pass found it by
    /// driving every inert row; no existing test asked the question, because
    /// every one of them asserted what an *operable* control does.
    ///
    /// # How the ring is walked, and why the message is the thing read
    ///
    /// The form is built **once** and `harness::tab_to` is called repeatedly, so
    /// the framework's own `focus_next` carries the focus one stop further each
    /// time — the whole ring, in the order a keyboard user meets it, including
    /// the dropdowns and the text inputs and the five toolkit buttons this page
    /// also draws. Enter is dispatched at every stop.
    ///
    /// Nothing here has to work out *which* widget is focused at a given stop:
    /// `FormToggleChanged` carries the row's own `name` (see
    /// [`toggle_message`]), so a published message names the row that produced
    /// it. That is what makes the assertion readable — it fails naming a row
    /// like `wayland`, not an opaque id.
    ///
    /// # The control half
    ///
    /// The same walk over a **Windows** form, where `row_enabled` is true for
    /// every row and the reference leaves them all editable. One of them must
    /// publish, or a wrapper that never activates anything would pass the Linux
    /// half above by doing nothing at all. The Windows half also names which row
    /// published, so the failure says whether the keyboard arm is dead or merely
    /// missing the row under test.
    #[test]
    fn a_row_the_reference_disables_cannot_be_flipped_from_the_keyboard() {
        use super::a11y::harness;
        use cosmic::iced::keyboard::Key;
        use cosmic::iced::keyboard::key::Named;

        let library = library("form-inert", &["Action"]);
        let runners = RunnerManager::new(&gamehandler_core::runners::SystemLaunchEnv);

        /// Every row name Enter published a `FormToggleChanged` for, walking the
        /// whole Tab ring of `form`.
        fn names_enter_flips(
            form: &GameForm,
            library: &Library,
            runners: &RunnerManager,
            covers: &CoverCache,
        ) -> Vec<String> {
            let mut element = page(form, library, runners, covers);
            // The ring, counted on a build of this same element. Only the length
            // is used: a *toolkit* widget's id differs between two builds — that
            // is the premise of `view/a11y.rs` — so an id read here would be a
            // different widget's, but the number of stops is not affected.
            let stops = harness::focusables(&mut element).len();
            assert!(
                stops > 0,
                "the form reports no Tab stop at all, so this walk would measure \
                 nothing"
            );
            let (mut tree, node) = harness::built(&mut element);

            let mut published = Vec::new();
            // `focus_next` wraps, so walking a few past the ring's length is
            // harmless and covers the case where the count above is a stop long
            // or short.
            for _ in 0..stops + 2 {
                harness::tab_to(&mut element, &mut tree, &node);
                let mut messages = Vec::new();
                let out = harness::dispatch(
                    &mut element,
                    &mut tree,
                    &node,
                    &harness::pressed(Key::Named(Named::Enter)),
                    &mut messages,
                );
                for message in messages.iter().chain(out.messages.iter()) {
                    if let Message::FormToggleChanged { name, .. } = message {
                        published.push(name.clone());
                    }
                }
            }
            published
        }

        // ---- the inert rows on a Linux form ---------------------------------
        let mut linux = form();
        // Linux, so every `windows_only` row is inert. Asserted rather than
        // assumed: on a Windows form all of them are live and this half would
        // pass having checked nothing.
        linux.is_linux = true;
        let inert: Vec<&str> = LAUNCH_TOGGLES
            .iter()
            .chain(COMPAT_TOGGLES.iter())
            .filter(|row| !row_enabled(row.windows_only, linux.is_linux))
            .map(|row| row.name)
            .collect();
        assert!(
            inert.len() >= 8,
            "the fixture found only {} inert rows, so this test would not exercise \
             the case it exists for; the table or `row_enabled` has changed shape",
            inert.len()
        );

        let covers = CoverCache::new();
        let flipped = names_enter_flips(&linux, &library, &runners, &covers);
        let reachable: Vec<&String> = flipped
            .iter()
            .filter(|name| inert.contains(&name.as_str()))
            .collect();
        assert!(
            reachable.is_empty(),
            "these rows are disabled on a Linux game — the reference writes \
             `enabled: !form.isLinux` — but Enter on them published \
             `FormToggleChanged`: {reachable:?}. The pointer cannot flip them and \
             neither may the keyboard: a control the user can see as inert and \
             change anyway is worse than one they cannot reach. Names Enter did \
             flip: {flipped:?}"
        );

        // ---- the control: the same rows, live ---------------------------------
        let mut windows = form();
        windows.is_linux = false;
        assert!(
            inert.iter().all(|name| LAUNCH_TOGGLES
                .iter()
                .chain(COMPAT_TOGGLES.iter())
                .any(|row| row.name == *name && row_enabled(row.windows_only, false))),
            "the rows this test calls inert must be live on a Windows form, or the \
             two halves are not the same rows and the control below proves nothing"
        );
        let covers = CoverCache::new();
        let flipped = names_enter_flips(&windows, &library, &runners, &covers);
        assert!(
            !flipped.is_empty(),
            "on a Windows form every row is live, so Enter must flip at least one \
             of them. Nothing was flipped, which means the wrapper's keyboard arm \
             is dead rather than the rows being gated — and this is the half that \
             keeps the assertion above from passing on a wrapper that cannot \
             activate anything at all"
        );
    }

    #[test]
    fn every_wrapped_control_on_the_real_form_is_a_tab_stop_and_a_named_node() {
        use super::a11y::harness;
        use cosmic::iced::core::id::IdEq;
        use iced_accessibility::accesskit::Role;

        let form = form_with_distinct_values();
        let library = library("form-controls", &[]);
        let runners = runners();
        let covers = CoverCache::new();
        let mut element = page(&form, &library, &runners, &covers);

        // Every name this form is expected to publish, with the role it must
        // announce as. The toggles are the two tables this module draws; the
        // rest are the hand-built rows, which is why they are written out.
        let mut expected: Vec<(String, Role)> = Vec::new();
        for row in TEXT_ROWS
            .iter()
            .chain([&DESKTOP_SIZE_ROW, &ENVIRONMENT_ROW])
        {
            expected.push((row.label.to_string(), Role::TextInput));
        }
        expected.push((LABEL_TYPE.to_string(), Role::ComboBox));
        expected.push((LABEL_CATEGORY.to_string(), Role::ComboBox));
        expected.push((CATEGORY_PLACEHOLDER.to_string(), Role::TextInput));
        expected.push((LABEL_RUNNER.to_string(), Role::ComboBox));
        for row in LAUNCH_TOGGLES.iter().chain(COMPAT_TOGGLES.iter()) {
            expected.push((format!("{}: {}", row.name, row.subtitle), Role::Switch));
        }

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
        for (index, id) in ids.iter().enumerate() {
            assert!(
                !ids[..index].iter().any(|earlier| IdEq::eq(earlier, id)),
                "two controls report the same id {id:?}, so a screen reader sees \
                 one control where the user sees two. The id comes from the \
                 accessible name (`a11y::stable_id`), so this is two rows sharing \
                 a name: {ids:?}"
            );
        }

        // ---- the nodes ------------------------------------------------------
        let nodes = harness::published(&mut element);
        let named = |label: &str, role: Role| {
            nodes
                .iter()
                .filter(|node| node.label.as_deref() == Some(label) && node.role == role)
                .count()
        };

        for (name, role) in &expected {
            assert_eq!(
                named(name, *role),
                1,
                "the control named {name:?} must publish exactly one node \
                 announcing as {role:?}. Zero means it is built from the bare \
                 toolkit widget, which publishes nothing at all; two means a node \
                 is published twice. Nodes: {:?}",
                nodes
                    .iter()
                    .map(|node| (&node.role, node.label.as_deref()))
                    .collect::<Vec<_>>()
            );
        }

        // ---- the values, which the names above do not imply ------------------
        //
        // Each text field this form draws, paired with the field it is drawn
        // from.
        //
        // **This comment used to say the combos were asserted "on the three
        // pages that own combos".** They were not asserted anywhere: this page
        // owns three combos of its own — Type, Category and Runner — and all
        // three published a `value` that no test read back, which is a claim
        // about another page's test standing in for a check that did not exist.
        // They are asserted below, here, where they are drawn.
        let mut value_expected: Vec<(&str, FormField)> = TEXT_ROWS
            .iter()
            .chain([&DESKTOP_SIZE_ROW, &ENVIRONMENT_ROW])
            .map(|row| (row.label, row.field))
            .collect();
        value_expected.push((CATEGORY_PLACEHOLDER, FormField::Category));

        // The premise of the loop below, asserted rather than assumed: two fields
        // holding the same text would let a node carrying the wrong row's value
        // pass. `form()` sets the name *before* `form_with_distinct_values`
        // overwrites it, so this is also what holds the fixture's own promise
        // honest if either changes.
        let mut held: Vec<&str> = value_expected
            .iter()
            .map(|(_, field)| form.field(*field))
            .collect();
        held.sort_unstable();
        let distinct = {
            let mut copy = held.clone();
            copy.dedup();
            copy.len()
        };
        assert_eq!(
            distinct,
            held.len(),
            "the fixture must give every field a text of its own, or the value \
             assertions below cannot tell one row's node from another's: {held:?}"
        );

        for (label, field) in &value_expected {
            let node = nodes
                .iter()
                .find(|node| node.role == Role::TextInput && node.label.as_deref() == Some(*label))
                .unwrap_or_else(|| panic!("the {label:?} node is asserted above"));
            assert_eq!(
                node.value.as_deref(),
                Some(form.field(*field)),
                "the {label:?} field publishes the value {:?}, but the form holds \
                 {:?} for {field:?}. This is the assertion a `value` argument \
                 dropped from the `a11y::input` call site — or replaced by a \
                 constant, or by the row's placeholder — fails; a node whose name \
                 and role are right and whose text is wrong tells a screen-reader \
                 user they are about to save something other than what they are. \
                 Every text node this form publishes: {:?}",
                node.value,
                form.field(*field),
                nodes
                    .iter()
                    .filter(|node| node.role == Role::TextInput)
                    .map(|node| (node.label.as_deref(), node.value.as_deref()))
                    .collect::<Vec<_>>()
            );
        }

        // ---- the three combo values, which the text-field loop cannot reach --
        //
        // The loop above reads `Role::TextInput` nodes only, so every combo this
        // form draws was unread: a `a11y::dropdown` call site that passed `None`,
        // a constant, or another list's entry for `selected` would keep every
        // assertion above green while telling a screen-reader user the field
        // holds an option it does not hold. That is `COSMIC-UX.md`'s UX-01 half
        // — the control is in the ring and announces as a combo — with the
        // *content* missing, which is the failure a reader of the node cannot
        // see and the user of it can.
        //
        // Each expected string is read off the same reader the call site uses
        // (`kind_index`, `category_index`, `runner_index` plus the list that
        // reader indexes into), so what this asserts is that the node and the
        // widget agree rather than that a constant was copied here correctly.
        // The fixture guarantees all three resolve — see
        // [`form_with_distinct_values`] for the category's case.
        // `shows` is asserted for the same reason: an index that resolves to
        // nothing would make both sides `None` and the comparison would pass
        // over the very defect it exists for.
        let choices = runners.choices();
        let runner_labels = crate::view::settings::runner_labels(&choices);
        let category_options = form_categories(&library);
        assert!(
            category_options.len() > 1,
            "the category combo needs more than one option for its value to              distinguish an index from a constant; the library has {}",
            category_options.len()
        );

        let mut combo_expected: Vec<(&str, Option<String>)> = vec![
            (
                LABEL_TYPE,
                KIND_OPTIONS
                    .get(kind_index(form.is_linux))
                    .map(|label| label.to_string()),
            ),
            (
                LABEL_CATEGORY,
                category_index(&category_options, form.field(FormField::Category))
                    .and_then(|index| category_options.get(index).cloned()),
            ),
        ];
        // The runner row is drawn on the same condition the view uses, so the
        // expectation follows the view rather than assuming the fixture's
        // platform. On a Linux game with [`RUNNER_ROW_HIDDEN_FOR_LINUX`] the row
        // is not drawn and must not be expected — an assertion for a control
        // that is not there would be a test of the fixture.
        if !form.is_linux || !RUNNER_ROW_HIDDEN_FOR_LINUX {
            let shown = runner_index(&choices, form.field(FormField::Runner));
            combo_expected.push((LABEL_RUNNER, runner_labels.get(shown).cloned()));
        }

        for (label, expected) in &combo_expected {
            assert!(
                expected.is_some(),
                "the {label:?} combo resolves to no option on this fixture, so the \
                 value assertion below would compare `None` with `None` and pass \
                 having checked nothing. Form: kind {:?}, category {:?}, runner \
                 {:?}",
                kind_index(form.is_linux),
                form.field(FormField::Category),
                form.field(FormField::Runner)
            );
        }
        let combo = |label: &str| {
            nodes
                .iter()
                .find(|node| node.role == Role::ComboBox && node.label.as_deref() == Some(label))
                .unwrap_or_else(|| panic!("the {label:?} combo node is asserted above"))
        };
        for (label, expected) in &combo_expected {
            assert_eq!(
                combo(label).value.as_deref(),
                expected.as_deref(),
                "the {label:?} combo publishes the value {:?}, but the option it is \
                 showing is {expected:?}. A node whose name and role are right and \
                 whose value is wrong tells a screen-reader user the game will be \
                 saved with a setting it will not be saved with. Every combo node \
                 this form publishes: {:?}",
                combo(label).value,
                nodes
                    .iter()
                    .filter(|node| node.role == Role::ComboBox)
                    .map(|node| (node.label.as_deref(), node.value.as_deref()))
                    .collect::<Vec<_>>()
            );
        }

        // ---- the category pair, which is the one row with two controls -------
        //
        // Both names are in `expected` above, so both nodes and both roles are
        // already asserted; what is added here is that they are two *different*
        // controls. Same node twice, or two nodes sharing an id, both pass the
        // loop above and both lose a control.
        //
        // `A11yId` and `Id` are the same types — iced re-exports
        // `iced_accessibility::id` (`iced/core/src/lib.rs:68`), which is why
        // `A11yNode::new` takes a widget `Id` at all — so the widget's id and the
        // node's id are compared with the one `IdEq` that knows a `Unique` and a
        // `Custom` id can still name the same node
        // (`iced/accessibility/src/id.rs:188-205`), which is also the relation
        // accesskit itself resolves by number.
        let node_id = |role: Role, label: &str| {
            nodes
                .iter()
                .find(|node| node.role == role && node.label.as_deref() == Some(label))
                .map(|node| node.id.clone())
                .unwrap_or_else(|| panic!("the {label:?} node is asserted above"))
        };
        let in_ring = |id: &iced_accessibility::A11yId| {
            stops
                .iter()
                .flatten()
                .any(|stop| IdEq::eq(&iced_accessibility::A11yId::from(stop.clone()), id))
        };

        let combo = node_id(Role::ComboBox, LABEL_CATEGORY);
        let field = node_id(Role::TextInput, CATEGORY_PLACEHOLDER);
        assert!(
            !IdEq::eq(&combo, &field),
            "the category row's combo and text field share the node id {combo:?}, \
             so one of them is unreachable: the row draws a single \
             {LABEL_CATEGORY:?} label for the pair and the accessible name is also \
             the id, so this is exactly what naming both controls after that one \
             label would produce. They must differ — see the note in `view`"
        );
        assert!(
            in_ring(&combo) && in_ring(&field),
            "both of the category row's controls must be Tab stops *under their own \
             node ids* — a control whose node id no focus report carries is one \
             assistive technology can see and cannot reach, and the reverse is a \
             Tab stop that announces nothing. Combo {combo:?} in ring: {}; field \
             {field:?} in ring: {}; ids reported: {ids:?}",
            in_ring(&combo),
            in_ring(&field),
        );

        // ---- every node this page publishes is a Tab stop --------------------
        //
        // The generalisation of the pair assertion above to the whole page,
        // toolkit-built nodes included. A node under an id no focus report
        // carries is a control a screen reader can read and a keyboard cannot
        // reach, and the reverse is a Tab stop that announces nothing; both are
        // the defect class `view/a11y.rs` exists for. Which roles count as
        // controls is `unreachable_controls`' business and its doc gives the
        // reason.
        let unreachable = harness::unreachable_controls(&mut element);
        assert!(
            unreachable.is_empty(),
            "these controls are published as nodes whose ids no focus report \
             carries, so a screen reader can read them and a keyboard cannot \
             reach them: {unreachable:?}. Reported ids: {ids:?}"
        );
    }

    /// **The two browse buttons are named, and by different names** — UX-12 on
    /// this page.
    ///
    /// # Why this is not the test above, run again
    ///
    /// The wrapped-controls test asserts the controls this module *builds a
    /// wrapper for* — the toggles, the dropdowns, the text fields — and its
    /// expected list does not mention the buttons at all, for the reason its own
    /// doc gives: libcosmic's button reports itself and names itself from its
    /// label, so the port never had to name one. These two are the exception the
    /// label path cannot cover: they have no label, and the reference gives them
    /// a `QQC2.ToolTip.text` instead.
    ///
    /// # What it measures
    ///
    /// Each hint drawn by the reference (`GameFormPage.qml:103`, `:161`) must
    /// appear **exactly once** as a `Role::Button` node on the built page. Zero
    /// is the pre-fix state — measured, not assumed: before the tooltip was
    /// added these buttons published a node labelled `Some("")`, and *after*
    /// `.tooltip(..)` alone they publish **no node at all**, because `iced`'s
    /// `Tooltip` implements no `a11y_nodes` and its default empty tree replaces
    /// the button's. Two would mean the node is published twice, which is what
    /// `a11y::tooltipped_button` would do if it were ever applied to a button the
    /// toolkit had not wrapped.
    ///
    /// The two names are asserted **different** because they are the whole
    /// point on this page: the same `document-open` glyph sits on two rows and
    /// does two different things.
    #[test]
    fn the_two_browse_buttons_publish_a_name_and_the_names_differ() {
        use super::a11y::harness;
        use iced_accessibility::accesskit::Role;

        assert_ne!(
            BROWSE_EXE_HINT, BROWSE_COVER_HINT,
            "one string for both buttons would name them alike, and the glyph \
             they share is exactly what the name has to disambiguate"
        );

        let form = form_with_distinct_values();
        let library = library("form-browse-names", &[]);
        let runners = runners();
        let covers = CoverCache::new();
        let mut element = page(&form, &library, &runners, &covers);
        let nodes = harness::published(&mut element);

        for hint in [BROWSE_EXE_HINT, BROWSE_COVER_HINT] {
            let found = nodes
                .iter()
                .filter(|node| node.role == Role::Button && node.label.as_deref() == Some(hint))
                .count();
            assert_eq!(
                found,
                1,
                "the browse button hinting {hint:?} must publish exactly one \
                 Button node carrying that name. Zero means nothing announces \
                 it — a bare `button::icon` names itself `\"\"`, and one wrapped \
                 in the toolkit's own `tooltip` publishes no node at all; two \
                 means one button is announced twice. Nodes: {:?}",
                nodes
                    .iter()
                    .map(|node| (&node.role, node.label.as_deref()))
                    .collect::<Vec<_>>()
            );
        }
    }

    /// **A hover on a browse button draws its hint** — the other half of UX-12.
    ///
    /// # What this proves, and what it does not
    ///
    /// It proves a hint is drawn, where it is drawn from, and that it is a
    /// *hover*: the pointer is placed over the button through a real
    /// `Widget::update` and the overlay is read off the same element, then the
    /// same read is taken with the pointer away from it. `iced`'s `Tooltip`
    /// opens only for a cursor it can see over its own bounds
    /// (`iced/widget/src/tooltip.rs:216-260`), so a check that never placed the
    /// pointer would measure an overlay that is absent in both the fixed and
    /// the broken build — the repository's own defect class.
    ///
    /// What it does **not** prove is the *string* the overlay draws. iced's
    /// `Tooltip::operate` traverses its content and never its tooltip
    /// (`iced/widget/src/tooltip.rs:372-383`), and the `Overlay` it returns
    /// implements no `operate` at all (`:433-586`), so no operation reaches the
    /// sentence: the text is not readable from a built element by any means this
    /// repository has. The sentence is pinned by the name assertion above
    /// instead — one binding feeds both — and the overlay is pinned here, so
    /// removing either half fails a test.
    #[test]
    fn a_browse_button_draws_its_hint_when_the_pointer_is_over_it() {
        use super::a11y::harness;
        use cosmic::iced::advanced::Layout;
        use cosmic::iced::{Event as CoreEvent, Point, Size, Vector, mouse};

        let row = TEXT_ROWS
            .iter()
            .find(|row| row.browse.is_some())
            .expect("the reference puts a browse button on the executable row");
        let window = Size::new(600.0, 120.0);

        let form = form_with_distinct_values();
        let mut element: Element<'_, Message> = text_control(row, &form, true);
        let (mut tree, node, nodes) = harness::laid_out_tree(&mut element, window);
        let button = nodes
            .iter()
            .find(|node| node.label.as_deref() == Some(BROWSE_EXE_HINT))
            .unwrap_or_else(|| {
                panic!(
                    "the row's browse button publishes no node named \
                     {BROWSE_EXE_HINT:?}, so there is no rectangle to hover. \
                     Nodes: {:?}",
                    nodes
                        .iter()
                        .map(|node| (&node.role, node.label.as_deref()))
                        .collect::<Vec<_>>()
                )
            });
        let bounds = button
            .bounds
            .expect("a laid-out control records the rectangle it was given");
        let button_rect = cosmic::iced::Rectangle::new(
            Point::new(bounds.x0 as f32, bounds.y0 as f32),
            Size::new(
                (bounds.x1 - bounds.x0) as f32,
                (bounds.y1 - bounds.y0) as f32,
            ),
        );
        let centre = Point::new(
            (bounds.x0 + bounds.x1) as f32 / 2.0,
            (bounds.y0 + bounds.y1) as f32 / 2.0,
        );

        let viewport = cosmic::iced::Rectangle::new(Point::ORIGIN, window);
        let mut messages: Vec<Message> = Vec::new();

        // Away from the button first: the row's own left-hand text field, one
        // pixel inside the row, which is far from the icon at its right.
        harness::dispatch_over(
            &mut element,
            &mut tree,
            &node,
            &CoreEvent::Mouse(mouse::Event::CursorMoved {
                position: Point::new(1.0, centre.y),
            }),
            Point::new(1.0, centre.y),
            &mut messages,
        );
        assert!(
            element
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &harness::renderer(),
                    &viewport,
                    Vector::ZERO,
                )
                .is_none(),
            "a pointer over the row but not over the button must not open the \
             hint, or this measurement is of something else"
        );

        harness::dispatch_over(
            &mut element,
            &mut tree,
            &node,
            &CoreEvent::Mouse(mouse::Event::CursorMoved { position: centre }),
            centre,
            &mut messages,
        );
        let mut overlay = element
            .as_widget_mut()
            .overlay(
                &mut tree,
                Layout::new(&node),
                &harness::renderer(),
                &viewport,
                Vector::ZERO,
            )
            .unwrap_or_else(|| {
                panic!(
                    "a pointer over the browse button at {centre:?} (its \
                     rectangle is {bounds:?}) opened no hint. The button is \
                     built with the toolkit's `.tooltip(..)`, which wraps it in \
                     `iced::widget::Tooltip`; without that call nothing is drawn \
                     on hover and this is the assertion that says so"
                )
            });
        let hint_bounds = overlay
            .as_overlay_mut()
            .layout(&harness::renderer(), window)
            .bounds();
        assert!(
            hint_bounds.width > 0.0 && hint_bounds.height > 0.0,
            "the hint opened with no area to draw in: {hint_bounds:?}"
        );
        assert_ne!(
            hint_bounds, button_rect,
            "the hint must be its own box beside the button, not the button's \
             own rectangle read back"
        );
    }

    /// The block finders find the widget they were asked for.
    ///
    /// The brace counter is a heuristic, so it is checked against the two shapes
    /// this module depends on rather than trusted: a control whose `id:` is the
    /// first line inside it, and a switch identified by the name it writes.
    #[test]
    fn a_block_is_the_widget_it_was_asked_for() {
        let source = qml();

        let name = qml_widget(&source, "nameField");
        assert!(name.starts_with('{') && name.ends_with('}'));
        assert!(name.contains("text: form.gameData.name || \"\""));
        assert!(
            !name.contains("argsField"),
            "the block ran past its own close"
        );

        let switch = qml_switch(&source, "mangohud");
        assert!(switch.contains("Kirigami.FormData.label: \"MangoHud:\""));
        assert!(
            !switch.contains("gamemode"),
            "the block ran past its own close"
        );

        // And the label lookup, for both shapes: one on the control, one on the
        // enclosing RowLayout.
        assert_eq!(governing_label(&source, "nameField"), "Name:");
        assert_eq!(governing_label(&source, "exeField"), "Executable:");
        assert_eq!(
            governing_label(&source, "desktopSizeField"),
            "Desktop size:"
        );
        assert_eq!(form_label("Kirigami.FormData.label: \"X\""), Some("X"));
    }

    /// **Every word the port draws is a word of the reference.**
    ///
    /// The constants above are transcribed from `GameFormPage.qml`, and a
    /// transcription is exactly the kind of fact that goes wrong quietly. This
    /// reads the reference and requires each one to be a substring of it, so a
    /// typo, a renamed label or a placeholder invented in the port fails here.
    ///
    /// It is a *substring* test and is not claimed to be more: it cannot tell
    /// which row a string belongs to. That association is what the two tests below
    /// assert, row by row, out of the reference's own structure.
    #[test]
    fn the_forms_words_are_the_reference_pages_words() {
        let source = qml();
        let mut words: Vec<String> = Vec::new();

        for word in [
            TITLE_ADD,
            TITLE_EDIT,
            ACTION_CANCEL,
            ACTION_ADD,
            ACTION_SAVE,
        ] {
            words.push(word.to_string());
        }
        for word in [
            SECTION_GAME,
            SECTION_LIBRARY,
            SECTION_RUNNER,
            SECTION_LAUNCH,
            SECTION_COMPAT,
            SECTION_ADVANCED,
            LABEL_TYPE,
            LABEL_CATEGORY,
            LABEL_COVER,
            LABEL_RUNNER,
            // The word the cover row draws when there is no artwork, read from
            // the widget that draws it rather than from a copy in this module —
            // see [`FIND_COVER`].
            crate::view::widgets::preview_label(),
            FIND_COVER,
            BROWSE_EXE_HINT,
            BROWSE_COVER_HINT,
            PREFIX_PLACEHOLDER,
            DESKTOP_SIZE_PLACEHOLDER,
            ADDITIONAL_APP_PLACEHOLDER,
            ENVIRONMENT_PLACEHOLDER,
            CATEGORY_PLACEHOLDER,
        ] {
            words.push(word.to_string());
        }
        for word in KIND_OPTIONS {
            words.push(word.to_string());
        }
        for row in TEXT_ROWS.iter() {
            words.push(row.label.to_string());
            words.push(row.placeholder.to_string());
        }
        for row in [&ENVIRONMENT_ROW, &DESKTOP_SIZE_ROW] {
            words.push(row.label.to_string());
            words.push(row.placeholder.to_string());
        }
        for row in LAUNCH_TOGGLES.iter().chain(COMPAT_TOGGLES.iter()) {
            words.push(row.label.to_string());
            words.push(row.subtitle.to_string());
        }

        // A floor, not a count: it is here so that a future edit which empties the
        // list above cannot turn this test into one that asserts nothing. The
        // empty placeholders are dropped for the same reason — `contains("")` is
        // always true — and they are checked as empty by the row test below.
        let words: Vec<String> = words.into_iter().filter(|word| !word.is_empty()).collect();
        assert!(
            words.len() >= 40,
            "this test has lost its material: {words:?}"
        );
        let missing: Vec<&String> = words
            .iter()
            .filter(|word| !source.contains(word.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "these are drawn by the port and are not in the reference: {missing:?}"
        );
    }

    /// **Each text row is the reference's own field**, read out of the reference.
    ///
    /// The label, the placeholder and the `enabled: !form.isLinux` are all taken
    /// from the block the reference gives that `id:`, so a row drawn under the
    /// wrong label or a placeholder that moved rows cannot pass. The `enabled`
    /// comparison is the one that matters most: it is the flag the *first* version
    /// of the builder ignored, and this is the check that would have caught it
    /// from the reference rather than from the port's own table.
    #[test]
    fn the_text_rows_are_the_reference_pages_fields() {
        let source = qml();
        let mut rows: Vec<&TextRow> = TEXT_ROWS.iter().collect();
        rows.push(&ENVIRONMENT_ROW);
        rows.push(&DESKTOP_SIZE_ROW);

        for row in rows {
            assert!(
                !row.id.is_empty(),
                "{:?} has no id, so it cannot be checked",
                row.field
            );
            assert_eq!(
                governing_label(&source, row.id),
                row.label,
                "{} is drawn under a different label",
                row.id
            );

            let block = qml_widget(&source, row.id);
            if row.placeholder.is_empty() {
                assert!(
                    !block.contains("placeholderText"),
                    "{} has no placeholder in the port and one in the reference: {block}",
                    row.id
                );
            } else {
                assert!(
                    block.contains(&format!("placeholderText: \"{}\"", row.placeholder)),
                    "{}'s placeholder is not the reference's: {block}",
                    row.id
                );
            }

            let reference_says = block.contains("enabled: !form.isLinux");
            assert_eq!(
                reference_says, row.windows_only,
                "{}: the table says windows_only={} and the reference's block says {reference_says}",
                row.id, row.windows_only
            );

            // The field the row addresses is the one the reference's own save
            // writes it to — `form.gameData.<key>`, at `:41-50`. This is what
            // binds the row to the model field rather than only to a label, and it
            // is why an exact `contains` is enough: `gameData.exePath` is a
            // substring of nothing else in the file.
            let key = row.field.form_key();
            assert!(
                source.contains(&format!("gameData.{key}")),
                "{} addresses {key:?}, which the reference never reads or writes",
                row.id
            );
        }

        // The builder addresses two of these by position, because they are laid
        // out in other sections. A reorder of the table would silently move the
        // builder onto a different row, so the two positions are pinned.
        assert_eq!(TEXT_ROWS[4].field, FormField::PrefixPath);
        assert_eq!(TEXT_ROWS[5].field, FormField::AdditionalApp);
    }

    /// **The two toggle tables are the reference's switches, in its order.**
    ///
    /// Four things at once, all out of `GameFormPage.qml`: the tables together are
    /// [`GameForm::TOGGLE_NAMES`] as a set and name no toggle twice; each row's
    /// *name* is bound to its label, subtitle and `enabled:` flag by the switch
    /// that writes that name; the rows appear in the reference's line order; and
    /// the launch/compatibility split falls where the reference's own
    /// `Compatibility` separator is.
    ///
    /// The name→label binding is the point. A table whose rows were reordered, or
    /// whose names were swapped between two rows, would keep every string in the
    /// file and still draw "DXVK:" over `fsync`.
    #[test]
    fn the_toggle_tables_are_the_reference_pages_switches() {
        let source = qml();
        let rows: Vec<&ToggleRow> = LAUNCH_TOGGLES.iter().chain(COMPAT_TOGGLES.iter()).collect();

        // 1. As a set, exactly `TOGGLE_NAMES`, once each.
        let mut names: Vec<&str> = rows.iter().map(|row| row.name).collect();
        names.sort_unstable();
        let mut expected: Vec<&str> = GameForm::TOGGLE_NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected, "the tables are not `TOGGLE_NAMES`");
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "a toggle is in the tables twice");

        // 2. Each row is the switch that writes its name.
        for row in &rows {
            let block = qml_switch(&source, row.name);
            assert_eq!(
                form_label(block),
                Some(row.label),
                "{} is drawn under a different label: {block}",
                row.name
            );
            assert!(
                block.contains(&format!("text: \"{}\"", row.subtitle)),
                "{}'s subtitle is not the reference's: {block}",
                row.name
            );
            let reference_says = block.contains("enabled: !form.isLinux");
            assert_eq!(
                reference_says, row.windows_only,
                "{}: the table says windows_only={} and the reference's switch says {reference_says}",
                row.name, row.windows_only
            );
        }

        // 3. The reference's order, and 4. where its own separator falls.
        let positions: Vec<usize> = rows
            .iter()
            .map(|row| {
                source
                    .find(&format!("form.gameData.{} = checked", row.name))
                    .unwrap_or_else(|| panic!("{} is written nowhere in the reference", row.name))
            })
            .collect();
        let mut ascending = positions.clone();
        ascending.sort_unstable();
        assert_eq!(
            positions, ascending,
            "the tables are not in the reference's order"
        );

        let divider = source
            .find("Kirigami.FormData.label: \"Compatibility\"")
            .expect("the reference separates Compatibility");
        assert!(
            positions[LAUNCH_TOGGLES.len() - 1] < divider
                && divider < positions[LAUNCH_TOGGLES.len()],
            "the launch/compatibility split is not where the reference's separator is: \
             {positions:?} against {divider}"
        );

        // The desktop-size row reads the toggle it is named after, and that name
        // is one of these rows — asserted rather than assumed, because
        // `desktop_size_enabled` looks it up by name and a typo there would simply
        // never be enabled.
        assert!(
            rows.iter().any(|row| row.name == VIRTUAL_DESKTOP_TOGGLE),
            "{VIRTUAL_DESKTOP_TOGGLE} is not a row of either table"
        );
    }

    /// **Every field of the form is either drawn from the table or accounted for
    /// by name.**
    ///
    /// A `FormField` added to the model and forgotten by the view is invisible:
    /// nothing fails, the value simply never reaches the screen. This is the check
    /// that makes the set complete, and it says *which* of the two things each
    /// field is rather than just counting them.
    #[test]
    fn the_hand_built_rows_are_the_ones_the_table_leaves_out() {
        let mut drawn: Vec<FormField> = TEXT_ROWS.iter().map(|row| row.field).collect();
        drawn.push(ENVIRONMENT_ROW.field);
        drawn.push(DESKTOP_SIZE_ROW.field);
        // Hand-built, and each for a stated reason rather than by omission:
        drawn.push(FormField::Category); // the editable combo (`:126-132`)
        drawn.push(FormField::CoverPath); // the preview and Find cover (`:134-165`)
        drawn.push(FormField::Runner); // the selector, when it is drawn (`:172-183`)

        for field in &drawn {
            assert_eq!(
                drawn.iter().filter(|other| *other == field).count(),
                1,
                "{field:?} is drawn twice"
            );
        }
        let absent: Vec<FormField> = FormField::ALL
            .iter()
            .copied()
            .filter(|field| !drawn.contains(field))
            .collect();
        assert_eq!(
            absent,
            vec![FormField::SteamAppid],
            "the set of fields the form does not draw changed; it was exactly the field the \
             reference has no control for either"
        );

        // That last claim, checked against the reference rather than asserted:
        // `steamAppid` appears only in the cover fetch's reply, which *writes* it.
        let source = qml();
        let connections_at = source
            .find("Connections {")
            .expect("the reference has the cover-fetch reply");
        let connections = balanced(&source, connections_at + "Connections ".len());
        assert!(
            source.matches("steamAppid").count() > 0,
            "the reference no longer mentions `steamAppid`; this checks nothing"
        );
        assert_eq!(
            connections.matches("steamAppid").count(),
            source.matches("steamAppid").count(),
            "the reference writes `steamAppid` outside the cover fetch, so it does have a \
             control for it and the port must draw one"
        );
    }

    /// The `enabled:` gate, as a function — including the case the first builder
    /// got wrong.
    ///
    /// `windows_only` is per *row*, not per form: the reference gates Wayland, HDR,
    /// Esync, Fsync and the six compatibility switches on `!form.isLinux`, and
    /// leaves MangoHud, GameMode, Prefer SDL and Gamescope alone. The first version
    /// of `view` passed the form-wide flag to every switch, which would have
    /// disabled four controls the reference keeps live.
    #[test]
    fn a_windows_only_row_is_disabled_only_on_a_linux_game() {
        assert!(row_enabled(false, false));
        assert!(
            row_enabled(false, true),
            "a row the reference does not gate must stay live on a Linux game"
        );
        assert!(row_enabled(true, false));
        assert!(!row_enabled(true, true));
    }

    /// `formCategories` (`bridge.py:344-350`): the built-ins, then the library's
    /// extras, in the reference's own order.
    #[test]
    fn form_categories_are_the_built_ins_then_the_libraries_own() {
        // "Zelda" first and "Arcade" second, so the extras' order is the
        // library's rather than the insertion order's.
        let held = library("cats", &["Zelda", "Uncategorized", "Action", "Arcade"]);
        let names = form_categories(&held);

        assert_eq!(
            names.len(),
            DEFAULT_CATEGORIES.len() + 2,
            "a built-in the library also holds was added a second time: {names:?}"
        );
        let built_ins: Vec<&str> = names
            .iter()
            .take(DEFAULT_CATEGORIES.len())
            .map(String::as_str)
            .collect();
        assert_eq!(built_ins, DEFAULT_CATEGORIES.to_vec());
        assert_eq!(
            &names[DEFAULT_CATEGORIES.len()..],
            &["Arcade".to_string(), "Zelda".to_string()],
            "the library's extras are not in `Library::categories` order"
        );

        // Deliberately not sorted: `Uncategorized` is first here and near-last in
        // a sorted list, and the selector's order is user-visible.
        let mut sorted = names.clone();
        sorted.sort();
        assert_ne!(
            names, sorted,
            "the list got sorted, which moves Uncategorized"
        );

        // A library with nothing in it still offers the built-ins.
        let no_games = library("cats-empty", &[]);
        assert_eq!(
            form_categories(&no_games),
            DEFAULT_CATEGORIES
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>()
        );
    }

    /// The Type selector's index is the mapping, in both directions.
    ///
    /// `readonly property bool isLinux: kindBox.currentIndex === 1`
    /// (`GameFormPage.qml:16`) — the index is what the reference stores, so the
    /// port's two directions have to agree with *that* rather than with the
    /// label's text.
    #[test]
    fn the_kind_selectors_two_directions_agree() {
        assert_eq!(KIND_OPTIONS.len(), 2);
        assert_eq!(kind_index(false), 0);
        assert_eq!(kind_index(true), 1);
        assert!(!kind_is_linux(kind_index(false)));
        assert!(kind_is_linux(kind_index(true)));

        assert!(matches!(kind_selection(0), Message::SetFormLinux(false)));
        assert!(matches!(kind_selection(1), Message::SetFormLinux(true)));

        assert!(
            qml().contains("property bool isLinux: kindBox.currentIndex === 1"),
            "the reference no longer maps the index this way"
        );
    }

    /// `valueRole: "runnerId"` (`:177`): the selector carries the **id**, so the
    /// message has to be built from `choices`' id half and not from the label the
    /// dropdown was handed.
    ///
    /// # The fixture's ids and labels are all different, and that is the test
    ///
    /// The first version of this used `("GE-Proton9-1", "GE-Proton9-1")` for the
    /// entry it selected, because that is what [`RunnerManager::choices`] produces
    /// for an unrenamed Proton build. Mutating `runner_selection` to read the
    /// *label* half then left the suite green: the two halves were the same
    /// string, so no assertion could tell them apart. Every pair below is
    /// deliberately unequal, and the entry that is selected is
    /// [`SYSTEM_WINE`], whose real id and real label are different strings.
    ///
    /// [`RunnerManager::choices`]: gamehandler_core::runners::RunnerManager::choices
    /// [`SYSTEM_WINE`]: gamehandler_core::models::SYSTEM_WINE
    #[test]
    fn the_runner_selection_carries_the_id_and_not_the_label() {
        // System Wine's real id and label, from `choices()` (`runners/mod.rs:1322-1327`).
        let choices = vec![
            (
                gamehandler_core::models::SYSTEM_WINE.to_string(),
                "System Wine".to_string(),
            ),
            (
                "GE-Proton9-1".to_string(),
                "Proton 9.1 (renamed)".to_string(),
            ),
        ];
        // No two halves in this fixture are equal, so a mutation that reads the
        // wrong half cannot pass by coincidence — the defect the doc above records.
        let mut halves: Vec<&String> = choices.iter().flat_map(|(id, label)| [id, label]).collect();
        halves.sort();
        let all = halves.len();
        halves.dedup();
        assert_eq!(
            halves.len(),
            all,
            "the fixture has an id equal to a label, so it \
            cannot tell `runnerId` from `textRole`: {choices:?}"
        );

        assert_eq!(runner_index(&choices, "GE-Proton9-1"), 1);
        // A runner that is not installed degrades to index 0, which is System
        // Wine, rather than to a blank selector (`:180-182`).
        assert_eq!(runner_index(&choices, "not-installed"), 0);

        assert!(matches!(
            runner_selection(&choices, 1),
            Message::FormFieldChanged { field: FormField::Runner, ref value }
                if value == "GE-Proton9-1"
        ));
        // Entry zero too, whose id and label are the least alike — this is the
        // assertion the old fixture could not make.
        assert!(matches!(
            runner_selection(&choices, 0),
            Message::FormFieldChanged { ref value, .. }
                if value == gamehandler_core::models::SYSTEM_WINE
        ));

        // An index past the end is the same collapse `runner_index` makes.
        for index in [2usize, 9] {
            assert!(matches!(
                runner_selection(&choices, index),
                Message::FormFieldChanged { ref value, .. }
                    if value == gamehandler_core::models::SYSTEM_WINE
            ));
        }
        assert!(matches!(
            runner_selection(&[], 0),
            Message::FormFieldChanged { ref value, .. }
                if value == gamehandler_core::models::SYSTEM_WINE
        ));

        // And the settings page's selector, which is the same mapping one module
        // over — asserted here so the two cannot drift apart silently.
        assert!(matches!(
            crate::view::settings::default_runner_selection(&choices, 1),
            Message::SetDefaultRunner(ref value) if value == "GE-Proton9-1"
        ));
        assert!(matches!(
            crate::view::settings::default_runner_selection(&choices, 0),
            Message::SetDefaultRunner(ref value)
                if value == gamehandler_core::models::SYSTEM_WINE
        ));

        // And the **Installers** page's, which was the third sibling and is the
        // one this test could not see. #97/D-55: the closure was
        // `move |index| Message::SetDefaultRunner(index.to_string())` — the
        // position, written as though it were the id — so the two lines below
        // assert the mapping the page was missing rather than a third copy of
        // the two above. Note which *destination* it maps to as well: the
        // per-install choice, not `SetDefaultRunner`; see [`D-55`] and
        // [`super::installers::seeded_install_runner`].
        //
        // [`D-55`]: ../../../../docs/migration/DECISIONS.md
        assert!(matches!(
            crate::view::installers::install_runner_selection(&choices, 1),
            Message::SetInstallRunner(ref value) if value == "GE-Proton9-1"
        ));
        assert!(matches!(
            crate::view::installers::install_runner_selection(&choices, 0),
            Message::SetInstallRunner(ref value)
                if value == gamehandler_core::models::SYSTEM_WINE
        ));

        // The category selector of that same page, which had #97's twin: its
        // closure's parameter was named `category` and was a `usize`, so
        // `category.to_string()` stored `"1"` where the model wanted
        // `"Launchers"`, and the catalog came back empty. `library.rs` fixed
        // this exact shape once already (`category_selection`); this is the same
        // mapping for this page's sentinel-carrying list.
        let categories = crate::view::installers::installer_categories();
        assert!(matches!(
            crate::view::installers::category_selection(&categories, 1),
            Message::SetInstallerCategory(ref value) if value == "Launchers"
        ));
        assert!(matches!(
            crate::view::installers::category_selection(&categories, 0),
            Message::SetInstallerCategory(ref value)
                if value == crate::view::installers::ALL_CATEGORIES
        ));
    }

    // ---- The widened guard: a dropdown's callback receives a *position* ------
    //
    // The test above compares three hand-named mappings, and D-55's whole point
    // is that it could not have caught #97: there were two mappings to compare
    // and the third selector had none, so there was nothing to compare it with.
    // A guard over an explicit list is blind to the unlisted, and adding a third
    // name to the list would have left the next selector exactly as invisible.
    //
    // So this one names no subjects at all. It reads the view layer's own source
    // — every `.rs` file the directory holds, found with `read_dir` rather than
    // listed, plus `main.rs` — and applies one rule to every dropdown callback
    // it finds:
    //
    // > `dropdown`'s `on_selected` is `impl Fn(usize) -> Message`
    // > (`libcosmic src/widget/dropdown/mod.rs:30`), so the callback's
    // > parameter is a **position**. A callback that converts that parameter
    // > itself — `|index| SetX(index.to_string())`, `|category|
    // > SetCategory(category.to_string())` — is passing the position where the
    // > model's value belongs. The value must come out of the model: either a
    // > bracket lookup (`SORT_OPTIONS[index].0`) or a call to a named mapping
    // > (`default_runner_selection(&choices, index)`), the shape `library.rs`
    // > settled on after the same defect.
    //
    // Three of these were live in `view/installers.rs` at `cc81be7`: the runner
    // selector (#97) and both halves of the category selector. `library.rs`'s
    // `category_selection` doc records the same bug being fixed there by hand,
    // which is what says a rule is worth having rather than a fourth fix.
    //
    // # What this cannot see, stated rather than implied
    //
    // It is a text parser over source, not a type check — the same class of
    // instrument as `tests/dispatch_coverage.rs`, with the same kind of limits:
    //
    //   * **Only the conversions it names.** A payload written as
    //     `String::from(index)`, `index as char`, or baked into a struct field
    //     by a helper escapes it. The set is `to_string`, `to_owned`, `clone`,
    //     `into`, `as_str`, `as_ref`, and any `format!` whose braces mention the
    //     parameter — the shapes that have actually appeared here.
    //   * **A wrong list passes.** `labels[index].clone()` looks up *a* model,
    //     and this rule cannot know which one the message wants. The value-level
    //     assertions above are what pin that half.
    //   * **A callback whose body calls a helper that itself stringifies the
    //     index passes**, because the body no longer mentions the parameter. That
    //     is deliberate: the helper *is* the fix's shape, and it is where the
    //     value-level test can reach.
    //   * **A dropdown built by another spelling** (`popup_dropdown`, or a
    //     `Dropdown::new` chain) is not a `widget::dropdown(` call and is not
    //     seen. The non-vacuity floor below is what fails loudly if the calls
    //     this reads are ever all renamed away.

    /// Every `.rs` file the view directory holds, plus `main.rs`, as
    /// `(name, source)` with comments blanked and the `#[cfg(test)]` modules
    /// cut — because this module's own samples are strings that contain the
    /// defect, and a scanner that read them would report itself.
    fn production_sources() -> Vec<(String, String)> {
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // Discovered, not listed. This used to read `src/view` and then push
        // `src/main.rs` by hand, which made the haystack an enumeration: the
        // day ARCH-11 moved a region of `main.rs` into `src/easy_install.rs`,
        // this scanner quietly stopped reading that region. It found nothing
        // there, so nothing went red — which is the whole problem. A scanner
        // that covers less than it claims passes on the defect it exists to
        // catch, and it does so silently. A directory read cannot go stale that
        // way: a new top-level module is scanned the moment it is a file.
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for dir in ["src", "src/view"] {
            paths.extend(
                std::fs::read_dir(crate_dir.join(dir))
                    .unwrap_or_else(|error| {
                        panic!("`{dir}` is where this crate keeps its code: {error}")
                    })
                    .map(|entry| entry.expect("a readable directory entry").path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "rs")),
            );
        }
        paths.sort();
        // Anti-vacuity, in the shape the rest of this audit uses: the two
        // directories this reads are asserted to have been read. Emptying the
        // list would otherwise report every source clean.
        assert!(
            paths.len() >= 20,
            "the scan found only {} source files, so it is not reading the tree: {paths:?}",
            paths.len()
        );
        assert!(
            paths.iter().any(|path| path.ends_with("src/main.rs")),
            "the scan did not find `src/main.rs`: {paths:?}"
        );

        paths
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
                let name = path
                    .strip_prefix(crate_dir)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                (name, production_source(&text))
            })
            .collect()
    }

    /// One file's production source: comments gone, test modules gone.
    fn production_source(text: &str) -> String {
        // Two passes over the same lexer. The structural pass blanks string
        // literals too, because brace matching must not count a `{` inside one;
        // the readable pass keeps them, because `format!("BOGUS-{index}")` is
        // one of the shapes the rule has to see, and its braces live in a
        // string. The cut is computed on the first and applied to both.
        let structural = lex(text, true);
        let readable = lex(text, false);
        let mut readable = readable;
        for (start, end) in test_module_ranges(&structural) {
            for character in &mut readable[start..end] {
                if *character != '\n' {
                    *character = ' ';
                }
            }
        }
        readable.into_iter().collect()
    }

    /// `src` as chars, with every comment blanked to spaces (offsets kept so a
    /// failure can name a line) and, when `blank_strings`, every string and
    /// char literal blanked with it.
    fn lex(src: &str, blank_strings: bool) -> Vec<char> {
        let chars: Vec<char> = src.chars().collect();
        let mut out = chars.clone();
        // Bounds-checked because an unterminated literal has to leave the
        // scanner running to the end rather than panicking on the way.
        let blank = |out: &mut Vec<char>, index: usize| {
            if blank_strings && index < out.len() && out[index] != '\n' {
                out[index] = ' ';
            }
        };
        let mut index = 0;
        while index < chars.len() {
            let current = chars[index];
            if current == '/' && chars.get(index + 1) == Some(&'/') {
                while index < chars.len() && chars[index] != '\n' {
                    out[index] = ' ';
                    index += 1;
                }
            } else if current == '/' && chars.get(index + 1) == Some(&'*') {
                out[index] = ' ';
                out[index + 1] = ' ';
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    out[index] = ' ';
                    index += 1;
                }
                for _ in 0..2 {
                    if index < chars.len() {
                        out[index] = ' ';
                        index += 1;
                    }
                }
            } else if current == '"' {
                blank(&mut out, index);
                index += 1;
                while index < chars.len() && chars[index] != '"' {
                    if chars[index] == '\\' {
                        blank(&mut out, index);
                        index += 1;
                    }
                    blank(&mut out, index);
                    index += 1;
                }
                blank(&mut out, index);
                index += 1;
            } else if let Some(length) = char_literal_at(&chars, index) {
                for _ in 0..length {
                    blank(&mut out, index);
                    index += 1;
                }
            } else {
                index += 1;
            }
        }
        out
    }

    /// The length of the char literal starting at `index`, or `None` when
    /// `chars[index]` is not a quote or the `'` opens a lifetime (`&'a str`).
    ///
    /// Lifetimes are why this exists: a lexer that treats every `'` as a literal
    /// opener swallows the rest of the line, and every view module is full of
    /// them in `Element<'a, Message>`.
    ///
    /// The `chars[index] == '\''` test is not decoration. Without it the arms
    /// below match on the three characters *after* `index`, so the `d` of
    /// `rest.find('"')` looks like a three-character literal `('` and the scan
    /// blanks `d('` and steps onto the `"`, which then opens a string that runs
    /// to the next quote on the following line. That is what made this file's
    /// `#[cfg(test)] mod tests` appear to end at `fn form_label` (line 957 of
    /// 2,100) and made the guard below report its own fixture strings as
    /// findings — a false positive whose cause was one blanked character.
    fn char_literal_at(chars: &[char], index: usize) -> Option<usize> {
        if chars.get(index) != Some(&'\'') {
            return None;
        }
        match (
            chars.get(index + 1),
            chars.get(index + 2),
            chars.get(index + 3),
        ) {
            (Some('\\'), Some(_), Some('\'')) => Some(4),
            (Some(_), Some('\''), _) => Some(3),
            _ => None,
        }
    }

    /// The `#[cfg(test)] mod` ranges of an already-lexed source.
    fn test_module_ranges(chars: &[char]) -> Vec<(usize, usize)> {
        let marker: Vec<char> = "#[cfg(test)]".chars().collect();
        let mut ranges = Vec::new();
        let mut from = 0usize;
        while let Some(start) = find_chars(chars, &marker, from) {
            let after = start + marker.len();
            let word: String = chars[after..]
                .iter()
                .skip_while(|character| character.is_whitespace())
                .take(3)
                .collect();
            if word == "mod" {
                let open = (after..chars.len())
                    .find(|index| chars[*index] == '{')
                    .expect("a module has a body");
                let end = matching(chars, open).unwrap_or(chars.len());
                ranges.push((start, end));
                from = end;
            } else {
                from = after;
            }
        }
        ranges
    }

    /// The first `needle` at or after `from`.
    fn find_chars(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (from..=haystack.len() - needle.len())
            .find(|start| haystack[*start..*start + needle.len()] == *needle)
    }

    /// The index of the `)` that closes the `(` at `open`.
    ///
    /// Bracket depth is tracked over all three kinds so a `)` inside a `[...]`
    /// or a `{...}` cannot close the call early. Strings are not consulted: both
    /// callers pass text whose string literals have already been handled.
    fn matching(chars: &[char], open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (offset, character) in chars[open..].iter().enumerate() {
            match character {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(open + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every `widget::dropdown(` call's third argument — the selection callback
    /// — as `(offset, text)`.
    fn dropdown_callbacks(source: &[char]) -> Vec<(usize, String)> {
        let needle: Vec<char> = "widget::dropdown(".chars().collect();
        let mut found = Vec::new();
        let mut from = 0usize;
        while let Some(start) = find_chars(source, &needle, from) {
            let open = start + needle.len() - 1;
            let Some(close) = matching(source, open) else {
                break;
            };
            let arguments: Vec<char> = source[open + 1..close].to_vec();
            let parts = split_top_level(&arguments);
            if let Some((offset, callback)) = parts.get(2) {
                // The callback's *own* line, not the call's. `offset` is where
                // the third argument begins, which is the line of the comma
                // before it; the leading whitespace is skipped so the number
                // below points at the closure a reader has to look at.
                let lead = callback
                    .iter()
                    .take_while(|character| character.is_whitespace())
                    .count();
                found.push((
                    open + 1 + offset + lead,
                    callback.iter().collect::<String>(),
                ));
            }
            from = close;
        }
        found
    }

    /// The comma-separated arguments of a call, split only at depth zero, each
    /// with the offset it starts at — offsets are what let a finding cite the
    /// callback's line rather than the call's.
    fn split_top_level(arguments: &[char]) -> Vec<(usize, Vec<char>)> {
        let mut parts = Vec::new();
        let mut current = Vec::new();
        let mut start = 0usize;
        let mut depth = 0i32;
        for (index, character) in arguments.iter().enumerate() {
            match character {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push((start, std::mem::take(&mut current)));
                    start = index + 1;
                    continue;
                }
                _ => {}
            }
            current.push(*character);
        }
        if !current.iter().all(|character| character.is_whitespace()) {
            parts.push((start, current));
        }
        parts
    }

    /// The closure a callback argument is, as `(parameter, body)`; `None` when
    /// it is a named function, which is the shape the rule is asking for.
    fn closure_parts(callback: &str) -> Option<(String, String)> {
        let chars: Vec<char> = callback.chars().collect();
        let first = chars.iter().position(|character| *character == '|')?;
        let second = chars[first + 1..]
            .iter()
            .position(|character| *character == '|')?
            + first
            + 1;
        if second == first + 1 {
            return None; // `||`: no parameters, nothing to get wrong
        }
        let header: String = chars[first + 1..second].iter().collect();
        let parameter = header.split(':').next()?.trim().to_string();
        let parameter = parameter
            .trim_start_matches("mut ")
            .trim_start_matches('&')
            .trim()
            .to_string();
        if parameter.is_empty() {
            return None;
        }
        Some((parameter, chars[second + 1..].iter().collect()))
    }

    /// The conversions of a callback's own parameter that the rule refuses.
    const INDEX_CONVERSIONS: [&str; 6] = [
        "to_string()",
        "to_owned()",
        "clone()",
        "into()",
        "as_str()",
        "as_ref()",
    ];

    /// Every dropdown callback in `src` that passes its own index where the
    /// model's value belongs. `src` is raw source, as it sits in the file.
    fn stringified_index_findings(src: &str) -> Vec<String> {
        let source = production_source(src);
        let chars: Vec<char> = source.chars().collect();
        let mut findings = Vec::new();
        for (offset, callback) in dropdown_callbacks(&chars) {
            let Some((parameter, body)) = closure_parts(&callback) else {
                continue;
            };
            let line = 1 + chars[..offset].iter().filter(|c| **c == '\n').count();
            let trimmed = callback.trim();
            for conversion in INDEX_CONVERSIONS {
                if body.contains(&format!("{parameter}.{conversion}")) {
                    findings.push(format!(
                        "line {line}: `{trimmed}` converts the callback's own index \
                         with `{parameter}.{conversion}` — the parameter is a \
                         position (`libcosmic src/widget/dropdown/mod.rs:30`), so \
                         the message carries the position where the model's value \
                         belongs"
                    ));
                }
            }
            for open in format_macro_openings(&body) {
                let arguments: Vec<char> = body.chars().collect();
                let Some(close) = matching(&arguments, open) else {
                    continue;
                };
                let inside: String = arguments[open + 1..close].iter().collect();
                if contains_word(&inside, &parameter) {
                    findings.push(format!(
                        "line {line}: `{trimmed}` formats the callback's own index \
                         into the message — `{inside}` is a rendered position, not \
                         the model's value"
                    ));
                }
            }
        }
        findings
    }

    /// The offsets of every `format!(` in `body`.
    fn format_macro_openings(body: &str) -> Vec<usize> {
        let chars: Vec<char> = body.chars().collect();
        let needle: Vec<char> = "format!".chars().collect();
        let mut found = Vec::new();
        let mut from = 0usize;
        while let Some(start) = find_chars(&chars, &needle, from) {
            let mut index = start + needle.len();
            while chars.get(index).is_some_and(|c| c.is_whitespace()) {
                index += 1;
            }
            if chars.get(index) == Some(&'(') {
                found.push(index);
            }
            from = index.max(start + 1);
        }
        found
    }

    /// Whether `word` appears in `haystack` as a whole identifier.
    fn contains_word(haystack: &str, word: &str) -> bool {
        let mut from = 0usize;
        while let Some(found) = haystack[from..].find(word) {
            let start = from + found;
            let end = start + word.len();
            let before = haystack[..start].chars().next_back();
            let after = haystack[end..].chars().next();
            let boundary = |character: Option<char>| {
                !character.is_some_and(|c| c.is_alphanumeric() || c == '_')
            };
            if boundary(before) && boundary(after) {
                return true;
            }
            from = end;
        }
        false
    }

    /// The instrument, driven on the defect it was written for and on the fix.
    ///
    /// A guard that reports nothing on the real tree is only worth anything if
    /// it reports something on the tree it was written to reject, so both live
    /// defects are run through the same entry point the scan uses — string
    /// literals, test-cut and all — and the three correct shapes with them.
    #[test]
    fn the_dropdown_guard_reports_the_defect_it_was_written_for() {
        // `view/installers.rs` at `cc81be7`, verbatim, and the `format!` variant
        // of the same mistake.
        for defect in [
            "let _ = cosmic::widget::dropdown(\n  labels,\n  runner_index(page.runners, page.runner_id),\n  move |index| Message::SetDefaultRunner(index.to_string()),\n);",
            "let _ = cosmic::widget::dropdown(\n  page.categories.to_vec(),\n  selected,\n  |category| Message::SetInstallerCategory(category.to_string()),\n);",
            "let _ = cosmic::widget::dropdown(\n  labels,\n  None,\n  |index| Message::SetInstallRunner(format!(\"BOGUS-{index}\")),\n);",
        ] {
            let findings = stringified_index_findings(defect);
            assert_eq!(
                findings.len(),
                1,
                "the instrument found {findings:?} in a callback that passes its \
                 own index where the model's value belongs: {defect}"
            );
        }

        // Every shape that is right, and must not be reported: the three named
        // mappings this tree has, the two bracket lookups, and a callback with
        // no parameters.
        for correct in [
            "let _ = cosmic::widget::dropdown(\n  labels,\n  Some(0),\n  move |index| crate::view::settings::default_runner_selection(&choices, index),\n);",
            "let _ = cosmic::widget::dropdown(\n  labels,\n  Some(0),\n  { let owned = choices.clone(); move |index| runner_selection(&owned, index) },\n);",
            "let _ = cosmic::widget::dropdown(\n  page.categories.to_vec(),\n  selected,\n  move |index| category_selection(&page_categories, index),\n);",
            "let _ = cosmic::widget::dropdown(\n  sort_labels(),\n  sort_index(page.sort_mode),\n  |index| Message::SetSortMode(SORT_OPTIONS[index].0.to_string()),\n);",
            "let _ = cosmic::widget::dropdown(\n  labels,\n  None,\n  || Message::SetX,\n);",
        ] {
            let findings = stringified_index_findings(correct);
            assert!(
                findings.is_empty(),
                "the instrument reported {findings:?} in the correct callback \
                 `{correct}` — a guard that fires on the fix is a guard somebody \
                 deletes"
            );
        }
    }

    /// And the instrument on this tree: nothing, over every file, with a floor
    /// so the silence cannot mean the scanner stopped matching.
    #[test]
    fn no_dropdown_callback_turns_its_index_into_the_payload() {
        let sources = production_sources();
        let mut findings = Vec::new();
        let mut callbacks = 0usize;
        let mut closures = 0usize;
        for (name, source) in &sources {
            let chars: Vec<char> = source.chars().collect();
            let found = dropdown_callbacks(&chars);
            closures += found
                .iter()
                .filter(|(_, callback)| closure_parts(callback).is_some())
                .count();
            callbacks += found.len();
            findings.extend(
                stringified_index_findings(source)
                    .into_iter()
                    .map(|finding| format!("{name}: {finding}")),
            );
        }

        assert!(
            findings.is_empty(),
            "a dropdown callback is carrying its index where the model's value \
             belongs — the defect `library.rs` fixed by hand and `#97` shipped \
             anyway:\n  {}",
            findings.join("\n  ")
        );
        assert!(
            callbacks >= 6,
            "the scan found only {callbacks} `widget::dropdown` callbacks over \
             {} files, so its silence above is not evidence: {sources:?}",
            sources.len()
        );
        assert!(
            closures >= 4,
            "the scan found {callbacks} callbacks but only {closures} that are \
             closures — the rule has no subjects, which is what a scanner that \
             stopped matching looks like"
        );
    }

    /// The other reason `#97` is a defect, and the one neither rule above can see.
    ///
    /// `#97` is usually told as two wrongs — the index where a value belongs, and
    /// the global default where the install belongs. Both are covered above. The
    /// third is the one the *repair's own reasoning* names and nothing enforced:
    /// the value has to be a runner **id**.
    ///
    /// `RunnerManager::choices` (`core/src/runners/mod.rs:1322`) hands the
    /// selector `(id, label)` pairs, and for System Wine those differ —
    /// `("wine-system", "System Wine")`. So a callback that carries the *label*
    /// reaches the right message, with a value no runner answers to, on that one
    /// row; `RunnerManager::get` (`mod.rs:1087`) answers an unknown id with System
    /// Wine **silently**, which is the runner the user picked, so the wrong value
    /// and the right one produce the same install. Measured when the gap was
    /// open: replacing the runner call site with
    /// `Message::SetInstallRunner(runner_choices[index].1.clone())` left the
    /// whole binary green — the value was never a runner id in any test that read
    /// the call site, because no test read the call site.
    ///
    /// **That hole is closed, and not by the test this block predicted.** The
    /// paragraph used to end by proposing "a source scan over
    /// `view/installers.rs`" and naming `install_runner_selection`'s callers;
    /// what actually landed is `both_installers_selectors_are_routed_through_their_own_mapping`
    /// below, in `a719972`, which reads `view/installers.rs` out of the tree and
    /// requires each `widget::dropdown` callback there to name exactly one of the
    /// two mappings. Re-measured at HEAD: the swap above now fails that scan
    /// (`the callback ... does not go through exactly one of the two mappings`),
    /// where it previously left the binary green. The count the block used to
    /// carry — "312 tests" — is gone with the gap rather than renumbered, since a
    /// number in a comment is evidence only beside the commit and the method that
    /// produced it (ARCH-20).
    ///
    /// The mapping is a named function so that both halves become checkable, and
    /// this is the second half: the page's own tests pin what
    /// `install_runner_selection` returns from an index, and this pins that the
    /// callbacks *are* that function. Either half alone is a guard with no
    /// subject — the first because a perfect function nobody calls is dead, the
    /// second because a call site is not a value.
    ///
    /// Both selectors, not the runner's: `#102` was this defect one line down, in
    /// the same file, and a fix aimed at one dropdown in a file that has two
    /// leaves the twin live. That is why the rule is a count of two *and* a
    /// per-callback assertion, rather than one assertion at the runner's call
    /// site — the count is the half that makes "both" true. Measured, so the
    /// claim is not assumed: pointing the category's callback at the runner's
    /// mapping does not compile (`expected &[String], found
    /// &Vec<(String, String)>`), so "their **own** mapping" is the type system's
    /// half and this rule only has to say that each callback has one.
    ///
    /// What it cannot see: a third selector added to this page, beyond the count
    /// below. It is a tripwire on the two this tree has, not a proof.
    #[test]
    fn both_installers_selectors_are_routed_through_their_own_mapping() {
        // The two mappings that know what their model holds.
        const MAPPINGS: [&str; 2] = ["category_selection", "install_runner_selection"];

        let (name, source) = production_sources()
            .into_iter()
            .find(|(name, _)| name == "src/view/installers.rs")
            .expect("the installers page is where both selectors live");
        let chars: Vec<char> = source.chars().collect();
        let callbacks = dropdown_callbacks(&chars);

        assert_eq!(
            callbacks.len(),
            2,
            "{name} no longer has the two `widget::dropdown` callbacks this rule \
             is written for, so its silence below is not evidence: {callbacks:?}"
        );

        for (_, callback) in &callbacks {
            let mapped: Vec<&str> = MAPPINGS
                .iter()
                .copied()
                .filter(|mapping| callback.contains(mapping))
                .collect();
            assert_eq!(
                mapped.len(),
                1,
                "the callback `{}` in {name} does not go through exactly one of \
                 the two mappings that know what the model holds. A callback that \
                 builds its own `Message` has the index, or the label, and there \
                 is nothing left that can tell: the model holds `(id, label)` \
                 pairs (`core/src/runners/mod.rs:1322`), so a label reaches the \
                 right control with a value no runner answers to, and `get` \
                 (`:1087`) answers it with System Wine *silently*",
                callback.trim()
            );
        }
    }

    /// The other half of `#97`, and the repair `D-55` warns is the plausible one.
    ///
    /// A callback can carry the *right* value to the *wrong* control, and no
    /// index-shaped rule can see it. `SetDefaultRunner` writes the app's default
    /// runner, which the reference gives exactly one control —
    /// `SettingsPage.qml`'s combo. The installers page never writes it:
    /// `InstallersPage.qml:49-64` has no write-back at all, `:55`/`:61` only
    /// *read* `defaultRunner`, and the runner the user picks goes to `installEasy`
    /// as an argument (`:128-130`), where `bridge.py:840`/`:855` treat it as the
    /// install's value and fall back to the global default only when it is empty.
    ///
    /// So routing that page's selector through
    /// [`crate::view::settings::default_runner_selection`] — the one-character
    /// repair that fixes the value and keeps the destination — compiles, reads
    /// like parity, passes every test written against either half alone, and
    /// makes choosing a runner for one install silently rewrite the app's
    /// default. This test is the one that fails on it.
    ///
    /// It is a scan for the *paths to that destination*, not for one spelling of
    /// it: the variant, and the helper that builds it. Both are needed — the
    /// plausible repair does not write `Message::SetDefaultRunner(`, it calls
    /// [`crate::view::settings::default_runner_selection`], which is a
    /// grep for the variant away from the defect. What this rule cannot see is a
    /// *third* route to that write, and it is not a proof that none exists: it is
    /// a tripwire on the two routes this tree has, and the doc above says why the
    /// second one is the dangerous one.
    #[test]
    fn the_global_default_runner_is_written_by_the_settings_page_alone() {
        const ROUTES: [&str; 2] = ["SetDefaultRunner", "default_runner_selection"];
        let sources = production_sources();
        let mut offenders = Vec::new();
        let mut owners = Vec::new();
        for (name, source) in &sources {
            let reached: Vec<&str> = ROUTES
                .iter()
                .copied()
                .filter(|route| source.contains(route))
                .collect();
            if reached.is_empty() {
                continue;
            }
            // `main.rs` is the handler, not a page: its arm *matches* the
            // variant, and matching is not writing.
            if name == "src/main.rs" {
                continue;
            }
            if name == "src/view/settings.rs" {
                owners.push(name.clone());
            } else {
                offenders.push(format!("{name} (`{}`)", reached.join("`, `")));
            }
        }

        assert!(
            offenders.is_empty(),
            "{} reaches the app's *global* default runner — a control \
             `SettingsPage.qml` owns and `InstallersPage.qml:49-64` deliberately \
             does not have (P-53's first clause is about the *install*). The \
             runner a page picks for its own action goes through that page's own \
             mapping (`install_runner_selection` here); see `D-55`",
            offenders.join(", ")
        );
        assert_eq!(
            owners,
            vec!["src/view/settings.rs".to_string()],
            "the settings page no longer reaches `SetDefaultRunner` by either \
             route, so the rule above has no subject and its silence is not \
             evidence"
        );
    }

    /// The category combo is `editable: true` (`:129`), so the port draws a
    /// dropdown *and* a text field: an index into the list is only a convenience,
    /// and a category the list does not have is not an error.
    #[test]
    fn the_category_selector_indexes_the_list_and_nothing_else() {
        let categories = form_categories(&library("category-index", &[]));
        let action = DEFAULT_CATEGORIES
            .iter()
            .position(|name| *name == "Action")
            .expect("Action is a built-in");

        assert_eq!(category_index(&categories, "Action"), Some(action));
        assert_eq!(
            category_index(&categories, "  Action  "),
            Some(action),
            "the reference writes `categoryBox.editText.trim()` (`:47`), so a padded \
             category is the same category"
        );
        assert_eq!(
            category_index(&categories, &UNCATEGORIZED.to_lowercase()),
            None,
            "the comparison is exact, not case-insensitive"
        );
        // No fallback to 0: highlighting a category the user did not type would be
        // a lie about their own form.
        assert_eq!(category_index(&categories, "A category nobody has"), None);

        assert!(matches!(
            category_selection(&categories, action),
            Message::FormFieldChanged { field: FormField::Category, ref value } if value == "Action"
        ));
        // Past the end is the empty string, which `GameForm::apply` folds to
        // `UNCATEGORIZED` — the same value a blank field gives.
        assert!(matches!(
            category_selection(&categories, 999),
            Message::FormFieldChanged { ref value, .. } if value.is_empty()
        ));
    }

    /// `enabled: nameField.text.trim().length > 0` (`:34`), and the message it
    /// gates.
    #[test]
    fn saving_is_gated_on_a_trimmed_name() {
        let mut form = GameForm::default();
        assert!(!can_save(&form), "an empty name is not a name");
        assert!(
            save_message(&form).is_none(),
            "the button sends nothing rather than a save the model refuses"
        );

        form.name = "   ".to_string();
        assert!(
            !can_save(&form),
            "the reference trims before it measures (`:34`)"
        );

        form.name = "  Celeste  ".to_string();
        assert!(can_save(&form));
        assert!(matches!(
            save_message(&form),
            Some(Message::SaveGameForm(_))
        ));
    }

    /// Each switch sends the name of *its own* row.
    ///
    /// The alias defect [`crate::view::plugins::install_message`] was corrected
    /// for: a builder that reached for a name other than the row it was drawing
    /// would flip one toggle and show another. Taken from the row rather than from
    /// a literal, so there is no second spelling to disagree.
    ///
    /// The round trip is asserted against **the value the switch was drawn at**,
    /// not against `false`: [`GameForm::new_template`] starts six of the fifteen
    /// switches *on*, because `_DEFAULTED_TOGGLES` does, so a test written the
    /// other way would have to know which six and would read here as a table that
    /// had stopped working.
    #[test]
    fn a_switch_sends_the_name_of_its_own_row() {
        let mut form = form();
        for row in LAUNCH_TOGGLES.iter().chain(COMPAT_TOGGLES.iter()) {
            let drawn_at = toggle_value(&form, row);
            assert!(
                matches!(
                    toggle_message(row, !drawn_at),
                    Message::FormToggleChanged { ref name, value } if name == row.name && value != drawn_at
                ),
                "{}'s switch does not send its own name",
                row.name
            );

            assert!(
                form.set_toggle(row.name, !drawn_at),
                "{} is not the form's own name",
                row.name
            );
            assert_eq!(
                toggle_value(&form, row),
                !drawn_at,
                "{} did not take the write",
                row.name
            );
            assert!(form.set_toggle(row.name, drawn_at));
            assert_eq!(toggle_value(&form, row), drawn_at);
        }

        // A name the form does not hold reads `false` rather than panicking; the
        // table being right is what makes that unreachable, and
        // `the_toggle_tables_…` is what keeps the table right.
        let row = ToggleRow {
            name: "not-a-toggle",
            label: "",
            subtitle: "",
            windows_only: false,
        };
        assert!(!toggle_value(&form, &row));
    }

    /// The desktop-size field's two gates (`:309`), which are not the row's own
    /// `windows_only` flag: it also needs the virtual-desktop switch on.
    #[test]
    fn the_desktop_size_field_needs_both_gates() {
        let mut form = form();
        form.set_toggle(VIRTUAL_DESKTOP_TOGGLE, false);
        assert!(
            !desktop_size_enabled(&form),
            "the switch that owns it is off"
        );

        form.set_toggle(VIRTUAL_DESKTOP_TOGGLE, true);
        assert!(desktop_size_enabled(&form));

        form.is_linux = true;
        assert!(
            !desktop_size_enabled(&form),
            "a Linux game has no Wine desktop to size"
        );

        // And the reference's line for it, so the second gate is not the port's
        // invention.
        assert!(
            qml().contains("enabled: !form.isLinux && desktopSwitch.checked"),
            "the reference no longer gates the size field on the switch"
        );
    }

    /// The two labels that depend on which form this is (`:18`, `:32`).
    #[test]
    fn the_title_and_the_action_follow_is_new() {
        assert_eq!(title(true), TITLE_ADD);
        assert_eq!(title(false), TITLE_EDIT);
        assert_eq!(action_label(true), ACTION_ADD);
        assert_eq!(action_label(false), ACTION_SAVE);
        // The reference's own expressions, so the port is pinned to them rather
        // than to a copy that happens to agree today.
        let source = qml();
        assert!(source.contains("title: isNew ? \"Add Game\" : \"Edit Game\""));
        assert!(source.contains("text: form.isNew ? \"Add\" : \"Save\""));
    }

    /// **UX-11: the cover row draws the reference's preview, and never the path
    /// as text.**
    ///
    /// # What the row found, and what the row's own remedy missed
    ///
    /// The preview stack — [`super::widgets::cover_preview`] and the two helpers
    /// under it — existed, agreed with the reference's numbers, and was
    /// constructed by nothing but its own unit test. The row drew
    /// `text::caption(form.cover_path)` in its place: the path, as a string,
    /// where `GameFormPage.qml:140-150` draws an `Image` and hides the
    /// "No cover yet" label under it.
    ///
    /// # The first version of this test asserted something the reference does
    /// not say, and failed
    ///
    /// It asserted that the row labels the preview with the *path*, and cited
    /// `GameFormPage.qml:147` for it. Reading the file to repair the failure is
    /// what showed the citation was invented: `:134-163` is an `Image`, a
    /// `QQC2.Label` reading "No cover yet", and two buttons, with **no path
    /// string anywhere in it**. The invented premise is recorded rather than
    /// quietly dropped, because "a citation that was never checked" is the exact
    /// defect shape this audit exists to find — here in the audit's own work,
    /// and here in the one test whose job was to catch it.
    ///
    /// So what is asserted is the reference's structure, read from the page:
    ///
    /// * nothing chosen — the picker's words, once, and nothing else;
    /// * a real cover file — the picture and no text at all;
    /// * a path that is not a cover file — the words, once, which is
    ///   [`super::widgets::cover_preview`]'s documented divergence from
    ///   `source !== ""`: the port shows the placeholder where the QML would
    ///   show a broken image.
    ///
    /// and, in every case, that `apply` — the half that ships — carries the same
    /// path into the saved game that the preview was drawn from. The path is
    /// asserted **not** to be drawn in any of the three, which is the assertion
    /// the old caption fails.
    #[test]
    fn the_cover_row_draws_the_reference_preview_and_never_the_path_as_text() {
        use crate::view::widgets::harness::{ids, texts, traversal};
        use crate::view::widgets::{PICTURE_ID, preview_label};
        use cosmic::widget::Id;

        let covers = CoverCache::new();
        let library = library("form-cover-preview", &[]);
        let runners = runners();
        let photo = crate::view::widgets::cover_fixture("photo", &crate::view::widgets::WEBP_COVER);

        // (label, cover path, does the path name a file that classifies as art?)
        let cases: [(&str, String, bool); 3] = [
            ("nothing chosen yet", String::new(), false),
            ("a path the browse button returned", photo, true),
            (
                "a path typed by hand, pointing at no file",
                " /tmp/typed.webp ".into(),
                false,
            ),
        ];

        for (label, cover_path, is_art) in cases {
            let mut form = form();
            form.cover_path = cover_path.clone();

            let mut element = page(&form, &library, &runners, &covers);
            let seen = traversal(&mut element);
            let drew = texts(&seen);

            assert!(
                cover_path.is_empty() || !drew.contains(&cover_path.as_str()),
                "the cover row drew the path itself as text for {label:?} — the \
                 reference draws a picture there, and a path is not a picture; \
                 the page drew {drew:?}"
            );

            if is_art {
                assert!(
                    ids(&seen).contains(&Id::from(PICTURE_ID)),
                    "a path naming a real cover file must be drawn as a picture, \
                     not as the placeholder; the page drew {drew:?} with ids {:?}",
                    ids(&seen)
                );
                assert_eq!(
                    drew.iter().filter(|word| **word == preview_label()).count(),
                    0,
                    "with a picture drawn, the label in its place must be hidden \
                     — `visible: !coverPreview.visible`; the page drew {drew:?}"
                );
            } else {
                assert_eq!(
                    drew.iter().filter(|word| **word == preview_label()).count(),
                    1,
                    "with no picture, the preview's place is taken by its words \
                     — once, not twice and not never; the page drew {drew:?}"
                );
            }

            // The same state, through the half that ships: what the save writes.
            // `apply` keeps this field untrimmed (`bridge.py:429-432`), which is
            // why the typed case's spaces survive — if either half started
            // trimming, this is where the two would part.
            let game = form.apply(None).expect("a named form applies");
            assert_eq!(
                game.cover_path, cover_path,
                "the game saved from the {label:?} form must carry the cover the \
                 preview showed; asserted here so a preview drawn from something \
                 other than the saved value cannot pass"
            );
        }
    }

    /// **The preview consults the cache the page was handed**, so what is drawn
    /// is a function of the file on disk and not of `cover_path` being non-empty.
    ///
    /// [`super::widgets::cover_preview`] documents that it draws the picture
    /// through `CoverCache`, and the page passes the shell's own
    /// ([`crate::State::cover_cache`]) — a *different* cache instance has the
    /// same answer, since the answer is the file's, so what this catches is
    /// narrower and worth naming: a page that drew a picture whenever the field
    /// was non-empty without asking the cache at all. The negative half is the
    /// load-bearing one, and it is the case the QML gets wrong in the other
    /// direction (`source !== ""` shows a broken image for a path with no file
    /// behind it).
    #[test]
    fn the_cover_rows_picture_is_what_the_cache_classified() {
        use crate::view::widgets::harness::{ids, traversal};
        use cosmic::widget::Id;

        let covers = CoverCache::new();
        let library = library("form-cover-cache", &[]);
        let runners = runners();
        let photo = crate::view::widgets::cover_fixture("photo", &crate::view::widgets::WEBP_COVER);

        let mut with_photo = form();
        with_photo.cover_path = photo.clone();
        let mut element = page(&with_photo, &library, &runners, &covers);
        let seen = traversal(&mut element);
        assert!(
            ids(&seen).contains(&Id::from(crate::view::widgets::PICTURE_ID)),
            "a real cover file on disk must be drawn as a picture, not as the \
             placeholder; the row drew {:?}",
            ids(&seen)
        );

        let mut without_photo = form();
        without_photo.cover_path = "/tmp/definitely-not-a-cover-file".to_string();
        let mut element = page(&without_photo, &library, &runners, &covers);
        let seen = traversal(&mut element);
        assert!(
            !ids(&seen).contains(&Id::from(crate::view::widgets::PICTURE_ID)),
            "a path that is not a cover file must fall back to the placeholder; \
             the row drew {:?}",
            ids(&seen)
        );
    }
}
