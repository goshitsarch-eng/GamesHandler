//! The application state, and the small types it is made of.
//!
//! Port of `Backend.__init__`'s fields (`bridge.py:123-142`) plus the pure
//! derived values that QML expressed as properties. This is
//! `docs/migration/architecture.md` §2.3, and it is the contract the interface
//! is written against: `view()` reads nothing that is not here, and `update()`
//! writes nothing that is not here.
//!
//! # Two sentinels become types
//!
//! `bridge.py` uses `-1.0` for "no progress" and the strings
//! `"idle"`/`"loading"`/`"ready"`/`"error: …"` for the release-list status.
//! Both are replaced here by [`Option<f32>`] and [`ReleasesStatus`], which
//! §2.3 records as a called-out cleanup with identical UI behaviour. The point
//! is not tidiness: with `-1.0` a progress bar that was never started and one
//! that is 0% done are the same value, and with the string protocol a typo in
//! `"error: …"` silently becomes a new state nobody handles.
//!
//! # Ownership
//!
//! This module and `main.rs` are the contract half of the interface: the
//! `Message` variants and these field names are what the widgets in
//! [`crate::view`] bind to. The variant set is fixed by
//! `architecture.md` §2.2; the field set by §2.3. Changing either is a rename
//! that the interface follows, not a redesign.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cosmic::widget::toaster::Toasts;
use gamehandler_core::models::Library;
use gamehandler_core::runners::families::ReleaseInfo;
use gamehandler_core::runners::RunnerManager;
use gamehandler_core::settings::Settings;

use crate::Message;

/// A game's identifier — a 32-character hex string (`uuid4().hex`).
///
/// An alias rather than a newtype: `Game.id` is a `String` in `core`, the
/// identifiers are opaque everywhere they are used, and a wrapper here would
/// mean converting at every boundary between this crate and `core` for no
/// invariant that `core` does not already enforce.
pub type GameId = String;

/// Identifies one cover lookup started from the add/edit form.
///
/// Concurrent lookups are disambiguated by this value, mirroring the game-id
/// re-check in `done()` (`bridge.py:547-549`): a late reply from a superseded
/// lookup is dropped rather than written over a newer one. It is a `u64`
/// counter rather than the game id because the form's game id is stable across
/// edits while the *lookup* is not — two searches typed into the same form must
/// not both be accepted because they share an id.
pub type FormToken = u64;

/// A top-level page. `architecture.md` §2.2, replacing the QML `pageStack`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Library,
    Installers,
    Runners,
    Plugins,
    Credits,
    Settings,
}

impl Page {
    /// Every page, in nav-bar order.
    ///
    /// The order is user-visible — it is the order the sidebar lists them in —
    /// so it is data here rather than a property of how the nav model happens
    /// to be built.
    pub const ALL: [Page; 6] = [
        Page::Library,
        Page::Installers,
        Page::Runners,
        Page::Plugins,
        Page::Credits,
        Page::Settings,
    ];

    /// The page's label in the nav bar.
    pub fn label(self) -> &'static str {
        match self {
            Page::Library => "Library",
            Page::Installers => "Installers",
            Page::Runners => "Runners",
            Page::Plugins => "Plugins",
            // `Main.qml:94` and `CreditsPage.qml:10` both read "About &
            // Credits"; this said "Credits" until T-13 (P-65). The page's own
            // title and the drawer's label are the same string in the reference
            // and are the same string here.
            Page::Credits => "About & Credits",
            Page::Settings => "Settings",
        }
    }
}

/// Which path a file chooser is being opened for.
///
/// QML passed a field name string to one chooser slot; the enum is what makes
/// an unhandled field a compile error rather than a silent no-op. The four
/// values correspond to the four path-like text fields in the game form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExeField {
    /// `exePath` — the executable to run.
    Exe,
    /// `workingDirectory` — the directory to start it in.
    WorkingDir,
    /// `additionalApp` — a launcher to run *around* the game (MangoHud's own
    /// wrapper, a mod loader) as opposed to instead of it.
    AdditionalApp,
    /// `prefixPath` — an existing Wine prefix to use.
    Prefix,
}

impl ExeField {
    /// The form field this chooser fills, as `bridge.py` names it.
    pub fn form_key(self) -> &'static str {
        match self {
            ExeField::Exe => "exePath",
            ExeField::WorkingDir => "workingDirectory",
            ExeField::AdditionalApp => "additionalApp",
            ExeField::Prefix => "prefixPath",
        }
    }
}

/// A Wine prefix tool the user can run against an installed game.
/// `bridge.py:487` (`runPrefixTool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrefixTool {
    /// `winecfg` — drive mappings, Windows version, libraries.
    WineCfg,
    /// `winetricks` — installs redistributables and works around known bugs.
    Winetricks,
}

impl PrefixTool {
    /// The command the tool is reached through.
    pub fn command(self) -> &'static str {
        match self {
            PrefixTool::WineCfg => "winecfg",
            PrefixTool::Winetricks => "winetricks",
        }
    }
}

/// What the release list for a runner family is currently doing.
///
/// Replaces the `"idle"`/`"loading"`/`"ready"`/`"error: …"` string protocol of
/// `bridge.py:692-716`. The error arm carries the message rather than a
/// prefixed string so a family whose fetch failed cannot be mistaken for one
/// whose fetch is still running by anything that forgets to parse the prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleasesStatus {
    /// Nothing has been requested yet.
    Idle,
    /// A fetch is in flight.
    Loading,
    /// Releases are in [`State::releases`].
    Ready,
    /// The fetch failed; the string is the message to show.
    Error(String),
}

/// Artwork found for a game.
///
/// A port of `covers.CoverHit` (`covers.py:258-269`) carrying the fields the
/// interface actually reads: the five `coverFetched` puts in a map
/// (`bridge.py:575-581`) reduce to these, and [`Self::origin_label`] is the
/// derived `origin` that the toast names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverHit {
    /// The Steam application id, `0` when the artwork did not come from Steam.
    pub appid: i64,
    /// The name the artwork is for — Steam's name, which may differ from the
    /// user's.
    pub name: String,
    /// The shelf it was found on, for a later re-fetch.
    pub category: String,
    /// Where the image was written.
    pub cover_path: PathBuf,
    /// Where it came from, for attribution.
    pub source_url: String,
    /// `STEAM_SOURCE` or `ICON_SOURCE`; see [`Self::origin_label`].
    pub source: String,
}

/// The `STEAM_SOURCE` marker in `covers.py`.
pub const STEAM_SOURCE: &str = "steam";
/// The `ICON_SOURCE` marker in `covers.py` — art taken from the executable's own
/// icon rather than from Steam.
pub const ICON_SOURCE: &str = "icon";

impl CoverHit {
    /// Artwork found on Steam, which is the default the Python dataclass
    /// carries (`source: str = STEAM_SOURCE`, `covers.py:264`).
    ///
    /// A constructor rather than a struct literal at each call site: the
    /// default is part of the ported shape, and spelling it out here is what
    /// keeps [`STEAM_SOURCE`] from being a constant nothing reads.
    pub fn from_steam(
        appid: i64,
        name: String,
        category: String,
        cover_path: PathBuf,
        source_url: String,
    ) -> Self {
        Self {
            appid,
            name,
            category,
            cover_path,
            source_url,
            source: STEAM_SOURCE.to_string(),
        }
    }

    /// Where the artwork came from, phrased for the toast that announces it.
    ///
    /// `covers.py:266-269`. Only the icon case is special-cased; everything
    /// else is called "Steam", which is what the original does — including for
    /// a source that is neither.
    pub fn origin_label(&self) -> &'static str {
        if self.source == ICON_SOURCE {
            "the app icon"
        } else {
            "Steam"
        }
    }
}

/// An easy-install that finished but could not find its game's executable.
///
/// Held until the user picks one, so the install can be completed rather than
/// restarted. A port of the `_pending_installs` entry (`bridge.py:887-892`);
/// the installer is held by name and the prefix by path, which is everything
/// [`State::easy_pending`]'s consumers need and nothing that would keep a
/// database handle alive across an overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingInstall {
    /// The installer's display name, for the dialog and the kept-prefix toast.
    pub installer_name: String,
    /// The prefix the installer ran in.
    pub prefix: PathBuf,
    /// The runner that was resolved for the install.
    pub runner_id: String,
    /// The game the install belongs to.
    pub game_id: GameId,
}

/// The field values of the add/edit game form.
///
/// Mirrors the `values` map `saveGame` reads (`bridge.py:404-446`) so the
/// validation and normalisation there ports as a method on this type rather
/// than as a map lookup that cannot fail to typecheck. Every field is a
/// string because that is what the text inputs hold — including
/// `steam_appid`, which `saveGame` parses with a `try`/`except` and defaults to
/// zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameForm {
    /// The game being edited. `None` on the add form; the id is generated when
    /// the form is opened, not when it is saved, so an edit and an add are the
    /// same shape.
    pub game_id: Option<GameId>,
    pub name: String,
    pub exe_path: String,
    pub arguments: String,
    pub working_directory: String,
    /// The form's "this is a Linux game" switch — `isLinux` in `bridge.py`.
    pub is_linux: bool,
    pub runner: String,
    pub prefix_path: String,
    pub additional_app: String,
    pub environment: String,
    pub category: String,
    pub virtual_desktop_size: String,
    pub cover_path: String,
    /// Kept as text, because the input is a text field and `saveGame` treats an
    /// unparseable value as zero rather than as an error.
    pub steam_appid: String,
    /// The fifteen toggles of `bridge.py:74-78`, in that order.
    pub toggles: BTreeMap<String, bool>,
}

impl GameForm {
    /// The toggle names, in `_TOGGLE_FIELDS` order.
    pub const TOGGLE_NAMES: [&'static str; 15] = [
        "mangohud",
        "gamemode",
        "prefer_sdl",
        "wayland",
        "hdr",
        "esync",
        "fsync",
        "dxvk",
        "vkd3d",
        "nvapi",
        "fsr",
        "battleye",
        "eac",
        "gamescope",
        "virtual_desktop",
    ];
}

/// Everything the interface reads or writes.
///
/// The field names follow `architecture.md` §2.3; the "was" comments name the
/// `Backend` field each replaces, because the Python names are what
/// `bridge.py`'s comments and tests refer to and a reader checking parity needs
/// the mapping rather than the translation.
#[derive(Debug)]
pub struct State {
    /// was `self.library`.
    pub library: Library,
    /// was `self.settings`.
    pub settings: Settings,
    /// was `self.runner_manager`.
    pub runners: RunnerManager,
    // `runners::proton` has landed, and it has no `ProtonManager` — so there is
    // deliberately no `pub proton` field here, and no such field is planned.
    // This comment used to promise one; it was corrected rather than left,
    // because a reader who follows a stale instruction builds the thing the
    // design rejected.
    //
    // The module is **free functions** — `install`, `uninstall`,
    // `fetch_available`, `is_installed`, `resolve_staged` — taking
    // `runners_directory` as an argument. Python's `ProtonManager` has exactly
    // one field and no invariant, so the struct existed to hold `self` for the
    // callers' convenience rather than to protect anything; the Rust port passes
    // the directory. A `ProtonManager` here would be a struct with no
    // invariant, existing so this field could exist.
    //
    // `FetchReleases`/`InstallRunner` therefore call `runners::proton`'s
    // functions directly, with `runners.runners_directory()`. Nothing in §2.2's
    // variant set needs a field for them.
    /// The visible page.
    pub page: Page,
    /// The open add/edit form, or `None` when no form is shown.
    pub game_form: Option<GameForm>,
    /// The game a delete is being confirmed for, or `None`.
    ///
    /// A behaviour change, recorded in §2.5: QML deleted without confirming.
    pub confirm_delete: Option<GameId>,
    /// was `_search_text`.
    pub search_text: String,
    /// was `_category_filter`, defaulting to "All".
    pub category_filter: String,
    /// was `_installer_search`.
    pub installer_search: String,
    /// was `_installer_category`, defaulting to "All".
    pub installer_category: String,
    /// was `_releases` — the list for [`Self::releases_family`] only.
    pub releases: Vec<ReleaseInfo>,
    /// was `_releases_family` — which family [`Self::releases`] describes.
    pub releases_family: String,
    /// was the `releasesStatus` string protocol.
    pub releases_status: ReleasesStatus,
    /// was `_runner_busy`. A guard, not a cancel handle: the download is not
    /// interrupted, it is merely not started twice.
    pub runner_busy: bool,
    /// was `_easy_busy`. The same guard for the easy-install path.
    pub easy_busy: bool,
    /// was `_pending_installs`, keyed by the token that identifies the
    /// interrupted install.
    pub easy_pending: BTreeMap<String, PendingInstall>,
    /// was the `-1.0` sentinel. `None` is idle; `Some(f)` is `f` in `0.0..=1.0`.
    pub progress: Option<f32>,
    /// The live toasts.
    ///
    /// `Toasts::new` wants a `fn(ToastId) -> Message` rather than a closure, so
    /// the close handler is [`crate::Message::DismissToast`] itself — which is
    /// why that variant exists and why it is a plain enum constructor.
    pub toasts: Toasts<Message>,
    /// was the token passed to `coverFetched`; incremented per form lookup.
    pub form_cover_token: FormToken,
    /// was `self._theme`. `None` means "follow the desktop", which is what the
    /// QML backend did when no theme manager was injected.
    pub theme_manager: Option<()>,
}

impl State {
    /// The state a freshly started application holds.
    ///
    /// Ports `Backend.__init__` (`bridge.py:123-142`): load the settings and
    /// the library from disk, build the runner manager, and start on the
    /// Library page with no dialog open.
    ///
    /// The two loads are the only filesystem work done here, and neither is
    /// fallible — `Settings.load` and `Library.load` both degrade to defaults
    /// rather than raising, which is what D-20 and `models.py` require.
    pub fn new(
        library: Library,
        settings: Settings,
        runners: RunnerManager,
    ) -> Self {
        Self {
            library,
            settings,
            runners,
            page: Page::Library,
            game_form: None,
            confirm_delete: None,
            search_text: String::new(),
            category_filter: "All".to_string(),
            installer_search: String::new(),
            installer_category: "All".to_string(),
            releases: Vec::new(),
            releases_family: String::new(),
            releases_status: ReleasesStatus::Idle,
            runner_busy: false,
            easy_busy: false,
            easy_pending: BTreeMap::new(),
            progress: None,
            toasts: Toasts::new(Message::DismissToast),
            form_cover_token: 0,
            theme_manager: None,
        }
    }

    /// `bridge.py:719-720` — the spinner is on when either long job is.
    pub fn busy(&self) -> bool {
        self.runner_busy || self.easy_busy
    }

    /// The next form-cover token, consuming the current one.
    ///
    /// A method rather than `+= 1` at the call site so the increment cannot be
    /// forgotten, and so no two callers can hand out the same token.
    pub fn next_form_cover_token(&mut self) -> FormToken {
        self.form_cover_token += 1;
        self.form_cover_token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_distinct_label_and_the_order_is_the_nav_order() {
        let mut labels: Vec<&str> = Page::ALL.iter().map(|page| page.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two pages share a label");
        assert_eq!(Page::ALL[0], Page::Library, "the app opens on the Library");
        assert_eq!(*Page::ALL.last().unwrap(), Page::Settings);
    }

    #[test]
    fn the_exe_fields_name_the_form_keys_bridge_py_reads() {
        // These strings are the wire format between the form and `saveGame`:
        // a rename here without a rename there silently stops the field being
        // saved, which is not a compile error on either side.
        assert_eq!(ExeField::Exe.form_key(), "exePath");
        assert_eq!(ExeField::WorkingDir.form_key(), "workingDirectory");
        assert_eq!(ExeField::AdditionalApp.form_key(), "additionalApp");
        assert_eq!(ExeField::Prefix.form_key(), "prefixPath");
    }

    #[test]
    fn the_toggle_list_is_the_fifteen_from_bridge_py_in_order() {
        // The order is load-bearing: `_TOGGLE_FIELDS` is iterated to apply
        // defaults and to build the form, and a field that moved would be
        // applied to the wrong switch if anything positional depended on it.
        assert_eq!(GameForm::TOGGLE_NAMES.len(), 15);
        assert_eq!(GameForm::TOGGLE_NAMES[0], "mangohud");
        assert_eq!(GameForm::TOGGLE_NAMES[5], "esync");
        assert_eq!(GameForm::TOGGLE_NAMES[14], "virtual_desktop");
        let mut sorted: Vec<&str> = GameForm::TOGGLE_NAMES.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "a toggle is listed twice");
    }

    #[test]
    fn only_the_icon_source_is_called_an_app_icon() {
        let hit = |source: &str| CoverHit {
            appid: 0,
            name: "x".to_string(),
            category: String::new(),
            cover_path: PathBuf::new(),
            source_url: String::new(),
            source: source.to_string(),
        };
        assert_eq!(hit(ICON_SOURCE).origin_label(), "the app icon");
        assert_eq!(hit(STEAM_SOURCE).origin_label(), "Steam");
        // The constructor's default is the Steam marker, as in the dataclass.
        let built = CoverHit::from_steam(
            0,
            "x".to_string(),
            String::new(),
            PathBuf::new(),
            String::new(),
        );
        assert_eq!(built.source, STEAM_SOURCE);
        assert_eq!(built.origin_label(), "Steam");
        // The original's `else` covers anything that is neither, which is worth
        // pinning: a third source would otherwise be attributed to Steam.
        assert_eq!(hit("something-else").origin_label(), "Steam");
    }

    #[test]
    fn progress_is_idle_rather_than_minus_one() {
        // The sentinel this type replaces. A `-1.0` progress and a `0.0`
        // progress are different states, and only one of them is a bar that has
        // not moved yet.
        let idle: Option<f32> = None;
        let started: Option<f32> = Some(0.0);
        assert_ne!(idle, started);
    }
}
