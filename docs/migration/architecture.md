# GameHandler → libcosmic: Architecture

Status: design doc (Phase 1). No code changes. All `file:line` citations refer to
the Python tree at HEAD (`cosmic-migration` branch, app version 0.7.2); all
libcosmic API names were verified against the reference clone at
`/tmp/cosmos-spike/libcosmic` (crate `libcosmic` 1.0.0, `rust-version = "1.93"`).
Anything not verified there is marked **[UNVERIFIED]**.

Premise correction (do not regress): the pre-Qt history is irrelevant. At HEAD the
app is **Python 3 + PySide6 + QML (Qt 6/Kirigami)**. There are no GObject
subclasses, no GTK anything. The entire UI contract is one `QObject` subclass,
`Backend` in `gamehandler/bridge.py` (1,072 lines), exposing `@Property`,
`@Slot`, and `Signal` to nine QML pages in `gamehandler/qml/`
(`Main`, `LibraryPage`, `GameFormPage`, `InstallersPage`, `RunnersPage`,
`PluginsPage`, `CreditsPage`, `SettingsPage`, `CoverArt`). Long work runs on
Python `threading.Thread`s; results return to the UI thread via the `_dispatch`
queued signal (`bridge.py:149-166`).

---

## 1. Module-by-module mapping and workspace layout

### 1.1 Proposed Cargo workspace

```text
gamehandler/                      # workspace root
├── Cargo.toml                    # [workspace], shared lints
├── crates/
│   ├── core/                     # gamehandler-core — pure logic, NO libcosmic/iced dep
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models.rs         # Game, Library, format_last_played, sort/search
│   │       ├── settings.rs       # Settings load/save/validate
│   │       ├── paths.rs          # XDG locations (config.py port)
│   │       ├── runners/          # runners.py port, split for size
│   │       │   ├── mod.rs        # Runner trait, WineRunner, ProtonRunner, RunnerManager
│   │       │   ├── families.rs   # RUNNER_FAMILIES, asset matching, ReleaseInfo
│   │       │   ├── proton.rs     # ProtonManager: fetch/install/uninstall
│   │       │   ├── archive.rs    # bounded tar extraction + staging validation
│   │       │   ├── launch_opts.rs# apply_launch_options, env parsing, DXVK, anticheat
│   │       │   ├── launch.rs     # launch(), LaunchedGame, _ErrorTail, tool_command
│   │       │   └── desktop.rs    # create_desktop_shortcut, escaping
│   │       ├── installers.rs     # catalog, download+verify, prefix scan, wait logic
│   │       ├── covers.rs         # Steam lookup, scoring, download, initials/accent
│   │       ├── exe_icons.rs      # PE icon parser
│   │       ├── netpaths.rs       # GVFS mapping, as_local_path
│   │       ├── plugins.rs        # PLUGINS, package-manager detection/commands
│   │       └── credits.rs        # CREDIT_SECTIONS, markdown rendering
│   └── app/                      # gamehandler (binary) — libcosmic UI + CLI
│       └── src/
│           ├── main.rs           # clap CLI (--list/--launch/--version) + cosmic::app::run
│           ├── state.rs          # State, Message, update(), pure view-model fns
│           ├── view.rs           # view() + per-page constructors (or view/*.rs)
│           └── tasks.rs          # Task constructors bridging core blocking calls
```

Justification — testability without a display server is the driver:

- `gamehandler-core` must compile and test with **only** `serde`/`serde_json`,
  an HTTP client, and std. No `libcosmic`, no `iced`, no async runtime in its
  public API (blocking fns + `progress: &dyn Fn(f32)` callbacks, mirroring the
  current `progress_cb` convention). Every one of the 14 existing test modules
  ports to `core` tests that run under plain `cargo test` on a headless CI box.
- The `app` binary crate owns `State`, `Message`, `update()`, `view()`, CLI
  parsing, and the thin `Task` wrappers. Its logic is deliberately dumb:
  `update()` mutates `State` and returns `Task<Message>`; everything
  interesting lives in `core` and is already tested there.
- Alternatives rejected: a single crate would drag `libcosmic` (windowing,
  GPU) into every unit test build; splitting per-domain crates
  (`gamehandler-runners`, …) adds publish/coordination overhead for a codebase
  whose Python modules already import each other freely (`installers.py`
  imports `runners.py`; `bridge.py` imports everything).

### 1.2 Per-module mapping

| Python module | Responsibility | Public surface to preserve | Rust home |
|---|---|---|---|
| `models.py` (212) | `Game` dataclass (38 fields), `Library` JSON persistence, `format_last_played`, `search`/`categories`/`all` sorting | `Game::{from_dict,to_dict,display_category,is_linux}`, `Library::{load,save,all,get,add,remove,update,mark_played,search,categories}`, `format_last_played`, `UNCATEGORIZED`, `SORT_MODES` | `core::models` |
| `settings.py` (77) | `Settings` dataclass (18 defaults), validation + atomic save | `Settings::{load,save,from_dict,to_dict}`, `COLOR_SCHEMES`, `VIEW_MODES` | `core::settings` |
| `config.py` (90) | XDG paths, `GAMEHANDLER_*` overrides, `ensure_dirs` | `data_home, config_home, games_file, settings_file, runners_dir, prefixes_dir, covers_dir, downloads_dir, ensure_dirs` | `core::paths` |
| `runners.py` (1,547) | Families/releases, `Runner` hierarchy, env construction, `launch()`, `tool_command`, shortcuts, DXVK, anticheat runtimes, archive safety | Everything in `__all__` (`runners.py:1503-1547`); esp. `launch`, `apply_launch_options`, `uses_proton_runtime`, `parse_env_block`, `extract_archive`, `ProtonManager::{fetch_available,install,uninstall,is_release_installed}`, `create_desktop_shortcut` | `core::runners::*` (split as above) |
| `installers.py` (702) | 9-entry catalog, `download_installer` + Authenticode verify, `build_installer_command`, `wait_for_installer`, `find_prefix_exe`, `game_from_install` | `__all__` (`installers.py:679-702`); `INSTALLERS`, `search_installers`, `installer_by_id`, `download_installer`, `verify_installer_authenticity`, `wait_for_installer`, `find_prefix_exe`, `resolve_case_insensitive` | `core::installers` |
| `covers.py` (406) | Steam store search/scoring, CDN download, exe-icon fallback, `initials`/`accent_index` | `fetch_cover`, `steam_cover`, `icon_cover`, `pick_best_match`, `score_title`, `normalize_title`, `cover_urls_for_app`, `copy_custom_cover`, `save_exe_icon`, `CoverHit`, `DEFAULT_CATEGORIES`, `GENRE_MAP` | `core::covers` |
| `exe_icons.py` (257) | PE resource parser → `.ico` bytes; bounded against hostile files | `extract_icon`, `icon_bytes`, limits (`MAX_*`) | `core::exe_icons` |
| `netpaths.py` (186) | GVFS FUSE mapping for `smb://` etc., `file://` unwrap | `as_local_path`, `is_remote_url`, `gvfs_root`, `unreachable_share_message` | `core::netpaths` |
| `plugins.py` (188) | 5 helper defs, Flatpak guard, package-manager commands | `PLUGINS`, `plugin_by_id`, `in_flatpak`, `detect_package_manager`, `install_command`, `privileged_command`, `install_plugin` | `core::plugins` |
| `credits.py` (413) | Static credit data + README markdown generator | `sections`, `section_by_id`, `all_credits`, `credit_by_name`, `markdown`, `ACKNOWLEDGEMENT`, `WHY_ALL_IN_ONE` | `core::credits` |
| `theme.py` (92) | Breeze-flavoured light/dark `QPalette`s, `ThemeManager`, `COVER_GRADIENTS` (8 entries — must match `COVER_ACCENTS = 8`) | `build_palette`, `ThemeManager::apply`, `ACCENT`, `COVER_GRADIENTS` | **Eliminated as code.** COSMIC supplies theming (`cosmic::theme`, `system_theme_update` hook); the 8 cover-plate gradients become constants in `app::view` for tile placeholders. The user-facing `color_scheme` setting ("system"/"light"/"dark") is kept and mapped onto the COSMIC theme mode instead of a `QPalette`. |
| `bridge.py` (1,072) | The `Backend(QObject)`: 23 `@Slot`s, 33 `@Property`s, 16 signals (full inventory in §2) | The whole contract — reproduced as `Message` + `State` + pure view-model fns | `app::state` (+ `core` helpers) |
| `main.py` (131) | CLI (`--list`, `--launch`, `--version`), headless-safe import line, `run_gui` | `_launch_from_cli`, `_list_games` behavior; `APP_ID`, `__version__` | `app::main` (§5) |
| `gamehandler/qml/*.qml` (9 files) | All presentation | Page structure: drawer nav (Library/Installers/Runners/Plugins/Credits/Settings), game form, layered dialogs, toasts | `app::view` with `cosmic::widget::nav_bar`, `dialog`, `toaster` (§2.4) |

Notes on faithful ports:

- `installers.py` imports `apply_launch_options`/`uses_proton_runtime`/`wine_prefix_root`
  from `runners.py` (`installers.py:25-33`); keep that one-way dependency
  (`core::installers` → `core::runners`), never the reverse.
- `covers.py` imports `extract_icon` from `exe_icons.py`; same direction in Rust.
- `bridge.py` helpers `_launcher_command` (91-96), `_TOGGLE_FIELDS`/`_DEFAULTED_TOGGLES`
  (74-82), `_GAME_TEXT_FIELDS` (84-88) encode form-mapping rules (e.g. Linux games
  force `runner = SYSTEM_WINE`, `saveGame` 419-420; empty name rejected, 411-413;
  `steamAppid` parse fallback to 0, 429-432). These move into a `GameForm`
  struct + `apply_form()` in `core::models` (or `app::state`), unit-tested.

---

## 2. The state model

Verified libcosmic shape (do not invent): implement `cosmic::Application`
(`src/app/mod.rs:323`) with `type Executor`, `type Flags`, `type Message:
Clone + Debug + Send + 'static`, `const APP_ID: &'static str`,
`init(core: Core, flags) -> (Self, Task<Message>)`,
`update(&mut self, message) -> Task<Message>`,
`view(&self) -> Element<Message>`, plus optional `subscription`,
`dialog`, `nav_model`/`on_nav_select`, `header_*`, `system_theme_update`.
`cosmic::app::Task<M>` is `iced::Task<cosmic::Action<M>>` (`src/app/mod.rs:18`);
construct tasks only via `cosmic::task::{batch, future, message, stream, none}`
(`src/task.rs:10-37`). There is **no `Command`** — iced removed it; the brief's
"`Task`/`Command`" maps to `Task` alone.

```rust
// app::state — signatures
pub struct State { /* see §2.2 */ }

pub fn init(core: cosmic::Core, flags: Flags) -> (App, cosmic::app::Task<Message>);
impl cosmic::Application for App {
    type Executor = cosmic::executor::Default; // as in examples/application/src/main.rs:125
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = "com.goshapps.GameHandler";
    fn update(&mut self, message: Message) -> cosmic::app::Task<Message>;
    fn view(&self) -> cosmic::Element<'_, Message>;
}
```

### 2.1 Complete `Backend` inventory (the contract source)

`bridge.py` exposes **23 slots**: `quit` (182), `setDefaultToggle` (264),
`getGame` (356), `newGameTemplate` (404→381), `saveGame` (404),
`removeGame` (447), `playGame` (456), `runPrefixTool` (487), `openPrefix` (503),
`createShortcut` (521), `fetchCover` (536), `fetchCoverForForm` (561),
`importCustomCover` (589), `urlToLocalFile` (600), `coverUrlFor` (605),
`fetchReleases` (694), `installRelease` (737), `uninstallRunner` (771),
`installEasy` (831), `completeEasyInstall` (920), `cancelEasyInstall` (937),
`refreshPlugins` (1002), `installPlugin` (1006).

**33 properties**: constants `appVersion` (170), `appId` (174),
`acknowledgement` (178), `sortOptions` (271), `formCategories` (344),
`runnerFamilies` (648), `runnerGuide` (662), `installerCategories` (785),
`pluginsIntro` (986), `creditSections` (1035), `whyAllInOne` (1056),
`aboutText` (1062); settable `colorScheme` (205), `viewMode` (216),
`sortMode` (230), `defaultRunner` (240), `closeOnLaunch` (252),
`searchText` (285), `categoryFilter` (295), `installerSearch` (797),
`installerCategory` (809); derived `defaultToggles` (262), `games` (332),
`librarySize` (337), `categories` (342), `runnerChoices` (619),
`installedRunners` (644), `releases` (687), `releasesStatus` (692),
`busy` (719-727), `progress` (727), `installers` (829), `plugins` (984).

**16 signals**: `notify`, `gameInstalled`, `requestHide`, `requestShow`,
`gamesChanged`, `categoriesChanged`, `settingsChanged`, `runnersChanged`,
`releasesChanged`, `installersChanged`, `pluginsChanged`, `busyChanged`,
`progressChanged`, `coverFetched`, `easyInstallNeedsExe`, `_dispatch` (99-121).

### 2.2 `Message` enum (≈55 variants)

Every slot and every property setter appears exactly once. Pure synchronous
reads (`getGame`, `urlToLocalFile`, `coverUrlFor`, `importCustomCover`'s copy)
need no message — `view()`/`update()` call the `core` fn directly.

```rust
#[derive(Clone, Debug)]
pub enum Message {
    // — Navigation / dialogs (replaces QML pageStack + Main.qml helpers) —
    NavigateTo(Page),                    // Page::{Library,Installers,Runners,Plugins,Credits,Settings}
    OpenNewGameForm,                     // was: newGameTemplate() + push GameFormPage
    OpenEditGameForm(GameId),            // was: getGame(id) + push GameFormPage
    CloseDialog,                         // was: layers.pop()
    ConfirmDeleteGame(GameId),           // was: removeDialog + removeGame() (the QML asks
                                         //   first; §2.5's item 1 was struck when that surfaced)
    DeleteGameConfirmed(GameId),
    PickExeFile { field: ExeField },     // file-chooser open; ExeField::{Exe,WorkingDir,AdditionalApp,Prefix}
    ExeFileChosen { field: ExeField, path: Option<String> }, // None = cancelled
    PickCoverFile,
    CoverFileChosen(Option<String>),     // None = cancelled
    DismissToast(cosmic::widget::toaster::ToastId), // [UNVERIFIED: exact ToastId name/API]
    Quit,                                // was: quit() — closes main window

    // — Settings (settable properties; each setter validates like bridge.py) —
    SetColorScheme(String),              // validated vs COLOR_SCHEMES, else ignore (197-199)
    SetViewMode(String),                 // validated vs VIEW_MODES (210-212)
    SetSortMode(String),                 // validated vs SORT_OPTIONS keys (222-226)
    SetDefaultRunner(String),            // ignored when empty (236)
    SetCloseOnLaunch(bool),
    SetDefaultToggle { name: String, value: bool }, // was: setDefaultToggle()

    // — Library view state —
    SetSearchText(String),               // was: searchText setter
    SetCategoryFilter(String),           // empty resets to "All" (292)

    // — Library CRUD / game actions —
    SaveGameForm(GameForm),              // was: saveGame(); validates name, normalises paths
    LaunchGame(GameId),                  // was: playGame(); marks played, spawns watch task
    LaunchWatchFinished { game_id: GameId, reason: Option<String> },
        // None = still running past grace period; Some(reason) = fast-exit error → toast + unhide
    RunPrefixTool { game_id: GameId, tool: PrefixTool }, // PrefixTool::{WineCfg,Winetricks}
    OpenPrefixFolder(GameId),            // was: openPrefix(); xdg-open via std::process
    CreateDesktopShortcut(GameId),       // was: createShortcut()

    // — Covers —
    FetchCover(GameId),                  // was: fetchCover()
    CoverFetchFinished { game_id: GameId, result: Result<CoverHit, String> },
    FetchCoverForForm { token: FormToken, game_id: GameId, name: String, exe: String },
    FormCoverFetchFinished { token: FormToken, result: Result<CoverHit, String> },
        // was: coverFetched(token, map) signal; stale tokens dropped by comparing
        // token, mirroring the game_id re-check in done() (bridge.py:547-549)

    // — Runners —
    FetchReleases { family: String },    // was: fetchReleases(); sets status "loading"
    ReleasesFetchFinished { family: String, result: Result<Vec<ReleaseInfo>, String> },
        // stale family results dropped (bridge.py:705-706)
    InstallRunner { tag: String },       // was: installRelease(); guarded by runner_busy
    RunnerProgress(f32),                 // streamed download fraction (was: _progress_cb)
    RunnerInstallFinished(Result<String, String>), // Ok(tag) / Err(msg)
    UninstallRunner(String),             // was: uninstallRunner(); synchronous, no task

    // — Easy installers —
    SetInstallerSearch(String),
    SetInstallerCategory(String),        // empty resets to "All" (806)
    StartEasyInstall { installer_id: String, runner_id: String }, // was: installEasy()
    EasyInstallProgress(f32),
    EasyInstallWizardFinished { found: Option<PathBuf>, returncode: i32 },
        // found=Some → finish; None → store PendingInstall + open exe-picker dialog
        // (was: easyInstallNeedsExe signal, bridge.py:886-895)
    CompleteEasyInstall { token: String, path: Option<String> },
        // None/empty → CancelEasyInstall path (bridge.py:926-928)
    CancelEasyInstall(String),           // was: cancelEasyInstall(token)
    EasyInstallFinished { game_id: GameId, message: String }, // was: gameInstalled signal

    // — Plugins —
    RefreshPlugins,                      // was: refreshPlugins(); recompute rows
    InstallPlugin(String),               // was: installPlugin()
    PluginInstallFinished { plugin_id: String, result: Result<bool, String> },

    // — Internal plumbing —
    Notify(String),                      // was: notify signal → toaster toast
    LaunchWatchTick,                     // [if polling chosen over grace-timeout; see §3.3]
}
```

Count: 12 nav/dialog + 6 settings + 2 filters + 7 library + 4 covers + 6 runners
+ 8 installers + 3 plugins + 2 internal ≈ **50–55 variants** (exact count
settles when `Page`/`ExeField` granularity is fixed). Unit tests must cover
every variant's `update()` transition: state change, persistence side effect,
and returned `Task` (usually `Task::none()` except async spawns).

### 2.3 `State` (replaces all `Backend.__init__` fields, 123-142)

```rust
pub struct State {
    pub library: Library,            // was: self.library
    pub settings: Settings,          // was: self.settings
    pub runners: RunnerManager,      // was: self.runner_manager
    pub proton: ProtonManager,       // was: self.proton_manager
    pub page: Page,
    pub game_form: Option<GameForm>, // open add/edit dialog + its field values
    pub confirm_delete: Option<GameId>,
    pub search_text: String,         // _search_text
    pub category_filter: String,     // _category_filter ("All")
    pub installer_search: String,    // _installer_search
    pub installer_category: String,  // _installer_category ("All")
    pub releases: Vec<ReleaseInfo>,  // _releases
    pub releases_family: String,     // _releases_family
    pub releases_status: ReleasesStatus, // Idle | Loading | Ready | Error(String)
    pub runner_busy: bool,           // _runner_busy — guard, not cancellable
    pub easy_busy: bool,             // _easy_busy — guard
    pub easy_pending: HashMap<String, PendingInstall>, // _pending_installs
    pub progress: Option<f32>,       // None == idle (-1.0 today); Some(f) == active
    pub toasts: cosmic::widget::toaster::Toasts<Message>, // [UNVERIFIED exact holder API]
    pub form_cover_token: u64,       // disambiguates concurrent form lookups
}
```

`progress: Option<f32>` replaces the `-1.0` sentinel (`bridge.py:141,724-731`);
`ReleasesStatus` enum replaces the `"idle"/"loading"/"ready"/"error: …"`
string protocol (`bridge.py:692-716`). Both are called-out cleanups with
identical UI behavior.

### 2.4 Properties → pure functions of `State`

No `gamesChanged`-style signals exist in Elm: `view()` re-runs after every
`update()`, so every derived QML property becomes a free function
(`app::state::view_models` or methods on `State`):

- `games(search, category, sort)` → `Vec<GameRow>` — ports `_game_row`
  (`bridge.py:304-322`): cover-exists check, `QUrl.fromLocalFile` → `file://`
  URI string (keep the exact `coverUrl` string format; COSMIC `image` widget
  takes a path — decide at implementation), `.ico` detection, `initials`,
  `accent_index`, subtitle composition. Pure and unit-testable.
- `categories()` → `["All", …]` (`339-342`); `library_size()`; `runner_choices()`,
  `installed_runners()` (ports the System-Wine-first row assembly, 621-642);
  `release_rows()` with `is_release_installed` flags (675-687);
  `installer_rows()` with notes-appended subtitles (813-829);
  `plugin_rows()` with installed/missing/unavailable states + Flatpak branch
  (951-984); `default_toggles_map()` (256-262).
- Constants become `const`/`fn`: `SORT_OPTIONS`, `RUNNER_FAMILIES`,
  `runner_guide_details`, `INSTALLER_CATEGORIES`, `creditSections`,
  `whyAllInOne`, `aboutText`, `acknowledgement`, `pluginsIntro`
  (Flatpak-dependent → pure fn of `in_flatpak()`).
- **`formCategories` fix**: QML declares it `constant=True` (344) so it goes
  stale after adding a game with a new category. Port as a pure fn of current
  `Library` — deliberate behavior improvement, same data (`DEFAULT_CATEGORIES`
  + library extras).
- `busy` = `runner_busy || easy_busy` (719-720) — trivial fn.
- `close_on_launch` (`requestHide`/`requestShow`, 470-483): COSMIC has no
  hide-to-tray primitive in the verified API — decide at implementation whether
  this minimizes the window (`cosmic::command`, **check exact API —
  [UNVERIFIED]**) or becomes a no-op with the error toast kept. Either way the
  launch-watch error path (`LaunchWatchFinished{Some}` → toast) is preserved.

QML pages map to `view()` sections behind `nav_bar::Model` entries
(verified pattern: `nav_model.insert().text(…).data(…)` in
`examples/application/src/main.rs:145-153`; selection via `on_nav_select`):
Library / Installers / Runners / Plugins / Credits / Settings, game form as
`dialog()` overlay, exe-picker asportal file chooser (see §6), notifications as
`toaster` (`src/widget/mod.rs:313-315` exports `Toast, ToastId, Toasts,
toaster` — exact push/expire API **[UNVERIFIED]**).

### 2.5 Proposed behavior changes (only these; everything else faithful)

1. ~~Confirm-before-delete (`ConfirmDeleteGame`): QML deletes instantly
   (`removeGame`, 447-454). Standard COSMIC UX; destructive and irreversible.~~
   **Struck in U3: the premise was wrong.** It was read off `bridge.removeGame`
   alone, but the confirm lives in the QML — `removeDialog`
   (`LibraryPage.qml:346-360`) opens before the slot ever runs — so asking
   first is parity, not a change. The strike stays visible so the next reader
   who diffs this list against the enum knows where item 1 went.
2. `formCategories` derived, not constant (above).
3. `progress: Option<f32>` / `ReleasesStatus` instead of sentinels/strings.
4. `quit()`: closes the main window via the normal COSMIC path rather than
   `QCoreApplication.quit()`; headless `--launch` never starts the GUI anyway.

---

## 3. Async model

### 3.1 `_dispatch` + `_async` → `Task`

`Backend._async(work, done, fail)` (`bridge.py:152-166`) = run blocking `work`
off-thread, marshal `done`/`fail` back to the UI thread via the `_dispatch`
queued signal (149-151, 145). The libcosmic equivalent is
`cosmic::task::future` (verified, `src/task.rs:17-22`): the future runs on the
`Executor` thread pool and its output becomes a `Message` on the UI thread —
no manual marshalling, no `threading` import in UI code:

```rust
// app::tasks — one constructor per former _async call site
pub fn fetch_releases(proton: ProtonManager, family: String) -> cosmic::app::Task<Message> {
    cosmic::task::future(async move {
        match tokio::task::spawn_blocking(move || proton.fetch_available(12, &family)).await {
            Ok(Ok(list)) => Message::ReleasesFetchFinished { family, result: Ok(list) },
            Ok(Err(e)) | Err(e) => Message::ReleasesFetchFinished { family, result: Err(e.to_string()) },
        }
    })
}
```

Rules: (a) `core` stays synchronous/blocking with `progress: &dyn Fn(f32)`
callbacks — identical to today's `progress_cb` convention (`runners.py:867`,
`installers.py:601`); (b) `app::tasks` wraps each blocking call in
`spawn_blocking` inside `Task::future` so the iced executor is never stalled
(this is a real regression risk — iced executors are small pools; see §8.1);
(c) `done`/`fail` closures become `Finished` message variants carrying
`Result<T, String>`, matched in `update()`; (d) stale-result guards
(`family_id != self._releases_family`, `bridge.py:705`) are kept by matching
the echoed key in `update()` and returning `Task::none()`.

Call-site map (`bridge.py` → Task):

| Today | Rust |
|---|---|
| `fetchCover` (536-559) | `FetchCover` → `future(fetch_cover…)` → `CoverFetchFinished` |
| `fetchCoverForForm` (561-587) | same, token echoed; `FormCoverFetchFinished` |
| `playGame` watch (473-485) | `LaunchGame` spawns `future(LaunchedGame::failure…)` (§3.3) → `LaunchWatchFinished` |
| `fetchReleases` (694-717) | above; `fail` path → `Err` variant, status `Error(msg)` keeps the `"error: {msg}"` text |
| `installRelease` (737-769) | `InstallRunner` → streamed progress (§3.2) → `RunnerInstallFinished` |
| `installEasy` (857-903) | `StartEasyInstall` → long future (§3.4) → `EasyInstallWizardFinished` |
| `installPlugin` (1007-1031) | `InstallPlugin` → `future(install_plugin…)` → `PluginInstallFinished`; returncode+`is_installed` recheck kept (1019) |

Fire-and-forget `subprocess.Popen` in `runPrefixTool` (487-501) and the
`additional_app` spawn in `launch()` (`runners.py:1414-1417`) stay synchronous
in `update()` (they return immediately) with errors → `Notify`.

### 3.2 Downloads with progress → streamed `Task`s

Today: `progress_cb` is invoked on the worker thread; `_progress_cb`
(733-735) re-emits through `_dispatch` to set `_progress` + `progressChanged`.
Rust: `cosmic::task::stream` (verified, `src/task.rs:29-33`) fed by an
`async_channel`/`tokio::sync::mpsc` channel that the blocking downloader's
`progress_cb` sends into:

```rust
pub fn install_runner(proton: ProtonManager, release: ReleaseInfo) -> cosmic::app::Task<Message> {
    let (tx, rx) = async_channel::bounded::<f32>(64); // crate TBD at implementation
    let worker = cosmic::task::future(async move {
        let r = tokio::task::spawn_blocking(move || {
            proton.install(&release, &|f| { let _ = tx.try_send(f); })
        }).await;
        Message::RunnerInstallFinished(r.unwrap_or_else(|e| Err(e.into())).map(|_| release.tag))
    });
    let progress = cosmic::task::stream(
        rx.map(Message::RunnerProgress) // futures::StreamExt::map; stream() maps Into<Message>
    );
    cosmic::task::batch([worker, progress]) // cosmic::task::batch verified, src/task.rs:10-15
}
```

`RunnerProgress(f)` sets `progress = Some(f)`; the terminal message resets to
`None` and clears the busy guard — exactly today's `done`/`fail` symmetry
(752-767). Same pattern for `EasyInstallProgress`. Channel-full drops are fine
(progress is lossy by nature; `try_send` never blocks the download).

### 3.3 Launch watching

`LaunchedGame.failure(timeout=6.0)` (`runners.py:1360-1374`) blocks up to
`LAUNCH_GRACE_SECONDS` then returns `None` (still running = success). That
blocking wait moves verbatim into `spawn_blocking` inside the `LaunchGame`
task; the message `LaunchWatchFinished{reason: Option<String>}` reproduces
today's `report()` (478-483) including the `requestShow` un-hide decision (§2.4).
`_ErrorTail` (`runners.py:1315-1347`) ports to `core::runners::launch` as a
`std::thread` draining stderr into a bounded `VecDeque<u8>` — no tokio
dependency in `core`.

### 3.4 Easy-install wizard flow (the longest future)

`installEasy`'s `work()` (857-872) does three blocking phases: download →
`subprocess.run` vendor wizard → `wait_for_installer` (up to
`INSTALL_SETTLE_TIMEOUT`, `installers.py:299`). It already emits a mid-flight
notification via `_dispatch` (859-865). In Rust this becomes one
`spawn_blocking` future emitting two messages: an immediate
`Notify("Launching the … installer…")` is instead returned as a second task
via `Task::batch([message(Notify…), future(…)])`, and the terminal
`EasyInstallWizardFinished{found, returncode}` drives the found/not-found
branch (874-895) in `update()`. No cancellation mid-wizard (matches today:
`cancelEasyInstall` only clears the *pending-picker* state, 937-948).

### 3.5 Cancellation and busy guards

Today there is **no true cancellation**: `_runner_busy`/`_easy_busy`
(`bridge.py:139-140`) are guards — `installRelease` silently returns when busy
(739), `installEasy` toasts "Another install is already running" (833-835).
Port as booleans on `State` with identical semantics. Dropping a `Task`
does not stop `spawn_blocking` work; document this at each call site rather
than pretending tasks cancel. `cancelEasyInstall(token)` semantics are kept
exactly: pop pending, clear `easy_busy`, reset progress, toast the kept-prefix
note (937-948).

---

## 4. Persistence

### 4.1 On-disk format (must not change)

- Library: `config.games_file()` = `$XDG_CONFIG_HOME/gamehandler/games.json`
  (`config.py:39-40`) — a JSON **array** of game objects.
- Settings: `config.settings_file()` = `…/gamehandler/settings.json`
  (`config.py:42-43`) — a JSON **object**.
- Runners/prefixes/covers/downloads live under `$XDG_DATA_HOME/gamehandler/`
  (`runners.py:47-64`); `GAMEHANDLER_DATA_HOME` / `GAMEHANDLER_CONFIG_HOME`
  overrides and per-test redirection via env vars (`config.py:16-36`) are kept
  — the Rust `paths` module reads the same variables with the same precedence.
- Atomic writes: write `*.tmp` + rename (`models.py:149-154`,
  `settings.py:69-74`, `covers.py:273-280`, `runners.py:899-920`). Port with
  `tempfile` (in dir) + `std::fs::rename`, same ordering (`fsync` before rename
  is an optional hardening — call out if added).

### 4.2 Serde mapping and compatibility rules

`Game` (38 fields, `models.py:19-53`) → struct with `#[derive(Serialize,
Deserialize)]`, `#[serde(default)]` on every field plus
`#[serde(default = "…")]` matching each Python default (`kind: "windows"`,
`runner: "wine-system"`, `category: "Uncategorized"`, esync/fsync/dxvk/vkd3d/
battleye/eac `true`, …). Unknown keys are ignored by default — this preserves
`from_dict`'s known-field filtering (`models.py:64-67`) and gives **forward**
compatibility (a newer Python-written file still loads).

Backward compatibility (Rust must read files the Python app wrote, byte-shape
unchanged):

- Timestamp normalization (`models.py:68-86`): non-numeric/NaN/negative `added`
  → field default (`time.time` — port as "now on load"... faithfully: missing
  `added` defaults to now; invalid `last_played` → `0.0`). Implement as a custom
  `deserialize_with` for the two `f64` fields. **Do not "fix"** with stricter
  types — a strict `f64` would accept NaN/negatives the Python app rejects.
- `Library.load` defensiveness (`models.py:128-147`): missing file → empty;
  bad JSON / non-list / non-dict items / `TypeError` rows → skipped; entries
  with empty `name` dropped. Port each arm; each gets a unit test fed with a
  real Python-written fixture (generate once from the Python app, check in
  under `crates/core/tests/fixtures/`).
- `Settings::from_dict` validation (`settings.py:40-51`): unknown keys
  dropped; bad `color_scheme`/`view_mode`/`sort_mode` reset to
  `"dark"`/`"grid"`/`"name"`. Note the asymmetry: `sort_mode` validates
  against `SORT_MODES` from `models` (`settings.py:10`), keep that import
  direction. `Settings::load` returns defaults on any IO/parse/non-dict input
  (57-67) — same in Rust.
- `Library::all` sort orders (`models.py:156-163`): name (casefold),
  recent (`-last_played`, never-played sinks), added (`-added`). Byte-identical
  ordering matters for `--list` output and tests.
- `search` (`188-199`): trim+lowercase; `"All"`/empty category disables the
  filter; matches name **or** display category. `categories()` (201-203):
  `Uncategorized` sorted last.
- `display_category` blank-folding (`60-62`) and `format_last_played` buckets
  (`93-117`, incl. the `<2 min` / singular-plural boundaries) port verbatim
  with the existing test vectors.

cosmic-config (`Config`/`ConfigGet`/`ConfigSet` + `watch`, as in
`examples/config/src/main.rs`) was considered and **rejected** for
library/settings: it stores per-key RON under its own path and cannot read the
existing `games.json`/`settings.json`. If live cross-instance sync is ever
wanted, add a file watcher later — not in this migration.

---

## 5. CLI parity

`main.py:118-131` parses `--version` / `--launch GAME_ID` / `--list`; all three
run **without importing Qt** (module docstring, 1-7) so desktop shortcuts work
headless. Rust port (`app::main`, e.g. `clap` derive):

- `--version` → prints `GameHandler {VERSION}` (`main.py:121`).
- `--list` → `ensure_dirs`, load library, empty → `GameHandler: the library is
  empty`, else `{id}\t{name}` per line, exit codes 0 (`main.py:49-57`).
  Keep stdout format byte-identical — scripts may parse it.
- `--launch GAME_ID` → `ensure_dirs`, lookup, missing → stderr `GameHandler:
  no game with id …` exit 1; `launch()` exception → stderr
  `GameHandler: could not launch {name}: …` exit 1; `mark_played`, then
  `failure()` grace check → stderr `{name} stopped right away: …` exit 1
  (`main.py:28-46`). All three messages and codes preserved — **desktop
  shortcuts depend on `--launch`** (`create_desktop_shortcut`,
  `runners.py:1449-1473`; command built by `_launcher_command`,
  `bridge.py:91-96`, which prefers a `gamehandler` on `PATH` and falls back to
  `python3 -m gamehandler` — the fallback becomes just `gamehandler` since the
  binary is always the launcher; keep the `PATH` lookup + `%`-escaping via
  `desktop_exec`, `runners.py:1444-1446`).
- GUI startup only when neither flag is given: `cosmic::app::run::<App>(settings,
  flags)` (verified, `src/app/mod.rs:130`). `Flags` carries parsed CLI args +
  `APP_ID = "com.goshapps.GameHandler"` (`__init__.py:4`).
- `--launch` in the GUI path too? Today QML `playGame` and CLI `_launch_from_cli`
  share `launch()` + `mark_played` + `failure()`. Keep one `core` fn used by both.

---

## 6. External process handling

`launch()` pipeline (`runners.py:1377-1426`) ports in full — it is the
most-tested behavior (`tests/test_runners.py`, 765 lines):

1. `resolve_game_paths` (GVFS; `1273-1289`) → remote-unmounted share raises
   `unreachable_share_message` — kept as `Err`, surfaced via toast/CLI stderr.
2. Linux games: `build_linux_command` (1264-1270). Windows: `runner.build_command`
   — `WineRunner` sets `WINEPREFIX=wine_prefix_root` (394-408);
   `ProtonRunner` prefers `umu-run` + `PROTONPATH/GAMEID/STORE` env
   (478-497) and falls back to raw-wine-with-`pfx`-fixup (503-510).
3. `env.update(parse_env_block(...))` early + `apply_launch_options` late
   (1396, 1412) — the double-apply is load-bearing (WINEARCH for DXVK setup vs.
   user-wins-final); keep both, comment why.
4. Hard requirements raise before spawn: NVAPI/FSR/Wayland need
   `uses_proton_runtime` (1402-1407); gamescope-missing raises with the Flatpak
   install hint (1253-1256).
5. Bundled DXVK install for raw Wine when `dxvk_root.is_dir()` (1408-1410);
   `install_bundled_dxvk` (1038-1085) with version-marker file,
   win32 detection, `merge_dll_overrides` — port verbatim incl. the `=n,b`
   override strings.
6. `additional_app` fire-and-forget `Popen` (1414-1417); main spawn with
   `stderr=PIPE`, stdout inherited, `cwd` fallback to exe dir (1419-1426).
7. `LaunchedGame::failure` grace check (§3.3) with `_readable_error` noise
   filtering (`_NOISE_PREFIXES`, `1300-1312`).

Rust mapping: `std::process::Command` + `tokio::process::Command` (async side
only in `app::tasks`; `core` uses std). Env = `HashMap<String,String>` built
from `std::env::vars()`. `shlex.split(game.arguments)` → `shell-words` crate
(or hand-rolled splitter — decide at implementation; behavior: POSIX split).
`tool_command` (1476-1500: `WINE`/`WINESERVER` env pinning, prefix mkdir) and
`openPrefix`'s folder-open (`QDesktopServices.openUrl` → `xdg-open` via
`std::process::Command` or `open` crate) likewise.

**Flatpak sandbox implications** (behavior preserved, mechanism changes):

- Today: `in_flatpak()` (`plugins.py:107-109`) disables plugin installation
  (`install_command` raises, 131-135; UI shows "unavailable", `bridge.py:957-962`,
  `pluginsIntro`, 988-994); gamescope-missing error names the Flathub
  extension (1255); DXVK root defaults to `/app/share/gamehandler/dxvk`
  (`runners.py:48`); Authenticode root falls back to `/app/share/gamehandler/…`
  (`installers.py:540-548`); umu/proton run inside the sandbox with
  `--filesystem` holes granted by the manifest.
- Rust: keep the identical `FLATPAK_ID`/`/.flatpak-info` detection in
  `core::plugins::in_flatpak`, keep all fallback paths, keep the error strings.
  Portal file chooser: libcosmic re-exports `ashpd` (`src/dialog/mod.rs:7`) with
  a `dialog::file_chooser` module — use the portal path so the sandbox can
  open files outside it; exact chooser call signature **[UNVERIFIED — read
  `src/dialog/file_chooser/` at implementation]**. Network downloads
  (`urlopen` in runners/covers/installers) need `--share=network` (already in
  the manifest — verify, don't assume). `subprocess.run([pkexec…])` plugin
  installs never run under Flatpak (guard stays).
- Auth binaries: `osslsigncode` (`installers.py:571`) and `wineserver -w`
  (`installers.py:332`) must be in the Flatpak runtime/SDK extensions as today;
  the migration must not silently drop `verify_installer_authenticity`
  (569-595) — no verifier binary = hard error, as now (572-573).

---

## 7. Testing strategy

### 7.1 What is pure and headless (→ `gamehandler-core` unit + integration tests)

Everything below already has Python tests; each ports ~1:1 and runs with no
display server, no GPU, no network (mock at the seam: injectable fetch fns or a
local `http.server` fixture — the Python suite's approach per
`tests/test_runners.py` / `test_installers.py` should be mirrored):

- `models`: roundtrip, unknown-key tolerance, timestamp normalization vectors,
  corrupt-file arms, sort/search/category semantics (`tests/test_models.py`,
  140 lines).
- `settings`: defaults, validation resets, corrupt-file → defaults
  (`test_settings.py`).
- `runners`: env construction matrices (esync/fsync/dxvk/vkd3d/nvapi/fsr/
  battleye/eac/gamescope/wayland/hdr/sdl × proton/non-proton), `parse_env_block`
  edge cases, `virtual_desktop_argv`, `normalize_desktop_size`,
  `merge_dll_overrides`, archive safety (tar bombs, escaping symlinks,
  `RENAME_NOREPLACE` semantics — Linux-only test), `install_id` hashing,
  `is_installed` legacy-metadata fallback, shortcut escaping incl. newline
  injection (`test_runners.py` 765 + `test_security.py` 460 lines).
- `installers`: catalog search, `installer_argv` exe/msi, origin-allowlist
  rejection, magic-byte checks, `resolve_case_insensitive`, `find_prefix_exe`
  layout/profile fallbacks, `wait_for_installer` with fake clock/sleep
  (injectable `sleep`/`clock` params exist precisely for this —
  `installers.py:366-367`), `game_from_install` (`test_installers.py`, 568).
- `covers`: title normalization/scoring incl. the single-word
  `GENERIC_QUERY_MINIMUM` guard, genre map, `pick_best_match` tie-breaks,
  `cover_urls_for_app` ordering (`test_covers.py`).
- `exe_icons`: crafted PE fixtures, truncation/bounds (`test_exe_icons.py`, 245).
- `netpaths`: GVFS name construction, scan fallback, `GAMEHANDLER_GVFS_ROOT`
  redirection (`test_netpaths.py`).
- `plugins`: manager detection, command shapes, Flatpak refusal
  (`test_plugins.py`); `credits`: markdown stability (`test_credits.py`).
- **New**: `GameForm::apply` validation (name-required, Linux runner forcing,
  appid fallback), all `view-model` row builders (`GameRow` subtitle logic,
  plugin states, installer subtitles, runner rows) — these are today's
  untestable QML-adjacent code in `bridge.py:_game_row/_plugin_row/_get_*`;
  in Rust they are pure fns in `core` (taking `&Library`/`&Settings`), tested
  headlessly. `tests/test_library_view.py` (166) is the seed.
- `tests/test_qml_smoke.py` and `kirigami_stub` retire: no QML remains.
  `test_packaging.py`/`test_discovery.py` port to manifest + CLI-shape tests.

### 7.2 Keeping libcosmic out of `core` tests

- Hard rule: `crates/core/Cargo.toml` must not depend on `libcosmic`, `iced`,
  or any GUI crate. Enforce with a CI check (`cargo deny` or a grep test —
  `test_packaging.py`'s successor).
- `app` crate tests cover only `update()` transitions: given `State` +
  `Message`, assert resulting `State` fields + whether the returned `Task` is
  `none` (iced `Task` is inspectable only as unit — assert side effects on
  `State`, and route all *decisions* through pure fns). Async worker bodies
  live in `core`, already tested; `app::tasks` constructors are thin and
  untested by design (same rationale as today's untested `Backend._async`).
- Env-dependent paths (`XDG_*`, `GAMEHANDLER_*`, `FLATPAK_ID`,
  `GAMEHANDLER_GVFS_ROOT`, `GAMEHANDLER_DXVK_ROOT`,
  `GAMEHANDLER_AUTHENTICODE_ROOT`) keep working via env-var redirection so
  tests use temp dirs — the established pattern from `config.py` must not be
  lost in the port.

---

## 8. Top risks (for Phase 2 planning)

1. **Blocking the iced executor.** Every `core` call is blocking by design
   (network, `subprocess.run` wizard waits lasting hours, `wineserver -w`).
   Each must be wrapped in `spawn_blocking`; a single direct call stalls input
   handling. Audit every `app::tasks` constructor.
2. **Progress-channel lifetime.** The `Task::stream` progress subscription must
   terminate when the worker finishes or the download stalls forever on a hung
   `resp.read` — today the thread just lingers; in iced a leaked stream keeps
   `update()` traffic alive. Add timeouts mirroring today's (`timeout=60`
   runners, `timeout=30` covers) and close the channel on worker exit.
3. **File-chooser portal API.** `cosmic::dialog::file_chooser` + re-exported
   `ashpd` are verified to exist; exact open-file/save semantics, filter setup,
   and how the result returns as a `Message` are **[UNVERIFIED]**. The exe-picker
   recovery flow (`easyInstallNeedsExe`) and cover picker both depend on it.
4. **Toaster/dialog exact APIs.** `widget::{toaster::{Toast,ToastId,Toasts,
   toaster}, dialog}` exports verified; push/expire/confirm-button wiring
   **[UNVERIFIED]**. The `Notify→toast`, confirm-delete, and pending-install
   dialogs cannot be finalized until read.
5. **Flatpak manifest parity.** umu, wineserver, osslsigncode, DXVK/hydra
   extensions, `--share=network`, portal permissions, and the
   `/app/share/gamehandler` fallbacks must all survive the SDK swap from
   PySide6 to Rust; a missing piece fails silently at runtime (e.g. DXVK
   silently skipped when `dxvk_root.is_dir()` is false — `runners.py:1409`).
   Port `test_packaging.py` first.
