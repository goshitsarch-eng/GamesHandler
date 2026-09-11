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
use gamehandler_core::models::{Game, Library, SYSTEM_WINE, UNCATEGORIZED};
use gamehandler_core::plugins::{self, PluginEnv, PluginRow};
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
    /// The installer's id, which is what [`crate::Message::CompleteEasyInstall`]
    /// needs to build the library entry.
    ///
    /// **This field was missing, and its absence made the entry unusable.** The
    /// reference's `_pending_installs[token]` holds the `Installer` *object*
    /// (`bridge.py:888`), and `completeEasyInstall` hands it straight back to
    /// `_finish_easy_install` (`:928`), which calls `game_from_install(installer,
    /// …)`. A name and a prefix cannot do that: the port could describe a
    /// pending install and could not complete one, which is the shape #65 names
    /// — the state exists, the thing it exists for does not. The id is stored
    /// rather than the `&'static Installer` because this struct is `Clone`d into
    /// a `BTreeMap` that outlives any borrow, and `installer_by_id` is the
    /// lookup that recovers it.
    pub installer_id: String,
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
    /// Which form this is — the reference's `isNew` property
    /// (`GameFormPage.qml:12`), set by whoever pushed the layer and read only by
    /// the title and the confirming action's label.
    ///
    /// **Not derivable from [`Self::game_id`]**, which is `Some` on both forms:
    /// `newGameTemplate` generates the id when the form opens (`bridge.py:384`),
    /// and that is what keeps a new game's identity stable across a save the name
    /// check rejects. Nor from whether the library holds the id — the reference
    /// decides add-vs-update that way (`bridge.py:406`) and the port does too, but
    /// a game deleted in another window between the form opening and Save would
    /// then flip the title with it.
    pub is_new: bool,
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

    /// The template `newGameTemplate()` builds (`bridge.py:382-399`).
    ///
    /// The id is generated here, when the form opens, rather than at save —
    /// which is what makes "add" and "edit" the same shape, and what keeps a
    /// game's identity stable across a save the name check rejects and the user
    /// has to retry.
    pub fn new_template(settings: &Settings, game_id: GameId) -> Self {
        let mut form = Self {
            game_id: Some(game_id),
            is_new: true,
            runner: settings.default_runner.clone(),
            category: UNCATEGORIZED.to_string(),
            virtual_desktop_size: DEFAULT_DESKTOP_SIZE.to_string(),
            ..Self::default()
        };
        for name in Self::TOGGLE_NAMES {
            form.toggles
                .insert(name.to_string(), default_toggle(settings, name));
        }
        form
    }

    /// The form that edits `game` — the `getGame(id)` map (`bridge.py:356-379`)
    /// as a [`GameForm`].
    ///
    /// The inverse of [`Self::apply`], and deliberately not written as one: the
    /// reference's two directions are `getGame` and `saveGame`, which are not
    /// symmetric — `getGame` hands back `display_category` (blanks already
    /// folded) and an integer `steamAppid`, where `saveGame` takes the raw text
    /// of a text field. An `apply`/`invert` pair would have to pick one
    /// spelling and be wrong about the other.
    pub fn from_game(game: &Game) -> Self {
        let mut form = Self {
            game_id: Some(game.id.clone()),
            is_new: false,
            name: game.name.clone(),
            exe_path: game.exe_path.clone(),
            arguments: game.arguments.clone(),
            working_directory: game.working_directory.clone(),
            is_linux: game.is_linux(),
            runner: game.runner.clone(),
            prefix_path: game.prefix_path.clone(),
            additional_app: game.additional_app.clone(),
            environment: game.environment.clone(),
            category: game.display_category().to_string(),
            virtual_desktop_size: game.virtual_desktop_size.clone(),
            cover_path: game.cover_path.clone(),
            steam_appid: game.steam_appid.to_string(),
            toggles: BTreeMap::new(),
        };
        for (name, value) in toggle_values(game) {
            form.toggles.insert(name.to_string(), value);
        }
        form
    }

    /// The current text of `field`.
    ///
    /// Read and write are a pair addressed by [`FormField`] rather than twelve
    /// pairs of public fields, for the reason [`Self::TOGGLE_NAMES`] is a table:
    /// it is what makes the field set a value in one place rather than a shape
    /// spread across a struct, a widget builder and a message handler.
    pub fn field(&self, field: FormField) -> &str {
        match field {
            FormField::Name => &self.name,
            FormField::ExePath => &self.exe_path,
            FormField::Arguments => &self.arguments,
            FormField::WorkingDirectory => &self.working_directory,
            FormField::Runner => &self.runner,
            FormField::PrefixPath => &self.prefix_path,
            FormField::AdditionalApp => &self.additional_app,
            FormField::Environment => &self.environment,
            FormField::Category => &self.category,
            FormField::VirtualDesktopSize => &self.virtual_desktop_size,
            FormField::CoverPath => &self.cover_path,
            FormField::SteamAppid => &self.steam_appid,
        }
    }

    /// Write `field`, storing the value **as typed**.
    ///
    /// No trimming or folding happens here, and that is deliberate: those are
    /// `saveGame`'s rules and they belong to [`Self::apply`], which is where the
    /// reference applies them. A field that normalised on every keystroke could
    /// not hold a half-typed path.
    pub fn set_field(&mut self, field: FormField, value: String) {
        match field {
            FormField::Name => self.name = value,
            FormField::ExePath => self.exe_path = value,
            FormField::Arguments => self.arguments = value,
            FormField::WorkingDirectory => self.working_directory = value,
            FormField::Runner => self.runner = value,
            FormField::PrefixPath => self.prefix_path = value,
            FormField::AdditionalApp => self.additional_app = value,
            FormField::Environment => self.environment = value,
            FormField::Category => self.category = value,
            FormField::VirtualDesktopSize => self.virtual_desktop_size = value,
            FormField::CoverPath => self.cover_path = value,
            FormField::SteamAppid => self.steam_appid = value,
        }
    }

    /// The stored value of the toggle named `name`, or `None` when `name` is not
    /// one of [`Self::TOGGLE_NAMES`].
    ///
    /// `None` rather than `false` because the two are different answers: a
    /// switch for a name the model does not have is a table disagreeing with the
    /// map, and a caller that reads `false` cannot tell that from a switch that
    /// is off.
    pub fn toggle(&self, name: &str) -> Option<bool> {
        self.toggles.get(name).copied()
    }

    /// Write the toggle named `name`; `false` when `name` is not in the map.
    ///
    /// Refuses to *create* the entry, so an unknown name cannot grow the map
    /// into a set the reference does not have.
    pub fn set_toggle(&mut self, name: &str, value: bool) -> bool {
        match self.toggles.get_mut(name) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// Port of `saveGame`'s body (`bridge.py:404-446`): the name check, the
    /// normalisation, and the field-for-field copy onto a [`Game`].
    ///
    /// `Ok` is the game to store. Which `Library` call stores it is the caller's
    /// decision and is made from whether `existing` was `Some` — the reference's
    /// own `existing is None` test — so the add/update branch is not duplicated
    /// here.
    ///
    /// `Err` is the one thing the reference refuses on: an empty name, whose
    /// message is [`NAME_REQUIRED`] verbatim, because it is user-visible and the
    /// reference's wording is the specification.
    ///
    /// # The one place this is not yet faithful
    ///
    /// `exePath` and `workingDirectory` are trimmed here where the reference
    /// calls `as_local_path` (`netpaths.py:144-160`) on them. That function —
    /// `file://` unwrapping, then mapping a network-share URL onto its GVFS
    /// mount — is not ported, and the two differ only for inputs nothing in this
    /// form can produce today: the Browse buttons behind both fields are
    /// `Message::PickExeFile`'s still-empty arm, so the text is hand-typed. A URL
    /// pasted into either field is therefore stored verbatim instead of being
    /// unwrapped, and losing that is this line, not a design choice.
    pub fn apply(&self, existing: Option<&Game>) -> Result<Game, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(NAME_REQUIRED.to_string());
        }

        let mut game = match existing {
            Some(game) => game.clone(),
            None => Game::new_named(""),
        };
        // The id is the *form's*, which `newGameTemplate` generated when the
        // form opened (`bridge.py:384`) — not the existing game's, and not a
        // fresh one. An edit therefore cannot rename a game's identity, and a
        // new game's id is stable across a retry after a rejected save.
        if let Some(id) = &self.game_id {
            game.id = id.clone();
        }

        game.name = name.to_string();
        game.exe_path = self.exe_path.trim().to_string();
        game.arguments = self.arguments.trim().to_string();
        game.working_directory = self.working_directory.trim().to_string();
        game.kind = if self.is_linux { "linux" } else { "windows" }.to_string();
        // Both branches collapse onto System Wine: a Linux game never uses a
        // runner, and a Windows game with no runner chosen falls back to it.
        // `bridge.py:410-412` states them as two separate expressions.
        game.runner = if self.is_linux {
            SYSTEM_WINE.to_string()
        } else {
            let chosen = self.runner.trim();
            if chosen.is_empty() {
                SYSTEM_WINE.to_string()
            } else {
                chosen.to_string()
            }
        };
        game.prefix_path = self.prefix_path.trim().to_string();
        game.additional_app = self.additional_app.trim().to_string();
        game.environment = self.environment.trim().to_string();
        game.category = {
            let chosen = self.category.trim();
            if chosen.is_empty() {
                UNCATEGORIZED.to_string()
            } else {
                chosen.to_string()
            }
        };
        // `strip() or "1920x1080"`, which is **not**
        // `launch_opts::normalize_desktop_size`: a malformed size is stored as
        // typed and rejected at launch, and folding the two would move that
        // rejection to save time.
        game.virtual_desktop_size = {
            let chosen = self.virtual_desktop_size.trim();
            if chosen.is_empty() {
                DEFAULT_DESKTOP_SIZE.to_string()
            } else {
                chosen.to_string()
            }
        };
        // Not trimmed. `bridge.py:429-432` reads this one with `str(...)` and no
        // `.strip()`, unlike the ten above it.
        game.cover_path = self.cover_path.clone();
        // An unparseable appid is zero, not an error: the reference wraps this in
        // `try`/`except (TypeError, ValueError)` and a text field is what feeds
        // it.
        game.steam_appid = self.steam_appid.trim().parse().unwrap_or(0);

        for toggle in Self::TOGGLE_NAMES {
            if let Some(value) = self.toggle(toggle) {
                write_toggle(&mut game, toggle, value);
            }
        }

        Ok(game)
    }
}

/// The user-visible message `saveGame` refuses an empty name with
/// (`bridge.py:407-409`).
pub const NAME_REQUIRED: &str = "A game needs a name";

/// The desktop-size fallback `saveGame` and `newGameTemplate` both use
/// (`bridge.py:394`, `:426`). See [`GameForm::apply`] for why this is not
/// `launch_opts::normalize_desktop_size`.
pub const DEFAULT_DESKTOP_SIZE: &str = "1920x1080";

/// `Game`'s fifteen toggles, by name, as writable slots.
///
/// `Game` addresses its toggles as fifteen named fields where `GameForm` uses a
/// map — the reference's own split, which is why `setattr(game, name, …)` over
/// `_TOGGLE_FIELDS` (`bridge.py:434-436`) needs no function there and does here.
///
/// These live beside [`GameForm::apply`] rather than on `Game` because the name
/// set is the *form's* ([`GameForm::TOGGLE_NAMES`]), and a table on `Game` would
/// put the form's list in the model. The cost of the split is that a name here
/// that the form does not have is a silent non-write, which is what
/// `the_toggle_tables_are_the_forms_own_names` is for: it compares both tables
/// against `TOGGLE_NAMES` rather than trusting this comment.
fn toggle_fields(game: &mut Game) -> [(&'static str, &mut bool); 15] {
    [
        ("mangohud", &mut game.mangohud),
        ("gamemode", &mut game.gamemode),
        ("prefer_sdl", &mut game.prefer_sdl),
        ("wayland", &mut game.wayland),
        ("hdr", &mut game.hdr),
        ("esync", &mut game.esync),
        ("fsync", &mut game.fsync),
        ("dxvk", &mut game.dxvk),
        ("vkd3d", &mut game.vkd3d),
        ("nvapi", &mut game.nvapi),
        ("fsr", &mut game.fsr),
        ("battleye", &mut game.battleye),
        ("eac", &mut game.eac),
        ("gamescope", &mut game.gamescope),
        ("virtual_desktop", &mut game.virtual_desktop),
    ]
}

/// The same fifteen, read-only, for [`GameForm::from_game`].
fn toggle_values(game: &Game) -> [(&'static str, bool); 15] {
    [
        ("mangohud", game.mangohud),
        ("gamemode", game.gamemode),
        ("prefer_sdl", game.prefer_sdl),
        ("wayland", game.wayland),
        ("hdr", game.hdr),
        ("esync", game.esync),
        ("fsync", game.fsync),
        ("dxvk", game.dxvk),
        ("vkd3d", game.vkd3d),
        ("nvapi", game.nvapi),
        ("fsr", game.fsr),
        ("battleye", game.battleye),
        ("eac", game.eac),
        ("gamescope", game.gamescope),
        ("virtual_desktop", game.virtual_desktop),
    ]
}

/// Write one of `Game`'s toggles by name; `false` for a name that is not one of
/// them.
fn write_toggle(game: &mut Game, name: &str, value: bool) -> bool {
    for (field, slot) in toggle_fields(game) {
        if field == name {
            *slot = value;
            return true;
        }
    }
    false
}

/// `Settings`'s per-toggle defaults, by name.
///
/// Thirteen of the fifteen. `wayland` and `hdr` have no default in the reference
/// either: `_DEFAULTED_TOGGLES` is filtered by `hasattr` (`bridge.py:79-82`), and
/// `newGameTemplate` falls back to the `Game` dataclass default for them, which
/// is `false` (`bridge.py:396-398`, `models.py:36`, `:38`).
fn settings_toggle_defaults(settings: &Settings) -> [(&'static str, bool); 13] {
    [
        ("mangohud", settings.default_mangohud),
        ("gamemode", settings.default_gamemode),
        ("prefer_sdl", settings.default_prefer_sdl),
        ("esync", settings.default_esync),
        ("fsync", settings.default_fsync),
        ("dxvk", settings.default_dxvk),
        ("vkd3d", settings.default_vkd3d),
        ("nvapi", settings.default_nvapi),
        ("fsr", settings.default_fsr),
        ("battleye", settings.default_battleye),
        ("eac", settings.default_eac),
        ("gamescope", settings.default_gamescope),
        ("virtual_desktop", settings.default_virtual_desktop),
    ]
}

/// The value a new form's toggle named `name` starts at:
/// `getattr(self.settings, f"default_{name}", Game.__dataclass_fields__[name].default)`
/// (`bridge.py:396-398`).
fn default_toggle(settings: &Settings, name: &str) -> bool {
    settings_toggle_defaults(settings)
        .into_iter()
        .find(|(field, _)| *field == name)
        .map(|(_, value)| value)
        .unwrap_or(false)
}

/// One field of the add/edit form that a message can write.
///
/// The reference's form is a `QVariantMap` the QML mutates by key and hands to
/// `saveGame` whole (`GameFormPage.qml:40-51`). A key that stops matching what
/// `saveGame` reads is a field that silently stops saving, and nothing on either
/// side is a compile error — so here the key set is an enum, for the reason
/// [`ExeField`]'s doc gives.
///
/// [`Self::form_key`] is the `bridge.py` spelling, and it is pinned against that
/// file rather than trusted, because it is also the wire format `getGame` and
/// `saveGame` use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormField {
    Name,
    ExePath,
    Arguments,
    WorkingDirectory,
    Runner,
    PrefixPath,
    AdditionalApp,
    Environment,
    Category,
    VirtualDesktopSize,
    CoverPath,
    SteamAppid,
}

impl FormField {
    /// Every field, once each.
    pub const ALL: [FormField; 12] = [
        FormField::Name,
        FormField::ExePath,
        FormField::Arguments,
        FormField::WorkingDirectory,
        FormField::Runner,
        FormField::PrefixPath,
        FormField::AdditionalApp,
        FormField::Environment,
        FormField::Category,
        FormField::VirtualDesktopSize,
        FormField::CoverPath,
        FormField::SteamAppid,
    ];

    /// This field's name in the map `saveGame` reads.
    pub fn form_key(self) -> &'static str {
        match self {
            FormField::Name => "name",
            FormField::ExePath => "exePath",
            FormField::Arguments => "arguments",
            FormField::WorkingDirectory => "workingDirectory",
            FormField::Runner => "runner",
            FormField::PrefixPath => "prefixPath",
            FormField::AdditionalApp => "additionalApp",
            FormField::Environment => "environment",
            FormField::Category => "category",
            FormField::VirtualDesktopSize => "virtualDesktopSize",
            FormField::CoverPath => "coverPath",
            FormField::SteamAppid => "steamAppid",
        }
    }
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
    /// The cards the Installers page draws, for the two filters it holds.
    ///
    /// Held rather than computed in `view_body` for a reason that is not
    /// performance: `view_body` returns `Element<'_>` borrowed from `&self`, so
    /// a catalog built inside its `Page::Installers` arm would be a local the
    /// returned element outlives — the E0515 that killed the first T-11/T-12
    /// wiring attempt. The three fields below are the same fact the Runners
    /// page's `installed`/`release_rows` are: everything a page draws has to
    /// live somewhere that outlives the frame.
    ///
    /// It is **not** a cache with a staleness problem of its own: the two
    /// writers are [`Self::refresh_installers`]' only callers, which are the same
    /// function that changes an input.
    pub installer_catalog: Vec<crate::view::installers::InstallerRow>,
    /// The category filter's options: `installer_categories()`' result.
    ///
    /// Cached rather than called at render because it returns an owned
    /// `Vec<String>` and the page takes a slice — the same lifetime reason as
    /// [`Self::installer_catalog`], not a filesystem one.
    pub installer_categories: Vec<String>,
    /// The runner selector's `(id, label)` options — `runner_choices()`.
    ///
    /// Cached for the Runners page's reason rather than the two above:
    /// `RunnerManager::choices` reads the runners directory and scans `PATH`,
    /// and a frame is the wrong rate for either.
    pub installer_runners: Vec<(String, String)>,
    /// The install that is running right now, or `None`.
    ///
    /// The reference keeps this in the worker closure (`installEasy`'s
    /// `work`/`done` close over `installer`, `prefix`, `resolved_runner_id` and
    /// `game_id`, `bridge.py:857-895`). A Rust worker has no closure to hold
    /// them and cannot reach `State`, so one of the two has to carry them across
    /// — and the shell is the side that has to *interpret* the reply, because
    /// `EasyInstallWizardFinished` carries only `found` and `returncode`. The
    /// alternative would have been widening that documented variant's shape to
    /// re-send data the shell already had.
    ///
    /// It is the same record [`Self::easy_pending`] holds, which is the
    /// reference's own shape: `_pending_installs[token]` is written when the
    /// wizard ends without a located executable, and the entry it writes is the
    /// same four fields this holds while the install is still running.
    pub running_install: Option<PendingInstall>,
    /// was `_releases` — the list for [`Self::releases_family`] only.
    pub releases: Vec<ReleaseInfo>,
    /// was `_releases_family` — which family [`Self::releases`] describes.
    pub releases_family: String,
    /// was the `releasesStatus` string protocol.
    pub releases_status: ReleasesStatus,
    /// The Runners page's "Installed" list, recomputed when its inputs change.
    ///
    /// Held rather than computed in `view_body`, which runs once per frame: the
    /// list reaches the filesystem twice — `RunnerManager::system_wine` is a
    /// `PATH` scan and `installed_protons` reads the runners directory — and a
    /// frame is the wrong rate for either. It also spawns a process: the system
    /// row's detail is `wine --version`. [`crate::view::runners::refresh`] is
    /// the only writer, and it writes through
    /// [`crate::Message::RunnersRefreshed`] because of that spawn.
    pub installed: Vec<crate::view::runners::InstalledRow>,
    /// The Runners page's "Available versions" list, same reasoning.
    pub release_rows: Vec<crate::view::runners::ReleaseRow>,
    /// Which [`Self::installed`]/[`Self::release_rows`] reply is current.
    ///
    /// The rows are computed on a worker thread — see
    /// [`crate::view::runners::refresh`] for why the spawn cannot happen where
    /// they are asked for — so two requests can be in flight and complete out
    /// of order. Bumped per request, echoed on
    /// [`crate::Message::RunnersRefreshed`], and a reply that does not match is
    /// dropped. Same shape as [`Self::form_cover_token`], and for the same
    /// reason: a reply describing an older world must not overwrite a newer.
    pub runner_rows_token: u64,
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
    /// was the `plugins` Property (`bridge.py:982-984`) — one row per helper,
    /// in the catalogue's order.
    ///
    /// Cached rather than computed at render, which is what the reference's
    /// `notify=pluginsChanged` does and what keeps five `which` lookups off the
    /// paint path. Starts empty and is filled by [`State::refresh_plugins`],
    /// which the shell calls once at construction — a page that rendered before
    /// that would show an empty list, so the two must not come apart.
    pub plugins: Vec<PluginRow>,
    /// was the `pluginsIntro` Property (`bridge.py:986-996`).
    ///
    /// `constant=True` in Python, so the reference computes it once. It is a
    /// field beside the rows rather than a constant because it depends on
    /// `detect_package_manager`, which reads the host.
    pub plugins_intro: String,
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
            installer_catalog: Vec::new(),
            installer_categories: Vec::new(),
            installer_runners: Vec::new(),
            running_install: None,
            releases: Vec::new(),
            releases_family: String::new(),
            releases_status: ReleasesStatus::Idle,
            installed: Vec::new(),
            release_rows: Vec::new(),
            runner_rows_token: 0,
            runner_busy: false,
            easy_busy: false,
            easy_pending: BTreeMap::new(),
            progress: None,
            toasts: Toasts::new(Message::DismissToast),
            form_cover_token: 0,
            theme_manager: None,
            plugins: Vec::new(),
            plugins_intro: String::new(),
        }
    }

    /// `pluginsChanged` — recompute the Plugins page's rows and its intro.
    ///
    /// `refreshPlugins()` (`bridge.py:1002-1003`) is this method, and the
    /// install's completion handler emits the same signal, so both paths reach
    /// the page through one function. Taking the environment as an argument is
    /// what keeps this testable without a display: a test drives it with a
    /// fabricated host instead of whatever the build machine happens to have
    /// installed, the same reason [`gamehandler_core::plugins::PluginEnv`]
    /// exists at all.
    pub fn refresh_plugins(&mut self, env: &dyn PluginEnv) {
        self.plugins = plugins::plugin_rows(env);
        self.plugins_intro = plugins::plugins_intro(env);
    }

    /// `bridge.py:719-720` — the spinner is on when either long job is.
    pub fn busy(&self) -> bool {
        self.runner_busy || self.easy_busy
    }

    /// Recompute the three things the Installers page draws, from the filters it
    /// holds.
    ///
    /// This is `_get_installers` (`bridge.py:813-828`) pulled off the render
    /// path: the reference recomputes it in a QML `Property` getter on every
    /// read, which a port cannot do because the element borrows what it is
    /// handed (see [`Self::installer_catalog`]). So the two filters call this
    /// when they change and the shell calls it once at construction, which is
    /// `refresh_plugins`' shape and for the same reason.
    ///
    /// The catalog and the categories are both derived here rather than at the
    /// call sites so they cannot be refreshed one without the other: a card
    /// whose category the filter cannot offer is #74, and that defect was
    /// exactly one of these two lists being updated and the other not.
    pub fn refresh_installers(&mut self) {
        self.installer_catalog = crate::view::installers::installer_rows(
            &self.installer_search,
            &self.installer_category,
        );
        self.installer_categories = crate::view::installers::installer_categories();
        self.installer_runners = crate::view::installers::runner_choices(&self.runners);
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

    // ---- The add/edit form -------------------------------------------------

    /// The checkout's copy of a file the reference lives in.
    ///
    /// Read at test time rather than pasted, so the assertion is against the
    /// reference's current text and not against a snapshot of it taken when the
    /// test was written. The three-marker probe is `pending_pages.rs`'s: a
    /// `target/` directory shared between two checkouts hands cargo a binary
    /// compiled in one and run in the other, and a test that reads files would
    /// then assert against the wrong tree — which passes, silently.
    fn read_repo_file(relative: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/app sits two levels below the repository root")
            .to_path_buf();
        for marker in ["Cargo.toml", "build-aux", "data"] {
            assert!(
                root.join(marker).exists(),
                "this test was compiled in {}, which is not the GameHandler checkout: \
                 {marker} is not there. A `target/` directory shared between checkouts hands \
                 cargo a test binary built in the other one.",
                root.display()
            );
        }
        let path = root.join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    /// One method's source text, from `def name(` to the next method.
    ///
    /// Indentation, not brace counting: the next `def` or `@Slot` at the class's
    /// own indent is the end, and Python has no braces to count.
    fn python_method<'a>(source: &'a str, name: &str) -> &'a str {
        let anchor = format!("def {name}(");
        let start = source
            .find(&anchor)
            .unwrap_or_else(|| panic!("bridge.py has no `{anchor}`"));
        let rest = &source[start..];
        let end = rest[1..]
            .find("\n    def ")
            .or_else(|| rest[1..].find("\n    @"))
            .map(|at| at + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// Every `values.get("K")` key in `body`, once each.
    fn python_values_keys(body: &str) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        let mut cursor = body;
        while let Some(at) = cursor.find("values.get(\"") {
            cursor = &cursor[at + "values.get(\"".len()..];
            let key: String = cursor.chars().take_while(|c| *c != '"').collect();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    /// The keys of the `data = { … }` literal in `body`, once each.
    fn python_literal_keys(body: &str) -> Vec<String> {
        let start = body.find("data = {").map_or(0, |at| at + "data = {".len());
        let rest = &body[start..];
        let block = &rest[..rest.find("\n        }").unwrap_or(rest.len())];
        let mut keys: Vec<String> = Vec::new();
        for line in block.lines() {
            let line = line.trim();
            let Some(after) = line.strip_prefix('"') else {
                continue;
            };
            let Some((key, _)) = after.split_once("\": ") else {
                continue;
            };
            if !keys.contains(&key.to_string()) {
                keys.push(key.to_string());
            }
        }
        keys
    }

    /// **`FormField` is exactly the key set `getGame` writes and `saveGame`
    /// reads.**
    ///
    /// `FormField::form_key` is the wire format between this crate and
    /// `bridge.py`'s two map functions. Nothing on either side is a compile
    /// error, so a rename on one side is a field that silently stops saving —
    /// which is the whole reason the enum exists, and it would be worth nothing
    /// if its own strings were unchecked.
    ///
    /// Two keys in both functions are deliberately **not** [`FormField`]s and
    /// are named here rather than filtered away quietly: `gameId`, which is the
    /// form's identity rather than a text field, and `isLinux`, which is the
    /// Type selector and reaches the model as the `bool` field `GameForm::is_linux`.
    /// Listing them means a third exception has to be added on purpose.
    #[test]
    fn the_form_field_keys_are_the_ones_get_game_and_save_game_use() {
        let bridge = read_repo_file("gamehandler/bridge.py");

        let mut ours: Vec<String> = FormField::ALL
            .iter()
            .map(|field| field.form_key().to_string())
            .collect();
        assert_eq!(ours.len(), 12, "a field is listed twice in `FormField::ALL`");
        ours.push("gameId".to_string());
        ours.push("isLinux".to_string());
        ours.sort();

        let mut saved = python_values_keys(python_method(&bridge, "saveGame"));
        saved.sort();
        assert_eq!(
            saved, ours,
            "`saveGame`'s `values.get(...)` keys and `FormField` disagree. A field \
             named on one side only is a value the form never collects or a value \
             the model never stores, and neither is a compile error."
        );

        let mut got = python_literal_keys(python_method(&bridge, "getGame"));
        got.sort();
        assert_eq!(
            got, ours,
            "`getGame`'s map keys and `FormField` disagree, so the edit form and \
             the save path do not describe the same game."
        );
    }

    /// The read and write pair addresses one slot each, and every key is its own.
    #[test]
    fn every_form_field_reads_back_what_was_written() {
        let mut seen: Vec<&str> = Vec::new();
        for field in FormField::ALL {
            assert!(
                !seen.contains(&field.form_key()),
                "two fields share the key {}",
                field.form_key()
            );
            seen.push(field.form_key());

            let mut form = GameForm::default();
            assert_eq!(form.field(field), "", "a default form is empty");
            let written = format!("value for {}", field.form_key());
            form.set_field(field, written.clone());
            assert_eq!(
                form.field(field),
                written,
                "`{}` did not read back what `set_field` wrote",
                field.form_key()
            );
        }
    }

    /// **Every name in the two projection tables is the form's name, in the
    /// form's order.**
    ///
    /// `toggle_fields` and `toggle_values` restate `TOGGLE_NAMES` as Rust field
    /// pairs, because `Game` has fifteen fields where `GameForm` has a map. A
    /// name that drifts there is a toggle that silently stops being applied — the
    /// silent non-write a `match` with a `_ => {}` arm would also have — and no
    /// value-based test can see a *missing* entry for one of the nine toggles
    /// whose `Game` default is `false`.
    #[test]
    fn the_toggle_tables_are_the_forms_own_names() {
        let mut game = Game::new_named("x");
        let written: Vec<&str> = toggle_fields(&mut game)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let read: Vec<&str> = toggle_values(&Game::new_named("x"))
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(written, GameForm::TOGGLE_NAMES, "the write table");
        assert_eq!(read, GameForm::TOGGLE_NAMES, "the read table");

        // The settings table is thirteen of the fifteen, and which two are
        // missing is the reference's own `hasattr` filter, not an oversight.
        let mut defaulted: Vec<&str> = settings_toggle_defaults(&Settings::default())
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        defaulted.push("wayland");
        defaulted.push("hdr");
        defaulted.sort_unstable();
        let mut all = GameForm::TOGGLE_NAMES.to_vec();
        all.sort_unstable();
        assert_eq!(defaulted, all, "the settings table");
    }

    /// **Every toggle reaches the `Game` field with its own name.**
    ///
    /// The read-back is written out here field by field rather than taken from
    /// `toggle_values`, and that is the point: a table shared with the code
    /// under test cannot disagree with it, so it could not fail. Each name is
    /// asserted with **both** values, because the fifteen `Game` defaults are
    /// not all the same — six of them are `true` — so one direction alone would
    /// leave those six unchecked.
    #[test]
    fn every_toggle_the_form_holds_reaches_its_own_game_field() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        form.set_field(FormField::Name, "Half-Life 2".to_string());

        for wanted in [true, false] {
            for name in GameForm::TOGGLE_NAMES {
                assert!(
                    form.set_toggle(name, wanted),
                    "`{name}` is a toggle of the form but not of its map"
                );
            }
            let game = form.apply(None).expect("the name is set above");
            for (name, value) in [
                ("mangohud", game.mangohud),
                ("gamemode", game.gamemode),
                ("prefer_sdl", game.prefer_sdl),
                ("wayland", game.wayland),
                ("hdr", game.hdr),
                ("esync", game.esync),
                ("fsync", game.fsync),
                ("dxvk", game.dxvk),
                ("vkd3d", game.vkd3d),
                ("nvapi", game.nvapi),
                ("fsr", game.fsr),
                ("battleye", game.battleye),
                ("eac", game.eac),
                ("gamescope", game.gamescope),
                ("virtual_desktop", game.virtual_desktop),
            ] {
                assert_eq!(
                    value, wanted,
                    "the form set `{name}` to {wanted} and the game's `{name}` is \
                     {value} — the form's name for this toggle is not the game's"
                );
            }
        }
    }

    /// **Every toggle the game holds reaches the form field with its own name.**
    ///
    /// The other direction, against a `Game` whose toggles alternate, so that no
    /// two *adjacent* entries share a value and a swapped pair in
    /// `toggle_values` fails. What this cannot see is a swap between two entries
    /// that happen to hold the same value — with fifteen booleans at least seven
    /// share a value — which `the_toggle_tables_are_the_forms_own_names` bounds
    /// from the other side by pinning the list's order.
    #[test]
    fn every_toggle_the_game_holds_reaches_its_own_form_field() {
        let mut game = Game::new_named("x");
        for (index, (_, slot)) in toggle_fields(&mut game).into_iter().enumerate() {
            *slot = index % 2 == 0;
        }

        let form = GameForm::from_game(&game);
        for (index, (name, value)) in toggle_values(&game).into_iter().enumerate() {
            assert_eq!(
                form.toggle(name),
                Some(value),
                "the game's `{name}` is {value} and the form's is {:?}",
                form.toggle(name)
            );
            assert_eq!(
                value,
                index % 2 == 0,
                "`{name}` is not the field this test wrote for it, so the \
                 assertion above is comparing two wrong readings"
            );
        }
    }

    /// An empty name is the one refusal, with the reference's own sentence.
    #[test]
    fn a_game_needs_a_name_and_the_sentence_is_the_references() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        assert_eq!(form.apply(None), Err(NAME_REQUIRED.to_string()));
        form.set_field(FormField::Name, "   ".to_string());
        assert_eq!(
            form.apply(None),
            Err(NAME_REQUIRED.to_string()),
            "the reference strips before testing, so spaces are empty too \
             (`bridge.py:406-408`)"
        );
        form.set_field(FormField::Name, "  Half-Life 2  ".to_string());
        assert_eq!(
            form.apply(None).expect("a name with text in it").name,
            "Half-Life 2",
            "and the stored name is the stripped one"
        );

        let bridge = read_repo_file("gamehandler/bridge.py");
        let save = python_method(&bridge, "saveGame");
        assert!(
            save.contains(&format!("\"{NAME_REQUIRED}\"")),
            "`{NAME_REQUIRED}` is no longer the wording `saveGame` refuses with"
        );
    }

    /// Both `kind` branches and the runner collapse behind them.
    #[test]
    fn a_linux_game_is_system_wine_and_a_windows_game_keeps_its_runner() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        form.set_field(FormField::Name, "Hades".to_string());
        form.set_field(FormField::Runner, "GE-Proton9-1".to_string());

        form.is_linux = false;
        let windows = form.apply(None).expect("named");
        assert_eq!(windows.kind, "windows");
        assert_eq!(windows.runner, "GE-Proton9-1", "a chosen runner is kept");

        form.is_linux = true;
        let linux = form.apply(None).expect("named");
        assert_eq!(linux.kind, "linux");
        assert_eq!(
            linux.runner, SYSTEM_WINE,
            "a Linux game runs natively, so the runner is forced back \
             (`bridge.py:410-412`)"
        );

        form.is_linux = false;
        form.set_field(FormField::Runner, "   ".to_string());
        assert_eq!(
            form.apply(None).expect("named").runner,
            SYSTEM_WINE,
            "and a Windows game with no runner falls back to System Wine"
        );
    }

    /// The three blanks the reference fills in, and the one it does not.
    ///
    /// The expected values here are **literals, not the constants the code
    /// uses**, and that is not a style choice. The first version of this test
    /// asserted `game.virtual_desktop_size == DEFAULT_DESKTOP_SIZE`, which is
    /// the constant compared against itself: changing that constant to
    /// `"1280x720"` left the suite green. It was caught by mutation rather than
    /// by reading, and fixed by writing the value out and tying it to the
    /// reference's own text below — so the literal cannot drift either.
    #[test]
    fn the_blank_fields_fold_to_the_references_fallbacks() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        form.set_field(FormField::Name, "Hades".to_string());

        // `newGameTemplate` already seeds the category and the desktop size, so
        // this is the *edited* form being blanked, which is the reachable case:
        // the category field is editable and the size field is a text field.
        form.set_field(FormField::Category, "   ".to_string());
        form.set_field(FormField::VirtualDesktopSize, "  ".to_string());
        let game = form.apply(None).expect("named");
        assert_eq!(game.category, "Uncategorized");
        assert_eq!(game.virtual_desktop_size, "1920x1080");

        let bridge = read_repo_file("gamehandler/bridge.py");
        let save = python_method(&bridge, "saveGame");
        assert!(
            save.contains("or \"1920x1080\""),
            "`saveGame` no longer falls back to 1920x1080 for a blank desktop size"
        );
        assert!(
            save.contains("or \"Uncategorized\""),
            "`saveGame` no longer folds a blank category to Uncategorized"
        );

        // A size the reference stores as typed and rejects at launch. This is
        // the difference from `launch_opts::normalize_desktop_size`, which
        // would have rewritten it here.
        form.set_field(FormField::VirtualDesktopSize, "wide".to_string());
        assert_eq!(
            form.apply(None).expect("named").virtual_desktop_size,
            "wide"
        );

        // The cover path is the one field `saveGame` does *not* strip
        // (`bridge.py:429-432`), so spaces in it survive to the model and the
        // file it names does not exist.
        form.set_field(FormField::CoverPath, " /tmp/a.png ".to_string());
        assert_eq!(
            form.apply(None).expect("named").cover_path,
            " /tmp/a.png "
        );
    }

    /// An appid that is not a number is zero, not a refusal.
    #[test]
    fn an_unparseable_appid_is_zero() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        form.set_field(FormField::Name, "Portal 2".to_string());
        for (typed, expected) in [("", 0), ("  620  ", 620), ("half", 0), ("-1", -1)] {
            form.set_field(FormField::SteamAppid, typed.to_string());
            assert_eq!(
                form.apply(None).expect("named").steam_appid,
                expected,
                "typed {typed:?}"
            );
        }
    }

    /// An edit keeps the identity; a new game gets one, and it is the form's.
    #[test]
    fn the_form_supplies_the_id_and_an_edit_keeps_the_rest() {
        let original = Game::new_named("Doom");
        let form = GameForm::from_game(&original);
        let saved = form.apply(Some(&original)).expect("the name is carried over");
        assert_eq!(saved.id, original.id, "an edit cannot rename the identity");
        assert_eq!(saved.added, original.added, "nor restamp it");
        assert_eq!(saved, original, "and nothing else moved either");

        // A new game takes the form's id — generated when the form opened, not
        // now — so a save the user has to retry keeps the same identity.
        let mut fresh = GameForm::new_template(&Settings::default(), "form-id".to_string());
        fresh.set_field(FormField::Name, "Portal 2".to_string());
        assert_eq!(fresh.apply(None).expect("named").id, "form-id");
        assert_eq!(
            fresh.game_id.as_deref(),
            Some("form-id"),
            "and the form still holds it, so a second save is the same game"
        );
    }

    /// An unknown toggle name is refused rather than added.
    #[test]
    fn an_unknown_toggle_name_writes_nothing() {
        let mut form = GameForm::new_template(&Settings::default(), "id".to_string());
        assert_eq!(form.toggle("not-a-toggle"), None);
        assert!(!form.set_toggle("not-a-toggle", true));
        assert_eq!(form.toggle("not-a-toggle"), None, "and does not create it");
        assert_eq!(
            form.toggles.len(),
            GameForm::TOGGLE_NAMES.len(),
            "the map is still exactly the fifteen"
        );
    }

    /// The template is `newGameTemplate`'s, including where its values come
    /// from.
    #[test]
    fn a_new_form_is_the_reference_template() {
        let settings = Settings {
            default_runner: "proton-ge".to_string(),
            default_mangohud: true,
            default_fsync: false,
            close_on_launch: true,
            ..Settings::default()
        };
        let form = GameForm::new_template(&settings, "id".to_string());

        assert_eq!(form.game_id.as_deref(), Some("id"));
        assert_eq!(form.runner, "proton-ge", "the runner is the stored default");
        // Literals, not the constants — see
        // `the_blank_fields_fold_to_the_references_fallbacks` for why.
        assert_eq!(form.category, "Uncategorized");
        assert_eq!(form.virtual_desktop_size, "1920x1080");
        assert!(!form.is_linux);
        assert_eq!(form.name, "");
        assert_eq!(form.field(FormField::Runner), form.runner);

        assert_eq!(form.toggle("mangohud"), Some(true), "from the setting");
        assert_eq!(form.toggle("fsync"), Some(false), "also from the setting");
        // `wayland` and `hdr` have no setting, so the `Game` dataclass default
        // decides — `false` for both.
        assert_eq!(form.toggle("wayland"), Some(false));
        assert_eq!(form.toggle("hdr"), Some(false));
        assert_eq!(
            form.toggles.len(),
            GameForm::TOGGLE_NAMES.len(),
            "the template seeds every toggle, not only the defaulted thirteen"
        );
    }
}
