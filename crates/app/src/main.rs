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
use cosmic::app::ApplicationExt;
use cosmic::widget::{container, icon, nav_bar, text, toaster};
use gamehandler_core::models::Library;
use gamehandler_core::runners::families::ReleaseInfo;
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};
use gamehandler_core::settings::Settings;
use gamehandler_core::{APP_ID, APP_NAME, VERSION};

mod state;
// TODO(T-09): remove once the pages call these components. See D-32.
//
// The module is declared now, rather than when the pages land, because a
// declaration is what makes its tests run: without this line `cargo test`
// reports success 44 assertions short of the truth, which is the same defect
// as a check that never executes. The allow covers everything nested inside
// `view`, and it is deleted as part of T-09's definition of done.
#[allow(dead_code)]
mod view;

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

/// The environment variables winit accepts as proof of a display.
///
/// `WAYLAND_SOCKET` is the odd one: it carries an inherited file descriptor
/// rather than a socket name, and winit takes it in place of
/// `WAYLAND_DISPLAY`.
const DISPLAY_VARS: [&str; 3] = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"];

/// Is a display reachable, by **winit's own rule**?
///
/// This must not be stricter than winit, or the port would refuse to start on
/// a machine where the window would have opened. winit's rule is
/// `winit/src/platform_impl/linux/mod.rs:87-96`: `WAYLAND_DISPLAY` or
/// `WAYLAND_SOCKET` (set *and* non-empty) selects Wayland, `DISPLAY` (set and
/// non-empty) selects X11, and either is enough. All three are read with
/// `env::var`, and an empty value counts as unset — winit's own comment above
/// that match says "Empty variables are also treated as not set", and the
/// `.filter(|var| !var.is_empty())` on both branches is what does it.
///
/// The empty case is not hypothetical: `DISPLAY=""` is what a launcher, a unit
/// file or a `VAR=` assignment in a shell leaves behind when it forwards a
/// display variable that was never set, and winit refuses to open a window for
/// it. A guard that only checked "is the name present" would send exactly
/// those users into the panic described on [`run_gui`].
///
/// The value's *shape* is deliberately not validated: winit does not, so
/// `DISPLAY=garbage` is accepted here and fails later in winit, with winit's
/// message. Being cleverer than winit here means refusing to start where a
/// window would have opened.
///
/// `env` is a lookup function rather than a read of the process environment so
/// this is testable without `set_var`, which cargo's parallel test runner
/// shares across threads and which would make these tests race. The one thing
/// that cannot be pinned from here is that the *caller* reads with
/// `std::env::var` — the signature only admits an `Option<String>`, so the
/// UTF-8 strictness lives at the call site and is stated there.
fn display_present(env: impl Fn(&str) -> Option<String>) -> bool {
    DISPLAY_VARS
        .iter()
        .any(|name| env(name).is_some_and(|value| !value.is_empty()))
}

/// The diagnostic printed when there is no display (N-01).
///
/// Actionable rather than merely truthful: it names the three variables that
/// were checked, says what to do about it, and — the part that matters most —
/// names the commands that do *not* need a display, because those are what a
/// `.desktop` shortcut invokes.
fn no_display_hint() -> String {
    format!(
        "  WAYLAND_DISPLAY, WAYLAND_SOCKET and DISPLAY are all unset or empty.\n\
         \x20 Run this inside a graphical session to use the interface.\n\
         \x20 These commands need no display:\n\
         \x20   {APP_NAME} --list\n\
         \x20   {APP_NAME} --launch <game id>"
    )
}

/// Start the graphical interface. Reached only when no CLI flag was given.
///
/// # Why the check is here and not around `run()`
///
/// winit *does* detect this and returns a `NotSupportedError`
/// (`winit/src/platform_impl/linux/mod.rs:106-117`), but that error never
/// reaches the `Err` arm below: iced_winit consumes it one call later with
/// `EventLoop::new().expect("Create event loop")`
/// (`iced/winit/src/lib.rs:92`), which panics with a raw traceback and an
/// exit code of 101. So the arm below cannot fire on this path — a fact worth
/// writing down, because the arm reads as if it handled this case and does
/// not. (Measured, not assumed: see the N-01 note in `ux.md`.)
///
/// `catch_unwind` is not the fix either: it cannot catch anything in a
/// `panic = "abort"` build, and where it does catch it leaves the process
/// half-initialised, so the diagnostic would depend on the build profile. The
/// environment is checked first instead, which puts the decision on the path
/// that decides and cannot be defeated by a profile.
///
/// The lookup is `std::env::var` — the same reader winit uses, and the reason
/// a variable that is not valid UTF-8 counts as absent rather than present.
/// `var_os` here would accept a value winit rejects, and this guard would then
/// pass a display through to the panic it exists to prevent.
fn run_gui() -> ExitCode {
    if !display_present(|name| std::env::var(name).ok()) {
        eprintln!("{APP_NAME}: cannot open the interface — no display is available.");
        eprintln!("{}", no_display_hint());
        return ExitCode::FAILURE;
    }

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

/// The icon a page shows in the sidebar.
///
/// Ported from the reference drawer's `Kirigami.Action`s (`Main.qml:67-108`),
/// one per page and in the same order. The names are the reference's own, KDE's
/// naming rather than freedesktop's — `run-install`, `plugins` and `help-about`
/// are not in every icon theme, and libcosmic draws nothing for a name it
/// cannot resolve. Keeping the reference's names is still the right default:
/// inventing "more likely to resolve" ones would be a silent visual change to
/// parity, and the resolution is a property of the *theme*, not of this port.
fn page_icon(page: Page) -> &'static str {
    match page {
        Page::Library => "applications-games",
        Page::Installers => "run-install",
        Page::Runners => "folder-download",
        Page::Plugins => "plugins",
        Page::Credits => "help-about",
        Page::Settings => "configure",
    }
}

/// The sidebar's model, built from [`Page::ALL`].
///
/// One row per page, in that list's order, each carrying its own [`Page`] as row
/// data — which is what lets a click be turned back into a page in
/// [`App::on_nav_select`] by reading the row rather than by counting positions.
///
/// The reference starts on the Library page (`Main.qml:56`,
/// `pageStack.initialPage: libraryPage`), so the first row is active.
///
/// The label comes from [`Page::label`], including for the one page where it
/// disagrees with the reference: `Main.qml:99` and `CreditsPage.qml:10` both say
/// **"About & Credits"**, and `Page::label` says "Credits". The mismatch is
/// recorded here and reported rather than patched over, because the fix is in
/// `core`'s table and a second copy of the labels here would be a third place
/// for them to drift.
fn build_nav_model() -> nav_bar::Model {
    let mut model = nav_bar::Model::default();
    for page in Page::ALL {
        model
            .insert()
            .text(page.label())
            .icon(icon::from_name(page_icon(page)))
            .data(page);
    }
    model.activate_position(0);
    model
}

/// Point the sidebar at `page`.
///
/// Split out of [`App::go_to`] so the page-to-row mapping can be tested without
/// a `Core`, which needs a display to construct. The position is looked up in
/// [`Page::ALL`] rather than written as a literal, so the two orders cannot
/// disagree.
fn activate_page(model: &mut nav_bar::Model, page: Page) -> bool {
    let position = Page::ALL
        .iter()
        .position(|candidate| *candidate == page)
        .expect("every Page is in Page::ALL");
    model.activate_position(position as u16)
}

/// The application state.
///
/// Holds the COSMIC `Core` (window and theme) and the ported [`State`]. The
/// split is the framework's, not ours: `Core` is what `cosmic::Application`
/// requires, and `State` is what `bridge.py`'s `Backend` fields became.
pub struct App {
    core: cosmic::Core,
    state: State,
    /// The sidebar, drawn by the framework from [`cosmic::Application::nav_model`].
    ///
    /// This is the *second* record of which page is showing, `state.page` being
    /// the first, because the framework wants a model it can read while
    /// rendering and `State` is not ours to restructure. Two records that must
    /// agree is a defect waiting to happen, so they are written together in
    /// exactly one place — [`App::go_to`] — and the tests below check that
    /// every page survives the round trip.
    nav_model: nav_bar::Model,
}

impl App {
    /// The one place the current page changes.
    ///
    /// Both halves move together: `state.page`, which the pages read, and the
    /// sidebar's selection, which the framework draws. A write to `state.page`
    /// anywhere else is how these two drift — a sidebar showing one page while
    /// the body shows another, which no test of either half alone would catch.
    fn go_to(&mut self, page: Page) {
        self.state.page = page;
        activate_page(&mut self.nav_model, page);
    }
}

/// The body of a page that has not been ported yet.
///
/// T-08 is the shell: the sidebar, the routing and the toaster. Each page's own
/// content is a later task, and the task is named **on screen** as well as in
/// the code so the shell cannot be mistaken for a finished interface — a page
/// that renders an empty body reads as a bug in the shell, which is the wrong
/// thing to go looking for. The Library page is T-09, so that arm is the first
/// to be replaced.
fn pending_page(page: Page, task: &str) -> cosmic::Element<'_, Message> {
    container(
        cosmic::widget::column::with_capacity(2)
            .push(text::title2(page.label()))
            .push(text::body(format!("This page has not been ported yet ({task}).")))
            .spacing(12),
    )
    .center(cosmic::iced::Length::Fill)
    .into()
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
        let mut app = App {
            core,
            state: State::new(library, settings, runners),
            nav_model: build_nav_model(),
        };

        // `Main.qml:11` (`title: "GameHandler"`) and the drawer's own
        // `title` at `:63` — one constant, the application's name. It is
        // deliberately **not** the current page: the reference's window title
        // does not change as the user navigates, and libcosmic's header bar
        // would otherwise start out empty, since `Core` defaults
        // `header_title` to `""` (`core.rs:161`) while showing the bar
        // (`core.rs:167`, `show_headerbar: true`).
        //
        // `set_window_title` takes a window id because this build has the
        // `multi-window` feature on — not by choice but as a consequence:
        // libcosmic's `wayland` feature implies it (`Cargo.toml:85-95`), and
        // this app needs `wayland`. The main window exists by the time `init`
        // runs, so the id is available; `None` is handled rather than
        // `expect`ed because a missing window is not worth taking the process
        // down for.
        app.set_header_title(APP_NAME.to_string());
        let title = match app.core().main_window_id() {
            Some(id) => app.set_window_title(APP_NAME.to_string(), id),
            None => cosmic::task::none(),
        };

        (app, title)
    }

    /// The sidebar.
    ///
    /// libcosmic draws the nav panel from this model itself
    /// (`src/app/mod.rs:398-417`, the default `nav_bar`), so returning `Some`
    /// here is the whole of "there is a sidebar", and a click arrives back as
    /// [`cosmic::Application::on_nav_select`].
    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_model)
    }

    /// A sidebar row was clicked.
    ///
    /// The page comes out of the *row's own data* rather than from its
    /// position: positions are an implementation detail of the model, and a row
    /// inserted in the wrong place would then select the wrong page silently.
    fn on_nav_select(&mut self, id: nav_bar::Id) -> cosmic::app::Task<Self::Message> {
        if let Some(page) = self.nav_model.data::<Page>(id).copied() {
            self.go_to(page);
        }
        cosmic::task::none()
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
            // The route used by everything that is not the sidebar itself — a
            // page's own buttons, the credits link, a shortcut. It goes through
            // `go_to` so the sidebar's selection moves with it; see that
            // function for why the two must not be written separately.
            Message::NavigateTo(page) => {
                self.go_to(page);
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
    ///
    /// This is the *page body*. The sidebar is not composed here: libcosmic
    /// calls [`cosmic::Application::nav_model`] and lays the panel out around
    /// whatever this returns, so a page can neither forget the sidebar nor draw
    /// a second one.
    fn view(&self) -> cosmic::Element<'_, Self::Message> {
        let body = match self.state.page {
            // TODO(T-09): the grid and list, with search, sort and the category
            // filter (`LibraryPage.qml`).
            Page::Library => pending_page(Page::Library, "T-09"),
            // TODO(T-12): the release list and the install progress.
            Page::Installers => pending_page(Page::Installers, "T-12"),
            // TODO(T-11): the runner manager's page.
            Page::Runners => pending_page(Page::Runners, "T-11"),
            // TODO(T-13): the plugin list.
            Page::Plugins => pending_page(Page::Plugins, "T-13"),
            // TODO(T-13): about and credits.
            Page::Credits => pending_page(Page::Credits, "T-13"),
            // TODO(T-13): the settings form.
            Page::Settings => pending_page(Page::Settings, "T-13"),
        };

        // Every failure path in `bridge.py` ends in the `notify` signal, so the
        // toaster is wrapped around the whole body rather than placed inside a
        // page: a toast raised by one page must survive a navigation to another,
        // and `Toasts` lives in `State` for exactly that reason.
        toaster(&self.state.toasts, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment built from `(name, value)` pairs, as the lookup
    /// [`display_present`] takes. Keeps every case below off the real process
    /// environment, which the test runner shares.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    /// With nothing set, there is no display — the case N-01 exists for.
    #[test]
    fn an_empty_environment_has_no_display() {
        assert!(!display_present(env(&[])));
    }

    /// The three names winit's rule is written against, spelled out here on
    /// purpose.
    ///
    /// This is a **transcription, not a reference**, and the difference is the
    /// whole point: the first version of this test iterated `DISPLAY_VARS`, so
    /// deleting a name from that constant deleted it from the test's coverage
    /// too and the suite stayed green with the guard silently missing
    /// `WAYLAND_SOCKET`. Measured, not imagined — that mutation survived until
    /// this constant was added.
    ///
    /// `winit/src/platform_impl/linux/mod.rs:87-96`.
    const WINIT_READS: [&str; 3] = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"];

    /// The guard checks **exactly** the variables winit reads: no fewer, and no
    /// more.
    ///
    /// No fewer, because each one alone is proof of a display that winit will
    /// accept, so leaving one out refuses to start where a window would have
    /// opened. No more, because a name winit does not read is not proof of
    /// anything — accepting one sends the process into the panic this guard
    /// exists to prevent.
    #[test]
    fn the_variables_checked_are_exactly_the_ones_winit_reads() {
        let mut ours: Vec<&str> = DISPLAY_VARS.to_vec();
        let mut winits: Vec<&str> = WINIT_READS.to_vec();
        ours.sort_unstable();
        winits.sort_unstable();
        assert_eq!(
            ours, winits,
            "the guard and winit must agree on which variables mean 'there is a \
             display'"
        );
    }

    /// Either Wayland variable alone is enough, and so is `DISPLAY` alone.
    ///
    /// Asserted one at a time rather than as "one of these works", because a
    /// guard that only accepted `WAYLAND_DISPLAY` would pass the weaker test
    /// and refuse to start for a user on X11 or on `WAYLAND_SOCKET`.
    #[test]
    fn any_one_display_variable_is_enough() {
        for name in WINIT_READS {
            assert!(
                display_present(env(&[(name, ":0")])),
                "{name} alone should be accepted, as winit accepts it"
            );
        }
    }

    /// **The subtlety that makes this worth a function.** A variable that is
    /// *set but empty* counts as unset, which is winit's
    /// `.filter(|var| !var.is_empty())`
    /// (`winit/src/platform_impl/linux/mod.rs:88-95`). `DISPLAY=""` is not
    /// hypothetical: it is what remains when a launcher or a unit file forwards
    /// a display variable that was never set.
    #[test]
    fn a_display_variable_set_to_the_empty_string_counts_as_unset() {
        assert!(!display_present(env(&[("DISPLAY", "")])));
        assert!(!display_present(env(&[("WAYLAND_DISPLAY", "")])));
        assert!(!display_present(env(&[("WAYLAND_SOCKET", "")])));
        assert!(
            !display_present(env(&[("DISPLAY", ""), ("WAYLAND_DISPLAY", "")])),
            "several empty variables are still no display"
        );
    }

    /// One real value among empty ones is enough, which is the mixed case a
    /// naive `all()` or `any(v.is_empty())` implementation gets wrong.
    #[test]
    fn one_real_value_among_empty_ones_is_enough() {
        assert!(display_present(env(&[
            ("WAYLAND_DISPLAY", ""),
            ("DISPLAY", ":0"),
        ])));
    }

    /// **This guard must not be stricter than winit.** A value that is merely
    /// non-empty is accepted, even when it cannot possibly name a display —
    /// because winit accepts it, and being cleverer here means refusing to
    /// start where the window would have opened. A stale `DISPLAY=:99` with no
    /// X server fails later, in winit, with winit's message; that is the right
    /// place for it. This test exists to stop a future "improvement" that
    /// validates the value's shape.
    #[test]
    fn any_non_empty_value_is_accepted_exactly_as_winit_accepts_it() {
        assert!(display_present(env(&[("DISPLAY", "garbage")])));
        assert!(display_present(env(&[("DISPLAY", " ")])));
        assert!(display_present(env(&[("DISPLAY", ":99.0")])));
    }

    /// A variable the lookup cannot read is "not set", and does not stop the
    /// others being tried.
    ///
    /// That is the case a value of `None` models: `std::env::var` answers
    /// `None` for a value that is not valid UTF-8, and winit reads these with
    /// `env::var` too, so the two agree on which variables exist. What this
    /// test pins is the fall-through — an implementation that returned early on
    /// a `None` would refuse to start for a Wayland session whose `DISPLAY`
    /// happened to be unreadable.
    ///
    /// It cannot pin the UTF-8 strictness itself: `display_present` only sees
    /// an `Option<String>`, so it cannot tell `var` from `var_os`. That choice
    /// lives in the one call site in `run_gui`, and is stated there.
    #[test]
    fn an_unreadable_variable_is_not_set_and_the_others_are_still_tried() {
        assert!(
            display_present(|name| if name == "WAYLAND_DISPLAY" {
                Some("wayland-0".into())
            } else {
                None
            }),
            "an unreadable DISPLAY must not stop WAYLAND_DISPLAY being checked; \
             the three variables are read independently and either is enough"
        );
    }

    /// The hint has to be actionable, so it names every variable that was
    /// checked and both commands that work without a display. A message that
    /// merely said "no display" would satisfy N-02 and fail N-01's purpose.
    #[test]
    fn the_hint_names_the_variables_and_the_headless_commands() {
        let hint = no_display_hint();
        for name in WINIT_READS {
            assert!(hint.contains(name), "the hint should name {name}");
        }
        assert!(hint.contains("--list"), "the hint should name --list");
        assert!(
            hint.contains("--launch"),
            "the hint should name --launch: shortcuts invoke it, and it is the \
             path that must never need a display"
        );
        assert!(hint.contains(APP_NAME), "the hint should name the program");
    }

    /// Every line of the hint is indented to the same depth, so it reads as one
    /// block under the error line above it rather than as a paragraph.
    ///
    /// This pins the `\x20` continuations: a Rust string continued with `\` at
    /// the end of a line loses the next line's leading whitespace, so without
    /// the escapes the indent would silently collapse — which is why the test
    /// is on the rendered lines and not on the source layout.
    #[test]
    fn every_line_of_the_hint_is_indented_and_none_has_a_trailing_space() {
        let hint = no_display_hint();
        let lines: Vec<&str> = hint.lines().collect();
        assert!(lines.len() > 1, "the hint is a block, not one line");
        for line in &lines {
            assert!(
                line.starts_with("  "),
                "{line:?} should be indented like the rest of the block"
            );
            assert!(
                !line.ends_with(' '),
                "{line:?} should not end in a space"
            );
        }
    }

    // ---- The shell: the sidebar and the routing ---------------------------

    /// The sidebar lists every page, in `Page::ALL` order, and each row carries
    /// its own page as data.
    ///
    /// The order is user-visible — it is the order the panel lists them in — and
    /// the row data is what turns a click back into a page, so a row inserted
    /// without its `data` would make the sidebar silently unclickable rather
    /// than wrong.
    #[test]
    fn the_sidebar_lists_every_page_in_order_with_its_label_and_its_page() {
        let model = build_nav_model();
        let rows: Vec<(&str, Page)> = model
            .iter()
            .map(|id| {
                let label = model.text(id).expect("every row is given a label");
                let page = *model
                    .data::<Page>(id)
                    .expect("every row carries its page as data");
                (label, page)
            })
            .collect();

        let expected: Vec<(&str, Page)> = Page::ALL
            .iter()
            .map(|page| (page.label(), *page))
            .collect();
        assert_eq!(
            rows, expected,
            "the panel's rows and Page::ALL must be the same list in the same \
             order: the order is what the user sees, and the page in each row is \
             what a click resolves to"
        );
    }

    /// The reference opens on the Library page (`Main.qml:56`), so the panel's
    /// first row is the active one before the user touches anything.
    #[test]
    fn the_sidebar_starts_on_the_library_page() {
        let model = build_nav_model();
        assert_eq!(
            model.active_data::<Page>(),
            Some(&Page::Library),
            "the initial page is `pageStack.initialPage: libraryPage`"
        );
    }

    /// Every page survives a round trip through the sidebar: activating the page
    /// leaves the model reporting that same page as active.
    ///
    /// This is the property `App::go_to` depends on — it sets `state.page` and
    /// the model's selection from the same `Page`, and if these two disagreed
    /// the panel would highlight one page while the body showed another. Run
    /// over all six rather than one, because a position computed as a literal
    /// would pass for `Library` and fail for everything after it.
    #[test]
    fn every_page_can_be_selected_and_read_back_from_the_sidebar() {
        let mut model = build_nav_model();
        for page in Page::ALL {
            assert!(
                activate_page(&mut model, page),
                "{page:?} should be selectable"
            );
            assert_eq!(
                model.active_data::<Page>(),
                Some(&page),
                "after selecting {page:?} the sidebar must report {page:?}"
            );
        }
    }

    /// Every page has a distinct, non-empty icon.
    ///
    /// Distinctness is the assertion with teeth: the six names are a
    /// transcription, and copying one row's name into the next is the mistake
    /// that produces a panel where two pages share a picture — invisible in a
    /// code review of the table, obvious on screen.
    #[test]
    fn every_page_has_its_own_icon() {
        let mut seen: Vec<&str> = Vec::new();
        for page in Page::ALL {
            let name = page_icon(page);
            assert!(!name.is_empty(), "{page:?} should have an icon name");
            assert!(
                !seen.contains(&name),
                "{name} is used by {page:?} and by an earlier page; the \
                 reference gives each page its own"
            );
            seen.push(name);
        }
        assert_eq!(seen.len(), Page::ALL.len());
    }

    /// The icon names are the reference drawer's, checked against `Main.qml`
    /// itself rather than against a copy of the list.
    ///
    /// `page_icon` is a transcription of `Main.qml:67-108`, and a transcription
    /// verified by eye is verified by the same eyes that made the mistake. The
    /// QML is still in the tree as the port's specification, so the comparison
    /// can be mechanical: this reads the six `icon.name:` values out of the
    /// drawer, in order, and requires them to be the six this shell uses.
    #[test]
    fn the_shells_icons_are_the_reference_drawers_icons_in_order() {
        let main_qml =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gamehandler/qml/Main.qml");
        let text = std::fs::read_to_string(&main_qml).unwrap_or_else(|err| {
            panic!(
                "{} should be readable: {err}\n\
                 It is the reference this shell was ported from, and the port's \
                 own tests read it. If it has been moved, this check needs a new \
                 path — and so does every citation in docs/migration/.",
                main_qml.display()
            )
        });

        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("icon.name:"))
            .filter_map(|rest| rest.trim().strip_prefix('"'))
            .filter_map(|rest| rest.split_once('"').map(|(name, _)| name))
            .collect();

        let ours: Vec<&str> = Page::ALL.iter().map(|page| page_icon(*page)).collect();
        assert_eq!(
            names.len(),
            Page::ALL.len(),
            "the drawer should name one icon per page, in nav order; found \
             {names:?}. If a page gained or lost an icon in the QML, this shell \
             and the reference have diverged."
        );
        assert_eq!(
            names, ours,
            "the shell's icons must be the reference's, in the reference's order"
        );
    }
}
