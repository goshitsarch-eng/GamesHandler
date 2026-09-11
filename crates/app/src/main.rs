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
use gamehandler_core::models::{Library, SORT_MODES};
use gamehandler_core::runners::families::ReleaseInfo;
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};
use gamehandler_core::settings::{COLOR_SCHEMES, Settings, VIEW_MODES};
use gamehandler_core::{APP_ID, APP_NAME, VERSION};

mod http;
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

/// The lines `--list` prints, exactly as `main.py:_list_games` prints them.
///
/// Ported from Python, which is `Library().all()` → the empty-library line, or
/// one `f"{game.id}\t{game.name}"` row per game. `all` with no argument sorts
/// by `name`, and [`Library::all`]'s fall-through arm is that same
/// case-insensitive `name` sort, so `"name"` is passed explicitly rather than
/// relied on — a caller that passed something else would change the output
/// silently, and this is what a script parses.
///
/// # The correctness criterion for this function is not "matches Python"
///
/// It was, and that was the bug: an earlier version of this file printed the
/// empty-library line unconditionally, with a doc comment arguing it was
/// correct *because* it was byte-identical to Python's empty case. It was
/// byte-identical — and therefore indistinguishable from a real empty library.
/// A script running `--list` on a machine with a full library got "the library
/// is empty" and `rc=0`, and could not tell that from a build that never opened
/// the file.
///
/// So the criterion is **"cannot be mistaken for the real thing"**, and output
/// identity is only the second question. For a stub the two pull in opposite
/// directions: the closer the fabricated text is to the truth, the more
/// convincing the lie. Nothing here is fabricated now — every line is computed
/// from the library that was actually loaded — and the rule for anything that
/// has to stay stubbed is in [`launch_failure`].
fn list_lines(library: &Library) -> Vec<String> {
    let games = library.all("name");
    if games.is_empty() {
        return vec![format!("{APP_NAME}: the library is empty")];
    }
    games
        .iter()
        .map(|game| format!("{}\t{}", game.id, game.name))
        .collect()
}

/// `--list`: print every game's id and name, tab-separated, one per line.
///
/// # The one deliberate divergence from Python
///
/// `main.py:_list_games` opens with `config.ensure_dirs()`, which creates the
/// config and runners directories. This does not, because creating directories
/// is a side effect a *read* command has no business having: `--list` is what a
/// script, a launcher or a `--help` wrapper calls, and on a fresh machine it
/// would leave two new directories behind having printed nothing but a refusal.
///
/// It is unobservable in the output, which is the part that is contract, and
/// observable only as the absence of a side effect that no caller asked for.
/// Recorded here rather than silently dropped, since the load path does not
/// need the directories: [`Library::load`] treats a missing file as an empty
/// library and never writes.
fn list_games() -> ExitCode {
    for line in list_lines(&Library::new(None)) {
        println!("{line}");
    }
    ExitCode::SUCCESS
}

/// Why `--launch` cannot start this game, and the code to exit with.
///
/// Returns `(stderr text, exit code)`. Every path is currently a failure,
/// because the launch itself is not ported yet — `runners::launch`,
/// `mark_played` and the P-46 immediate-failure grace check are T-03's next
/// module. When they land, the success case (`(None, 0)`) is added here and
/// the caller stops printing on it; the two failure paths below do not change.
///
/// # Why an unported launch is an error and not a silent success
///
/// A stub for this command has three options and only one of them is honest:
/// print a plausible success (a lie — `.desktop` shortcuts invoke this, so the
/// user gets a menu entry that opens nothing and reports nothing), print
/// Python's unknown-id message for a game that *does* exist (also a lie, and
/// the one an earlier version of this file told), or say what is actually true.
/// The load and the lookup are real, so the only thing left to admit is that
/// the launch is not: that is what the second arm does, with a non-zero code so
/// a script cannot mistake it for a launched game.
///
/// The lookup is not incidental to that. It is the part of
/// `main.py:_launch_from_cli` that needs no launch machinery, and doing it for
/// real is what makes the unknown-id message mean what it says — the same
/// message for an id that is in the library was the inverted criterion again.
fn launch_failure(library: &Library, game_id: &str) -> (String, u8) {
    let Some(game) = library.get(game_id) else {
        return (format!("{APP_NAME}: no game with id {game_id}"), 1);
    };
    (
        format!(
            "{APP_NAME}: could not launch {}: launching is not implemented in \
             this build",
            game.name
        ),
        1,
    )
}

/// `--launch <GAME_ID>`: start a game and report whether it stayed up.
///
/// Ported from `main.py:_launch_from_cli` as far as the library allows: the
/// lookup and its message are Python's, and the launch itself is
/// [`launch_failure`]'s second arm.
fn launch_game(game_id: &str) -> ExitCode {
    let (message, code) = launch_failure(&Library::new(None), game_id);
    eprintln!("{message}");
    ExitCode::from(code)
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
/// `winit/src/platform_impl/linux/mod.rs:89-95`: `WAYLAND_DISPLAY` or
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
        "  WAYLAND_DISPLAY, WAYLAND_SOCKET and DISPLAY are all unset or empty,\n\
         \x20 so no window can be opened. Run this inside a graphical session.\n\
         \x20 These commands need no display:\n\
         \x20   {APP_NAME} --list\n\
         \x20   {APP_NAME} --launch <game id>"
    )
}

/// Refuse to start the interface when there is no display, and say why.
///
/// Returns `true` when it refused. Writes the whole diagnostic to `out` — the
/// one line naming the failure and the hint block under it — and writes nothing
/// when a display is present, which is itself asserted: a function that printed
/// the refusal to a working session would be a bug no test of the string alone
/// could see.
///
/// `out` is a parameter rather than `std::io::stderr()` captured inside so this
/// is testable without a display and without capturing the process's stderr,
/// which the test runner shares with every other test in the binary.
fn display_refusal(env: impl Fn(&str) -> Option<String>, out: &mut impl std::io::Write) -> bool {
    if display_present(env) {
        return false;
    }
    // The write failure is ignored on purpose: this is the last thing the
    // process does, and a broken stderr is not a reason to change the exit code
    // that a `.desktop` shortcut or a script reads.
    let _ = writeln!(
        out,
        "{APP_NAME}: cannot open the interface — no display is available."
    );
    let _ = writeln!(out, "{}", no_display_hint());
    true
}

/// What came of asking the interface to start.
///
/// A value rather than an [`ExitCode`], so the three outcomes can be told apart:
/// `std::process::ExitCode` is opaque and cannot be compared in a test, and
/// "failed" and "refused" both exit non-zero — but only one of them means the
/// event loop was never reached, which is the thing `run_gui`'s doc comment
/// spends a paragraph on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GuiStart {
    /// No display: nothing was attempted, and the diagnostic was written.
    NoDisplay,
    /// The interface ran and closed normally.
    Ran,
    /// The interface could not start; the reason was written as well as
    /// returned.
    Failed(String),
}

impl GuiStart {
    /// The status a shell or a `.desktop` shortcut sees.
    ///
    /// A `u8` rather than an [`ExitCode`] because `ExitCode` is opaque and
    /// cannot be compared in a test, which would leave this decision — the one
    /// a script branches on — unwatched. `run_gui` converts with
    /// `ExitCode::from`, so what is untested is std's conversion and nothing of
    /// ours.
    fn exit_status(&self) -> u8 {
        match self {
            // `Ran` is the only outcome where the interface was actually shown.
            GuiStart::Ran => 0,
            GuiStart::NoDisplay | GuiStart::Failed(_) => 1,
        }
    }
}

/// Decide, and report, what starting the interface does.
///
/// The environment, the writer and the starter are all parameters so the whole
/// of `run_gui` outside its three wiring lines is reachable from a test — in
/// particular the case a display-free machine hits, which is the case the
/// application's own N-01 diagnostic exists for and which no test could reach
/// while the prints lived inside `run_gui`.
fn start_gui(
    env: impl Fn(&str) -> Option<String>,
    out: &mut impl std::io::Write,
    start: impl FnOnce() -> Result<(), String>,
) -> GuiStart {
    if display_refusal(env, out) {
        return GuiStart::NoDisplay;
    }
    match start() {
        Ok(()) => GuiStart::Ran,
        Err(error) => {
            // A failure to start is reported here rather than by the caller, so
            // there is one place that decides what a user sees and one place
            // that can be tested for it.
            let _ = writeln!(
                out,
                "{APP_NAME}: could not start the interface: {error}"
            );
            GuiStart::Failed(error)
        }
    }
}

/// Start the graphical interface. Reached only when no CLI flag was given.
///
/// # Why the check is here and not around `run()`
///
/// winit *does* detect this and returns a `NotSupportedError`
/// (`winit/src/platform_impl/linux/mod.rs:116`), but that error never
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
    // Everything this function decides is decided by `start_gui`, which takes
    // the environment, the writer and the starter as parameters so a test can
    // supply its own. What is left here is the three things a test cannot
    // supply: the process's environment, its stderr, and the real event loop.
    //
    // That split is not tidiness. With the prints inline here, the tests could
    // only call `no_display_hint` and check the string it returns — so deleting
    // both `eprintln!`s, the whole user-visible half of N-01, failed nothing.
    // The same was true of the `Err` arm, which had no test at all: a start
    // that failed said nothing to anyone.
    let outcome = start_gui(
        |name| std::env::var(name).ok(),
        &mut std::io::stderr(),
        || {
            let settings =
                cosmic::app::Settings::default().size(cosmic::iced::Size::new(1200.0, 800.0));
            // `String` rather than the framework's error type so the seam does
            // not depend on it; the message is all that is used.
            cosmic::app::run::<App>(settings, ()).map_err(|error| error.to_string())
        },
    );

    ExitCode::from(outcome.exit_status())
}

/// Messages handled by `App::update`.
///
/// Not an intra-doc link: `update` is `cosmic::Application`'s method, not an
/// inherent one, and `[`App::update`]` does not resolve. Its inherent half is
/// [`Shell::update`], which does.
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
    /// Clear the search box **and** the category filter together.
    ///
    /// One message rather than two: the reference's button does both writes
    /// (`LibraryPage.qml:118-121`), and a single message is what makes them
    /// atomic — so the page cannot be drawn with one cleared and not the other.
    ClearFilters,

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
    /// The install finished.
    ///
    /// `tag` is on the message rather than only on the success arm because both
    /// of Python's callbacks name the release: `done` says "Installed
    /// {release.tag}." (`bridge.py:756-758`) and `fail` says "Failed to install
    /// {release.tag}: {message}" (`:767`). A `Result<String, String>` could
    /// carry it on one arm only, and the failure line would then read "Failed
    /// to install: …" — a sentence the reference never renders.
    RunnerInstallFinished {
        tag: String,
        result: Result<(), String>,
    },
    /// Delete an installed build. `uninstallRunner()` — synchronous, so this
    /// returns no task.
    UninstallRunner(String),
    /// The Runners page's two row bundles, computed off the update thread.
    ///
    /// Not a user action and not a Python concept: the reference builds these
    /// in a QML `Property` getter, on the UI thread, at render time. Held rows
    /// have to arrive from somewhere, and this is that somewhere — see
    /// [`crate::view::runners::refresh`], which is the only sender.
    ///
    /// `token` is the stale-reply guard: a reply whose token is not the current
    /// one is dropped.
    RunnersRefreshed {
        token: u64,
        installed: Vec<crate::view::runners::InstalledRow>,
        release_rows: Vec<crate::view::runners::ReleaseRow>,
    },

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
/// `App::on_nav_select` by reading the row rather than by counting positions.
///
/// The reference starts on the Library page (`Main.qml:56`,
/// `pageStack.initialPage: libraryPage`), so the first row is active.
///
/// The label comes from [`Page::label`], which is now the reference's own
/// wording for every page. It said "Credits" where `Main.qml:94` and
/// `CreditsPage.qml:10` both say **"About & Credits"** until T-13 fixed it
/// (P-65); the mismatch was recorded in the `KNOWN_LABEL_DIVERGENCE` table
/// below rather than patched here, because a second copy of the labels in this
/// file would be a third place for them to drift.
///
/// (`state.rs`, not `core`: an earlier version of this comment sent the reader to
/// `core`'s table for [`Page::label`], which lives in `crate::state`.)
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
/// Split out of [`Shell::show_page`] so the page-to-row mapping can be tested
/// without
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

/// The application: the COSMIC [`cosmic::Core`] plus a [`Shell`].
///
/// The split is not cosmetic. `Core` is what [`cosmic::Application`] requires
/// (window, theme, header bar) and it can only be built by the framework, so an
/// `App` cannot be constructed in a test — which is exactly why every handler
/// lives on [`Shell`] instead, and why [`App`] is three delegations and a
/// `Quit`.
pub struct App {
    core: cosmic::Core,
    shell: Shell,
}

/// Everything the shell does that is not the framework's.
///
/// [`App`] is this plus a [`cosmic::Core`], and the split is deliberate: `Core`
/// can only be built by the framework (it owns the window, and
/// [`cosmic::Core::default`] leaves `main_window` as `None`), so an `App` cannot
/// be constructed in a test — and a handler that only touches these two fields
/// does not need one. `App::update` is a delegation to [`Shell::update`], so
/// the handlers a test calls are the same ones the running app calls, down to
/// the match arms.
pub struct Shell {
    state: State,
    /// The sidebar, drawn by the framework from [`cosmic::Application::nav_model`].
    ///
    /// This is the *second* record of which page is showing, `state.page` being
    /// the first, because the framework wants a model it can read while
    /// rendering and `State` is not ours to restructure. Two records that must
    /// agree is a defect waiting to happen, so they are written together in
    /// exactly one place — [`Shell::show_page`] — and the checks below are what
    /// keep that true rather than merely stated.
    nav_model: nav_bar::Model,
}

/// What arriving at `page` starts, if anything. D-48's page-entry emission.
///
/// The counterpart of the reference's `Component.onCompleted`, which fires once
/// per page *instance*: `RunnersPage.qml:19` fetches the selected family's
/// releases when the page is completed, and `:26` re-fetches when the user
/// changes the family. Both routes reach the same handler, so the two cannot
/// drift apart.
///
/// `Runners` is the only page with entry work. Every other page draws state the
/// shell already holds, so arriving there is a repaint and nothing more — an
/// empty arm rather than a missing one, which is why the wildcard is written
/// out rather than left to fall through.
///
/// # Why this goes through the page's own `update`
///
/// Entering the page is the same event as a family change: it must set
/// `releases_family`, mark the fetch in flight, clear the stale list, recompute
/// the rows and start the download. Writing that out again here would be a
/// second copy of `FetchReleases`'s transition — the copy that goes stale when
/// the transition changes. Calling the handler instead is what makes "entering
/// the page fetches the selected family" true by construction.
///
/// # Which family
///
/// The one the page is showing, which is `state.releases_family` and defaults
/// to [`view::runners::default_family`] — `"proton-ge"` — before the user has
/// ever changed it, which is the reference's own default
/// (`RunnersPage.qml:14`).
///
/// **This is a deliberate divergence, and it is worth stating because the
/// reference's literal behaviour is different.** In QML, `selectedFamilyId` is
/// a page-local property, so a fresh page instance resets to `"proton-ge"` and
/// re-entering the page always refetches Proton-GE even if the user had chosen
/// another family a moment earlier. This port hoisted that property into
/// `State` — the decision is recorded in `view::runners`'s header, because the
/// dropdown has to be bound to *something* and a value that vanishes on
/// navigation cannot be it — and once the family outlives the page, fetching
/// anything but the family the dropdown is showing would draw a list under a
/// selector that names a different family. The two must be the same value.
fn page_entry_task(state: &mut State, page: Page) -> cosmic::app::Task<Message> {
    match page {
        Page::Runners => {
            let family = if state.releases_family.is_empty() {
                view::runners::default_family().to_string()
            } else {
                state.releases_family.clone()
            };
            view::runners::update(state, &Message::FetchReleases { family })
                .unwrap_or_else(cosmic::app::Task::none)
        }
        _ => cosmic::app::Task::none(),
    }
}

impl Shell {
    /// A shell over a real library, settings and runner manager, the way
    /// [`App::init`] builds one — but with no window, so a test can have one.
    #[cfg(test)]
    fn new() -> Self {
        let mut state = State::new(
            Library::new(None),
            Settings::load(None),
            RunnerManager::new(&SystemLaunchEnv),
        );
        // The Plugins page reads rows that only `refreshPlugins` fills, so the
        // shell primes them here rather than rendering an empty list first —
        // `bridge.py` gets the same non-empty answer because its `plugins`
        // Property is evaluated on first read.
        state.refresh_plugins(&gamehandler_core::plugins::SystemPluginEnv);
        Self {
            state,
            nav_model: build_nav_model(),
        }
    }

    /// The one place the current page changes.
    ///
    /// Both halves move together: `state.page`, which the pages read, and the
    /// sidebar's selection, which the framework draws. A write to `state.page`
    /// anywhere else is how these two drift — a sidebar showing one page while
    /// the body shows another, which no test of either half alone would catch.
    ///
    /// # The invariant is checked, not merely written down
    ///
    /// That paragraph used to be the whole of the guarantee, and it was not one.
    /// Nothing in the framework closes this: `Action::NavBar` reaches
    /// [`cosmic::Application::on_nav_select`], and the nav bar widget only
    /// *publishes* a selection — it never calls `model.activate` itself — so the
    /// model's selection is whatever this function last set it to. And
    /// `State::page` is a public field, so any handler may assign it directly,
    /// compile, leave the panel showing the previous page, and fail nothing.
    ///
    /// Two structures close it, and neither is a comment:
    ///
    /// - the `debug_assert!` below, which fires when this function fails to move
    ///   the sidebar — `activate_page` answering `false` because the model does
    ///   not carry the page, i.e. `build_nav_model` and [`Page::ALL`] having
    ///   diverged. It is deliberately *not* able to see a handler that assigned
    ///   `state.page` directly: this function writes both records before
    ///   checking them, so a later call repairs such a write before the check
    ///   runs. That is why the second structure exists.
    /// - `no_handler_leaves_the_sidebar_out_of_step`, which drives every message
    ///   through [`Shell::update`] and checks the two agree afterwards, so a
    ///   handler that writes `state.page` directly fails a test rather than
    ///   confusing a user.
    ///
    /// Making `State::page` private with a setter would close it once more, and
    /// is a T-09 decision rather than a T-08 fix: T-09 is where handlers start
    /// setting pages from page content, and the right shape depends on how many
    /// of them there turn out to be. Until then the field stays public and the
    /// two checks above are what make the invariant real.
    fn show_page(&mut self, page: Page) -> cosmic::app::Task<Message> {
        // Compared **before** the assignment, so the page-entry work runs when
        // the user arrives at a page rather than every time something asks for
        // it. D-48 names this as the trigger; a re-navigation to the page
        // already showing must not redo it, which is what the comparison is
        // for. The two records below are still written unconditionally: they
        // must agree after *every* call, including the one that changes
        // nothing, and `no_handler_leaves_the_sidebar_out_of_step` is what
        // holds that.
        let arriving = self.state.page != page;
        self.state.page = page;
        activate_page(&mut self.nav_model, page);
        debug_assert!(
            self.pages_agree(),
            "the sidebar shows {:?} while `state.page` is {:?}: `show_page` is \
             the only writer of the pair, and this fires when something else \
             assigned `state.page` directly",
            self.nav_model.active_data::<Page>(),
            self.state.page,
        );
        // After the check, so an inconsistent pair panics before the entry work
        // has had a chance to write state of its own.
        if arriving {
            page_entry_task(&mut self.state, page)
        } else {
            cosmic::task::none()
        }
    }

    /// The body under the sidebar, for whichever page is showing.
    ///
    /// This is the page dispatch: one arm per [`Page`], and the arms that call
    /// `pending_page` are the pages that have not been ported. Two things about
    /// its shape are load-bearing and neither is cosmetic.
    ///
    /// # It is a method on `Shell`, not a free function and not a method on `App`
    ///
    /// - Not on [`App`], because `App` cannot be built without a display
    ///   ([`cosmic::Core`] is only ever constructed by the framework), so a
    ///   method there could not be called from a test at all. Everything the
    ///   body reads is in `State`, and `Shell` is a `State` with a sidebar.
    /// - Not a free function over `&State`, because
    ///   `crates/app/tests/pending_pages.rs` — Packaging's pin on the set of
    ///   unported pages — parses this dispatch out of the source text and is
    ///   anchored on `match self.state.page`. A free function's `match
    ///   state.page` is a shape it does not know, and it fails loudly rather
    ///   than asserting over an empty list (which is its own guard working as
    ///   designed). Renaming the parameter to keep the anchor would be a lie
    ///   told to a parser; the method is the honest form of it.
    ///
    /// # The arms are written out
    ///
    /// There is no wildcard arm, so adding a [`Page`] is a compile error until
    /// it is given a body. Each unported arm names the task that will build it,
    /// on screen as well as in the code, so a page that has not landed says so.
    fn view_body(&self) -> cosmic::Element<'_, Message> {
        match self.state.page {
            Page::Library => {
                let page = view::library::LibraryPage {
                    library: &self.state.library,
                    search: &self.state.search_text,
                    category: &self.state.category_filter,
                    sort_mode: &self.state.settings.sort_mode,
                    view_mode: &self.state.settings.view_mode,
                    runners: &self.state.runners,
                };
                view::library::view(page)
            }
            // TODO(T-12): the release list and the install progress.
            Page::Installers => pending_page(Page::Installers, "T-12"),
            // T-11/T-12. Both bundles are borrowed from `State` rather than
            // built here: building the installed rows spawns `wine --version`,
            // and this runs once per frame. [`view::runners::refresh`] is the
            // only writer and says why.
            Page::Runners => {
                let page = view::runners::RunnersView {
                    installed: &self.state.installed,
                    selected_family: &self.state.releases_family,
                    status: &self.state.releases_status,
                    releases: &self.state.release_rows,
                    progress: self.state.progress,
                };
                view::runners::view(page)
            }
            Page::Plugins => {
                let page = view::plugins::PluginsPage {
                    intro: &self.state.plugins_intro,
                    rows: &self.state.plugins,
                };
                view::plugins::view(page)
            }
            // T-13. `view::credits::view` takes no `&State`: the page is a
            // reading surface over `core::credits`, and its argument is a marker
            // rather than a borrow so that the dispatch arm reads as the others
            // do without pretending the page has something to read. The
            // catalogue itself is reached through that module's accessors, never
            // transcribed here.
            Page::Credits => view::credits::view(view::credits::CreditsPage),
            Page::Settings => {
                let page = view::settings::SettingsPage {
                    settings: &self.state.settings,
                    runners: &self.state.runners,
                };
                view::settings::view(page)
            }
        }
    }

    /// Whether the two records of the current page agree.
    fn pages_agree(&self) -> bool {
        self.nav_model.active_data::<Page>().copied() == Some(self.state.page)
    }

    /// The page the sidebar draws as selected.
    #[cfg(test)]
    fn sidebar_page(&self) -> Option<Page> {
        self.nav_model.active_data::<Page>().copied()
    }

    /// Handle one message.
    ///
    /// # Why this is on `Shell` and not on `App`
    ///
    /// A handler whose whole effect is on [`State`] does not need a window, and
    /// `App` cannot exist without one. Putting the dispatcher here is what makes
    /// the handlers testable: `Shell::new()` builds one with no `Core`, so a
    /// test calls the same match the running application calls. The previously
    /// untested arms — all five of them — were untested only because the test
    /// would have had to construct an `App`.
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
    /// does not mean the message is a no-op by design. The ones that carry a
    /// real body are:
    ///
    /// - the shell, from T-08: [`Message::NavigateTo`], [`Message::CloseDialog`],
    ///   [`Message::DismissToast`], [`Message::Notify`] and [`Message::Quit`] —
    ///   navigation, the two ways a dialog closes, the toaster the whole app
    ///   reports through, and quitting;
    /// - the Library toolbar, from T-09: [`Message::SetSearchText`],
    ///   [`Message::SetCategoryFilter`], [`Message::ClearFilters`],
    ///   [`Message::SetViewMode`] and [`Message::SetSortMode`] — the search box,
    ///   the category filter, the button that clears both, and the two settings
    ///   the toolbar's selectors write;
    /// - the Settings page, from T-13: [`Message::SetColorScheme`],
    ///   [`Message::SetDefaultRunner`], [`Message::SetCloseOnLaunch`] and
    ///   [`Message::SetDefaultToggle`] — the four controls the page's two
    ///   selectors, two switches and thirteen default toggles write through;
    /// - the Plugins page, from T-26: [`Message::RefreshPlugins`],
    ///   [`Message::InstallPlugin`] and [`Message::PluginInstallFinished`] —
    ///   the refresh, the button that runs the reference's install command, and
    ///   the outcome it reports back.
    ///
    /// `ClearFilters` is one of them rather than two writes at the call site so
    /// that the search box and the category can never be observed cleared one
    /// without the other.
    ///
    /// Each of the four from T-13 carries a guard the reference has and this
    /// port did not: an out-of-set colour scheme is dropped
    /// (`bridge.py:197-199`), an empty default runner is dropped (`:235-238`),
    /// and both the switch and the toggles are written only when the value
    /// actually differs (`:247-250`, `:264-269`). Those guards are why the
    /// samples in `every_message` carry **non-default** values: with a sample
    /// that writes what is already there, a correct guarded handler and an
    /// unwritten arm produce the same silence — the D-34 shape, in the samples
    /// rather than in the handler.
    ///
    /// That count is not a comment. `only_the_written_handlers_change_anything`
    /// drives every message in `every_message` through this function and
    /// requires the set that has any effect to be exactly those fifteen (plus
    /// `Quit`, which needs the window and so is `App::update`'s one arm, and
    /// `DismissToast`, whose arm is real but unobservable). A handler that
    /// regresses to `{}` shrinks that set and fails; a new handler landing grows
    /// it and fails until it is added deliberately. The earlier version of this
    /// paragraph said "three" and named three, omitting `CloseDialog` and
    /// `Notify` — in the sentence a reviewer trusts to know what is live.
    fn update(&mut self, message: Message) -> cosmic::app::Task<Message> {
        match message {
            // ---- Navigation and dialogs -----------------------------------
            // The route used by everything that is not the sidebar itself — a
            // page's own buttons, the credits link, a shortcut. It goes through
            // `go_to` so the sidebar's selection moves with it; see that
            // function for why the two must not be written separately.
            Message::NavigateTo(page) => {
                return self.show_page(page);
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
            // `Quit` is handled by [`App::update`] rather than here: it closes
            // the window, which is the framework's, and this shell has none. It
            // is matched there rather than here so this function stays a
            // function of `State` alone — see [`Shell`].
            Message::Quit => {}

            // ---- Settings --------------------------------------------------
            // `_set_color_scheme` (`bridge.py:197-203`) ignores a value outside
            // `COLOR_SCHEMES` rather than storing it, and the load path already
            // folds one (`settings.rs:89-91`) — so the message path was the
            // missing half, exactly as for the two below.
            //
            // **What this does not do is the other half of that setter**: the
            // reference then calls `self._theme.apply(value)`, and this port has
            // no theme to apply it to (`Shell` has no `theme()` override, so
            // `cosmic::Theme` follows the system and nothing reads
            // `settings.color_scheme`). Storing the choice is the whole of the
            // behaviour here, and it is stored so the setting survives a restart
            // and so the page's selector round-trips. See D-13 (`PLAN.md:479`),
            // which names T-13 as where the default is decided; the decision is
            // recorded in DECISIONS rather than closed silently here.
            Message::SetColorScheme(value) => {
                if COLOR_SCHEMES.contains(&value.as_str()) {
                    self.state.settings.color_scheme = value;
                }
            }
            // `_set_view_mode` (`bridge.py:210-214`) and `_set_sort_mode`
            // (`223-228`) both *ignore* a value outside the allowed set rather
            // than storing it. The load path already does this
            // (`settings.rs:148-153`); the message path did not, so a stale UI
            // could write a mode the code does not handle and the next start
            // would silently fold it back — the same defect D-34 names, on the
            // other side of the file.
            Message::SetViewMode(value) => {
                if VIEW_MODES.contains(&value.as_str()) {
                    self.state.settings.view_mode = value;
                }
            }
            Message::SetSortMode(value) => {
                if SORT_MODES.contains(&value.as_str()) {
                    self.state.settings.sort_mode = value;
                }
            }
            // `_set_default_runner` (`bridge.py:235-238`): an **empty** value is
            // ignored, and a value equal to what is stored is not written. The
            // emptiness check is the reference's own guard and not a stand-in for
            // "the runner exists" — the reference does not check that either, and
            // the selector's own fallback (`SettingsPage.qml:83-94`) is what
            // keeps a stored runner that is no longer installed selectable.
            Message::SetDefaultRunner(value) => {
                if !value.is_empty() && value != self.state.settings.default_runner {
                    self.state.settings.default_runner = value;
                }
            }
            // `_set_close_on_launch` (`bridge.py:247-250`): `bool(value)` then a
            // compare, so writing the stored value is a no-op rather than a
            // redundant save.
            Message::SetCloseOnLaunch(value) => {
                if value != self.state.settings.close_on_launch {
                    self.state.settings.close_on_launch = value;
                }
            }
            // `setDefaultToggle` (`bridge.py:264-269`): the name must be one of
            // the defaulted toggles, and the value must differ — both checked in
            // one place, `view::settings::set_toggle`, so the page's read and the
            // message path's write cannot disagree about which field a key means.
            // An unknown name is ignored, which is the reference's behaviour and
            // the reason `set_toggle` returns a `bool` instead of asserting.
            Message::SetDefaultToggle { name, value } => {
                view::settings::set_toggle(&mut self.state.settings, &name, value);
            }

            // ---- Library view state ----------------------------------------
            Message::SetSearchText(text) => {
                self.state.search_text = text;
            }
            // `_set_category_filter` (`bridge.py:290-293`): empty folds to
            // "All", so the stored value is never the empty string.
            Message::SetCategoryFilter(filter) => {
                self.state.category_filter = if filter.is_empty() {
                    view::library::ALL_CATEGORIES.to_string()
                } else {
                    filter
                };
            }
            // Both halves, in one handler, so neither can be observed alone.
            Message::ClearFilters => {
                self.state.search_text.clear();
                self.state.category_filter = view::library::ALL_CATEGORIES.to_string();
            }

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
            // T-11's messages, delegated rather than written here: the page
            // owns them, and `view::runners::update` is where their transitions
            // and their tasks live. It answers `None` for a message that is not
            // the page's, which no arm below can be — so the `unwrap_or_else` is
            // unreachable by construction and says so rather than panicking.
            //
            // `RunnersRefreshed` is in the list even though no widget sends it:
            // it is the page's reply to itself, and routing it here rather than
            // handling it above is what keeps one page's state and one page's
            // transitions in one file.
            message
                @ (Message::FetchReleases { .. }
                | Message::ReleasesFetchFinished { .. }
                | Message::InstallRunner { .. }
                | Message::RunnerProgress(_)
                | Message::RunnerInstallFinished { .. }
                | Message::UninstallRunner(_)
                | Message::RunnersRefreshed { .. }) => {
                return view::runners::update(&mut self.state, &message)
                    .unwrap_or_else(cosmic::app::Task::none);
            }

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
            // `refreshPlugins()` — re-read the host and rebuild the rows.
            Message::RefreshPlugins => {
                self.state
                    .refresh_plugins(&gamehandler_core::plugins::SystemPluginEnv);
            }
            // `installPlugin()` (`bridge.py:1006-1020`). The notice is pushed
            // before the work starts, as the reference does, so the user has
            // something on screen while a package manager prompts for a
            // password; an id that does not resolve is a silent no-op rather
            // than a notice about a helper that does not exist.
            Message::InstallPlugin(plugin_id) => {
                if let Some((notice, task)) = view::plugins::install_plan(&plugin_id) {
                    let toast = self
                        .state
                        .toasts
                        .push(cosmic::widget::toaster::Toast::new(notice))
                        .map(cosmic::Action::App);
                    return cosmic::app::Task::batch([toast, task]);
                }
            }
            // The install's `done`/`fail` half (`bridge.py:1012-1024`). The
            // rows are rebuilt first because `pluginsChanged.emit()` is the
            // signal the page redraws from — reporting before refreshing would
            // toast an outcome beside a button that still said "Install".
            Message::PluginInstallFinished { plugin_id, result } => {
                self.state
                    .refresh_plugins(&gamehandler_core::plugins::SystemPluginEnv);
                let name = gamehandler_core::plugins::plugin_by_id(&plugin_id)
                    .map(|plugin| plugin.name)
                    .unwrap_or(plugin_id.as_str());
                let text = match result {
                    Ok(true) => view::plugins::installed_message(name),
                    Ok(false) => view::plugins::not_installed_message(name),
                    Err(error) => view::plugins::install_failed_message(name, &error),
                };
                return self
                    .state
                    .toasts
                    .push(cosmic::widget::toaster::Toast::new(text))
                    .map(cosmic::Action::App);
            }

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
}

/// The pages whose body is still a placeholder: the task that will build each.
///
/// # Why this exists
///
/// T-08 is the shell: the sidebar, the routing and the toaster. Each page's own
/// content is a later task, and until it lands that page's body says so **on
/// screen** as well as in the code — a page that rendered an empty body would
/// read as a bug in the shell, which is the wrong thing to go looking for.
///
/// The danger in a placeholder is not that it is wrong but that it is
/// **invisible**: it compiles, it renders, and it satisfies every test that
/// checks the process came up. `gui-stays-up` in `scripts/smoke-test.sh` asserts
/// exactly that and nothing more, so a build whose whole interface is six
/// placeholders passes the gate suite. This list is what makes the placeholders
/// countable, and `the_pending_pages_are_exactly_the_ones_whose_body_says_so`
/// is what counts them — against the rendered body, not against this list.
///
/// # Landing a page is one deletion
///
/// This is a slice, not a `[(Page, &str); N]`, and that is deliberate: a
/// fixed-length array would make removing a line a two-part edit — delete the
/// line, then correct the length — and the second part is mechanical. A
/// mechanical edit forced by a failure message that says the count is pinned
/// "deliberately" is how a gate stops meaning anything: the reader learns to
/// clear red by editing a constant. With a slice, the one edit is the one that
/// carries the meaning, and the test above is what holds the other half — it
/// reads what `view_body` actually draws.
///
/// **T-19's acceptance is that this is empty**, at which point the placeholder
/// function goes with the last entry.
#[cfg(test)]
const PENDING_PAGES: &[(Page, &str)] = &[
    (Page::Installers, "T-12"),
];

/// The task that will build `page`, or `None` once its body has landed.
#[cfg(test)]
fn pending_task(page: Page) -> Option<&'static str> {
    PENDING_PAGES
        .iter()
        .find(|(pending, _)| *pending == page)
        .map(|(_, task)| *task)
}

/// The body of a page that has not been ported yet.
///
/// The task is named on screen as well as in the code. See `PENDING_PAGES`,
/// which is `cfg(test)`-gated and so cannot be linked from here.
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

/// What one message does to a shell, as far as a test can see it.
///
/// Two observations, not one, because an arm can do its work in either place:
///
/// - `state`, read by diffing `Debug` before and after the call. [`State`] and
///   everything under it derive `Debug`, so this sees every field a handler
///   writes and nothing else — it is coarse, but it cannot be gamed by a
///   handler that writes a field no test inspects;
/// - `task_units`, the size of the [`cosmic::Task`] the arm returned, via
///   iced's public [`Task::units`]. `Task::none()` is zero units, so this is
///   what tells a real `Task` from an empty arm — and it is why a handler whose
///   whole effect is the task it returns (`Message::X(_) => some_task()`, which
///   is the shape `FetchCover` will take) is not silently counted as a
///   placeholder. A `let _ = …` on the returned task would have hidden exactly
///   that.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct Effect {
    /// The state was different afterwards.
    state_changed: bool,
    /// How many units of work the arm asked the runtime for; `0` for
    /// `Task::none()`.
    task_units: usize,
}

/// Run `message` through the real dispatcher and report what it did.
///
/// This is the whole instrument behind the handler tests: it calls
/// [`Shell::update`] — the same match the running application calls — rather
/// than inspecting the source, so a mutation that empties an arm is observed
/// as a change in behaviour rather than as a change in text.
#[cfg(test)]
fn observe(shell: &mut Shell, message: Message) -> Effect {
    let before = format!("{:?}", shell.state);
    let task = shell.update(message);
    Effect {
        state_changed: format!("{:?}", shell.state) != before,
        task_units: task.units(),
    }
}

/// Did `message` do anything at all?
///
/// "Anything" is deliberately the union of both observations: a handler that
/// only speaks to the runtime counts, even though it writes no state, because
/// that is a written handler and misreporting it as a placeholder is the
/// inverse of the failure these tests exist to catch.
#[cfg(test)]
fn is_handled(shell: &mut Shell, message: Message) -> bool {
    let effect = observe(shell, message);
    effect.state_changed || effect.task_units > 0
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
        let mut state = State::new(library, settings, runners);
        // Same priming as `Shell::new`: the Plugins page's rows are a cache the
        // host refresh fills, not something the constructor can know.
        state.refresh_plugins(&gamehandler_core::plugins::SystemPluginEnv);
        let mut app = App {
            core,
            shell: Shell {
                state,
                nav_model: build_nav_model(),
            },
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
        Some(&self.shell.nav_model)
    }

    /// A sidebar row was clicked.
    ///
    /// The page comes out of the *row's own data* rather than from its
    /// position: positions are an implementation detail of the model, and a row
    /// inserted in the wrong place would then select the wrong page silently.
    fn on_nav_select(&mut self, id: nav_bar::Id) -> cosmic::app::Task<Self::Message> {
        if let Some(page) = self.shell.nav_model.data::<Page>(id).copied() {
            return self.shell.show_page(page);
        }
        cosmic::task::none()
    }

    /// Handle one message.
    ///
    /// Two things happen here and nothing else: `Quit` is answered with the
    /// window the framework owns, and everything else is delegated to
    /// [`Shell::update`]. The delegation is not a convenience — it is what makes
    /// the handlers testable, since [`App`] cannot be built off a display.
    ///
    /// Adding a variant to [`Message`] is still a compile error until it is
    /// listed, because [`Shell::update`]'s match is exhaustive and has no
    /// wildcard arm; the guarantee is unchanged, it has only moved.
    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        // The one handler that needs the framework: `Core` tracks the main
        // window id, and iced's `window::close` is the supported way to end it —
        // the same path the window manager's own close button takes, so no
        // `std::process::exit` shortcut.
        if matches!(&message, Message::Quit) {
            return match self.core.main_window_id() {
                Some(id) => cosmic::iced::window::close(id),
                None => cosmic::task::none(),
            };
        }
        self.shell.update(message)
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
        // Every failure path in `bridge.py` ends in the `notify` signal, so the
        // toaster is wrapped around the whole body rather than placed inside a
        // page: a toast raised by one page must survive a navigation to another,
        // and `Toasts` lives in `State` for exactly that reason. The body itself
        // is [`view_body`], which a test can call without an `App`.
        toaster(&self.shell.state.toasts, self.shell.view_body())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamehandler_core::models::Game;

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

    /// A library at a temp path, populated with `(id, name)` games.
    ///
    /// Built through `Library::add`, which saves, so the games are read back
    /// from a real file rather than injected — a listing that worked only on an
    /// in-memory library would pass here and print nothing for a user.
    fn library_with(label: &str, games: &[(&str, &str)]) -> (std::path::PathBuf, Library) {
        let root = std::env::temp_dir().join(format!(
            "gh-cli-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("games.json");
        let mut library = Library::new_at(Some(path), 0.0);
        for (id, name) in games {
            let mut game = Game::new_named(*name);
            game.id = (*id).to_string();
            library.add(game).unwrap();
        }
        (root, library)
    }

    /// `--list` prints one `id\tname` row per game, in the case-insensitive
    /// name order Python's default sort produces.
    #[test]
    fn list_lines_are_the_ids_and_names_in_name_order() {
        let (_root, library) = library_with(
            "rows",
            &[("b", "Beta"), ("a", "Alpha"), ("c", "gamma")],
        );
        assert_eq!(
            list_lines(&library),
            vec!["a\tAlpha", "b\tBeta", "c\tgamma"]
        );
    }

    /// The sort is on the **lowercased** name, which is the one place a byte
    /// divergence between the two implementations could hide.
    ///
    /// The pair is chosen to discriminate: a plain `str` sort is by code point,
    /// where `Z` (0x5A) precedes `a` (0x61), so `Zebra` would come first. A
    /// case-insensitive sort puts `apple` first. Only one of the two orders can
    /// be produced by a given implementation, so this cannot pass vacuously.
    #[test]
    fn the_listing_sorts_case_insensitively_like_python_s_default() {
        let (_root, library) = library_with(
            "case",
            &[("z", "Zebra"), ("a", "apple"), ("m", "Mango")],
        );
        assert_eq!(
            list_lines(&library),
            vec!["a\tapple", "m\tMango", "z\tZebra"],
            "lowercased order, not code-point order"
        );
    }

    /// An empty library says so once, and naming the file is not the point —
    /// the message is Python's, verbatim, because scripts parse it.
    #[test]
    fn an_empty_library_prints_the_empty_line_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("gh-cli-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // A path with no file behind it: the missing-file case, which is what a
        // first run looks like.
        let library = Library::new_at(Some(root.join("games.json")), 0.0);
        assert_eq!(
            list_lines(&library),
            vec!["GameHandler: the library is empty"]
        );
    }

    /// The regression this whole change exists for: a **populated** library
    /// must not print the empty line.
    ///
    /// The earlier stub printed it unconditionally, so this is the assertion
    /// that would have failed it. Written against a real two-game library
    /// because the failure it guards was only reachable on one.
    #[test]
    fn a_populated_library_does_not_claim_to_be_empty() {
        let (_root, library) = library_with("populated", &[("a", "Alpha"), ("b", "Beta")]);
        let lines = list_lines(&library);
        assert_eq!(lines.len(), 2);
        assert!(
            !lines.iter().any(|line| line.contains("empty")),
            "a library with games in it reported emptiness: {lines:?}"
        );
    }

    /// An id that is not in the library is refused with Python's message.
    #[test]
    fn launching_an_unknown_id_is_refused_with_pythons_message() {
        let (_root, library) = library_with("unknown", &[("a", "Alpha")]);
        let (message, code) = launch_failure(&library, "nope");
        assert_eq!(message, "GameHandler: no game with id nope");
        assert_eq!(code, 1);
    }

    /// A game that **is** in the library is not reported as missing.
    ///
    /// This is the inverted-criterion bug again, one level down: the old stub
    /// printed the unknown-id message for every id, including real ones, which
    /// is a fabricated fact about the user's library. The message must say the
    /// launch is unavailable, not that the game does not exist.
    #[test]
    fn launching_a_known_id_does_not_claim_the_game_is_missing() {
        let (_root, library) = library_with("known", &[("a", "Alpha")]);
        let (message, code) = launch_failure(&library, "a");
        assert_eq!(code, 1, "an unported launch must not exit 0");
        assert!(
            !message.contains("no game with id"),
            "a real game was reported as missing: {message}"
        );
        assert!(
            message.contains("Alpha"),
            "the message should name the game: {message}"
        );
        assert!(
            message.contains("not implemented"),
            "the message must admit the launch did not happen: {message}"
        );
    }

    /// Every path out of `--launch` is non-zero while the launch is unported.
    ///
    /// Stated as a property over both branches rather than per-branch, because
    /// the failure mode this guards is a later change adding a `return
    /// ExitCode::SUCCESS` for a game it did not start.
    #[test]
    fn no_unported_launch_path_reports_success() {
        let (_root, library) = library_with("status", &[("a", "Alpha"), ("b", "Beta")]);
        for game_id in ["a", "b", "missing"] {
            let (_, code) = launch_failure(&library, game_id);
            assert_ne!(code, 0, "{game_id} reported success without launching");
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
    /// `winit/src/platform_impl/linux/mod.rs:89-95`.
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
    /// (`winit/src/platform_impl/linux/mod.rs:89-95`). `DISPLAY=""` is not
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
    ///
    /// This is a test of the *words*, and on its own it proves nothing about
    /// behaviour: `no_display_hint` is only reachable through
    /// [`display_refusal`], and
    /// `a_displayless_start_is_refused_with_the_diagnostic_on_stderr` is what
    /// checks the words reach stderr.
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

    /// **The displayless path writes the diagnostic and refuses.**
    ///
    /// This is the half the string tests cannot reach. `no_display_hint` is a
    /// pure function of nothing, so a test of its output says only that the
    /// sentence exists — it was satisfied by a version where the two
    /// `eprintln!`s that print it had been deleted, which is the entire
    /// user-visible part of N-01.
    ///
    /// Here the writer is a `Vec<u8>`, so the bytes are the assertion: the
    /// refusal line, the hint, and no output at all when a display is present.
    #[test]
    fn a_displayless_start_is_refused_with_the_diagnostic_on_stderr() {
        let mut printed = Vec::new();
        let refused = display_refusal(|_| None, &mut printed);
        let printed = String::from_utf8(printed).expect("the diagnostic is UTF-8");

        assert!(refused, "no display must refuse to start");
        assert!(
            printed.contains(APP_NAME) && printed.contains("no display"),
            "the first line must name the program and the problem; got {printed:?}"
        );
        assert!(
            printed.contains("--launch") && printed.contains("--list"),
            "the hint must be printed too, not merely available; got {printed:?}"
        );
        assert_eq!(
            printed.lines().count(),
            no_display_hint().lines().count() + 1,
            "the output is the refusal line plus the whole hint block and              nothing else; got {printed:?}"
        );
    }

    /// **A displayless start never reaches the event loop.**
    ///
    /// This is the behaviour `run_gui`'s doc comment is about: winit's own
    /// detection exists but iced_winit consumes it in an `expect`, so the
    /// process panics with a raw traceback unless this path refuses first. The
    /// starter is a closure here, so "never reached the event loop" is
    /// observed rather than assumed — the closure records that it ran.
    ///
    /// It also pins the wiring that the earlier version of these tests could
    /// not: `run_gui` discarding `start_gui`'s answer and starting anyway is
    /// exactly this assertion failing.
    #[test]
    fn a_displayless_start_never_reaches_the_event_loop() {
        let mut printed = Vec::new();
        let mut started = false;
        let outcome = start_gui(
            |_| None,
            &mut printed,
            || {
                started = true;
                Ok(())
            },
        );

        assert_eq!(outcome, GuiStart::NoDisplay);
        assert!(
            !started,
            "the starter must not be called without a display: on this path the \
             framework's own error handling panics rather than returning, so \
             the guard is the only thing standing between the user and a \
             traceback"
        );
        assert!(
            String::from_utf8_lossy(&printed).contains("--launch"),
            "and the refusal must say what to do instead; got {:?}",
            String::from_utf8_lossy(&printed)
        );
    }

    /// **A start that fails says why.**
    ///
    /// The `Err` arm of the run had no test before this one, so a failure to
    /// open a window — a broken GPU driver, a missing Wayland socket that the
    /// guard let through — printed nothing at all and exited 1.
    #[test]
    fn a_start_that_fails_reports_the_reason() {
        let mut printed = Vec::new();
        let outcome = start_gui(
            |name| (name == "WAYLAND_DISPLAY").then(|| "wayland-0".to_string()),
            &mut printed,
            || Err("no DRM device".to_string()),
        );

        assert_eq!(outcome, GuiStart::Failed("no DRM device".to_string()));
        let printed = String::from_utf8_lossy(&printed);
        assert!(
            printed.contains("no DRM device"),
            "the framework's reason must reach the user, not only the caller's \
             exit code; got {printed:?}"
        );
        assert!(printed.contains(APP_NAME), "got {printed:?}");
    }

    /// **Only a start that actually showed the interface exits zero.**
    ///
    /// The status is what a script or a `.desktop` shortcut branches on, and it
    /// is the one part of `run_gui` no other test can reach: `ExitCode` is
    /// opaque, so the decision is taken as a `u8` here and converted with
    /// `ExitCode::from` at the one place that needs the type.
    #[test]
    fn only_a_start_that_ran_exits_zero() {
        assert_eq!(GuiStart::Ran.exit_status(), 0);
        assert_eq!(
            GuiStart::NoDisplay.exit_status(),
            1,
            "a refused start must be a failure: a shortcut that treats it as \
             success reports the game as launched"
        );
        assert_eq!(GuiStart::Failed("no DRM device".into()).exit_status(), 1);
    }

    /// A start that succeeds writes nothing and says so.
    ///
    /// The guard for the two above: an implementation that wrote the failure
    /// line unconditionally would pass both of them.
    #[test]
    fn a_start_that_succeeds_writes_nothing() {
        let mut printed = Vec::new();
        let outcome = start_gui(
            |name| (name == "DISPLAY").then(|| ":0".to_string()),
            &mut printed,
            || Ok(()),
        );

        assert_eq!(outcome, GuiStart::Ran);
        assert!(
            printed.is_empty(),
            "a session that opens must not print a diagnostic; got {:?}",
            String::from_utf8_lossy(&printed)
        );
    }

    /// **A display that is present writes nothing.**
    ///
    /// The vacuity guard for the test above: a `display_refusal` that always
    /// printed and always refused would satisfy every assertion there, and would
    /// break every graphical session.
    #[test]
    fn a_present_display_is_not_refused_and_says_nothing() {
        let mut printed = Vec::new();
        let refused = display_refusal(
            |name| (name == "WAYLAND_DISPLAY").then(|| "wayland-0".to_string()),
            &mut printed,
        );

        assert!(!refused, "a Wayland session must start");
        assert!(
            printed.is_empty(),
            "nothing may be written when the interface is going to open; got {:?}",
            String::from_utf8_lossy(&printed)
        );
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


    /// Every message the application can receive, once each.
    ///
    /// A function rather than a `const` because some payloads have no literal
    /// form. It is written out variant by variant rather than derived from the
    /// enum, because Rust cannot enumerate a type's variants.
    ///
    /// # How this stays in step with [`Message`]
    ///
    /// Nothing forces a variant into this list — that is not achievable, and
    /// pretending otherwise would be worse than saying so. What is achievable is
    /// making the omission *fail*:
    ///
    /// - `Shell::update`'s match and [`variant_name`] below are both exhaustive
    ///   with no wildcard arm, so adding a variant to `Message` is a compile
    ///   error in two places that both have to be opened;
    /// - [`every_variant_of_message_is_in_the_list`] pins this list's length, so
    ///   once the enum compiles again that test is red until the new variant is
    ///   also driven here.
    ///
    /// The order is the enum's own declaration order, so the two are read side
    /// by side.
    /// Declare the sample list and the variant names together, from one input.
    ///
    /// # Why this is a macro and not two lists
    ///
    /// `every_message` and `variant_name` were two hand-maintained lists of the
    /// same 49 variants, and the previous version of this comment claimed that
    /// pinning `every_message`'s *length* kept them in step. **That claim was
    /// false, and it was measured false**: adding a variant to the enum, adding
    /// arms to `Shell::update` *and* `variant_name`, and omitting the
    /// `every_message` line left `cargo test` green at 94 passed. The pin was
    /// `assert_eq!(every_message().len(), 49)` against the hand-written list, so
    /// a *missing* entry keeps the count at 49 and only a superfluous one moves
    /// it. It could not see the case its own comment named, and it was inverted
    /// besides: it fired when the list was correctly updated and stayed silent
    /// when the list was stale.
    ///
    /// The failure that hid is the one this whole set of tests exists for. A
    /// variant added to `Message` and left out of `every_message` is a variant
    /// no test drives, and `only_the_written_handlers_change_anything` cannot
    /// notice, because it iterates that same list — the variant is outside its
    /// universe entirely. T-09 to T-15 add variants, so this was about to happen
    /// repeatedly and silently.
    ///
    /// # How the class is removed
    ///
    /// One input, two outputs, both generated. Adding a variant to `Message`
    /// breaks the compile in two places, and the only way to fix either is the
    /// edit that supplies the sample:
    ///
    /// - `Shell::update`'s match is exhaustive with no wildcard arm;
    /// - the match this macro generates is exhaustive with no wildcard arm, so
    ///   it fails against the widened enum, and the line that fixes it is here.
    ///
    /// There is no edit that adds the variant to the enum and leaves the list
    /// stale, because the list is not a separate text. The other direction — a
    /// line here for a variant that does not exist, or the same variant twice —
    /// is an unreachable pattern, which `-D warnings` turns into a failure.
    ///
    /// What that removes with it: `every_variant_of_message_is_in_the_list`, the
    /// length pin. It is deleted rather than repaired, because both of its
    /// checks (nothing missing, nothing doubled) are now compile errors.
    macro_rules! message_variants {
        ($( $variant:pat => ($name:literal, $sample:expr) ),* $(,)?) => {
            /// Every message the application can receive, once each.
            ///
            /// Generated from the same input as [`variant_name`], so the two
            /// cannot drift; see the macro's own doc for why that matters. A
            /// function rather than a `const` because some payloads have no
            /// literal form.
            fn every_message() -> Vec<Message> {
                vec![ $( $sample ),* ]
            }

            /// A message's variant name, without its payload.
            ///
            /// The match is exhaustive and has no wildcard arm, which is what
            /// makes adding a variant to [`Message`] a compile error until a line
            /// is added to the invocation below — the line that also adds the
            /// sample every test in this module iterates.
            fn variant_name(message: &Message) -> &'static str {
                match message {
                    $( $variant => $name ),*
                }
            }

            /// Every sample is the variant its own name says, and no two
            /// entries share a name.
            ///
            /// **The sample is the one field the macro cannot tie down.** The
            /// pattern and the name are structural — the match arm pairs them —
            /// but `$sample` is a free expression, so
            /// `Message::LaunchWatchTick => ("LaunchWatchTick", Message::Quit)`
            /// compiles, covers the right arm, and makes `every_message` drive
            /// `Quit` twice and `LaunchWatchTick` never. Measured: that mutation
            /// survived everything else here, including the assertion below it,
            /// because nothing tied the sample to the line it sits on.
            ///
            /// So this is the tie, and it is generated from the same input
            /// rather than written by hand. It has two halves and both are
            /// load-bearing:
            ///
            /// - each sample's own variant name must be the name recorded beside
            ///   it, which is what a swapped sample fails;
            /// - the names must be distinct, because `changed` and `expected` in
            ///   `only_the_written_handlers_change_anything` are compared as
            ///   *names*: two variants sharing one would let that comparison pass
            ///   while naming a different handler.
            #[test]
            fn every_sample_is_the_variant_its_name_says() {
                let pairs: Vec<(&str, Message)> = vec![ $( ($name, $sample) ),* ];

                for (name, sample) in &pairs {
                    assert_eq!(
                        variant_name(sample),
                        *name,
                        "`every_message` carries a sample whose variant is not \
                         the one this list names beside it"
                    );
                }

                let mut names: Vec<&str> = pairs.iter().map(|(name, _)| *name).collect();
                names.sort_unstable();
                let listed = names.len();
                names.dedup();
                assert_eq!(
                    names.len(),
                    listed,
                    "two entries share a name, so `changed` and `expected` can \
                     compare equal while naming different handlers: {names:?}"
                );
            }
        };
    }

    // The one list: a pattern per variant, its name, and a sample value.
    //
    // In `Message`'s declaration order, so this and the enum are read side by
    // side. Each sample is a value the variant could really carry, because
    // several of these messages are classified by what the handler does with
    // the payload.
    message_variants! {
        Message::NavigateTo(_) => ("NavigateTo", Message::NavigateTo(Page::Settings)),
        Message::OpenNewGameForm => ("OpenNewGameForm", Message::OpenNewGameForm),
        Message::OpenEditGameForm(_) => ("OpenEditGameForm", Message::OpenEditGameForm("g".to_string())),
        Message::CloseDialog => ("CloseDialog", Message::CloseDialog),
        Message::ConfirmDeleteGame(_) => ("ConfirmDeleteGame", Message::ConfirmDeleteGame("g".to_string())),
        Message::DeleteGameConfirmed(_) => ("DeleteGameConfirmed", Message::DeleteGameConfirmed("g".to_string())),
        Message::PickExeFile { .. } => ("PickExeFile", Message::PickExeFile { field: ExeField::Exe }),
        Message::ExeFileChosen { .. } => ("ExeFileChosen", Message::ExeFileChosen {
                            field: ExeField::Prefix,
                            path: Some("/tmp/g.exe".to_string()),
                        }),
        Message::PickCoverFile => ("PickCoverFile", Message::PickCoverFile),
        Message::CoverFileChosen(_) => ("CoverFileChosen", Message::CoverFileChosen(Some("/tmp/c.png".to_string()))),
        Message::DismissToast(_) => ("DismissToast", Message::DismissToast(cosmic::widget::toaster::ToastId::default())),
        Message::Quit => ("Quit", Message::Quit),
        Message::SetColorScheme(_) => ("SetColorScheme", Message::SetColorScheme("light".to_string())),
        // A value *other than the default*: on a default shell `"grid"` and
        // `"name"` write what is already there, which makes a guarded setter
        // and an unwritten one produce the same silence — the defect D-34
        // names, in the samples rather than in the handler.
        Message::SetViewMode(_) => ("SetViewMode", Message::SetViewMode("list".to_string())),
        Message::SetSortMode(_) => ("SetSortMode", Message::SetSortMode("recent".to_string())),
        Message::SetDefaultRunner(_) => ("SetDefaultRunner", Message::SetDefaultRunner("proton-ge".to_string())),
        Message::SetCloseOnLaunch(_) => ("SetCloseOnLaunch", Message::SetCloseOnLaunch(true)),
        Message::SetDefaultToggle { .. } => ("SetDefaultToggle", Message::SetDefaultToggle {
                            name: "mangohud".to_string(),
                            value: true,
                        }),
        Message::SetSearchText(_) => ("SetSearchText", Message::SetSearchText("half".to_string())),
        Message::SetCategoryFilter(_) => ("SetCategoryFilter", Message::SetCategoryFilter("Action".to_string())),
        Message::ClearFilters => ("ClearFilters", Message::ClearFilters),
        Message::SaveGameForm(_) => ("SaveGameForm", Message::SaveGameForm(GameForm::default())),
        Message::LaunchGame(_) => ("LaunchGame", Message::LaunchGame("g".to_string())),
        Message::LaunchWatchFinished { .. } => ("LaunchWatchFinished", Message::LaunchWatchFinished {
                            game_id: "g".to_string(),
                            reason: None,
                        }),
        Message::RunPrefixTool { .. } => ("RunPrefixTool", Message::RunPrefixTool {
                            game_id: "g".to_string(),
                            tool: PrefixTool::WineCfg,
                        }),
        Message::OpenPrefixFolder(_) => ("OpenPrefixFolder", Message::OpenPrefixFolder("g".to_string())),
        Message::CreateDesktopShortcut(_) => ("CreateDesktopShortcut", Message::CreateDesktopShortcut("g".to_string())),
        Message::FetchCover(_) => ("FetchCover", Message::FetchCover("g".to_string())),
        Message::CoverFetchFinished { .. } => ("CoverFetchFinished", Message::CoverFetchFinished {
                            game_id: "g".to_string(),
                            result: Ok(CoverHit::from_steam(
                                0,
                                "Half-Life 2".to_string(),
                                "Action".to_string(),
                                PathBuf::from("/tmp/cover.png"),
                                "https://example.invalid/cover.png".to_string(),
                            )),
                        }),
        Message::FetchCoverForForm { .. } => ("FetchCoverForForm", Message::FetchCoverForForm {
                            token: 1,
                            game_id: "g".to_string(),
                            name: "Half-Life 2".to_string(),
                            exe: "/tmp/g.exe".to_string(),
                        }),
        Message::FormCoverFetchFinished { .. } => ("FormCoverFetchFinished", Message::FormCoverFetchFinished {
                            token: 1,
                            result: Err("lookup failed".to_string()),
                        }),
        Message::FetchReleases { .. } => ("FetchReleases", Message::FetchReleases {
                            family: "proton-ge".to_string(),
                        }),
        Message::ReleasesFetchFinished { .. } => ("ReleasesFetchFinished", Message::ReleasesFetchFinished {
                            family: "proton-ge".to_string(),
                            result: Ok(vec![ReleaseInfo::new("v1.0", "GE-Proton", "https://x/y", 1)]),
                        }),
        Message::InstallRunner { .. } => ("InstallRunner", Message::InstallRunner {
                            tag: "v1.0".to_string(),
                        }),
        Message::RunnerProgress(_) => ("RunnerProgress", Message::RunnerProgress(0.5)),
        Message::RunnerInstallFinished { .. } => ("RunnerInstallFinished", Message::RunnerInstallFinished {
                            tag: "v1.0".to_string(),
                            result: Ok(()),
                        }),
        Message::UninstallRunner(_) => ("UninstallRunner", Message::UninstallRunner("v1.0".to_string())),
        // `token: 0`, which is the token a shell starts with, so the sample is
        // *accepted* by the guard rather than dropped as stale — a sample the
        // handler discards would leave two states equal and report a written arm
        // as unwritten. The rows are non-empty for the same reason: they are what
        // the arm writes, and `only_the_written_handlers_change_anything`
        // observes exactly that.
        Message::RunnersRefreshed { .. } => ("RunnersRefreshed", Message::RunnersRefreshed {
                            token: 0,
                            installed: Vec::new(),
                            release_rows: vec![crate::view::runners::ReleaseRow {
                                tag: "v1.0".to_string(),
                                detail: "Proton-GE · v1.0.tar.gz · 1 MB".to_string(),
                                installed: false,
                            }],
                        }),
        Message::SetInstallerSearch(_) => ("SetInstallerSearch", Message::SetInstallerSearch("steam".to_string())),
        Message::SetInstallerCategory(_) => ("SetInstallerCategory", Message::SetInstallerCategory("launchers".to_string())),
        Message::StartEasyInstall { .. } => ("StartEasyInstall", Message::StartEasyInstall {
                            installer_id: "steam".to_string(),
                            runner_id: "proton-ge".to_string(),
                        }),
        Message::EasyInstallProgress(_) => ("EasyInstallProgress", Message::EasyInstallProgress(0.5)),
        Message::EasyInstallWizardFinished { .. } => ("EasyInstallWizardFinished", Message::EasyInstallWizardFinished {
                            found: Some(PathBuf::from("/tmp/g.exe")),
                            returncode: 0,
                        }),
        Message::CompleteEasyInstall { .. } => ("CompleteEasyInstall", Message::CompleteEasyInstall {
                            token: "t".to_string(),
                            path: Some("/tmp/g.exe".to_string()),
                        }),
        Message::CancelEasyInstall(_) => ("CancelEasyInstall", Message::CancelEasyInstall("t".to_string())),
        Message::EasyInstallFinished { .. } => ("EasyInstallFinished", Message::EasyInstallFinished {
                            game_id: "g".to_string(),
                            message: "installed".to_string(),
                        }),
        Message::RefreshPlugins => ("RefreshPlugins", Message::RefreshPlugins),
        // `"mangohud"` rather than a placeholder string: the reference's
        // `installPlugin` returns silently for an id that does not resolve
        // (`except KeyError: return`, `bridge.py:1007-1008`), so a sample like
        // `"p"` drives the empty branch and would report a working handler as an
        // unwritten arm. Every sample here has to be a value its handler acts
        // on, which is the same rule the search-text and category fixtures
        // state above.
        Message::InstallPlugin(_) => ("InstallPlugin", Message::InstallPlugin("mangohud".to_string())),
        Message::PluginInstallFinished { .. } => ("PluginInstallFinished", Message::PluginInstallFinished {
                            plugin_id: "mangohud".to_string(),
                            result: Ok(true),
                        }),
        Message::Notify(_) => ("Notify", Message::Notify("something happened".to_string())),
        Message::LaunchWatchTick => ("LaunchWatchTick", Message::LaunchWatchTick),
    }

    /// A shell with something for each of the written handlers to act on.
    ///
    /// Built rather than defaulted: `CloseDialog` is a no-op on a shell with no
    /// open form and no pending delete, so a default shell would report it as a
    /// placeholder for the wrong reason.
    fn shell_with_work_to_do() -> Shell {
        let mut shell = Shell::new();
        // Start somewhere other than the page every message navigates to, so
        // `NavigateTo` has an effect to observe.
// The entry task is dropped: these tests are about the two records
        // `show_page` writes, both of which are written before the task is
        // built. Driving it would reach the network.
        let _ = shell.show_page(Page::Library);
        shell.state.game_form = Some(GameForm::default());
        shell.state.confirm_delete = Some("g".to_string());
        // A search and a filter that are *not* the defaults — and, just as
        // importantly, not the *sample* values either. `SetSearchText`'s sample
        // is `"half"` and `SetCategoryFilter`'s is `"Action"`, so a fixture
        // holding either of those makes the corresponding handler write what is
        // already there and vanish from the set below: a handler that works
        // perfectly, reported as an unwritten arm. The fixture has to differ
        // from both the default and the sample for the change to be visible.
        shell.state.search_text = "portal".to_string();
        shell.state.category_filter = "Puzzle".to_string();
        // The Runners page's two guards need something to accept, or a working
        // arm is reported here as unwritten: `ReleasesFetchFinished` is dropped
        // unless the reply's family is the current one, and `InstallRunner`
        // unless the tag is in the list the page is showing.
        shell.state.releases_family = "proton-ge".to_string();
        shell.state.releases = vec![ReleaseInfo::new("v1.0", "GE-Proton", "https://x/y", 1)];
        // The plugin rows are emptied for the same reason and by the same rule:
        // `Shell::new` primes them, so a fixture that left them alone would make
        // `RefreshPlugins` a write of what is already there and hide a working
        // handler behind `observe`'s whole-state comparison. Emptying them is
        // also the honest state for a shell nothing has refreshed yet.
        shell.state.plugins.clear();
        shell.state.plugins_intro.clear();
        shell
    }

    /// **The set of messages that do anything at all is the set of written
    /// handlers, and nothing else.**
    ///
    /// This is the check that was missing. All five real arms of `update` were
    /// replaced with inert bodies and all five survived, because nothing ever
    /// called them: `App` cannot be built without a display, so no test could.
    /// Moving the dispatcher onto [`Shell`] is what makes the arms callable, and
    /// this drives every one of the fifty messages through the real match and
    /// requires the set that has an effect to be exactly the written handlers.
    ///
    /// Both directions are pinned. A handler that regresses to `{}` disappears
    /// from this list; a new handler landing appears in it. Neither can pass
    /// unnoticed, which is what keeps the `T-0x` markers honest.
    ///
    /// # The one blind spot, and the one exclusion
    ///
    /// - **`Quit`** is excluded by name. It is handled by [`App::update`] and
    ///   not by [`Shell::update`], because it closes the framework's window;
    ///   `Shell`'s arm for it is deliberately empty, so counting it here would
    ///   mean counting an empty arm as a handler.
    /// - **`DismissToast`** is *not* excluded, but it *is* invisible: the arm is
    ///   real (`toasts.remove(id)`) and no test can build an id naming a live
    ///   toast, so it cannot be told apart from `{}` — see
    ///   [`a_test_cannot_observe_which_toast_was_dismissed`], which measures
    ///   that rather than asserting it. The list below therefore names fifteen
    ///   handlers where sixteen bodies are written, and says which is which.
    ///
    /// [`observe`] counts a returned [`cosmic::Task`] as well as a state
    /// change, so the *other* class of invisible handler — one whose only
    /// effect is the task it returns, which is the shape T-09's `FetchCover`
    /// will take — is caught here rather than declared away.
    #[test]
    fn only_the_written_handlers_change_anything() {
        let mut changed: Vec<&str> = every_message()
            .into_iter()
            // `Quit` is `App::update`'s arm, so `Shell::update`'s body for it is
            // deliberately empty and it is not part of the claim.
            .filter(|message| !matches!(message, Message::Quit))
            .filter(|message| {
                let mut shell = shell_with_work_to_do();
                is_handled(&mut shell, message.clone())
            })
            .map(|message| variant_name(&message))
            .collect();

        let mut expected: Vec<&str> = vec![
            "NavigateTo",
            "CloseDialog",
            "Notify",
            // T-09's five. They are live because the Library page needs them:
            // the search box, the category filter, the button that clears both
            // at once, and the two settings the toolbar's selectors write.
            "SetSearchText",
            "SetCategoryFilter",
            "ClearFilters",
            "SetViewMode",
            "SetSortMode",
            // T-13's four. Live because the Settings page draws the controls
            // that produce them, and guarded because the reference guards them.
            "SetColorScheme",
            "SetDefaultRunner",
            "SetCloseOnLaunch",
            "SetDefaultToggle",
            // T-26's three. Live because the Plugins page draws the list and
            // offers the install button that produces them.
            "RefreshPlugins",
            "InstallPlugin",
            "PluginInstallFinished",
            // T-11's six. Live because the Runners page draws the controls that
            // produce them: the family selector, its two callbacks, the Install
            // button, the progress reports, and the Remove button.
            //
            // `ReleasesFetchFinished` and `InstallRunner` are in the fixture's
            // reach only because `shell_with_work_to_do` now holds a release
            // list and a `releases_family` — without them the first is dropped
            // by the staleness guard and the second by the tag lookup, both
            // correctly, and both would be reported here as unwritten arms.
            "FetchReleases",
            "ReleasesFetchFinished",
            "InstallRunner",
            "RunnerProgress",
            "RunnerInstallFinished",
            "UninstallRunner",
            // `RunnersRefreshed` is not a seventh page action: nothing draws it
            // and no user sends it. It is the reply `view::runners::refresh`
            // sends itself, so its arm writes the two row bundles and returns no
            // task — which `observe` sees as a state change, and this list
            // therefore has to name. A page whose rows could only arrive from a
            // task *and* whose handler was empty would be a page that never
            // learns what is installed.
            "RunnersRefreshed",
        ];
        // `DismissToast` is written and cannot be observed; see the doc above.
        expected.sort_unstable();
        changed.sort_unstable();

        assert_eq!(
            changed,
            expected,
            "these are the arms of `Shell::update` that do anything, and the \
             list is now checked rather than described. `Quit` is excluded \
             because `App::update` owns it; `DismissToast` is written but \
             unobservable, and `a_test_cannot_observe_which_toast_was_dismissed` \
             is the evidence. The doc comment on `Shell::update` said \"three\" \
             and named three — omitting `CloseDialog` and `Notify` — in the \
             sentence a reviewer trusts to know what is live"
        );
    }

    /// **`DismissToast`'s arm cannot be told apart from an empty one by a
    /// test.**
    ///
    /// The arm is written — `self.state.toasts.remove(id)` — and it is the one
    /// written handler that no test can exercise, because exercising it needs a
    /// [`ToastId`](cosmic::widget::toaster::ToastId) that names a live toast and
    /// libcosmic's `Toasts` exposes no way to obtain one: `push` returns only
    /// the `Task` that schedules the expiry, the slot map and its queue are
    /// private, and the only `ToastId` a test can build is
    /// `Default::default()`, whose key names no slot.
    ///
    /// So this measures the gap rather than asserting it. It is not a test of
    /// the handler; it is the evidence for excluding the handler, written down
    /// where the exclusion is made so that the next reader does not have to
    /// rediscover it. If a future libcosmic gives `Toasts` an accessor, this
    /// test's second assertion goes red and points at the gap to close.
    #[test]
    fn a_test_cannot_observe_which_toast_was_dismissed() {
        // A toast really is live, so the no-op below is not "nothing to remove".
        let mut shell = shell_with_work_to_do();
        let _ = shell.update(Message::Notify("a toast worth dismissing".to_string()));
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 1"),
            "the fixture should have pushed exactly one toast"
        );

        let id = cosmic::widget::toaster::ToastId::default();
        let effect = observe(&mut shell, Message::DismissToast(id));
        assert!(
            !effect.state_changed && effect.task_units == 0,
            "if this ever fails, a `ToastId` a test can construct now names a \
             live toast — which means `Toasts` grew the accessor this test was \
             written without, and `DismissToast` can be covered for real"
        );
    }

    /// **The Library page's body is the Library page, and it is not the
    /// placeholder.**
    ///
    /// Deleting the `PENDING_PAGES` entry is what makes the page "landed" as
    /// far as every guard in this file is concerned — so the deletion is the
    /// claim, and this is the check that the claim is true. It drives
    /// [`Shell::view_body`] rather than `view::library::view`, because the
    /// dispatch arm is the thing under test: a page whose builder is correct
    /// and whose arm still draws a placeholder is exactly the state that
    /// `PINNED_PENDING` counts and cannot see.
    ///
    /// Both directions: the assert on the drawn strings would pass for an empty
    /// body, so `NO_GAMES_TITLE` and the button are required to be *present*,
    /// not merely the placeholder required to be absent.
    #[test]
    fn the_library_page_draws_the_library_and_not_the_placeholder() {
        let shell = Shell::new();
        assert_eq!(shell.state.library.len(), 0, "this fixture is the empty library");
        let drawn = drawn_strings(shell.view_body());

        assert!(
            drawn.iter().any(|text| text == crate::view::library::NO_GAMES_TITLE),
            "an empty library draws the empty state; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == crate::view::library::ADD_FIRST_GAME),
            "and the button out of it; drawn: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text.contains("has not been ported yet")),
            "the placeholder is gone from the dispatch arm; drawn: {drawn:?}"
        );
    }

    /// **A filter that hides every game draws the other empty state.**
    ///
    /// The single assertion that separates the two states, driven through the
    /// real dispatch: a library with games in it and a search matching none
    /// must not say "No games yet". Keyed on the filtered count alone, this is
    /// the case that reports a full library as an empty one.
    #[test]
    fn a_search_that_matches_nothing_is_not_an_empty_library() {
        let root = std::env::temp_dir().join(format!("gh-lib-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut shell = Shell::new();
        shell.state.library = Library::new_at(Some(root.join("games.json")), 0.0);
        shell
            .state
            .library
            .add(gamehandler_core::models::Game::new_named("Celeste"))
            .unwrap();
        shell.state.search_text = "no such game".to_string();

        let drawn = drawn_strings(shell.view_body());
        assert!(
            drawn.iter().any(|text| text == crate::view::library::NO_MATCHES_TITLE),
            "a library with a game in it whose search matches nothing says so; \
             drawn: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text == crate::view::library::NO_GAMES_TITLE),
            "and must not claim the library is empty; drawn: {drawn:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **An out-of-set view mode or sort mode is ignored, not stored.**
    ///
    /// `bridge.py:210-214` and `223-228` both *ignore* an unrecognised value,
    /// and the load path already folds one (`settings.rs:148-153`) — so the
    /// message path was the one place the check was missing, and a stale UI
    /// could write a mode the code does not handle.
    ///
    /// The in-set half is asserted too, because "ignores everything" would
    /// satisfy the first half alone.
    #[test]
    fn an_out_of_set_view_mode_or_sort_mode_is_ignored() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::SetViewMode("nonsense".to_string()));
        assert!(!effect.state_changed, "an unrecognised view mode must not be stored");
        assert_eq!(shell.state.settings.view_mode, "grid");

        let effect = observe(&mut shell, Message::SetViewMode("list".to_string()));
        assert!(effect.state_changed, "a known view mode must be stored");
        assert_eq!(shell.state.settings.view_mode, "list");

        let effect = observe(&mut shell, Message::SetSortMode("nonsense".to_string()));
        assert!(!effect.state_changed, "an unrecognised sort mode must not be stored");
        assert_eq!(shell.state.settings.sort_mode, "name");

        // `"grid"` is a view mode, and must not be accepted as a sort.
        let effect = observe(&mut shell, Message::SetSortMode("grid".to_string()));
        assert!(
            !effect.state_changed,
            "the two allowed sets are not interchangeable"
        );

        let effect = observe(&mut shell, Message::SetSortMode("recent".to_string()));
        assert!(effect.state_changed, "a known sort mode must be stored");
        assert_eq!(shell.state.settings.sort_mode, "recent");
    }

    /// **Clearing the filters empties both, and one message does it.**
    ///
    /// The reference's button writes two things (`LibraryPage.qml:118-121`).
    /// One message is what makes them unobservable apart, so this asserts both
    /// and not the one the fixture happened to set.
    #[test]
    fn clearing_the_filters_empties_both_records_at_once() {
        let mut shell = shell_with_work_to_do();
        assert!(!shell.state.search_text.is_empty(), "the fixture is filtering");
        assert_ne!(shell.state.category_filter, "All", "the fixture is filtering");

        observe(&mut shell, Message::ClearFilters);

        assert!(shell.state.search_text.is_empty());
        assert_eq!(shell.state.category_filter, crate::view::library::ALL_CATEGORIES);
    }

    /// `bridge.py:290-293`: an empty category folds to "All" rather than being
    /// stored as the empty string, which is what makes "no filter" have one
    /// spelling instead of two.
    #[test]
    fn an_empty_category_filter_folds_to_all() {
        let mut shell = shell_with_work_to_do();
        observe(&mut shell, Message::SetCategoryFilter(String::new()));
        assert_eq!(shell.state.category_filter, crate::view::library::ALL_CATEGORIES);
    }

    /// **The Settings page's body is the Settings page, and it is not the
    /// placeholder.**
    ///
    /// The same check T-09 needed, for the same reason: deleting the
    /// `PENDING_PAGES` entry is the claim, and this is what makes the claim
    /// true. Driven through [`Shell::view_body`] rather than
    /// `view::settings::view`, because the dispatch arm is the thing under
    /// test — a correct builder behind a placeholder arm is exactly the state
    /// `PINNED_PENDING` counts and cannot see.
    ///
    /// Both directions again: the strings are required to be *present*, not
    /// merely the placeholder required to be absent, because an empty body
    /// would satisfy the negative alone.
    ///
    /// # What this cannot see, measured rather than assumed
    ///
    /// **The thirteen toggle labels and the close-on-launch explanation are not
    /// in `drawn`.** [`drawn_strings`] collects text through iced's `Operation`
    /// traversal, which observes child `Text` widgets — and `Toggler::draw`
    /// paints its label with a direct `iced_widget::text::draw` call
    /// (`libcosmic src/widget/toggler.rs:316-321`), so there is no child widget
    /// and nothing for the operation to see. Verified by running this with the
    /// explanation asserted: it fails with the explanation absent from a page
    /// that is displaying it.
    ///
    /// That is the same blind spot as #46's failed decode, on a different
    /// widget: the page is right and the instrument is blind. It is written here
    /// because the honest reading is that **fourteen of this page's strings have
    /// no render-level check** — they rest on the pure functions
    /// `the_toggles_are_the_reference_pages_toggles_in_order` and
    /// `the_toggle_label_is_the_references_wording`, which check the table and
    /// the formatting but cannot prove the widget was ever built. What is
    /// asserted below is the set that *is* observable.
    #[test]
    fn the_settings_page_draws_the_settings_and_not_the_placeholder() {
        let mut shell = Shell::new();
// The entry task is dropped: these tests are about the two records
        // `show_page` writes, both of which are written before the task is
        // built. Driving it would reach the network.
        let _ = shell.show_page(Page::Settings);
        let drawn = drawn_strings(shell.view_body());

        for expected in [
            // The four section headings.
            crate::view::settings::SECTION_APPEARANCE,
            crate::view::settings::SECTION_NEW_GAMES,
            crate::view::settings::SECTION_BEHAVIOR,
            crate::view::settings::SECTION_SHORTCUTS,
            // The three form rows, whose labels are `text::body` children and so
            // do reach the operation — unlike the togglers'.
            "Color scheme:",
            "Library layout:",
            "Default runner:",
            // A shortcut row, which is also a `text::body` child.
            "Ctrl+Q:",
        ] {
            assert!(
                drawn.iter().any(|text| text == expected),
                "the Settings page should draw {expected:?}; drawn: {drawn:?}"
            );
        }
        assert!(
            !drawn.iter().any(|text| text.contains("has not been ported yet")),
            "the placeholder is gone from the dispatch arm; drawn: {drawn:?}"
        );
    }

    /// **The Plugins page draws the live catalogue and not the placeholder.**
    ///
    /// The counterpart of the Settings test above, and the one that makes the
    /// wiring a claim rather than a hope. `view::plugins`'s own tests pin the
    /// labels, the button and the four notifications, but nothing in that module
    /// can see whether `view_body` ever calls it — which is exactly finding #61
    /// one layer up: a module whose tests all pass while nothing reaches it.
    /// `Shell::new` primes the rows, so this is what a user sees on first open.
    #[test]
    fn the_plugins_page_draws_the_catalogue_and_not_the_placeholder() {
        let mut shell = Shell::new();
// The entry task is dropped: these tests are about the two records
        // `show_page` writes, both of which are written before the task is
        // built. Driving it would reach the network.
        let _ = shell.show_page(Page::Plugins);
        let drawn = drawn_strings(shell.view_body());

        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::plugins::SECTION_HOST_PLUGINS),
            "the page heading is not drawn; drawn: {drawn:?}"
        );
        // The intro is the sentence `detect_package_manager` decides, so this
        // is also the check that the shell primed it: an unprimed `State` would
        // draw an empty string here and the page would look like a bug.
        assert!(
            drawn.contains(&shell.state.plugins_intro),
            "the intro is not drawn; drawn: {drawn:?}"
        );

        // Every helper in the catalogue, by name, with the button its state
        // offers. A page that drew the heading and nothing else would pass the
        // two assertions above.
        assert_eq!(
            shell.state.plugins.len(),
            gamehandler_core::plugins::plugins().len(),
            "the primed rows must be the whole catalogue"
        );
        for row in &shell.state.plugins {
            assert!(
                drawn.iter().any(|text| text == row.name),
                "{} is in the catalogue but not drawn; drawn: {drawn:?}",
                row.name
            );
        }

        assert!(
            !drawn.iter().any(|text| text.contains("has not been ported yet")),
            "the placeholder is gone from the dispatch arm; drawn: {drawn:?}"
        );
    }

    /// **The About & Credits page draws the reference and not the placeholder.**
    ///
    /// The counterpart of the Plugins test above and the reason landing T-13's
    /// page is a claim rather than a hope. `view::credits`'s own thirteen tests
    /// pin the catalogue, the copy and the links, but nothing inside that module
    /// can see whether [`Shell::view_body`] ever calls it — finding #61 one layer
    /// up, where every unit test passes and no user reaches the code.
    ///
    /// The page takes no `State`, so there is no fixture to prime: what it draws
    /// is the reference data, which is exactly what is asserted. The section
    /// headings are required to be *present* rather than only the placeholder
    /// required to be absent, because an empty body would satisfy the negative
    /// alone.
    ///
    /// `PAGE_TITLE` is deliberately **not** in the list below, and the first
    /// draft of this test had it there and failed. It is the drawer row's label
    /// (`Page::Credits::label()`, `CreditsPage.qml:10`, `Main.qml:94`) and not a
    /// string the body draws: the body opens on `LEAD_HEADING`. `view::credits`
    /// pins that separation from its side — `the_title_is_the_drawers_label_and_
    /// the_qmls` holds the label and the two QML files together, and asserts
    /// `Page::Credits.label() == PAGE_TITLE` — so the fact is checked where it
    /// belongs and is recorded here only so the next reader does not repeat the
    /// mistake.
    #[test]
    fn the_credits_page_draws_the_reference_and_not_the_placeholder() {
        let mut shell = Shell::new();
        let _ = shell.show_page(Page::Credits);
        let drawn = drawn_strings(shell.view_body());

        for expected in [
            crate::view::credits::LEAD_HEADING,
            crate::view::credits::MAKER_LINE,
            crate::view::credits::WHY_HEADING,
            crate::view::credits::GITHUB_LABEL,
        ] {
            assert!(
                drawn.iter().any(|text| text == expected),
                "the Credits page should draw {expected:?}; drawn: {drawn:?}"
            );
        }

        // Every section title, from the data layer rather than from a literal
        // here — so a page that drew the five headings above and stopped would
        // fail on the first section it dropped.
        let sections = crate::view::credits::credit_sections();
        assert!(!sections.is_empty(), "the catalogue is empty; this checks nothing");
        for section in sections {
            assert!(
                drawn.iter().any(|text| text == section.title),
                "section {} is in the catalogue but not drawn; drawn: {drawn:?}",
                section.id
            );
        }

        assert!(
            !drawn.iter().any(|text| text.contains("has not been ported yet")),
            "the placeholder is gone from the dispatch arm; drawn: {drawn:?}"
        );
    }

    /// **The toggles' labels are drawn and are not observable** — the limit
    /// [`the_settings_page_draws_the_settings_and_not_the_placeholder`]
    /// documents, pinned so it cannot quietly stop being true.
    ///
    /// If a future libcosmic makes `Toggler` build a child text widget, this
    /// test fails and the doc above becomes wrong — which is the point. Until
    /// then it is the reason the fourteen toggle strings rest on pure functions
    /// rather than on a render.
    #[test]
    fn the_toggler_labels_do_not_reach_the_text_operation() {
        use cosmic::widget::toggler;
        let drawn = drawn_strings::<Message>(
            toggler(true).label("A LABEL THAT IS DRAWN".to_string()).into(),
        );
        assert!(
            drawn.is_empty(),
            "if this now lists the label, `Toggler` gained a child text widget — \
             update the note on the dispatch test and assert the thirteen toggle \
             labels there. Drawn: {drawn:?}"
        );
    }

    /// **An out-of-set colour scheme is ignored, not stored** —
    /// `bridge.py:197-199`. The in-set half is asserted too, because "ignores
    /// everything" would satisfy the first half alone.
    #[test]
    fn an_out_of_set_color_scheme_is_ignored() {
        let mut shell = shell_with_work_to_do();
        assert_eq!(shell.state.settings.color_scheme, "dark", "the default");

        let effect = observe(&mut shell, Message::SetColorScheme("nonsense".to_string()));
        assert!(!effect.state_changed, "an unrecognised scheme must not be stored");
        assert_eq!(shell.state.settings.color_scheme, "dark");

        let effect = observe(&mut shell, Message::SetColorScheme("light".to_string()));
        assert!(effect.state_changed, "a known scheme must be stored");
        assert_eq!(shell.state.settings.color_scheme, "light");
    }

    /// **An empty default runner is ignored, and so is a write of the value
    /// already stored** — both halves of `bridge.py:235-238`.
    ///
    /// The emptiness check is the reference's own and is not a stand-in for "the
    /// runner exists": the reference does not check that, and the selector's
    /// fallback is what keeps a runner the user uninstalled selectable.
    #[test]
    fn an_empty_default_runner_is_ignored() {
        let mut shell = shell_with_work_to_do();
        let original = shell.state.settings.default_runner.clone();

        let effect = observe(&mut shell, Message::SetDefaultRunner(String::new()));
        assert!(!effect.state_changed, "an empty runner id must not be stored");
        assert_eq!(shell.state.settings.default_runner, original);

        let effect = observe(&mut shell, Message::SetDefaultRunner(original.clone()));
        assert!(
            !effect.state_changed,
            "writing the stored runner must report no change"
        );

        let effect = observe(&mut shell, Message::SetDefaultRunner("GE-Proton9-1".to_string()));
        assert!(effect.state_changed, "a different runner must be stored");
        assert_eq!(shell.state.settings.default_runner, "GE-Proton9-1");
    }

    /// **`SetDefaultToggle` writes only when the value differs, and ignores a
    /// name that is not one of the thirteen** — `bridge.py:264-269`.
    ///
    /// `mangohud` defaults to `false` and `esync` to `true`, so this exercises
    /// both directions of the value with values the model does not already
    /// hold. The unknown name is `wayland`, which the reference deliberately
    /// excludes from the defaulted set (`bridge.py:79-82`).
    ///
    /// **The same-value assertion below is not evidence of the reference's
    /// compare.** `observe` sees the `Debug` of the state, so a write of the
    /// stored value is indistinguishable from no write — measured: `set_toggle`
    /// with its compare deleted leaves that assertion green. What *is* caught is
    /// the `wayland` half: `toggle_slot`'s unknown-key arm was mutated to write
    /// `mangohud` and this test failed, so the name-to-field guard is real.
    #[test]
    fn the_default_toggle_writes_only_a_real_change() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::SetDefaultToggle {
            name: "mangohud".to_string(),
            value: true,
        });
        assert!(effect.state_changed, "mangohud defaults to false, so this is a change");
        assert!(shell.state.settings.default_mangohud);

        // Not "the compare fired": indistinguishable from no write. See above.
        let effect = observe(&mut shell, Message::SetDefaultToggle {
            name: "mangohud".to_string(),
            value: true,
        });
        assert!(!effect.state_changed, "the state is unchanged by a repeated write");

        let effect = observe(&mut shell, Message::SetDefaultToggle {
            name: "wayland".to_string(),
            value: true,
        });
        assert!(
            !effect.state_changed,
            "`wayland` has no `default_wayland` field and must not be written"
        );

        // The other direction of the compare: `esync` is true by default.
        let effect = observe(&mut shell, Message::SetDefaultToggle {
            name: "esync".to_string(),
            value: false,
        });
        assert!(effect.state_changed, "esync defaults to true, so turning it off is a change");
        assert!(!shell.state.settings.default_esync);
    }

    /// **Close-on-launch round-trips through the state** — `bridge.py:247-250`.
    ///
    /// # What this does not check, and why the name says so
    ///
    /// The reference's setter also *compares* before writing — `if bool(value)
    /// != self.settings.close_on_launch` — and **this test cannot see that
    /// guard**. `observe` compares the `Debug` of the whole state, so a write of
    /// the value already stored leaves it byte-identical whether or not the
    /// guard is there. Measured: deleting the compare from the handler leaves
    /// this test green. The guard is kept because it is the reference's
    /// behaviour, not because anything here verifies it, and the four
    /// assertions below are what the test actually proves — the value moves when
    /// it should, in both directions.
    #[test]
    fn close_on_launch_round_trips_through_the_state() {
        let mut shell = shell_with_work_to_do();
        assert!(!shell.state.settings.close_on_launch, "the default");

        let effect = observe(&mut shell, Message::SetCloseOnLaunch(true));
        assert!(effect.state_changed);
        assert!(shell.state.settings.close_on_launch);

        // Not "the guard fired": a write of the same value is indistinguishable
        // from no write at all. See the doc above.
        let effect = observe(&mut shell, Message::SetCloseOnLaunch(true));
        assert!(!effect.state_changed, "the state is unchanged by a repeated write");

        let effect = observe(&mut shell, Message::SetCloseOnLaunch(false));
        assert!(effect.state_changed);
        assert!(!shell.state.settings.close_on_launch);
    }

    /// **The Credits label is the reference's, and the divergence record that
    /// used to hold it is empty.**
    ///
    /// `KNOWN_LABEL_DIVERGENCE` held `Credits → "About & Credits"` because the
    /// port said "Credits" and `Main.qml:94` says "About & Credits". Its own arm
    /// asserted `ours != qml_label`, so it *cannot* survive the fix — this is the
    /// assertion that the fix happened and that the record went with it.
    #[test]
    fn the_credits_label_matches_the_reference_and_the_record_is_empty() {
        assert_eq!(Page::Credits.label(), "About & Credits");
        assert!(
            KNOWN_LABEL_DIVERGENCE.is_empty(),
            "a fixed divergence must delete its record, not leave it (P-65)"
        );
    }

    /// **`NavigateTo` moves both records of the current page.**
    ///
    /// It is the route everything that is not the sidebar itself takes — a
    /// page's own buttons, a shortcut — and the two records drifting apart is
    /// the desync that shows one page in the panel and another in the body.
    #[test]
    fn navigate_to_moves_both_records_of_the_current_page() {
        let mut shell = Shell::new();
        assert_eq!(shell.state.page, Page::Library, "the reference's start page");

        let _ = shell.update(Message::NavigateTo(Page::Plugins));

        assert_eq!(shell.state.page, Page::Plugins, "`state.page` must move");
        assert_eq!(
            shell.sidebar_page(),
            Some(Page::Plugins),
            "the sidebar must move with it, or the panel highlights one page \
             while the body draws another"
        );
        assert!(shell.pages_agree());
    }

    /// `CloseDialog` clears both of the things a dialog can be: the game form
    /// and the pending delete.
    ///
    /// Asserted on both, because the arm writes two fields and dropping either
    /// one is invisible in the other's check.
    #[test]
    fn closing_the_dialog_clears_the_form_and_the_delete_confirmation() {
        let mut shell = shell_with_work_to_do();

        let _ = shell.update(Message::CloseDialog);

        assert!(shell.state.game_form.is_none(), "the form must be closed");
        assert!(
            shell.state.confirm_delete.is_none(),
            "a pending delete must not survive the dialog closing"
        );
    }

    /// `Notify` pushes a toast. Every failure path in `bridge.py` ends here, so
    /// an inert `Notify` would make the whole interface report nothing.
    #[test]
    fn notifying_pushes_a_toast() {
        let mut shell = Shell::new();
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 0"),
            "a new shell has no toasts"
        );

        let effect = observe(&mut shell, Message::Notify("the launch failed".to_string()));

        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 1"),
            "`Notify` must leave a toast behind: it is the channel every \
             failure path in the reference reports through"
        );
        assert!(
            effect.task_units > 0,
            "`Toasts::push` returns the task that schedules the toast's expiry, \
             and `Notify` returns it. Returning `Task::none()` here would push \
             the toast and never expire it — a toast that is drawn and then \
             stays on screen forever, which is why this asserts the task is not \
             empty rather than only that the toast exists"
        );
    }

    /// **No handler can leave the sidebar showing a different page from the
    /// body.**
    ///
    /// `State::page` is a public field, so a handler may assign it directly,
    /// compile, and leave the panel on the previous page. The `debug_assert!`
    /// in [`Shell::show_page`] fires at the moment of such a write when the
    /// handler goes through `show_page` first; this drives every message
    /// through the real dispatcher and requires the two records to agree
    /// afterwards, which is what covers a write that happens with the assert
    /// compiled out or that comes after a navigation.
    ///
    /// This is the T-08 decision on the invariant: checked by structure rather
    /// than by convention, without making the field private (that is T-09's
    /// call, when it is clear how many handlers set pages).
    #[test]
    fn no_handler_leaves_the_sidebar_out_of_step() {
        for message in every_message() {
            let name = variant_name(&message);
            let mut shell = shell_with_work_to_do();
            let _ = shell.update(message);
            assert_eq!(
                shell.sidebar_page(),
                Some(shell.state.page),
                "after {name} the sidebar shows {:?} while `state.page` is {:?}",
                shell.sidebar_page(),
                shell.state.page
            );
        }
    }

    /// **`show_page` panics rather than leaving the sidebar behind.**
    ///
    /// What the `debug_assert!` in [`Shell::show_page`] actually catches is
    /// worth writing down, because the first version of this test asserted
    /// something it cannot do. It does **not** catch a handler that assigns
    /// `state.page` directly: `show_page` writes both records before checking
    /// them, so a later call overwrites the bad write and the pair agrees
    /// again. What it catches is a `show_page` that *cannot* move the sidebar —
    /// `activate_page` answering `false`, which happens when the nav model does
    /// not carry the page, i.e. when `build_nav_model` and [`Page::ALL`] have
    /// diverged. That failure would otherwise be silent: the body would show one
    /// page and the panel another.
    ///
    /// The direct-write case is caught by
    /// [`no_handler_leaves_the_sidebar_out_of_step`] instead, which is why both
    /// exist.
    #[test]
    #[should_panic(expected = "the sidebar shows")]
    fn a_sidebar_that_cannot_show_the_page_trips_the_invariant() {
        let mut shell = Shell::new();
        assert!(shell.pages_agree(), "the shell starts in agreement");

        // A sidebar built from something other than `Page::ALL`: one row.
        let mut stub = nav_bar::Model::default();
        stub.insert().text(Page::Library.label()).data(Page::Library);
        assert!(stub.activate_position(0));
        shell.nav_model = stub;

        // `Page::Settings` is the sixth page, and this model has one row.
// The entry task is dropped: these tests are about the two records
        // `show_page` writes, both of which are written before the task is
        // built. Driving it would reach the network.
        let _ = shell.show_page(Page::Settings);
    }

    /// **Arriving at the Runners page starts its fetch; re-visiting it does
    /// not.**
    ///
    /// The reference's `Component.onCompleted` (`RunnersPage.qml:19`) runs once
    /// per page *instance*, which is the whole reason `show_page` compares
    /// before it assigns (D-48). Both halves are asserted, because only the pair
    /// distinguishes compare-first from a `show_page` that emits unconditionally:
    /// the second call would pass the same assertion if the first were the only
    /// one checked and the entry work simply never ran.
    ///
    /// The assertions are on state rather than on the returned task, because the
    /// task's *effect* is a network request and its state writes are the part
    /// that happens synchronously — `FetchReleases` sets the family, marks the
    /// fetch in flight and clears the stale list before it builds the task
    /// (`bridge.py:697-700`).
    #[test]
    fn arriving_at_the_runners_page_fetches_and_a_re_visit_does_not() {
        let mut shell = Shell::new();
        assert_eq!(shell.state.page, Page::Library, "the shell starts on Library");

        // The entry task is dropped: driving it reaches GitHub.
        let _ = shell.show_page(Page::Runners);
        assert_eq!(
            shell.state.releases_family,
            crate::view::runners::default_family(),
            "arriving did not choose the page's family"
        );
        assert_eq!(
            shell.state.releases_status,
            crate::state::ReleasesStatus::Loading,
            "arriving did not mark the fetch in flight"
        );

        // A status only a *second* fetch would overwrite. `Component.onCompleted`
        // does not fire again for the page already showing, so this must survive
        // the call below — and a `show_page` that emitted unconditionally would
        // set it back to `Loading`.
        shell.state.releases_status = crate::state::ReleasesStatus::Ready;
        let _ = shell.show_page(Page::Runners);
        assert_eq!(
            shell.state.releases_status,
            crate::state::ReleasesStatus::Ready,
            "re-navigating to the page already showing re-ran the page-entry work"
        );
    }

    /// **The pages whose body is a placeholder are exactly the ones the table
    /// lists, checked against the body itself.**
    ///
    /// The failure this exists for: `view_body` returns a placeholder for every
    /// page, `scripts/smoke-test.sh`'s `gui-stays-up` only checks that the
    /// process is alive, and so the repository's whole gate suite passes on a
    /// build whose entire interface is six "not been ported yet" notices.
    ///
    /// # Why this renders instead of comparing two lists
    ///
    /// `pending_task` reads [`PENDING_PAGES`], so comparing a list of pages
    /// derived from that function against `PENDING_PAGES` is the same list
    /// against itself — it cannot fail and would not be a test. The thing that
    /// can be wrong is the *other* half: `view_body`'s arms, which carry the
    /// task string as a literal. So this calls `view_body` and reads the text it
    /// actually draws, which makes the check falsifiable in all four directions:
    ///
    /// - a page the table calls pending whose body is real fails;
    /// - a page the table calls pending whose body names a *different* task
    ///   fails;
    /// - a page the table calls done whose body still says "not been ported"
    ///   fails;
    /// - a page removed from the table without its body landing fails.
    ///
    /// The render is what makes the claim about `include_str!` unnecessary
    /// here: matching this file's own text with `include_str!` would match the
    /// assertion's own literals, so the claim is made against the rendered
    /// element instead.
    ///
    /// **T-19's acceptance is that [`PENDING_PAGES`] is empty**, at which point
    /// this test asserts that no page draws a placeholder and `pending_page` is
    /// deleted with the last entry.
    #[test]
    fn the_pending_pages_are_exactly_the_ones_whose_body_says_so() {
        let mut shell = Shell::new();
        for page in Page::ALL {
// The entry task is dropped: these tests are about the two records
            // `show_page` writes, both of which are written before the task is
            // built. Driving it would reach the network.
            let _ = shell.show_page(page);
            let drawn = drawn_strings(shell.view_body());

            let says_pending = drawn.iter().any(|text| text.contains("has not been ported yet"));

            match pending_task(page) {
                Some(task) => {
                    assert!(
                        says_pending,
                        "{page:?} is listed in PENDING_PAGES as {task} but its \
                         body does not draw the placeholder; drawn: {drawn:?}"
                    );
                    assert!(
                        drawn.iter().any(|text| text.contains(task)),
                        "{page:?} should name the task that builds it ({task}) \
                         on screen as well as in the code; drawn: {drawn:?}"
                    );
                }
                None => assert!(
                    !says_pending,
                    "{page:?} is no longer listed as pending, so its body must \
                     be real — it is still drawing the placeholder: {drawn:?}"
                ),
            }
        }
    }

    /// The strings a real element hands the operation traversal.
    ///
    /// The same mechanism `crate::view::widgets`'s tests use, and for the same
    /// reason: iced exposes no downcast, so the text a widget draws is reachable
    /// only through `Widget::operate`.
    fn drawn_strings<M: Clone + 'static>(mut element: cosmic::Element<'_, M>) -> Vec<String> {
        use cosmic::iced::advanced::widget::{Operation, Tree};
        use cosmic::iced::advanced::{layout::Limits, Layout};
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

        // `layout` and `operate` take the renderer by shared reference; passing
        // it by `&mut` is `clippy::unnecessary_mut_passed`.
        let renderer = cosmic::Renderer::new(Font::default(), Pixels(16.0));
        let mut tree = Tree::new(element.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = element.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let mut texts = Texts::default();
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut texts);
        texts.0
    }

    /// Labels where the port and the reference disagree: `(page, what
    /// `Main.qml` says, what this port says today)`.
    ///
    /// A record of an unfixed divergence, not an allowance, and it is meant to
    /// empty. Every entry is a live, user-visible parity difference — the
    /// drawer in the real application says something this port does not — and
    /// the fix is to change [`Page::label`], not to edit this.
    ///
    /// # Why both sides are written down
    ///
    /// An entry that recorded only the reference's wording would let the port's
    /// label drift to anything at all on that page and stay green — which is the
    /// defect this whole test was rewritten to remove. Recording both means each
    /// field is checked against an independent source: the first against the QML
    /// on disk, the second against [`Page::label`]. Neither is compared to the
    /// other, so neither can be satisfied by an identity.
    ///
    /// **Empty is the target, and it is empty as of T-13.** See P-65.
    ///
    /// It held one entry — `Page::Credits`, whose label said "Credits" where
    /// `Main.qml:94` says "About & Credits" — until T-13 corrected
    /// [`Page::label`] `(state.rs:89)`. The entry could not simply be left: its
    /// own arm asserts `ours != qml_label`, so a fixed label fails the *record*
    /// as stale ("delete it (P-65)") rather than passing quietly. That is what
    /// made the fix verifiable rather than declarable, and it is why deleting
    /// this line is the proof the divergence is gone.
    const KNOWN_LABEL_DIVERGENCE: [(Page, &str, &str); 0] = [];

    /// **The shell's labels are the reference drawer's labels, read off the
    /// QML.**
    ///
    /// The previous version of this check built its `expected` from
    /// `page.label()` and compared it against rows that `build_nav_model` also
    /// set from `page.label()` — an identity, not a comparison. Three mutations
    /// survived it, including one that moved every label to its neighbour.
    ///
    /// This reads `Main.qml` instead, which is the port's specification and is
    /// still in the tree. That is what makes it falsifiable, and it is what
    /// caught the divergence T-13 fixed: `Main.qml:94` says **"About &
    /// Credits"** where `Page::label` said "Credits", recorded in
    /// [`KNOWN_LABEL_DIVERGENCE`] rather than hidden. **That record is empty
    /// now** — emptying it is how the fix is proved, because the entry asserted
    /// its own staleness (`assert_ne!`), so it cannot outlive the divergence it
    /// records.
    #[test]
    fn the_shells_labels_are_the_reference_drawers_labels_in_order() {
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

        // Slice to the drawer's `actions: [...]` block first. The file has other
        // `text:` properties — the drawer's own content area at `:111`, the
        // window's `title:` — and matching those would compare the wrong list.
        let actions = text
            .split_once("        actions: [")
            .and_then(|(_, rest)| rest.split_once("\n        ]"))
            .map(|(block, _)| block)
            .expect("Main.qml should have a drawer `actions: [...]` block");

        let labels: Vec<&str> = actions
            .lines()
            .filter_map(|line| line.trim().strip_prefix("text:"))
            .filter_map(|rest| rest.trim().strip_prefix('"'))
            .filter_map(|rest| rest.split_once('"').map(|(label, _)| label))
            .collect();

        assert_eq!(
            labels.len(),
            Page::ALL.len(),
            "the drawer should name one label per page, in nav order; found \
             {labels:?}. If a page gained or lost an action in the QML, this \
             shell and the reference have diverged."
        );

        for (qml_label, page) in labels.iter().zip(Page::ALL) {
            let ours = page.label();
            match KNOWN_LABEL_DIVERGENCE.iter().find(|(p, _, _)| *p == page) {
                Some((_, reference_wording, port_wording)) => {
                    assert_eq!(
                        qml_label, reference_wording,
                        "the recorded divergence for {page:?} is stale: `Main.qml` \
                         now says {qml_label:?}"
                    );
                    assert_eq!(
                        ours, *port_wording,
                        "the port's label for {page:?} is not the one this \
                         divergence records. If you changed `Page::label`, \
                         either update this record or — if it now matches the \
                         reference — delete it (P-65)"
                    );
                    assert_ne!(
                        ours, *qml_label,
                        "{page:?} now matches the reference, so the \
                         KNOWN_LABEL_DIVERGENCE entry is fixed — delete it \
                         (P-65). Leaving it recorded would hide the next \
                         divergence on this page"
                    );
                }
                None => assert_eq!(
                    ours, *qml_label,
                    "the shell's labels must be the reference's, in the \
                     reference's order: {page:?} is {ours:?} here and \
                     {qml_label:?} in `Main.qml`. A mismatch is a live parity \
                     divergence (P-65), not cosmetics."
                ),
            }
        }
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
