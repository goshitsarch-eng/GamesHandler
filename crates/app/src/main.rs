//! GameHandler — the binary: a libcosmic interface plus a headless CLI.
//!
//! # The CLI never touches the GUI
//!
//! `--list`, `--launch` and `--version` are parsed and dispatched **before**
//! anything that could create a window, a renderer or an event loop
//! (DECISIONS D-12). This is not a nicety: the app writes desktop shortcuts
//! that invoke `gamehandler --launch <GAME_ID>`, and those shortcuts are
//! already on users' disks. If that path needed a display or a GPU, every
//! shortcut would break for anyone on a headless or remote session.
//!
//! # This file decides nothing
//!
//! Per `docs/migration/architecture.md` §1.1 the binary crate stays thin:
//! `update()` mutates state and `view()` renders it, and every non-trivial
//! decision lives in `gamehandler-core`, where it is tested without a display.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gamehandler_core::models::Library;
use gamehandler_core::runners::families::ReleaseInfo;
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};
use gamehandler_core::settings::Settings;
use gamehandler_core::{APP_ID, APP_NAME, VERSION};

mod state;

pub use state::{
    CoverHit, ExeField, FormToken, GameForm, GameId, Page, PendingInstall, PrefixTool,
    ReleasesStatus, State,
};

/// Command-line interface.
///
/// Mirrors `main.py`'s argparse contract: the same flag names, the same
/// `--version` text (`GameHandler {VERSION}`) and the same exit codes, because
/// scripts and shortcuts may parse them.
///
/// One deliberate divergence: `main.py` uses `parse_known_args` and forwards
/// unrecognised flags on to Qt, which no longer exists here, so unknown
/// arguments are now rejected. Nothing the app itself writes passes anything
/// but `--launch <id>`.
#[derive(Debug, Parser)]
#[command(
    name = "gamehandler",
    bin_name = "gamehandler",
    display_name = APP_NAME,
    version = VERSION,
    about = "A game manager for Linux, for running Windows games through Wine and Proton",
    long_about = None,
)]
struct Cli {
    /// Print the library's game ids and names, one game per line, then exit.
    #[arg(long)]
    list: bool,

    /// Launch a game from the library by its id.
    ///
    /// This is the form the app's own desktop shortcuts use.
    #[arg(long, value_name = "GAME_ID")]
    launch: Option<String>,
}

/// Application entry point.
///
/// Returns the exit code rather than calling `std::process::exit`, so
/// destructors run on the way out.
fn main() -> ExitCode {
    // `--version` is handled inside `parse` by clap and exits there; it never
    // reaches any code below, display or not.
    let cli = Cli::parse();

    // DECISIONS D-12: dispatch the headless paths before anything GUI-shaped
    // is even named. Order matches `main.py`: `--list` wins over `--launch`.
    if cli.list {
        return list_games();
    }
    if let Some(game_id) = cli.launch.as_deref() {
        return launch_game(game_id);
    }

    run_gui()
}

/// `--list`: print every game's id and name, tab-separated, one per line.
///
/// TODO(T-07): load the library through `gamehandler_core::models::Library`
/// and port `main.py:_list_games` in full — the empty-library line and the
/// `{id}\t{name}` row format must stay byte-identical, since scripts parse
/// this. The stub below already prints the correct empty-library line.
fn list_games() -> ExitCode {
    println!("{APP_NAME}: the library is empty");
    ExitCode::SUCCESS
}

/// `--launch <GAME_ID>`: start a game and report whether it stayed up.
///
/// TODO(T-07): port `main.py:_launch_from_cli` — look the game up, call
/// `gamehandler_core::runners::launch`, `mark_played`, then run the
/// immediate-failure grace check (P-46) and exit 1 with the reason. The stub
/// reproduces the exit code and stderr text for an unknown id, which is the
/// only outcome reachable before the library lands.
fn launch_game(game_id: &str) -> ExitCode {
    eprintln!("{APP_NAME}: no game with id {game_id}");
    ExitCode::from(1)
}

/// Start the graphical interface. Reached only when no CLI flag was given.
fn run_gui() -> ExitCode {
    // TODO(T-08, N-01/N-02): with neither WAYLAND_DISPLAY nor DISPLAY set,
    // iced's winit backend panics (exit 101, a raw traceback) where the Qt app
    // printed an actionable hint. Catching that and exiting non-zero with a
    // message is the recorded parity fix for this path.
    let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(1200.0, 800.0));
    match cosmic::app::run::<App>(settings, ()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{APP_NAME}: could not start the interface: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Messages handled by [`App::update`].
///
/// This is the interface's contract, and it is fixed before its handlers exist
/// on purpose. Every slot and every settable property in `bridge.py` appears
/// here exactly once (the inventory is `architecture.md` §2.1, the mapping
/// §2.2), so a widget can name the message it emits — and be written, and
/// reviewed, and tested — before the code that acts on it lands. The alternative
/// is that the whole interface waits on the state machine, which is a much
/// longer serial chain than it needs to be.
///
/// # Once, and only once
///
/// A pure synchronous read — `getGame`, `urlToLocalFile`, `coverUrlFor` — has
/// no variant, because `view()` and `update()` can call the `core` function
/// directly. A message exists only where something must *change*.
///
/// # Why there is no `_ => …` arm below
///
/// `update()` matches every variant explicitly, with `Task::none()` where the
/// handler has not landed and a `T-0x` marker naming the task that fills it in.
/// The match is exhaustive, so adding a variant here is a compile error in
/// `update()` rather than a message that is silently dropped at runtime — which
/// is the failure mode a wildcard arm would create, and the reason this enum
/// was left uninhabited until the whole set could land at once.
#[derive(Clone, Debug)]
pub enum Message {
    // ---- Navigation and dialogs -------------------------------------------
    // Replaces the QML `pageStack` and the helpers in `Main.qml`.
    /// Show a top-level page.
    NavigateTo(Page),
    /// Open the add-game form, with a fresh template. `newGameTemplate()`.
    OpenNewGameForm,
    /// Open the edit form for an existing game. `getGame(id)` + push.
    OpenEditGameForm(GameId),
    /// Close whatever overlay is open. `layers.pop()`.
    CloseDialog,
    /// Ask before deleting. **A behaviour change** — QML deleted immediately
    /// (`removeGame`, `bridge.py:447-454`); see `architecture.md` §2.5.
    ConfirmDeleteGame(GameId),
    /// The user confirmed; do it.
    DeleteGameConfirmed(GameId),
    /// Open a file chooser for one of the form's path fields.
    PickExeFile { field: ExeField },
    /// The chooser closed. `None` means it was cancelled.
    ExeFileChosen { field: ExeField, path: Option<String> },
    /// Open a chooser for a cover image, as opposed to fetching one.
    PickCoverFile,
    /// The cover chooser closed. `None` means it was cancelled.
    CoverFileChosen(Option<String>),
    /// A toast expired or was dismissed.
    DismissToast(cosmic::widget::toaster::ToastId),
    /// Close the main window. `quit()`.
    Quit,

    // ---- Settings ---------------------------------------------------------
    // Each setter validates the way `bridge.py` does: an unrecognised value is
    // ignored rather than stored, so a stale UI cannot write a mode the code
    // does not handle.
    /// vs `COLOR_SCHEMES` (`bridge.py:197-199`); unrecognised is ignored.
    SetColorScheme(String),
    /// vs `VIEW_MODES` (`210-212`).
    SetViewMode(String),
    /// vs the `SORT_OPTIONS` keys (`222-226`).
    SetSortMode(String),
    /// Ignored when empty (`236`).
    SetDefaultRunner(String),
    /// Whether launching a title hides the window.
    SetCloseOnLaunch(bool),
    /// `setDefaultToggle()` (`bridge.py:264`) — the per-toggle defaults new
    /// games inherit.
    SetDefaultToggle { name: String, value: bool },

    // ---- Library view state ----------------------------------------------
    /// The search box. `searchText`.
    SetSearchText(String),
    /// The category filter; empty resets to "All" (`bridge.py:292`).
    SetCategoryFilter(String),

    // ---- Library: the games themselves -----------------------------------
    /// Save the open form — add or update, decided by the form's game id.
    /// `saveGame()`; validates the name and normalises the paths.
    SaveGameForm(GameForm),
    /// Start a title. `playGame()`; marks it played and begins the grace watch.
    LaunchGame(GameId),
    /// The grace watch ended.
    ///
    /// `reason` is `None` when the title was still running when the grace
    /// period expired, which is the success case; `Some(_)` is the runner's
    /// failure report, shown as a toast with the window brought back.
    LaunchWatchFinished {
        game_id: GameId,
        reason: Option<String>,
    },
    /// Run `winecfg` or `winetricks` against the game's prefix.
    RunPrefixTool { game_id: GameId, tool: PrefixTool },
    /// Open the prefix folder in the desktop's file manager. `openPrefix()`.
    OpenPrefixFolder(GameId),
    /// Write a `.desktop` shortcut. `createShortcut()`.
    CreateDesktopShortcut(GameId),

    // ---- Covers -----------------------------------------------------------
    /// Fetch artwork for a saved game. `fetchCover()`.
    FetchCover(GameId),
    /// A saved game's artwork lookup finished.
    CoverFetchFinished {
        game_id: GameId,
        result: Result<CoverHit, String>,
    },
    /// Fetch artwork for the game *currently being typed* into the form.
    /// `fetchCoverForForm()`, reported through the `coverFetched` signal.
    FetchCoverForForm {
        token: FormToken,
        game_id: GameId,
        name: String,
        exe: String,
    },
    /// A form lookup finished. A reply whose `token` is not the form's current
    /// one is stale and must be dropped, mirroring the game-id re-check in
    /// `done()` (`bridge.py:547-549`).
    FormCoverFetchFinished {
        token: FormToken,
        result: Result<CoverHit, String>,
    },

    // ---- Runners ----------------------------------------------------------
    /// List the downloadable releases for a family. `fetchReleases()`; sets
    /// the status to loading.
    FetchReleases { family: String },
    /// A release listing finished. A reply for a family the user has navigated
    /// away from is stale (`bridge.py:705-706`).
    ReleasesFetchFinished {
        family: String,
        result: Result<Vec<ReleaseInfo>, String>,
    },
    /// Download and install a release. `installRelease()`; guarded by
    /// `runner_busy`.
    InstallRunner { tag: String },
    /// A download fraction, in `0.0..=1.0`. `_progress_cb`.
    RunnerProgress(f32),
    /// The install finished. `Ok(tag)` on success, `Err(message)` otherwise.
    RunnerInstallFinished(Result<String, String>),
    /// Delete an installed build. `uninstallRunner()` — synchronous, so this
    /// returns no task.
    UninstallRunner(String),

    // ---- Easy installers --------------------------------------------------
    /// The installer search box.
    SetInstallerSearch(String),
    /// The installer category filter; empty resets to "All" (`bridge.py:806`).
    SetInstallerCategory(String),
    /// Start a one-click install. `installEasy()`.
    StartEasyInstall { installer_id: String, runner_id: String },
    /// The install's progress fraction.
    EasyInstallProgress(f32),
    /// The installer process ended.
    ///
    /// `found = Some(path)` means the game's executable was located and the
    /// install can finish. `None` means it was not, so a
    /// [`PendingInstall`] is stored and the file picker opens — the
    /// `easyInstallNeedsExe` signal (`bridge.py:886-895`).
    EasyInstallWizardFinished {
        found: Option<PathBuf>,
        returncode: i32,
    },
    /// The user picked the executable the installer would not reveal.
    ///
    /// `path` of `None` or empty means they cancelled, which takes the
    /// `cancelEasyInstall` path (`bridge.py:926-928`).
    CompleteEasyInstall {
        token: String,
        path: Option<String>,
    },
    /// Abandon an interrupted install, keeping the prefix it made.
    CancelEasyInstall(String),
    /// The install produced a game. The `gameInstalled` signal.
    EasyInstallFinished { game_id: GameId, message: String },

    // ---- Plugins ----------------------------------------------------------
    /// Re-read the plugin list and recompute the rows. `refreshPlugins()`.
    RefreshPlugins,
    /// Install a plugin. `installPlugin()`.
    InstallPlugin(String),
    /// A plugin install finished.
    PluginInstallFinished {
        plugin_id: String,
        result: Result<bool, String>,
    },

    // ---- Internal plumbing ------------------------------------------------
    /// Show a toast. The `notify` signal, which every `bridge.py` failure path
    /// reaches for.
    Notify(String),
    /// One tick of the launch grace watch.
    ///
    /// Used only if the watch polls rather than awaiting a single grace
    /// timeout — `architecture.md` §3.3 leaves that open, and the variant
    /// exists now so the choice does not change the contract later.
    LaunchWatchTick,
}

/// The application state.
///
/// Holds the COSMIC `Core` (window and theme) and the ported [`State`]. The
/// split is the framework's, not ours: `Core` is what `cosmic::Application`
/// requires, and `State` is what `bridge.py`'s `Backend` fields became.
pub struct App {
    core: cosmic::Core,
    state: State,
}

impl cosmic::Application for App {
    /// The tokio-backed executor, as in libcosmic's own application example.
    type Executor = cosmic::executor::Default;

    /// No startup arguments yet; the library and settings are read from disk
    /// rather than passed on the command line.
    type Flags = ();

    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: Self::Flags) -> (Self, cosmic::app::Task<Self::Message>) {
        // The two loads are the only filesystem work done at startup, and
        // neither is fallible: `Settings.load` and `Library.load` degrade to
        // defaults rather than raising (D-20, `models.py`).
        let settings = Settings::load(None);
        let library = Library::new(None);
        let runners = RunnerManager::new(&SystemLaunchEnv);
        (
            App {
                core,
                state: State::new(library, settings, runners),
            },
            // There is no `Command` in this iced generation; the no-op is
            // `cosmic::task::none()`.
            cosmic::task::none(),
        )
    }

    /// Handle one message.
    ///
    /// # Every arm is written out, and none of them is a wildcard
    ///
    /// The match is exhaustive and every arm is spelled individually, even
    /// where the handler has not landed. That is deliberate: a `_ => …` arm
    /// would swallow a variant added later, turning "this message is not
    /// handled" into a silent no-op that looks like working code. Here, adding
    /// a variant to [`Message`] is a compile error until it is listed, and each
    /// placeholder carries a `T-0x` marker naming the task that fills it in.
    ///
    /// # What the placeholders do, and do not, mean
    ///
    /// A `Task::none()` below means *nothing has been implemented yet* — it
    /// does not mean the message is a no-op by design. The three that carry a
    /// real body are [`Message::NavigateTo`], [`Message::DismissToast`] and
    /// [`Message::Quit`], because they are the ones the shell in T-08 needs to
    /// be usable at all.
    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        match message {
            // ---- Navigation and dialogs -----------------------------------
            Message::NavigateTo(page) => {
                self.state.page = page;
            }
            // TODO(T-09): build a `GameForm` from `newGameTemplate` — the
            // settings-derived toggle defaults are `GameForm::TOGGLE_NAMES`
            // crossed with `Settings::default_*`.
            Message::OpenNewGameForm => {}
            // TODO(T-09): load the game into a `GameForm`.
            Message::OpenEditGameForm(_game_id) => {}
            Message::CloseDialog => {
                self.state.game_form = None;
                self.state.confirm_delete = None;
            }
            // TODO(T-09) / T-10: the confirmation overlay.
            Message::ConfirmDeleteGame(_game_id) => {}
            // TODO(T-10): `Library::remove`, then persist and re-derive.
            Message::DeleteGameConfirmed(_game_id) => {}
            // TODO(T-09): open the portal file chooser for `field`.
            Message::PickExeFile { field: _field } => {}
            // TODO(T-11): write the chosen path into the form field, or a no-op
            // when cancelled.
            Message::ExeFileChosen { field: _field, path: _path } => {}
            // TODO(T-11): the cover file chooser.
            Message::PickCoverFile => {}
            // TODO(T-11): import the chosen image and set `cover_path`.
            Message::CoverFileChosen(_path) => {}
            // The one toast handler that exists, because `Toasts` needs it to
            // expire a toast at all.
            Message::DismissToast(id) => {
                self.state.toasts.remove(id);
            }
            // The other handler that already works, because a Quit that did
            // nothing would be a lie about the state of the app. `Core` tracks
            // the main window id, and iced's `window::close` is the supported
            // way to end it — the same path the window manager's own close
            // button takes, so no `std::process::exit` shortcut.
            Message::Quit => {
                if let Some(id) = self.core.main_window_id() {
                    return cosmic::iced::window::close(id);
                }
            }

            // ---- Settings --------------------------------------------------
            // TODO(T-13): validate against the allowed sets before storing —
            // `bridge.py` ignores an unrecognised value rather than saving it,
            // and the validation is part of the behaviour, not a guard.
            Message::SetColorScheme(_value) => {}
            Message::SetViewMode(_value) => {}
            Message::SetSortMode(_value) => {}
            Message::SetDefaultRunner(_value) => {}
            Message::SetCloseOnLaunch(_value) => {}
            Message::SetDefaultToggle { name: _name, value: _value } => {}

            // ---- Library view state ----------------------------------------
            // TODO(T-09): these two are trivially storable, but they land with
            // the Library page so the filter and the list move together.
            Message::SetSearchText(_text) => {}
            Message::SetCategoryFilter(_filter) => {}

            // ---- Library: the games themselves -----------------------------
            // TODO(T-09): validate, normalise the paths, then `Library::add` or
            // `update`, persist, and kick off a cover fetch when empty.
            Message::SaveGameForm(_form) => {}
            // TODO(T-10): `mark_played`, spawn the grace watch, and honour
            // `close_on_launch`.
            Message::LaunchGame(_game_id) => {}
            // TODO(T-10): toast the reason on `Some`, and bring the window back.
            Message::LaunchWatchFinished { game_id: _game_id, reason: _reason } => {}
            // TODO(T-10): `runners::run_tool` in a blocking task.
            Message::RunPrefixTool { game_id: _game_id, tool: _tool } => {}
            // TODO(T-10): `xdg-open` via `std::process`, off the UI thread.
            Message::OpenPrefixFolder(_game_id) => {}
            // TODO(T-10): `runners::desktop::create_desktop_shortcut`.
            Message::CreateDesktopShortcut(_game_id) => {}

            // ---- Covers ----------------------------------------------------
            // TODO(T-11): the saved-game fetch; `covers::fetch_cover` is
            // network-bound, so it belongs in a future task, never on the UI
            // thread.
            Message::FetchCover(_game_id) => {}
            Message::CoverFetchFinished { game_id: _game_id, result: _result } => {}
            // TODO(T-11): allocate the token with
            // `State::next_form_cover_token` at *emission* time, not here.
            Message::FetchCoverForForm {
                token: _token,
                game_id: _game_id,
                name: _name,
                exe: _exe,
            } => {}
            // TODO(T-11): drop the reply when `token` is no longer the form's
            // current one — `bridge.py:547-549` re-checks the game id for
            // exactly this reason.
            Message::FormCoverFetchFinished { token: _token, result: _result } => {}

            // ---- Runners ---------------------------------------------------
            // TODO(T-12): set `releases_family` and `releases_status` to
            // `Loading`, then fetch.
            Message::FetchReleases { family: _family } => {}
            // TODO(T-12): drop the result when `family` has been superseded.
            Message::ReleasesFetchFinished { family: _family, result: _result } => {}
            // TODO(T-12): guard on `runner_busy`.
            Message::InstallRunner { tag: _tag } => {}
            Message::RunnerProgress(_fraction) => {}
            Message::RunnerInstallFinished(_result) => {}
            // TODO(T-12): synchronous, so this returns no task.
            Message::UninstallRunner(_tag) => {}

            // ---- Easy installers -------------------------------------------
            // TODO(T-13) / T-10: the installer page's two filters.
            Message::SetInstallerSearch(_text) => {}
            Message::SetInstallerCategory(_category) => {}
            // TODO(T-13): guard on `easy_busy`.
            Message::StartEasyInstall {
                installer_id: _installer_id,
                runner_id: _runner_id,
            } => {}
            Message::EasyInstallProgress(_fraction) => {}
            // TODO(T-13): `found = None` stores a `PendingInstall` under the
            // game id and opens the picker — the `easyInstallNeedsExe` path.
            Message::EasyInstallWizardFinished {
                found: _found,
                returncode: _returncode,
            } => {}
            // TODO(T-13): a cancelled or empty path takes the
            // `CancelEasyInstall` route.
            Message::CompleteEasyInstall { token: _token, path: _path } => {}
            Message::CancelEasyInstall(_token) => {}
            // TODO(T-13): `Library::add`, then `notify`.
            Message::EasyInstallFinished {
                game_id: _game_id,
                message: _message,
            } => {}

            // ---- Plugins ---------------------------------------------------
            // TODO(T-13): recompute the plugin rows.
            Message::RefreshPlugins => {}
            // TODO(T-13): install, then report through
            // `PluginInstallFinished`.
            Message::InstallPlugin(_plugin_id) => {}
            Message::PluginInstallFinished {
                plugin_id: _plugin_id,
                result: _result,
            } => {}

            // ---- Internal plumbing -----------------------------------------
            // The `notify` signal. Every failure path in `bridge.py` ends here,
            // so this is the first handler worth having: it makes the others
            // implementable by reporting rather than by silently doing nothing.
            Message::Notify(text) => {
                // `Toasts::push` returns `Task<Message>` (it is what schedules
                // the toast's expiry), so it is mapped into the
                // `Action<Message>` the application trait hands back. Returning
                // `Task::none()` here would push the toast and never expire it.
                return self
                    .state
                    .toasts
                    .push(cosmic::widget::toaster::Toast::new(text))
                    .map(cosmic::Action::App);
            }
            // TODO(T-10): one tick of the grace watch, if the polling shape is
            // chosen over a single grace timeout (`architecture.md` §3.3).
            Message::LaunchWatchTick => {}
        }
        cosmic::task::none()
    }

    /// The explicit `'_` is load-bearing: omitting it trips
    /// `mismatched_lifetime_syntaxes`, which `-D warnings` turns into a build
    /// failure.
    fn view(&self) -> cosmic::Element<'_, Self::Message> {
        // Placeholder shell. T-08 replaces this with the nav bar, the page
        // routing and the toaster.
        cosmic::widget::container(cosmic::widget::text(APP_NAME))
            .center(cosmic::iced::Length::Fill)
            .into()
    }
}
