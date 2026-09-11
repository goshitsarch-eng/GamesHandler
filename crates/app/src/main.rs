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

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use cosmic::app::ApplicationExt;
use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc::{UnboundedSender, unbounded as unbounded_channel};
use cosmic::widget::{icon, nav_bar, toaster};
use gamehandler_core::installers::{
    Installer, SystemClock, build_installer_command, download_installer, game_from_install,
    installer_by_id, prepare_prefix, wait_for_installer, wait_for_prefix_idle,
};
use gamehandler_core::models::{Game, Library, SORT_MODES};
use gamehandler_core::netpaths::NetpathsShares;
use gamehandler_core::paths::{self, SystemEnv};
use gamehandler_core::plugins::{PluginEnv, SystemPluginEnv};
use gamehandler_core::runners::families::ReleaseInfo;
use gamehandler_core::runners::launch::{LaunchedGame, tool_command};
use gamehandler_core::runners::{
    LAUNCH_GRACE_SECONDS, RunnerError, RunnerManager, SystemLaunchEnv, prefix_drive_c,
};
use gamehandler_core::runners::{desktop, launch};
use gamehandler_core::settings::{COLOR_SCHEMES, Settings, VIEW_MODES};
use gamehandler_core::{APP_ID, APP_NAME, VERSION};

mod http;
mod shortcuts;
mod state;
mod theme;
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
    CoverHit, ExeField, FormField, FormToken, GameForm, GameId, Page, PendingInstall, PrefixTool,
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
/// from the library that was actually loaded.
///
/// A function named `launch_failure` used to be cited here as the rule for
/// anything that had to stay stubbed. It is deleted rather than re-pointed:
/// **there is no stub left in this file.** `--launch` stopped being one in
/// T-07, and the two halves of #31 are now both real, so that sentence has no
/// subject. The criterion above still governs, and the way to satisfy it is to
/// compute the answer rather than to phrase the excuse well.
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

/// `main.py:31-34`: the library lookup and the sentence a miss prints.
///
/// Pure over `(&Library, &str)`, which is the property that makes the miss
/// sentence assertable without opening a library the test process does not own
/// — and it is the same property the deleted `launch_failure` had, kept because
/// the tests that pin Python's unknown-id message still need a pure function to
/// call. [`launch_game`] is the only caller that reads the real library.
///
/// The message is Python's verbatim, `APP_NAME` included, because a shortcut
/// wrapper or a script may match on it.
fn library_lookup<'a>(library: &'a Library, game_id: &str) -> Result<&'a Game, String> {
    library
        .get(game_id)
        .ok_or_else(|| format!("{APP_NAME}: no game with id {game_id}"))
}

/// What `--launch` prints and what it exits with, given how the attempt went.
///
/// This is the whole of the reference's `_launch_from_cli` below the launch
/// itself (`main.py:35-46`) — three `f`-strings and two `return`s, in the same
/// order and with the same wording. Keeping it pure is what lets the tests
/// below pin every sentence a real `--launch` can print, which is not true of a
/// function that spawns a game: the honest-error criterion this file is built
/// around ("a shortcut that opens nothing must not report success") is
/// otherwise checkable only by running one.
///
/// `attempt` is `launch::launch`'s result fused with the grace watch:
///
/// - `Err(text)` — `launch()` raised; `text` is `str(exc)`. Nothing was
///   started, so there is no grace to watch.
/// - `Ok(None)` — `started.failure()` was `None`: the title was still running
///   when the grace period expired. This is the **only** arm that exits 0.
/// - `Ok(Some(reason))` — it started and then stopped, and `reason` says why.
///
/// # The wording is the CLI's, not the GUI's
///
/// `Could not launch “{name}”: {exc}` (`bridge.py:468`, on
/// [`Message::LaunchStarted`]) and this function's `could not launch {name}:
/// {exc}` are two different strings in the reference itself: the GUI's is
/// capitalised with typographic quotes and the CLI's is lowercase with a plain
/// colon (`main.py:38`). They are not unified here, because the CLI's copy is
/// what a `.desktop` shortcut's stderr shows and the tests pin it as such. What
/// the two *do* share is the `{exc}` text, which is why both are built from the
/// error's `Display` rather than from a second rendering of the failure.
fn launch_report(name: &str, attempt: Result<Option<String>, String>) -> (Option<String>, u8) {
    match attempt {
        Err(error) => (
            Some(format!("{APP_NAME}: could not launch {name}: {error}")),
            1,
        ),
        Ok(None) => (None, 0),
        Ok(Some(reason)) => (
            Some(format!("{APP_NAME}: {name} stopped right away: {reason}")),
            1,
        ),
    }
}

/// `--launch <GAME_ID>`: start a game and report whether it stayed up.
///
/// The port of `main.py:_launch_from_cli` (P-70), and the verb every `.desktop`
/// shortcut this app writes invokes — [`shortcut_command`] builds
/// `gamehandler --launch <id>`. It runs **before** anything GUI-shaped, per
/// D-12, because those shortcuts are already on users' disks and must work on a
/// headless or remote session.
///
/// # There is no display check here, and that is the reference's decision
///
/// [`display_refusal`] guards [`run_gui`] and nothing else. Three reasons, in
/// increasing order of weight:
///
/// 1. `_launch_from_cli` has no such check.
/// 2. The *launch* needs no display. Wine is what might, and its own failure
///    comes back through `started.failure()` as `stopped right away: …` — a
///    sentence from the runner that names the real cause, where a pre-emptive
///    refusal here would name a variable and guess.
/// 3. A guard that was stricter than the reference would refuse a launch that
///    would have worked, from exactly the environment (a `.desktop` file, a
///    thin session) the verb exists to serve.
///
/// # The one divergence, and it is the GUI's precedent rather than a new one
///
/// The reference's `library.mark_played(game.id)` (`:40`) raises on a failed
/// save, which in `_launch_from_cli` is an uncaught traceback: the exit code is
/// 1 and the grace watch never runs, so a title that *did* start is reported as
/// a failed shortcut and its real failure reason is never collected. Here the
/// save error is printed and the watch still runs, which is
/// [`Message::LaunchStarted`]'s arm making the same call for the same reason
/// ("every other store write in this shell reports instead"). The exit code
/// stays the launch's, because the game did launch.
///
/// `config.ensure_dirs()`, the reference's first line (`:29`), is not called —
/// the same omission [`list_games`] records, for a different reason. It is
/// unobservable here rather than merely unobjectionable: the config directory
/// is created by the `mark_played` save that needs it
/// (`json::write_python_file` creates parents) and the runners directory is
/// read by `RunnerManager`, whose missing-directory arm yields the same
/// "runner is not available" error the reference gets from listing an empty
/// one. Both paths end at the same sentence, which is the part that is
/// contract.
///
/// The body is [`launch_game_at`], which is this function with the two global
/// reads turned into parameters — see its doc for why that seam exists.
fn launch_game(game_id: &str) -> ExitCode {
    launch_game_at(
        &mut Library::new(None),
        &RunnerManager::new(&SystemLaunchEnv),
        game_id,
    )
}

/// [`launch_game`] with its two global reads passed in.
///
/// The same seam, and the same reason, as [`display_refusal`]'s `out` parameter
/// and [`display_present`]'s `env`: the reference reads `Library()` and
/// `RunnerManager()` as globals, and a function that reads them itself can only
/// be tested by running it against the developer's real library. That is not a
/// hypothetical — **the mutation that found this seam deleted the whole
/// `mark_played` call and left the suite green** (298 passed), because nothing
/// could reach the line. With the seam,
/// `a_real_launch_records_the_game_as_played` drives the real `launch::launch`,
/// the real `started.failure()`, the real `mark_played` and the real exit code
/// against a native title in a temp library, and the deletion goes red.
///
/// The name ends `_at` rather than `_with` to match [`Library::new_at`] and
/// [`RunnerManager::at`], which are what a caller of this function builds.
fn launch_game_at(library: &mut Library, runners: &RunnerManager, game_id: &str) -> ExitCode {
    let game = match library_lookup(library, game_id) {
        Ok(game) => game.clone(),
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(1);
        }
    };
    // `launch(game, RunnerManager())` (`main.py:36`), inside the `try`: a
    // missing runner, a runner that cannot build a command and a missing game
    // executable all arrive here as `Err`.
    let attempt = match launch_process(&game, runners) {
        Err(error) => Err(error.to_string()),
        Ok(mut started) => {
            // `library.mark_played(game.id)` (`:40`) — after the launch and
            // *before* `started.failure()` (`:42`), which is the reference's
            // own order and not incidental: the grace period is up to
            // `LAUNCH_GRACE_SECONDS`, and the "played" record must be durable
            // by the time the process exits either way.
            if let Err(error) = library.mark_played(&game.id) {
                eprintln!(
                    "{APP_NAME}: could not record “{}” as played: {error}",
                    game.name
                );
            }
            Ok(started.failure(launch_grace()))
        }
    };
    let (message, code) = launch_report(&game.name, attempt);
    if let Some(message) = message {
        eprintln!("{message}");
    }
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
    /// One of the form's text fields changed. The reference has one
    /// `QVariantMap` the QML writes by key; this is that key and its new value.
    FormFieldChanged { field: FormField, value: String },
    /// One of the form's fifteen switches changed, addressed by its
    /// `_TOGGLE_FIELDS` name.
    FormToggleChanged { name: String, value: bool },
    /// The form's Type selector. `bridge.py` reads `isLinux` off the combo's
    /// index at save time (`GameFormPage.qml:16`, `:44`); this writes it when it
    /// changes, so the rows the reference disables for a Linux game follow the
    /// selector rather than waiting until Save.
    SetFormLinux(bool),
    /// Start a title. `playGame()`; marks it played and begins the grace watch.
    ///
    /// The launch itself happens on a worker: `launch()` forks a runner, creates
    /// the prefix directory and may copy the bundled DXVK runtime into it, which
    /// is not work for the thread that draws the frame. The outcome comes back
    /// as [`Message::LaunchStarted`] and the watch as
    /// [`Message::LaunchWatchFinished`], in that order, from one worker.
    LaunchGame(GameId),
    /// The runner process exists — or the attempt failed, before any grace.
    ///
    /// This is the message that splits the reference's `playGame` where it is
    /// actually split: `launch()` may raise, and everything after it —
    /// `mark_played`, the "Launching…" toast, the close-on-launch hide — happens
    /// only if it did not (`bridge.py:461-476`). `Err` carries the rendered
    /// `Could not launch “{name}”: {exc}`, built where the name and the error are
    /// both in hand rather than re-derived from an id.
    LaunchStarted {
        game_id: GameId,
        result: Result<(), String>,
    },
    /// The grace watch ended.
    ///
    /// `reason` is `None` when the title was still running when the grace
    /// period expired, which is the success case — and the reference does
    /// nothing at all with it (`report`, `bridge.py:478-483`), so neither does
    /// this. `Some(_)` is the runner's failure report, shown as a toast with the
    /// window brought back.
    LaunchWatchFinished {
        game_id: GameId,
        reason: Option<String>,
    },
    /// Run `winecfg` or `winetricks` against the game's prefix.
    RunPrefixTool { game_id: GameId, tool: PrefixTool },
    /// The prefix tool was started, or could not be.
    PrefixToolStarted {
        game_id: GameId,
        tool: PrefixTool,
        result: Result<(), String>,
    },
    /// Open the prefix folder in the desktop's file manager. `openPrefix()`.
    OpenPrefixFolder(GameId),
    /// The prefix folder was opened — the one outcome that reaches the user is
    /// a failure, because `openPrefix` has no success notice: the file manager
    /// window *is* the feedback (`bridge.py:503-518`).
    ///
    /// No `game_id`, deliberately: the reply names nothing the handler needs,
    /// and a field carried for symmetry is a field no one reads — the shape
    /// finding #81 was filed about.
    PrefixFolderOpened { result: Result<(), String> },
    /// Write a `.desktop` shortcut. `createShortcut()`.
    CreateDesktopShortcut(GameId),
    /// The shortcut was written, or could not be. The path is the whole
    /// message, so — as with [`Message::PrefixFolderOpened`] — there is no id
    /// to carry.
    ShortcutCreated { result: Result<PathBuf, String> },

    // ---- Links ------------------------------------------------------------
    /// Open a URL in the user's browser. P-65.
    ///
    /// The reference reaches this through `Kirigami.UrlButton`
    /// (`CreditsPage.qml:85-89`, `:137-139`), a QML widget that calls
    /// `Qt.openUrlExternally` internally — so there is no `bridge.py` line for
    /// this one and no Python-side error to mirror. The port's equivalent of
    /// that widget is `button::link`, which carries a label and **no href**: an
    /// href is what the caller supplies through `on_press`, which is why the
    /// credits page's twenty-seven links rendered disabled and recorded the gap
    /// as `LINKS_OPEN` until this variant landed.
    ///
    /// The URL is a `String` rather than a parsed `Url` because nothing between
    /// the data layer and `xdg-open` reads it: `Credit::url` is a
    /// `&'static str` (`core::credits`) and the reference passes its string
    /// through untouched, so a parser here would be a second opinion about a
    /// value no branch consults.
    OpenUrl(String),
    /// A link was opened, or could not be.
    ///
    /// The failure is *reported*, and that is a deliberate departure from the
    /// reference — recorded here rather than left for a reader to discover.
    /// `UrlButton` has no error surface at all, so a desktop with no browser
    /// registered gives a click that does nothing and says nothing, which on
    /// this page is indistinguishable from the disabled-link defect this
    /// variant exists to close. It is the same reasoning and the same shape as
    /// [`Message::PrefixFolderOpened`], one arm away.
    UrlOpened { result: Result<(), String> },

    /// Hide the window, or bring it back. The `requestHide`/`requestShow`
    /// signal pair (`bridge.py:159-160`), which `Main.qml:170-177` answers with
    /// `visible = false` and `visible = true` + `raise()` + `requestActivate()`.
    ///
    /// A message rather than a direct call because it needs the framework's
    /// window id, which only [`App`] holds — the same reason [`Message::Quit`]
    /// is answered there. `true` is `requestHide`, `false` is `requestShow`.
    SetWindowHidden(bool),

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
    /// Ask before deleting an installed build. The reference's delete button
    /// opens `removeRunnerDialog` (`RunnersPage.qml:82-85,257-272`) rather than
    /// deleting; the dialog closes on Cancel and sends the row's `runnerId` to
    /// `uninstallRunner` on Remove. P-37.
    ConfirmRemoveRunner {
        runner_id: String,
        name: String,
    },
    /// The user confirmed; do it. `uninstallRunner()` — synchronous, so this
    /// returns no task.
    RemoveRunnerConfirmed(String),
    /// Delete an installed build. `uninstallRunner()` — synchronous, so this
    /// returns no task.
    ///
    /// Kept as the direct route alongside the confirm pair: the dialog's
    /// confirmation sends [`Message::RemoveRunnerConfirmed`], and this variant
    /// stays the message the removal itself is. Both reach the same arm.
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
    /// The runner the **next install** will use — not the app's default.
    ///
    /// The Installers page's selector, and the one message of T-38's set that is
    /// an addition rather than a rename of a `bridge.py` setter: the reference's
    /// combo stores nothing (`InstallersPage.qml:49-64` has no write-back), so
    /// there is no `_set_` for it to mirror. It is a message all the same
    /// because a libcosmic view is rebuilt every frame and the choice has to
    /// live somewhere; [`crate::state::State::installer_runner`] is where, and
    /// it is what `StartEasyInstall`'s `runner_id` is read from. See D-55.
    SetInstallRunner(String),
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
    /// The install's worker gave up before there was a wizard to watch.
    ///
    /// **An addition to `architecture.md` §2.2's variant set, and it is there
    /// because the set had no channel for a real path.** `installEasy` runs its
    /// `work()` through `_async`, whose `fail` branch (`bridge.py:152-163`) does
    /// three things on any exception: clear `_easy_busy`, reset the progress,
    /// and `notify(f"Could not install {name}: {message}")`. Every one of the
    /// documented variants describes a *later* stage — a download that failed
    /// has no `found`, no `returncode` and no pending prefix. Without this
    /// variant the worker's error would either be swallowed or reported without
    /// clearing the guard, and a page stuck `busy` forever is the exact failure
    /// `view::installers`' header warns an arm must not have.
    ///
    /// It carries only the error text: the installer's name is in
    /// [`State::running_install`], which is the record this message consumes.
    EasyInstallFailed { message: String },
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
/// The Runners page's removal confirmation, `removeRunnerDialog`
/// (`RunnersPage.qml:257-272`), drawn as a modal over the page body.
///
/// The reference's dialog is a `Kirigami.PromptDialog`: its `title` is
/// `"Remove " + pendingRemove.name + "?"`, its `subtitle` the fixed sentence,
/// and its footer is Cancel plus a custom Remove action — Cancel closes, and
/// Remove sends the row's `runnerId` to `uninstallRunner` and then closes
/// (`RunnersPage.qml:259-271`). The port renders the same three parts through
/// libcosmic's own composition: `cosmic::widget::dialog` builds the
/// titled card (`src/widget/dialog.rs`), and `cosmic::widget::popover` lays it
/// over the body the way libcosmic itself lays an application dialog over its
/// view (`src/app/mod.rs:874-887`), `modal(true)` so background input is
/// captured and `on_close` so dismissing it sends [`Message::CloseDialog`].
///
/// This is a free function over `(body, pending)` rather than a method on
/// [`State`] or a page module for the same reason [`Shell::view_body`]'s arms
/// borrow rather than build: the dialog needs the already-built body beneath
/// it, and the page's view does not know a dialog is open. P-37.
///
/// # Why a `Column`, and not `popover`
///
/// The first version composed the dialog over the body with
/// `cosmic::widget::popover` — the same widget libcosmic itself uses to lay an
/// application dialog over its view (`src/app/mod.rs:874-887`). It draws
/// correctly and behaves correctly, but it is untestable in exactly the way
/// that matters: `Popover::operate` returns early when `modal && popup.is_some()`
/// (`src/widget/popover.rs:138-141`), skipping the background content *and*
/// the popup, so the `drawn_strings` instrument this file's overlay tests use
/// sees neither half — an open dialog photographs as `[]`. A passing render
/// test would then be asserting the absence it was written to refute, which is
/// the D-44 defect wearing a dialog's clothes.
///
/// So the dialog is drawn **inline above the body**, in a `Column`: title,
/// subtitle and both actions are ordinary widgets in the tree, `operate` sees
/// them, and the tests below photograph what the user sees. The cost is
/// stated rather than hidden: this is not a floating overlay — the page sits
/// below the dialog rather than dimmed behind it — and a libcosmic `popover`
/// whose popup an `Operation` could reach would be the composition to return
/// to. What is preserved is the behaviour the reference's dialog promises:
/// the title names the pending removal, both actions are drawn, Cancel closes
/// without removing, and Remove removes the pending id and then closes.
///
/// # The Remove button's message, and the gap beside it
///
/// Remove sends [`Message::RemoveRunnerConfirmed`], not
/// [`Message::UninstallRunner`] — and the choice is load-bearing rather than
/// nominal: the confirmed arm clears the pending removal before removing, and
/// a button sending the direct route would remove while leaving the dialog
/// open. **No test here sees that choice**: a built `Button`'s message is
/// opaque (the sources are cited at `installed_card`'s call site), so
/// `the_runner_dialog_is_a_modal_over_the_page_it_names` photographs the
/// *label* "Remove" and cannot tell which of the two variants it carries —
/// mutating one into the other leaves all 322 green, measured. This paragraph
/// is the record of that gap, and the fix is the same one the codebase
/// already uses for it: a named press helper carrying the value, as
/// `remove_press` does for the delete
/// button — except the helper would live in this file, beside the button,
/// rather than across the page boundary.
fn remove_runner_dialog<'a>(
    body: cosmic::Element<'a, Message>,
    pending: &crate::state::PendingRunnerRemoval,
) -> cosmic::Element<'a, Message> {
    use cosmic::widget::{button, dialog, Column};
    let popup: cosmic::Element<'a, Message> = dialog()
        .title(pending.title())
        .body(crate::state::remove_runner_subtitle())
        .secondary_action(button::standard("Cancel").on_press(Message::CloseDialog))
        .primary_action(
            button::destructive("Remove")
                .on_press(Message::RemoveRunnerConfirmed(pending.runner_id.clone())),
        )
        .into();
    Column::new().push(popup).push(body).into()
}

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
        // The Installers page's three arguments, for the same reason and with
        // the same failure otherwise: an unrefreshed catalog renders "No
        // matching installers" over a catalog of nine, which is a page that
        // looks like an empty search rather than like a page nothing filled.
        state.refresh_installers();
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

    /// `Ctrl+F` — the reference's two actions: `root.showPage("library")` and
    /// `root.libraryPage.focusSearch()` (`Main.qml:126-134`).
    ///
    /// A method on `Shell` rather than the body of `App::on_search`, for the
    /// same reason [`Shell::view_body`] is: `App` cannot be built off a display,
    /// so the navigation half would otherwise be unreachable from a test. Here
    /// the navigation half is assertable — `state.page`, the sidebar, and
    /// `show_page`'s own `pages_agree` invariant all move — and only the focus
    /// half is left unobservable.
    ///
    /// # Why the focus can be returned alongside the navigation
    ///
    /// A widget operation is applied to the tree that exists when it runs, so a
    /// `focus(id)` returned in the same task as the navigation would do nothing
    /// if the tree were still the *previous* page's — the search input would not
    /// be in it, and the operation would silently find no match. **It is not:**
    /// the runtime drains the update's actions and *then* rebuilds the
    /// interfaces from `program.view()` before running them
    /// (`iced/winit/src/lib.rs:1518-1537`, and the same order at `:1151-1169`).
    /// The tree the operation sees is the one drawn after this navigation, so
    /// `Page::Library`'s search box is in it.
    ///
    /// That ordering is the whole reason this can be one task rather than a
    /// deferred second message, and it is a property of the runtime rather than
    /// of anything here — which is why it is cited rather than assumed. It is
    /// also not testable from here: the task has no accessor, so a test cannot
    /// see the operation, only the state the navigation wrote.
    fn focus_library_search(&mut self) -> cosmic::app::Task<Message> {
        let navigate = self.show_page(Page::Library);
        // The id is the view's, not this function's — `view/library.rs` is where
        // the widget that carries it is built, so the two cannot drift into a
        // focus that names a widget nothing draws.
        let focus = cosmic::iced::widget::operation::focus(view::library::SEARCH_INPUT_ID);
        cosmic::app::Task::batch([navigate, focus])
    }

    /// The body under the sidebar, for whichever page is showing.
    ///
    /// This is the page dispatch: one arm per [`Page`], and **every arm now
    /// draws the real page** — T-38 wired the last one, `Page::Installers`, and
    /// the `PENDING_PAGES` list went with it. (Code spans rather than links for
    /// that list and for `pending_page`: both are `#[cfg(test)]`, so rustdoc
    /// does not build them and a link to either is a broken link — which is how
    /// this comment was written the first time, and the doc gate said so.)
    /// Two things about the shape are
    /// load-bearing and neither is cosmetic.
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
    /// it is given a body. While an arm was unported it named the task that
    /// would build it, on screen as well as in the code, so a page that had not
    /// landed said so.
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
                    // Read once per frame, so every row's "Played … ago" is
                    // computed against the same instant. `format_last_played`
                    // takes `now` as a parameter for exactly this; see
                    // `LibraryPage::now`.
                    now: gamehandler_core::models::now(),
                };
                view::library::view(page)
            }
            // The catalog, the two filters and the runner selector, all
            // borrowed from `State` rather than built here: a catalog computed
            // in this arm would be a local the returned element outlives. See
            // `State::installer_catalog`.
            //
            // The `InstallersView` really is a local, and the element it
            // produces does borrow *through* it — but not *from* it: every field
            // it holds is a reference into `self.state`, and `view` takes the
            // struct by value, so what the element carries out of this arm is
            // `&self.state.*` and not `&page`. T-38 changed `view`'s signature
            // from `&'a InstallersView<'_>` to `InstallersView<'a>` for exactly
            // this arm; the doc on that function has the compiler error that
            // the reference form produces.
            Page::Installers => {
                let page = view::installers::InstallersView {
                    catalog: &self.state.installer_catalog,
                    search: &self.state.installer_search,
                    category: &self.state.installer_category,
                    categories: &self.state.installer_categories,
                    runners: &self.state.installer_runners,
                    // The install's runner, not `settings.default_runner`. That
                    // was D-55's defect: this one binding made the page's
                    // selector write the Settings page's control and install
                    // under whichever runner the index happened to resolve to.
                    runner_id: &self.state.installer_runner,
                    busy: view::installers::installing(&self.state),
                    progress: view::installers::progress_fraction(&self.state),
                };
                view::installers::view(page)
            }
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

    /// The page body, with anything open **above** it.
    ///
    /// The reference pushes the game form as a layer over the pages
    /// (`GameFormPage.qml:1`), so it is not a [`Page`] and the dispatch above
    /// knows nothing about it. This is where the two meet.
    ///
    /// # The layer replaces the page rather than being stacked on it
    ///
    /// `none` is drawn, not `Stack`: a pushed page covers the one beneath it, and
    /// a composition that drew both would let a test find the form's strings and
    /// the library's in the same body — which cannot tell a layer from a merge.
    /// It is also what the reference does: `Kirigami.ScrollablePage` under a
    /// pushed page is not painted.
    ///
    /// # Why this is separate from [`Self::view_body`]
    ///
    /// `view_body` is the *page*, and every page test drives it — they are about
    /// the dispatch arm and must not start depending on whether a form happens to
    /// be open. The overlay is a second question with its own tests, and this is
    /// the function they drive.
    fn view_with_overlays(&self) -> cosmic::Element<'_, Message> {
        // The game form takes precedence: it is a full-page layer that covers
        // whatever is open, while the runner dialog is a modal over the page
        // it belongs to (see [`remove_runner_dialog`]). Two keyboards-full of
        // dialog at once would need a stacking order, and the reference has no
        // such state — each QML layer closes the other — so the form wins and
        // the pending removal waits underneath it.
        if let Some(form) = &self.state.game_form {
            return view::form::view(view::form::GameFormView {
                form,
                library: &self.state.library,
                runners: &self.state.runners,
            });
        }
        let body = self.view_body();
        match &self.state.confirm_remove_runner {
            Some(pending) => remove_runner_dialog(body, pending),
            None => body,
        }
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
    /// - the launch flow, from T-29: [`Message::LaunchGame`],
    ///   [`Message::LaunchStarted`], [`Message::LaunchWatchFinished`],
    ///   [`Message::RunPrefixTool`], [`Message::PrefixToolStarted`],
    ///   [`Message::OpenPrefixFolder`], [`Message::PrefixFolderOpened`],
    ///   [`Message::CreateDesktopShortcut`] and [`Message::ShortcutCreated`] —
    ///   `playGame`, `runPrefixTool`, `openPrefix` and `createShortcut`
    ///   (`bridge.py:461-533`) and the four replies their deferred halves come
    ///   back on. Each of the four entry points performs its lookup and its
    ///   guards here and hands the blocking part — a fork, a `mkdir`, an
    ///   `xdg-open`, a file write — to a worker; each reply is what that worker
    ///   sends. Two are worth naming for what they *do not* do: an id the
    ///   library does not hold is silence in three of the four (only `playGame`
    ///   says "Select a game first"), and `LaunchWatchFinished { reason: None }`
    ///   — the title that is still running — is silent in the reference
    ///   (`report`, `:478-483`), so it is silent here.
    /// - the links, from P-65: [`Message::OpenUrl`] and [`Message::UrlOpened`] —
    ///   the credits page's twenty-seven `button::link`s and the outcome. There
    ///   is no `bridge.py` line to mirror: the reference's `Kirigami.UrlButton`
    ///   opens the URL itself (its own type documentation: *"will open the URL
    ///   when left-clicked, tapped, or activated with the keyboard"*), so the
    ///   port has to supply what the QML widget did for free.
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
    /// # The three arms that are empty on purpose
    ///
    /// `Quit` and [`Message::SetWindowHidden`] are answered by
    /// [`Application::update`](cosmic::Application::update), which holds the
    /// window — and their arms here must stay empty, because an
    /// arm that moved the window would need the `Core` this type deliberately
    /// has none of. They are *excluded by name* from the guard below, exactly as
    /// `Quit` already was.
    ///
    /// `LaunchWatchTick` is the third and is a different case: it needs no
    /// exclusion, because it genuinely has no effect — §3.3 chose one grace
    /// timeout over a poll, so nothing constructs it and its arm does nothing by
    /// decision rather than by omission. That is why it no longer carries a
    /// `TODO(T-29)` marker, which would now read as work still owed.
    /// `dispatch_coverage` is what fails if a poll ever starts emitting it.
    ///
    /// That list is not a comment. `only_the_written_handlers_change_anything`
    /// drives every message in `every_message` through this function and
    /// requires the set that has any effect to be exactly the handlers named
    /// there (plus `Quit`, which needs the window and so is `App::update`'s one
    /// arm, and `DismissToast`, whose arm is real but unobservable). A handler
    /// that regresses to `{}` shrinks that set and fails; a new handler landing
    /// grows it and fails until it is added deliberately.
    ///
    /// This paragraph used to carry its own count — "those fifteen" — beside
    /// the list's, and the two disagreed: T-11 grew the list to twenty-two and
    /// the number here was left at fifteen, in the sentence a reviewer trusts
    /// to know what is live. Before that it said "three" and named three,
    /// omitting `CloseDialog` and `Notify`. The number is gone rather than
    /// corrected: the list is the authority and a second number describing it
    /// is a thing that can disagree with it silently.
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
            // `newGameTemplate()` (`bridge.py:382-399`): a fresh id, the stored
            // default runner, and the fifteen toggles from
            // `Settings::default_*` where a setting exists and the `Game`
            // dataclass default where one does not. `GameForm::new_template` is
            // that whole function, tested without a display.
            Message::OpenNewGameForm => {
                let id = gamehandler_core::models::new_id();
                self.state.game_form = Some(GameForm::new_template(
                    &self.state.settings,
                    id,
                ));
                self.state.confirm_delete = None;
            }
            // `getGame(id)` + push (`bridge.py:356-379`). An id the library does
            // not hold opens nothing rather than an empty form: the reference's
            // `getGame` returns `{}` and the QML's `gameData` would then be a map
            // whose every read falls through to its `|| ""`, which draws a blank
            // Add form under an "Edit Game" title. Doing nothing is the honest
            // version of that.
            Message::OpenEditGameForm(game_id) => {
                if let Some(game) = self.state.library.get(&game_id) {
                    self.state.game_form = Some(GameForm::from_game(game));
                    self.state.confirm_delete = None;
                }
            }
            Message::CloseDialog => {
                self.state.game_form = None;
                self.state.confirm_delete = None;
                self.state.confirm_remove_runner = None;
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
            // `_set_color_scheme` (`bridge.py:197-203`) does two things and this
            // port did only the first: it ignores a value outside
            // `COLOR_SCHEMES` (the load path already folds one,
            // `settings.rs:89-91`) **and then applies it**. Storing is the guard;
            // `theme::apply` is the second half, which the reference spells
            // `self._theme.apply(value)` (`bridge.py:201-202`).
            //
            // The comment here used to say this port had "no theme to apply it
            // to ... because `Shell` has no `theme()` override". Both halves of
            // that were wrong and the correction is in `theme.rs`'s module doc:
            // there is no `theme()` hook on `cosmic::Application` at all, and
            // libcosmic's `app::Settings` default is already
            // `system_preference()` — so the app followed the system and the
            // selector changed nothing. `theme::apply` is the mechanism
            // libcosmic actually provides (`command::set_theme`).
            //
            // Note that the task is returned even when the value is **stored
            // unchanged**: applying it is idempotent, and a guard that skipped
            // the task for a repeated value would be the reference's own
            // `if value == ...: return` (`bridge.py:198`) with nothing to
            // observe — the app's theme is a global in libcosmic, not a field
            // this handler could compare.
            Message::SetColorScheme(value) => {
                if COLOR_SCHEMES.contains(&value.as_str()) {
                    self.state.settings.color_scheme = value;
                    // Applies the value **stored**, not the one received: they
                    // are equal here (the guard just checked), and reading it
                    // back is what keeps the two from being able to diverge.
                    return theme::apply(&self.state.settings.color_scheme);
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
            // `saveGame` (`bridge.py:404-446`). `GameForm::apply` is the whole of
            // the validation and normalisation, tested without a display; this
            // arm is the persistence and the report.
            //
            // # The one place the sequence differs from the QML's
            //
            // `GameFormPage.qml:51-52` calls `backend.saveGame(gameData)` and then
            // `closeForm()` unconditionally, so a save the name check rejects
            // closes the form and throws the user's typing away — the toast
            // (`bridge.py:407-409`) is the only trace of it. Here the form stays
            // open on a rejection, which is the reference's own dialog
            // behaviour applied one level up: nothing is destroyed on a refusal.
            //
            // That branch is **unreachable from the drawn form**, and this is the
            // honest reading rather than a claim: `enabled: nameField.text.trim()
            // .length > 0` (`:34`) means the button cannot be pressed with an
            // empty name, and `view::form::can_save` enforces the same gate on
            // the message. It is kept because `SaveGameForm` is a message any
            // caller can send — the CLI and the tests do — and because a form
            // that closed on a rejection would be a worse bug than an
            // unreachable arm.
            //
            // `Library::add`/`update` return `io::Result` and a write failure is
            // not the user's to fix, so it is reported rather than raised, the
            // same way every other store in this shell reports one.
            Message::SaveGameForm(form) => {
                // The reference's add-vs-update test (`bridge.py:406`): whether the
                // library already holds this id, not what the form says it is.
                let existing = self
                    .state
                    .library
                    .get(form.game_id.as_deref().unwrap_or_default());
                match form.apply(existing) {
                    Err(message) => return self.state.toast_task(message),
                    Ok(game) => {
                        let is_new = existing.is_none();
                        let name = game.name.clone();
                        let stored = if is_new {
                            self.state.library.add(game)
                        } else {
                            self.state.library.update(game)
                        };
                        if let Err(error) = stored {
                            return self.state.toast_task(format!(
                                "Could not save “{name}”: {error}"
                            ));
                        }
                        // The two sentences are the reference's, em dashes and
                        // all (`bridge.py:439`, `:442`).
                        self.state.game_form = None;
                        let notice = if is_new {
                            format!("Added “{name}”")
                        } else {
                            format!("Updated “{name}”")
                        };
                        return self.state.toast_task(notice);
                    }
                }
            }
            // The form's text fields. Twelve of them through one variant, because
            // the reference has one `QVariantMap` and the alternative is twelve
            // near-identical arms.
            Message::FormFieldChanged { field, value } => {
                if let Some(form) = self.state.game_form.as_mut() {
                    form.set_field(field, value);
                }
            }
            // The form's switches, addressed by `_TOGGLE_FIELDS` name. Refused for
            // a name the form does not hold, which is `GameForm::set_toggle`'s
            // `false` — see that method for why an unknown name writes nothing
            // rather than creating the entry.
            Message::FormToggleChanged { name, value } => {
                if let Some(form) = self.state.game_form.as_mut() {
                    form.set_toggle(&name, value);
                }
            }
            // The Type selector. `bridge.py` derives `isLinux` from the combo's
            // index at save time; this writes it as it changes, which is what
            // makes the rows gated on `!isLinux` follow the selector.
            Message::SetFormLinux(is_linux) => {
                if let Some(form) = self.state.game_form.as_mut() {
                    form.is_linux = is_linux;
                }
            }
            // `playGame()` (`bridge.py:461-485`). The lookup and its sentence are
            // the reference's first two lines: an id the library does not hold
            // is "Select a game first", not silence — this is the one entry
            // point of the four that says so.
            Message::LaunchGame(game_id) => {
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return self.state.toast_task("Select a game first".to_string());
                };
                let runners = self.state.runner_manager();
                // One worker for both messages, so the "Launching…" report
                // cannot overtake the launch that justifies it and the grace
                // cannot begin before the process exists. The shape is
                // `view::runners::install_runner_task`'s — a thread and a
                // channel, because the work is blocking and there is more than
                // one thing to say — and the receiver's drop ends the stream.
                let (sender, receiver) = unbounded_channel::<Message>();
                std::thread::spawn(move || launch_and_watch(&game, &runners, &sender));
                return cosmic::app::Task::stream(receiver.map(cosmic::Action::App));
            }
            // Everything `playGame` does *after* a successful `launch()`:
            // `mark_played`, the "Launching…" toast, and the close-on-launch
            // hide (`bridge.py:470-476`).
            Message::LaunchStarted { game_id, result } => {
                if let Err(message) = result {
                    return self.state.toast_task(message);
                }
                let name = self.state.game_name(&game_id);
                // The reference's `library.mark_played(game.id)` raises on a
                // failed save, which in a Qt slot means a traceback and no
                // notice. Every other store write in this shell reports instead
                // (`SaveGameForm`), so this one does too — recorded as a
                // divergence in the arm's own words rather than left as a
                // silent swallow.
                let mut tasks = Vec::new();
                if let Err(error) = self.state.library.mark_played(&game_id) {
                    tasks.push(self.state.toast_task(format!(
                        "Could not save “{name}”: {error}"
                    )));
                }
                tasks.push(self.state.toast_task(format!("Launching “{name}”…")));
                if self.state.settings.close_on_launch {
                    tasks.push(hide_window(true));
                }
                return cosmic::app::Task::batch(tasks);
            }
            // `report()` (`bridge.py:478-483`). `None` is the success case and
            // the reference does nothing with it; `Some` is an error nobody can
            // see if close-on-launch hid the window, so it is brought back
            // before the notice is shown.
            Message::LaunchWatchFinished { game_id, reason } => {
                let Some(reason) = reason else {
                    return cosmic::task::none();
                };
                let name = self.state.game_name(&game_id);
                return cosmic::app::Task::batch([
                    hide_window(false),
                    self.state
                        .toast_task(format!("“{name}” stopped right away: {reason}")),
                ]);
            }
            // `runPrefixTool()` (`bridge.py:487-500`). An id the library does
            // not hold returns silently, which is the reference's own first two
            // lines — unlike `playGame`, there is no "Select a game first" here.
            Message::RunPrefixTool { game_id, tool } => {
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return cosmic::task::none();
                };
                if game.is_linux() {
                    return self.state.toast_task(
                        "Prefix tools are only available for Windows games".to_string(),
                    );
                }
                let runners = self.state.runner_manager();
                return cosmic::app::Task::perform(
                    async move {
                        let result = start_prefix_tool(&game, &runners, tool);
                        Message::PrefixToolStarted {
                            game_id: game.id.clone(),
                            tool,
                            result,
                        }
                    },
                    cosmic::Action::App,
                );
            }
            // `Opening {tool} for “{name}”` on success; `str(exc)` on failure,
            // which is the reference's bare exception text with no prefix
            // (`bridge.py:495-499`). `tool` is spelled the way the reference
            // spells it — the `winecfg` / `winetricks` string the QML sends.
            Message::PrefixToolStarted { game_id, tool, result } => {
                let name = self.state.game_name(&game_id);
                let text = match result {
                    Ok(()) => format!("Opening {} for “{name}”", tool.command()),
                    Err(message) => message,
                };
                return self.state.toast_task(text);
            }
            // `openPrefix()` (`bridge.py:503-518`). Silent for an unknown id, a
            // sentence for a Linux game, and no success notice at all: the file
            // manager window that opens *is* the report.
            Message::OpenPrefixFolder(game_id) => {
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return cosmic::task::none();
                };
                if game.is_linux() {
                    return self.state
                        .toast_task("Linux games do not use a Wine prefix".to_string());
                }
                return cosmic::app::Task::perform(
                    async move {
                        let result = open_prefix_folder(&game);
                        Message::PrefixFolderOpened { result }
                    },
                    cosmic::Action::App,
                );
            }
            // The only silent success in the launch flow, and it is the
            // reference's: `QDesktopServices.openUrl` reports nothing when it
            // works (`bridge.py:517`).
            Message::PrefixFolderOpened { result } => {
                return match result {
                    Ok(()) => cosmic::task::none(),
                    Err(message) => self.state.toast_task(message),
                };
            }
            // P-65. On a worker, like every other fork in this file (D-48): the
            // spawn is a `fork`+`exec`, and doing it in `update` would run it in
            // the test process — `every_message` drives this arm, so a
            // synchronous version would launch a browser from `cargo test`.
            Message::OpenUrl(url) => {
                return cosmic::app::Task::perform(
                    async move { Message::UrlOpened { result: open_url(&url) } },
                    cosmic::Action::App,
                );
            }
            // Unlike the prefix folder, whose open has no failure to report in
            // the reference *and* whose directory creation is the reported half,
            // this one has nothing else to say: either the desktop took the URL
            // or the click did nothing, and only the user can tell those apart.
            // See [`Message::UrlOpened`].
            Message::UrlOpened { result } => {
                return match result {
                    Ok(()) => cosmic::task::none(),
                    Err(message) => self.state.toast_task(message),
                };
            }
            // `createShortcut()` (`bridge.py:520-533`). Silent for an unknown id;
            // `shortcut_command` builds the command, so the shortcut reaches this
            // install the same way the reference's reaches its own.
            Message::CreateDesktopShortcut(game_id) => {
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return cosmic::task::none();
                };
                return cosmic::app::Task::perform(
                    async move {
                        let command = shortcut_command(&game);
                        let result = desktop::create_desktop_shortcut(&game, &command, None)
                            .map_err(|error| format!("Could not create the shortcut: {error}"));
                        Message::ShortcutCreated { result }
                    },
                    cosmic::Action::App,
                );
            }
            Message::ShortcutCreated { result } => {
                return match result {
                    Ok(path) => self
                        .state
                        .toast_task(format!("Shortcut created at {}", path.display())),
                    Err(message) => self.state.toast_task(message),
                };
            }
            // `requestHide`/`requestShow` are answered by [`App::update`], which
            // holds the window id — the same split as [`Message::Quit`], and for
            // the same reason. `Shell` has no `Core` and must not grow one: it is
            // a function of `State` alone, which is what makes every arm here
            // callable from a test.
            Message::SetWindowHidden(_hidden) => {}

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
            //
            // The confirm pair (`ConfirmRemoveRunner` sets the pending removal
            // the dialog draws; `RemoveRunnerConfirmed` runs the removal) is
            // routed here for the same reason rather than handled above: the
            // pending state is the runners dialog's, and splitting the pair
            // across files would put the set and the clear in two places. P-37.
            message
                @ (Message::FetchReleases { .. }
                | Message::ReleasesFetchFinished { .. }
                | Message::InstallRunner { .. }
                | Message::RunnerProgress(_)
                | Message::RunnerInstallFinished { .. }
                | Message::ConfirmRemoveRunner { .. }
                | Message::RemoveRunnerConfirmed(_)
                | Message::UninstallRunner(_)
                | Message::RunnersRefreshed { .. }) => {
                return view::runners::update(&mut self.state, &message)
                    .unwrap_or_else(cosmic::app::Task::none);
            }

            // ---- Easy installers -------------------------------------------
            // `_set_installer_search` / `_set_installer_category`
            // (`bridge.py:789-806`), which are the page's own business and are
            // answered by the page. What is *not* the page's business is the
            // catalog those two filters select from: it is derived state on
            // `State`, so the shell recomputes it here rather than leaving the
            // page to write a field it does not own.
            Message::SetInstallerSearch(_) | Message::SetInstallerCategory(_) => {
                let task = view::installers::update(&mut self.state, &message)
                    .unwrap_or_else(cosmic::task::none);
                self.state.refresh_installers();
                return task;
            }
            // The Installers page's runner selector (P-53's first clause, D-55).
            // Local state like the two above, and answered by the page for the
            // same reason — but **not** followed by `refresh_installers`: the
            // seed keeps a choice that is still in the list, so a refresh here
            // would be harmless and a refresh that ever stopped keeping it would
            // silently discard the user's pick on the next keystroke in the
            // search box. The choice is read by the page's `runner_id` and by
            // nothing else.
            Message::SetInstallRunner(_) => {
                return view::installers::update(&mut self.state, &message)
                    .unwrap_or_else(cosmic::task::none);
            }
            // `installEasy` (`bridge.py:831-903`): the guards, then the worker.
            Message::StartEasyInstall {
                installer_id,
                runner_id,
            } => {
                return start_easy_install(&mut self.state, &installer_id, &runner_id);
            }
            // `_progress_cb` → `_set_progress` (`bridge.py:733-735`), the same
            // one-line write the Runners page's `RunnerProgress` is.
            Message::EasyInstallProgress(fraction) => {
                self.state.progress = Some(fraction);
            }
            // `done(result)` (`bridge.py:874-895`): the found branch finishes the
            // install, the other stores it and asks for an executable.
            Message::EasyInstallWizardFinished { found, returncode } => {
                return easy_install_wizard_finished(&mut self.state, found.as_deref(), returncode);
            }
            // `completeEasyInstall` (`bridge.py:921-936`): `as_local_path` of
            // nothing is the cancel path, which is why that check comes first.
            Message::CompleteEasyInstall { token, path } => {
                return complete_easy_install(&mut self.state, &token, path.as_deref());
            }
            // `cancelEasyInstall` (`bridge.py:937-948`).
            Message::CancelEasyInstall(token) => {
                return cancel_easy_install(&mut self.state, &token);
            }
            // `gameInstalled` (`bridge.py:918`). The library add has already
            // happened by the time this arrives — the reference adds first and
            // emits second, and the arm that emits is the arm that adds, so
            // there is no second place for a game to enter the library from.
            //
            // The reference answers the signal in `Main.qml:154-158`, and both
            // halves of that answer are here: `root.showPage("library")` — the
            // user is moved to the entry they just made, which is the only way
            // they can see it landed — and a **long** passive notification
            // carrying a **Play** action, because "installing a store launcher
            // is only half the job — the user still has to open it and sign in,
            // so offer that right here".
            //
            // `show_page` is the one writer of both page records, which is why
            // the navigation is a call to it rather than a write to
            // `state.page`: `Shell::new`'s `debug_assert!(self.pages_agree())`
            // is the check that a second writer would break.
            //
            // # The one thing the reference does that this does not
            //
            // Clicking the toast's action in QQC2 **dismisses the
            // notification** and then runs it. Here the action is
            // `Message::LaunchGame` and the toast stays until its own duration
            // expires; the action button is real, the game launches, and the
            // difference is that the toast can sit there for up to
            // `Duration::Long` afterwards. Closing it too needs a variant
            // carrying both the game id and the `ToastId` the action closure is
            // handed — not built, and named here rather than left as a
            // difference a reader would have to find by launching something.
            Message::EasyInstallFinished { game_id, message } => {
                let navigate = self.show_page(Page::Library);
                let toast = self
                    .state
                    .toasts
                    .push(
                        toaster::Toast::new(message)
                            // `"long"` is QQC2's own word in the reference, and
                            // `toaster::Duration::Long` is 15 s — the toolkit's
                            // spelling of the same idea, not a number chosen
                            // here.
                            .duration(toaster::Duration::Long)
                            .action("Play".to_string(), move |_| {
                                Message::LaunchGame(game_id.clone())
                            }),
                    )
                    .map(cosmic::Action::App);
                return cosmic::app::Task::batch([navigate, toast]);
            }
            // `fail(message)` (`bridge.py:896-900`): clear the guard, reset the
            // bar, and say why — in that order, because a notice over a page
            // that is still disabled reads as a second install being possible.
            Message::EasyInstallFailed { message } => {
                let name = self
                    .state
                    .running_install
                    .take()
                    .map(|record| record.installer_name);
                self.state.easy_busy = false;
                self.state.progress = None;
                let text = match name {
                    Some(name) => format!("Could not install {name}: {message}"),
                    // Unreachable from the worker, which only runs while a
                    // record exists. Reported without a name rather than
                    // dropped, because a failure nobody sees is worse.
                    None => message.clone(),
                };
                return self.state.toast_task(text);
            }

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
            // **Empty by decision, not by omission — and T-29 is the task that
            // decided it.** `architecture.md` §3.3 offers two shapes for the
            // grace watch and names one: "`LaunchedGame.failure(timeout=6.0)` …
            // blocks up to `LAUNCH_GRACE_SECONDS` then returns `None`. That
            // blocking wait moves verbatim into `spawn_blocking` inside the
            // `LaunchGame` task." One grace timeout is what landed
            // (`launch_and_watch` below), so there is no tick to handle and no
            // production code anywhere constructs this variant. The arm stays so
            // that the choice is visible: a poll that starts emitting it fails
            // `dispatch_coverage`, because an emission must not land in a body
            // that does nothing.
            //
            // (The variant itself cannot be deleted yet: `view/runners.rs:1772`
            // constructs it in a test asserting that page *declines* messages it
            // does not own — UX's file, and a reason the deletion is a separate
            // conversation rather than an edit here.)
            Message::LaunchWatchTick => {}
        }
        cosmic::task::none()
    }
}

impl State {
    /// A toast, as the [`Task`](cosmic::Task) the application trait wants.
    ///
    /// `Toasts::push` returns a task of its own — it is what schedules the
    /// toast's expiry — so a handler cannot both push a toast and finish with
    /// `Task::none()`: the toast would appear and never leave. This is the
    /// mapping, in one place rather than repeated in each of the handlers that
    /// report something.
    ///
    /// [`Message::Notify`] does the same thing and is the route a *view* takes,
    /// because a view can only return a message. This is the route an
    /// `update` arm takes, where the text is already in hand and routing it
    /// through the enum would mean a second pass through the match.
    fn toast_task(&mut self, text: String) -> cosmic::app::Task<Message> {
        self.toasts
            .push(cosmic::widget::toaster::Toast::new(text))
            .map(cosmic::Action::App)
    }

    /// The name to put in a sentence about `game_id`, or `""` when the library
    /// no longer holds it.
    ///
    /// A reply can outlive the entry it describes — a delete while a launch is
    /// in flight is the ordinary way — and the reference's sentences are built
    /// from a game object it captured *before* the work started. Re-reading it
    /// here is the closest thing to that capture, and an empty name is what the
    /// reference would print if the object had been mutated in place.
    fn game_name(&self, game_id: &str) -> String {
        self.library
            .get(game_id)
            .map(|game| game.name.clone())
            .unwrap_or_default()
    }

    /// A manager for the configured runners directory, owned rather than
    /// borrowed so it can cross into a worker.
    ///
    /// `RunnerManager` is a `PathBuf` and nothing else, so this is a copy of one
    /// field rather than a second scan: `RunnerManager::new(&SystemLaunchEnv)`
    /// would re-derive the same directory, and deriving it from the field
    /// instead keeps the manager that was configured at startup the only source
    /// of truth for where runners live.
    fn runner_manager(&self) -> RunnerManager {
        RunnerManager::at(self.runners.runners_directory().to_path_buf())
    }
}

// ---------------------------------------------------------------------------
// The launch flow
// ---------------------------------------------------------------------------

/// The hide/show pair the window is driven with.
///
/// `bridge.py`'s `requestHide`/`requestShow` signals (`:159-160`), answered by
/// the view with `visible = false` and `visible = true` + `raise()` +
/// `requestActivate()` (`Main.qml:170-177`). The port's three calls map
/// one-for-one — see [`Application::update`](cosmic::Application::update),
/// which is where they are issued, because only `App` holds the window id.
fn hide_window(hidden: bool) -> cosmic::app::Task<Message> {
    cosmic::app::Task::done(cosmic::Action::App(Message::SetWindowHidden(hidden)))
}

/// `launch(game, RunnerManager())` (`main.py:36`) — the reference's own call.
///
/// Shared by both callers rather than restated in each. The four arguments are
/// the port's substitutions for the modules the reference reads as globals
/// (`SystemLaunchEnv` stands in for `os.environ`, a real `NetpathsShares` for
/// the share map `netpaths` builds at import), and a second spelling of them is
/// a second chance to hand `launch` a different environment — the class of
/// divergence that produces a command that is right in the GUI and wrong from a
/// shortcut.
fn launch_process(game: &Game, runners: &RunnerManager) -> Result<LaunchedGame, RunnerError> {
    launch::launch(game, runners, &SystemLaunchEnv, &NetpathsShares::new(&SystemEnv))
}

/// The grace period `started.failure()` is given, as a `Duration`.
///
/// `LaunchedGame.failure`'s own default (`runners.py:1360`), and the single
/// timeout §3.3 chose over a poll — the same reason [`Message::LaunchWatchTick`]
/// has no producer. Two callers read the constant through this function so that
/// the CLI and the GUI cannot end up watching for different lengths of time,
/// which is the one thing about the grace a user could notice as an
/// inconsistency between a shortcut and the Play button.
fn launch_grace() -> Duration {
    Duration::from_secs_f64(LAUNCH_GRACE_SECONDS)
}

/// `launch()` and then the grace watch, on one worker, reporting both.
///
/// This is `playGame`'s body from `launch(game, self.runner_manager)` down
/// (`bridge.py:464-485`), minus the parts that need state — see
/// [`Message::LaunchStarted`]. It runs on the thread [`Message::LaunchGame`]
/// spawns rather than on the UI thread, and that is the port's one structural
/// divergence here: the reference calls `launch()` from a Qt slot, so its
/// prefix `mkdir`, its DXVK copy and its fork all block the frame. Nothing
/// observable changes — `Popen` returning is the same event either way — and the
/// suite that would have caught a *behavioural* difference is the core module's
/// own, which drives real children.
///
/// # Why this does not call [`launch_report`]
///
/// The CLI's [`launch_game`] and this function drive the same two steps and
/// deliberately do not share the *shape*: the GUI reports between them. It must
/// `mark_played` and say "Launching…" — and may hide the window — the moment
/// the process exists, which is why [`Message::LaunchStarted`] and
/// [`Message::LaunchWatchFinished`] are two messages. The CLI has nobody to
/// report to until it exits, so it fuses them. What the two must not restate is
/// [`launch_process`] and [`launch_grace`], and neither does.
///
/// The two sends are ordered and both are best-effort: a send fails only when
/// the receiver is gone, i.e. the task was dropped, and there is then nobody to
/// report to.
fn launch_and_watch(game: &Game, runners: &RunnerManager, sender: &UnboundedSender<Message>) {
    let mut started = match launch_process(game, runners) {
        Err(error) => {
            // `f"Could not launch “{game.name}”: {exc}"` (`bridge.py:468`).
            let _ = sender.unbounded_send(Message::LaunchStarted {
                game_id: game.id.clone(),
                result: Err(format!("Could not launch “{}”: {error}", game.name)),
            });
            return;
        }
        Ok(started) => started,
    };
    let _ = sender.unbounded_send(Message::LaunchStarted {
        game_id: game.id.clone(),
        result: Ok(()),
    });
    // `started.failure()` with `LaunchedGame.failure`'s own default — see
    // [`launch_grace`].
    let reason = started.failure(launch_grace());
    let _ = sender.unbounded_send(Message::LaunchWatchFinished {
        game_id: game.id.clone(),
        reason,
    });
}

/// `runners::tool_command` followed by `subprocess.Popen` (`bridge.py:492-494`).
///
/// `tool.command()` is the string the reference receives from the QML —
/// `"winecfg"` or `"winetricks"` — so the core function's `UnknownTool` arm is
/// unreachable from here by construction rather than by luck.
fn start_prefix_tool(
    game: &Game,
    runners: &RunnerManager,
    tool: PrefixTool,
) -> Result<(), String> {
    let command = tool_command(game, runners, tool.command(), &SystemLaunchEnv)
        .map_err(|error| error.to_string())?;
    let Some((program, arguments)) = command.argv.split_first() else {
        // Unreachable: both `Ok` arms of `tool_command` push a binary first
        // (`launch.rs:473`, `:479`). Refused rather than asserted, because a
        // panic on a worker is a dead application where a notice is a sentence.
        return Err("the runner produced no command to run".to_string());
    };
    // `subprocess.Popen(argv, env=env)` **replaces** the environment; `envs`
    // alone would layer the map over the inherited one, which is a different
    // child. `command.env` is `env.environ()` plus the prefix variables
    // (`launch.rs:451`), so the replacement is complete.
    std::process::Command::new(program)
        .args(arguments)
        .env_clear()
        .envs(&command.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Detached and unreaped, exactly as `Popen` leaves it — and as
        // `launch()` leaves its own `additional_app` helper. `winecfg` outlives
        // the click that opened it; waiting here would block the worker for as
        // long as the user leaves it open.
        .spawn()
        .map(|_child| ())
        .map_err(|error| error.to_string())
}

/// `openPrefix`'s body from the prefix resolution down (`bridge.py:509-517`).
///
/// The one reportable failure is the directory creation — the reference prints
/// `Could not open the prefix folder: {exc}` and returns *without* opening
/// anything, because a file manager pointed at a path that does not exist is a
/// worse answer than a sentence. `QDesktopServices.openUrl`'s own return value
/// is discarded in the reference (`:517`) and is discarded here.
fn open_prefix_folder(game: &Game) -> Result<(), String> {
    let prefix = prefix_folder(game);
    let target = match prefix_drive_c(&prefix) {
        // `drive_c.parent` — the prefix itself, which is what a user wants to
        // browse when the title installed through Proton's `pfx` indirection.
        Some(drive_c) => drive_c.parent().map(Path::to_path_buf).unwrap_or(prefix),
        None => prefix,
    };
    std::fs::create_dir_all(&target)
        .map_err(|error| format!("Could not open the prefix folder: {error}"))?;
    // `QUrl.fromLocalFile(str(target.resolve()))` — resolved, so a prefix under
    // a symlinked home opens at its real location. `canonicalize` cannot fail
    // here (the directory was just created and its parents exist), and the
    // fallback is the unresolved path rather than an error: the reference has no
    // failure to report at this point.
    let resolved = target.canonicalize().unwrap_or(target);
    let _ = std::process::Command::new("xdg-open")
        .arg(&resolved)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    Ok(())
}

/// The command that opens `url` in the user's browser.
///
/// Split out from [`open_url`] so the argument list can be *read* rather than
/// trusted: `Command` exposes its program and its argv, and what the tests need
/// to see is that the URL is one argv element and not a fragment of a shell
/// line. A URL is attacker-adjacent data in a way a prefix path is not — the
/// credit catalogue is compiled in, but a `String` payload is a `String`
/// payload — and `Command::arg` is what makes `;`, `&` or a backtick a
/// character in a URL instead of a second command.
fn open_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command
}

/// Open `url` in the desktop's browser. P-65.
///
/// `xdg-open` for the same reason [`open_prefix_folder`] uses it, and with the
/// same three `Stdio::null()`s: this process must not hold the child's pipes
/// open, because a browser that inherits them outlives the app that started it.
/// The child is left unreaped for the same reason the prefix folder's is — a
/// browser window is not something to wait four minutes for.
///
/// The `Result` is real rather than decorative: `spawn` fails when `xdg-open`
/// is not on the path, which is the flatpak's failure mode as much as a
/// container's, and `the_open_url_error_arm_is_not_a_fabricated_string` drives
/// that arm with a program that does not exist instead of asserting the string.
fn open_url(url: &str) -> Result<(), String> {
    open_url_command(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_child| ())
        .map_err(|error| format!("Could not open {url}: {error}"))
}

/// A game's Wine prefix: its own `prefix_path`, or `<prefixes_dir>/<id>`.
///
/// `Path(game.prefix_path or str(config.prefixes_dir() / game.id))`
/// (`bridge.py:509`) — Python's `or` is a truthiness test, so the empty string
/// falls through to the default and a whitespace-only path does not. This is
/// `runners::game_prefix`'s rule, restated here because that function is
/// private to `core`: `openPrefix` resolves the same path `launch` does, and the
/// two must not drift.
fn prefix_folder(game: &Game) -> PathBuf {
    if game.prefix_path.is_empty() {
        paths::prefixes_dir().join(&game.id)
    } else {
        PathBuf::from(&game.prefix_path)
    }
}

/// `f"{_launcher_command()} --launch {game.id}"` (`bridge.py:525`).
fn shortcut_command(game: &Game) -> String {
    format!("{} --launch {}", launcher_command(), game.id)
}

/// The command a desktop shortcut runs to reach this install
/// (`_launcher_command`, `bridge.py:91-96`).
///
/// The reference's two branches are kept: a `gamehandler` on `PATH` when there
/// is one, and the bare name otherwise — which is where the port's fallback
/// differs, because `python3 -m gamehandler` has no meaning for a binary
/// (`architecture.md`, §"the fallback becomes just `gamehandler` since the
/// binary is always the launcher").
///
/// The `PATH` **lookup is kept** rather than replaced with
/// `std::env::current_exe()`, which is the honest reading of that note and the
/// more conservative one: a shortcut written from inside a Flatpak or a
/// development build must name the command a *user* has, not the path this
/// process happens to be running from.
///
/// `which` is [`gamehandler_core::plugins::PluginEnv`]'s rather than a second
/// `PATH` scan. It is the tree's `shutil.which` port, down to CPython's
/// empty-`PATH`-entry and default-path rules, which is exactly the lookup the
/// reference performs.
fn launcher_command() -> String {
    let found = SystemPluginEnv.which("gamehandler");
    let Some(found) = found else {
        return "gamehandler".to_string();
    };
    let text = found.to_string_lossy().into_owned();
    // `shlex.quote(found) if " " in found else found` — the *reference* quotes
    // only for a space, so a path holding a `$` or a `"` is written unquoted
    // there and is written unquoted here. That is a defect in the reference
    // (`Exec=` needs the spec's own quoting, and `shlex.quote`'s single quotes
    // are not it), reproduced rather than silently fixed: a shortcut that
    // changes shape under the port is a parity change nobody asked for.
    if text.contains(' ') {
        shlex_quote(&text)
    } else {
        text
    }
}

/// `shlex.quote` (`Lib/shlex.py`), which `_launcher_command` reaches through
/// `shlex.quote`.
///
/// CPython's three cases: the empty string becomes `''`, a string made only of
/// `[A-Za-z0-9_]`, `@%+=:,./-` is returned unchanged, and anything else is
/// single-quoted with each `'` rewritten as `'"'"'`. Only the last case is
/// reachable from [`launcher_command`] — it calls this only for a value
/// containing a space, and a space is not in the safe set — so the other two are
/// here because they are what the function *is*, not because a caller needs
/// them.
fn shlex_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let safe = value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || character == '_'
            || "@%+=:,./-".contains(character)
    });
    if safe {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

// ---------------------------------------------------------------------------
// The easy-install flow (P-53…P-59)
// ---------------------------------------------------------------------------

/// The download's own timeout, `download_installer`'s `timeout: int = 60`
/// (`installers.py:602`).
const INSTALLER_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// `installEasy`'s guards and its setup, then the worker
/// (`bridge.py:832-857`).
///
/// # The four refusals, and which of them speak
///
/// The reference's pre-flight is four checks with three different answers, and
/// the differences are the kind that get flattened by a rewrite:
///
/// - already busy → `"Another install is already running"` (`:833-835`);
/// - an installer id that is not in the catalog → **silence**
///   (`except KeyError: return`, `:836-839`);
/// - a runner that is not installed → `"{name} is not available. Download a
///   runner first."` (`:840-843`);
/// - a prefix that cannot be created → `"Could not create a prefix for {name}:
///   {exc}"` (`:844-847`).
///
/// The third is the one worth reading twice: the runner is resolved *through*
/// `runner_manager.get`, which falls back to system Wine for an id it does not
/// know, so this refusal is about a Wine that is not there and not about a bad
/// parameter.
///
/// # The order that matters
///
/// `_easy_busy` is set **after** the prefix exists (`:849`). Setting it first
/// and clearing it in each of the three `return` arms would work and would be
/// three places to forget; the reference's order is kept because it is the one
/// that cannot leak.
fn start_easy_install(
    state: &mut State,
    installer_id: &str,
    runner_id: &str,
) -> cosmic::app::Task<Message> {
    if state.easy_busy {
        return state.toast_task("Another install is already running".to_string());
    }
    let Ok(installer) = installer_by_id(installer_id) else {
        return cosmic::task::none();
    };
    // `runner_id or self.settings.default_runner` — Python's `or` is a
    // truthiness test, so the empty string means "the default", which is what
    // the page's selector sends before anything has been chosen.
    let resolved = if runner_id.is_empty() {
        state.settings.default_runner.clone()
    } else {
        runner_id.to_string()
    };
    {
        let runner = state.runners.get(&resolved, &SystemLaunchEnv);
        if !runner.is_available() {
            let text = format!("{} is not available. Download a runner first.", runner.name());
            return state.toast_task(text);
        }
    }
    // Generated before the install so a cancelled wizard leaves a prefix named
    // for the entry the user gets if they try again (`installers.py:649-652`).
    let game_id = gamehandler_core::models::new_id();
    let prefix = match prepare_prefix(&game_id) {
        Ok(prefix) => prefix,
        Err(error) => {
            let text = format!("Could not create a prefix for {}: {error}", installer.name);
            return state.toast_task(text);
        }
    };
    // From here the reference is already busy (`:849-852`).
    state.easy_busy = true;
    state.progress = Some(0.0);
    state.running_install = Some(crate::state::PendingInstall {
        installer_id: installer.id.to_string(),
        installer_name: installer.name.to_string(),
        prefix: prefix.clone(),
        runner_id: resolved.clone(),
        game_id,
    });
    let runners = state.runner_manager();
    let (sender, receiver) = unbounded_channel::<Message>();
    std::thread::spawn(move || easy_install_worker(installer, runners, prefix, resolved, &sender));
    // The notice and the work are one task, so the page cannot be busy with
    // nothing on screen saying why — the shape
    // `view::runners::install_runner_task` uses for the same reason.
    cosmic::app::Task::batch([
        state.toast_task(format!("Downloading {}…", installer.name)),
        cosmic::app::Task::stream(receiver.map(cosmic::Action::App)),
    ])
}

/// `installEasy`'s `work()` (`bridge.py:857-872`), on the worker thread.
///
/// Four phases, in the reference's order: download and verify, tell the user
/// the vendor's wizard is starting, run it, then wait for the executable it
/// should have produced. Every failure leaves through
/// [`Message::EasyInstallFailed`], which is the only terminal message that
/// clears the busy guard on this path — see that variant for why the
/// documented set needed it.
///
/// `runner_id` is resolved to a runner here rather than being carried in, for
/// the reason `Message::LaunchGame` re-derives its manager: a `Box<dyn Runner>`
/// is not `Send`, and the manager — which is a `PathBuf` and a directory scan —
/// is.
fn easy_install_worker(
    installer: &'static Installer,
    runners: RunnerManager,
    prefix: PathBuf,
    runner_id: String,
    sender: &UnboundedSender<Message>,
) {
    let launch_env = SystemLaunchEnv;
    let fail = |message: String| {
        let _ = sender.unbounded_send(Message::EasyInstallFailed { message });
    };
    let progress = |fraction: f64| {
        let _ = sender.unbounded_send(Message::EasyInstallProgress(fraction as f32));
    };
    let archive = match download_installer(
        installer,
        &paths::downloads_dir(),
        Some(&progress),
        INSTALLER_DOWNLOAD_TIMEOUT,
        &crate::http::UreqClient,
        &launch_env,
    ) {
        Ok(archive) => archive,
        Err(error) => return fail(error.to_string()),
    };
    // `bridge.py:859-865`, emitted from the worker because it must arrive
    // *after* the download: the point of the sentence is that the vendor's
    // window is about to appear, and a notice that precedes a two-minute
    // download says the opposite.
    let _ = sender.unbounded_send(Message::Notify(format!(
        "Launching the {} installer… Finish the vendor wizard, then close it — \
         GameHandler adds it as soon as the install lands.",
        installer.name
    )));
    let runner = runners.get(&runner_id, &launch_env);
    let command =
        match build_installer_command(&*runner, &prefix, installer, &archive, &launch_env) {
            Ok(command) => command,
            Err(error) => return fail(error.to_string()),
        };
    // `Path(prefix).mkdir(parents=True, exist_ok=True)` (`bridge.py:867`),
    // which is redundant here — `prepare_prefix` made it — and is kept because
    // the reference's line sits between the two steps that need it.
    let _ = std::fs::create_dir_all(&prefix);
    // `subprocess.run(argv, env=env, cwd=str(archive.parent), check=False)`
    // (`:868-870`): the *download's* directory, which is where an installer that
    // resolves its payload relative to its own location finds it.
    let cwd = archive
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let returncode = match run_installer(&command, &cwd) {
        Ok(code) => code,
        Err(error) => return fail(error.to_string()),
    };
    // `wait_for_installer(runner, env, prefix, installer.expected_exe)`
    // (`:871`) — P-56's loop: poll for the expected executable, slice the
    // wineserver wait so a leftover store client cannot hide a finished
    // install, and give up at `INSTALL_SETTLE_TIMEOUT_SECONDS` (six hours).
    let found = wait_for_installer(
        &*runner,
        &command.env,
        &prefix,
        installer.expected_exe,
        &SystemClock,
        &|runner, env, seconds| {
            wait_for_prefix_idle(runner, env, seconds, &SystemLaunchEnv)
        },
    );
    let _ = sender.unbounded_send(Message::EasyInstallWizardFinished { found, returncode });
}

/// The vendor's wizard as a child process, inherited stdio and all.
///
/// `subprocess.run(argv, env=env, cwd=…, check=False)` — so the environment is
/// **replaced**, not layered (`env_clear` first, as in `start_prefix_tool`),
/// and the exit status is reported rather than turned into an error.
///
/// A child killed by a signal has no exit code; `-1` is what
/// `core::runners::launch` reports for the same case and for the same reason
/// (Python's `returncode` would be the negated signal, a number that names
/// nothing a user can act on).
fn run_installer(command: &gamehandler_core::runners::Command, cwd: &Path) -> std::io::Result<i32> {
    let Some((program, arguments)) = command.argv.split_first() else {
        // Unreachable: `build_installer_command` refuses an empty argv with
        // `InstallerError::EmptyCommand` before returning.
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the runner produced no command to run",
        ));
    };
    let status = std::process::Command::new(program)
        .args(arguments)
        .env_clear()
        .envs(&command.env)
        .current_dir(cwd)
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

/// `done(result)` (`bridge.py:874-895`): the found branch finishes the install,
/// the other one stores it and asks the user for an executable.
///
/// The not-found branch leaves `easy_busy` **true**, which is the reference's
/// own asymmetry and not an oversight: the install is not abandoned, it is
/// waiting for the user, and `completeEasyInstall`/`cancelEasyInstall` are the
/// two ways it ends. A port that cleared the guard here would let a second
/// install start on top of a pending one — P-59's guard, defeated by the only
/// path that has something to lose.
fn easy_install_wizard_finished(
    state: &mut State,
    found: Option<&Path>,
    returncode: i32,
) -> cosmic::app::Task<Message> {
    // A reply whose install is gone — the task was dropped, or a cancel raced
    // it. Nothing to interpret it against, so nothing is reported: this is the
    // same silence `Message::LaunchWatchFinished` keeps for a game the library
    // no longer holds.
    let Some(record) = state.running_install.clone() else {
        return cosmic::task::none();
    };
    let Some(executable) = found else {
        // `suffix = f" (installer exited {returncode})" if returncode else ""`
        // — a zero exit says nothing worth appending.
        let suffix = if returncode != 0 {
            format!(" (installer exited {returncode})")
        } else {
            String::new()
        };
        let text = format!(
            "Could not find the {} executable in the prefix{suffix}. Pick it \
             yourself if the install finished.",
            record.installer_name
        );
        state.running_install = None;
        // Keyed by the game id, which is what the reference uses as the token
        // (`bridge.py:888-889`).
        state.easy_pending.insert(record.game_id.clone(), record);
        return state.toast_task(text);
    };
    finish_easy_install(state, &record, executable)
}

/// `completeEasyInstall` (`bridge.py:921-936`).
///
/// The pending entry is popped **first**, and that order is the reference's: a
/// token nobody holds is a silent return that clears no guard, and the cancel
/// this falls into when the user picked nothing finds a map that no longer has
/// the entry — so the kept-prefix notice does *not* fire on this path. That is
/// the reference's behaviour, measured (`:923` pops, `:947` toasts only when
/// the pop found something), and it is kept rather than tidied into the more
/// obvious "cancel always toasts".
fn complete_easy_install(
    state: &mut State,
    token: &str,
    path: Option<&str>,
) -> cosmic::app::Task<Message> {
    // `pending = self._pending_installs.pop(token, None)` (`:923`), the pop
    // itself and not a lookup followed by one.
    let Some(record) = state.easy_pending.remove(token) else {
        return cosmic::task::none();
    };
    match path {
        // `as_local_path(file_url)` — the picker hands a path, and the URL
        // decoding the reference does on the way is the chooser's own job
        // (T-15); what reaches here is already the path or nothing.
        Some(path) if !path.is_empty() => finish_easy_install(state, &record, Path::new(path)),
        // The cancel of a token that has already been popped, which is the
        // reference's own shape: `cancelEasyInstall` clears the guards either
        // way and finds nothing to toast about.
        _ => cancel_easy_install(state, token),
    }
}

/// `cancelEasyInstall` (`bridge.py:937-948`).
///
/// The guards are cleared whether or not a pending install was found: the
/// reference clears them before the `if pending` (`:940-943`), so a cancel for a
/// token that is not there still un-busies the page. That is the behaviour that
/// makes this function the *only* way out of the busy state the not-found branch
/// leaves behind, so getting it wrong is a page stuck disabled forever.
fn cancel_easy_install(state: &mut State, token: &str) -> cosmic::app::Task<Message> {
    let record = state.easy_pending.remove(token);
    state.easy_busy = false;
    state.progress = None;
    state.running_install = None;
    let Some(record) = record else {
        return cosmic::task::none();
    };
    let text = format!(
        "Kept the {} prefix. Add it later from Add Game if you want.",
        record.installer_name
    );
    state.toast_task(text)
}

/// `_finish_easy_install` (`bridge.py:905-919`): the entry, the guards, and the
/// signal.
///
/// # What is not here, and why it is named rather than skipped
///
/// `bridge.py:907-911` sets the entry's cover to the executable's own icon —
/// `game.cover_path = str(save_exe_icon(exe_path, game_id))`, with an empty
/// string when that raises. **`save_exe_icon` has no counterpart in this tree:**
/// it is `covers.py:307-317` over `exe_icons.extract_icon`, and `core::exe_icons`
/// does not exist (`crates/core/src/lib.rs` names it as planned and nothing
/// else does). So the entry is created with no cover, which is the reference's
/// *own* fallback for a file whose icon cannot be read — a real state, not a
/// placeholder — and P-58's "with the vendor icon" is therefore **unmet**, not
/// deferred to Phase 3. The port that closes it is `core::exe_icons`, which
/// task T-05 owns.
fn finish_easy_install(
    state: &mut State,
    record: &crate::state::PendingInstall,
    executable: &Path,
) -> cosmic::app::Task<Message> {
    let Ok(installer) = installer_by_id(&record.installer_id) else {
        // The record names an id the catalog does not have, which cannot happen
        // from `StartEasyInstall` — it resolves the recipe before writing the
        // record. Reported as a failure rather than unwrapped, because the
        // guards below have to run either way.
        let message = format!("unknown installer {}", record.installer_id);
        state.easy_busy = false;
        state.progress = None;
        state.running_install = None;
        return state.toast_task(format!("Could not install {}: {message}", record.installer_name));
    };
    let game = game_from_install(
        installer,
        executable,
        &record.prefix,
        &record.runner_id,
        Some(&record.game_id),
    );
    let game_id = game.id.clone();
    let name = game.name.clone();
    let added = state.library.add(game);
    state.easy_busy = false;
    state.progress = None;
    state.running_install = None;
    // The reference's `_library_updated()` follows the add; here the library is
    // read by the view directly, so the only thing left to tell anyone is the
    // shell — and a failed save is reported instead of raised, which is the same
    // divergence `Message::LaunchStarted` records for `mark_played`.
    if let Err(error) = added {
        return state.toast_task(format!("Could not save “{name}”: {error}"));
    }
    // `gameInstalled.emit(game.id, f"Installed “{game.name}”")` (`:918`) — the
    // signal, as a message, which is what carries the id to the toast's Play
    // action.
    cosmic::app::Task::done(cosmic::Action::App(Message::EasyInstallFinished {
        game_id,
        message: format!("Installed “{name}”"),
    }))
}

/// The pages whose body is still a placeholder: the task that will build each.
///
/// **Empty, and T-38 emptied it.** It held one entry — `(Page::Installers,
/// "T-12")` — which was the last page still routed through `pending_page`; the
/// install flow landed that page, and the entry and the placeholder function
/// went with it.
///
/// # The entry named a task that was not the one that removed it
///
/// `"T-12"` was right when the line was written and is the reason the two ids
/// have to be read apart rather than reconciled into one. T-12 built
/// `view/installers.rs` and deliberately left the page pinned, because at that
/// point seven of its nine P-items were unmet and dispatching it would have
/// deleted the last entry in this list while the page underneath was inert —
/// `#68`. T-38 is the install flow itself, so it is the task that meets those
/// items and the task that owns the deletion. A reader who finds `T-12` in the
/// git log of the line this doc describes is looking at the pin, not at the
/// landing.
///
/// # Why this existed
///
/// T-08 is the shell: the sidebar, the routing and the toaster. Each page's own
/// content was a later task, and until it landed that page's body said so **on
/// screen** as well as in the code — a page that rendered an empty body would
/// read as a bug in the shell, which is the wrong thing to go looking for.
///
/// The danger in a placeholder is not that it is wrong but that it is
/// **invisible**: it compiles, it renders, and it satisfies every test that
/// checks the process came up. `gui-stays-up` in `scripts/smoke-test.sh` asserts
/// exactly that and nothing more, so a build whose whole interface is six
/// placeholders passed the gate suite. This list is what made the placeholders
/// countable, and `the_pending_pages_are_exactly_the_ones_whose_body_says_so`
/// is what counts them — against the rendered body, not against this list.
///
/// # Landing a page was one deletion
///
/// This is a slice, not a `[(Page, &str); N]`, and that was deliberate: a
/// fixed-length array would have made removing a line a two-part edit — delete
/// the line, then correct the length — and the second part is mechanical. A
/// mechanical edit forced by a failure message that says the count is pinned
/// "deliberately" is how a gate stops meaning anything: the reader learns to
/// clear red by editing a constant. With a slice, the one edit was the edit
/// that carried the meaning, and the test above holds the other half — it reads
/// what `view_body` actually draws.
///
/// # Why the empty slice and `pending_task` survive the deletion
///
/// The list is now `[]` and every arm of [`Shell::view_body`] draws a real page,
/// so `pending_task` returns `None` for all nine and the test above asserts, for
/// each of them, that the body does **not** draw the placeholder. That is a
/// claim with content: it fails the moment a page is moved back behind a
/// placeholder, which is exactly the regression the guard was built for. The
/// const and the function stay so that the *mechanism* is still the thing under
/// test rather than a description of it — deleting them would leave the test
/// asserting a property of a table that no longer exists.
///
/// **The placeholder function itself is gone**, with the last entry, as this
/// doc used to say it would be. `crates/app/tests/pending_pages.rs` locates a
/// page's arm by the text `pending_page` in `view_body`, so a page cannot
/// quietly be sent back to a placeholder: there is no longer a function to send
/// it to.
#[cfg(test)]
const PENDING_PAGES: &[(Page, &str)] = &[];

/// The task that will build `page`, or `None` once its body has landed.
#[cfg(test)]
fn pending_task(page: Page) -> Option<&'static str> {
    PENDING_PAGES
        .iter()
        .find(|(pending, _)| *pending == page)
        .map(|(_, task)| *task)
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
        // And the same for the Installers page, whose *four* arguments are the
        // same kind of cache. This line was missing until the D-55 repair and
        // `Shell::new` had it, so every test that renders the page passed while
        // the shipped app drew "No matching installers" over a catalog of nine,
        // and an empty runner selector beside it, until the user typed in the
        // search box. A test-side primer that production does not share is a
        // test that is not measuring production.
        state.refresh_installers();
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

        // `main.py:106`'s `theme.apply(backend.settings.color_scheme)` — the
        // startup half of P-67, and the same `theme::apply` the change handler
        // calls. It is a task here rather than a field on
        // `cosmic::app::Settings` because that field is `pub(crate)` with no
        // builder; `theme.rs`'s module doc records the measurement.
        //
        // Batched with the title rather than returned instead of it: both are
        // one-shot startup work and neither depends on the other.
        let startup_theme = theme::apply(&app.shell.state.settings.color_scheme);

        (app, cosmic::app::Task::batch([title, startup_theme]))
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

    /// P-68, the three accelerators libcosmic does not bind.
    ///
    /// `keyboard::listen()` delivers an event only when no widget took it
    /// (`iced/futures/src/keyboard.rs:9-19`), so a focused text field keeps its
    /// own keystrokes — which is what [`shortcuts`]' own doc records, along with
    /// why `Ctrl+F` is absent here: the framework already binds it and routes it
    /// to [`cosmic::Application::on_search`] below, and binding it twice would
    /// fire it twice.
    ///
    /// The mapping is [`shortcuts::shortcut_for`], a pure function, because this
    /// one is not testable: a `Subscription` has no accessor, so nothing can
    /// read back what was composed here.
    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        cosmic::iced::keyboard::listen().filter_map(|event| shortcuts::shortcut_for(&event))
    }

    /// `Ctrl+F` — libcosmic's own `Action::Search`, emitted by
    /// `keyboard_nav::subscription()` (`src/keyboard_nav.rs:50-55`) and routed
    /// here by `Cosmic::update` (`src/app/cosmic.rs:850`).
    ///
    /// Delegated so the navigation half is testable; see
    /// [`Shell::focus_library_search`], which also records why the returned
    /// focus operation reaches the search box at all.
    ///
    /// One divergence worth naming: libcosmic matches `Character("f")` with
    /// Control and does **not** reject Shift, so `Ctrl+Shift+F` reaches this
    /// hook, where Qt's `Shortcut` would not match it. It is the framework's
    /// binding rather than one written here; the alternative — a second binding
    /// in [`subscription`](cosmic::Application::subscription) with the
    /// exact-modifier guard — would fire the same key twice, which is worse than
    /// being liberal in this one case.
    fn on_search(&mut self) -> cosmic::app::Task<Self::Message> {
        self.shell.focus_library_search()
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
        // The other two that need it, and they are the same need: `requestHide`
        // and `requestShow` act on the framework's window, which `Shell` has no
        // handle on. The reference answers them with `visible = false` and
        // `visible = true` + `raise()` + `requestActivate()`
        // (`Main.qml:170-177`); iced's `minimize` is its hide and its unminimize
        // with `gain_focus` is its show-and-raise — the same pair libcosmic
        // itself uses to bring a window back.
        //
        // `Message::SetWindowHidden` is deliberately *not* answered in
        // `Shell::update`: a shell that could move the window would need a
        // `Core`, and every arm in it is callable from a test precisely because
        // it has none. The empty arm there carries the note.
        if let Message::SetWindowHidden(hidden) = &message {
            let Some(id) = self.core.main_window_id() else {
                return cosmic::task::none();
            };
            return match hidden {
                true => cosmic::iced::window::minimize(id, true),
                false => cosmic::app::Task::batch([
                    cosmic::iced::window::minimize(id, false),
                    cosmic::iced::window::gain_focus(id),
                ]),
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
        // is [`Shell::view_with_overlays`], which a test can call without an
        // `App` and which falls through to [`Shell::view_body`] when nothing is
        // open above it.
        toaster(&self.shell.state.toasts, self.shell.view_with_overlays())
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
        assert_eq!(
            library_lookup(&library, "nope"),
            Err("GameHandler: no game with id nope".to_string())
        );
    }

    /// A game that **is** in the library is not reported as missing.
    ///
    /// This is the inverted-criterion bug again, one level down: the old stub
    /// printed the unknown-id message for every id, including real ones, which
    /// is a fabricated fact about the user's library. A real id must resolve,
    /// and the message a real launch can print must never be the miss sentence.
    #[test]
    fn launching_a_known_id_does_not_claim_the_game_is_missing() {
        let (_root, library) = library_with("known", &[("a", "Alpha")]);
        let game = library_lookup(&library, "a").expect("a real id must resolve");
        assert_eq!(game.name, "Alpha");
        // And no arm of the report can produce the miss sentence for it.
        for attempt in [
            Err("the runner produced no command".to_string()),
            Ok(Some("exit status 2".to_string())),
            Ok(None),
        ] {
            let (message, _) = launch_report(&game.name, attempt);
            let message = message.unwrap_or_default();
            assert!(
                !message.contains("no game with id"),
                "a real game was reported as missing: {message}"
            );
        }
    }

    /// **Only a title that was still running when the grace expired exits 0.**
    ///
    /// This is the property that replaced `no_unported_launch_path_reports_success`
    /// when the stub went away, and it is the stronger form of the same
    /// criterion: the old test could only require that *every* path failed,
    /// because every path did. The failure it guards now is the sharper one a
    /// real launch makes possible — an arm that returns `ExitCode::SUCCESS` for
    /// a launch that did not happen, which is `#31`'s original defect (a stub
    /// that printed a plausible success) one layer down, in `.desktop`
    /// shortcuts whose whole job is to open something.
    ///
    /// `Err` is `launch()` raising and `Ok(Some(_))` is `started.failure()`
    /// finding a reason; neither is a running game, and both must be non-zero
    /// **and** say something on stderr, because a shortcut that exits 1 in
    /// silence is the other half of the same complaint.
    #[test]
    fn only_a_title_that_stayed_up_reports_success() {
        let failures = [
            Err("no Proton runner is configured".to_string()),
            Ok(Some("exit status 1".to_string())),
        ];
        for attempt in failures {
            let (message, code) = launch_report("Alpha", attempt);
            assert_ne!(code, 0, "a launch that did not stay up reported success");
            assert!(
                message.is_some_and(|text| !text.is_empty()),
                "a non-zero exit said nothing on stderr"
            );
        }
        // The control arm: the one case that *is* a running game. Without it
        // the assertions above would pass on a function that failed everything,
        // which is the stub this task deleted.
        assert_eq!(launch_report("Alpha", Ok(None)), (None, 0));
    }

    /// The three sentences are the reference's, verbatim.
    ///
    /// Pinned as whole strings rather than as substrings, and that is the point
    /// of [`launch_report`] being pure: a shortcut's stderr is what a user
    /// pastes into a bug report, and `{name}` appearing in the right place is
    /// not checkable by `contains`.
    ///
    /// The `_` in the second case is deliberate and is not a typographic quote:
    /// the CLI's sentence is lowercase with a plain colon (`main.py:38`) where
    /// the GUI's is capitalised with `“…”` (`bridge.py:468`). A port that
    /// unified them would change what this verb prints, so the difference is
    /// asserted rather than smoothed over.
    #[test]
    fn the_three_cli_sentences_are_pythons() {
        assert_eq!(
            launch_report("Alpha", Err("no Proton runner is configured".to_string())),
            (
                Some("GameHandler: could not launch Alpha: no Proton runner is configured".to_string()),
                1
            )
        );
        assert_eq!(
            launch_report("Alpha", Ok(Some("exit status 1".to_string()))),
            (
                Some("GameHandler: Alpha stopped right away: exit status 1".to_string()),
                1
            )
        );
        assert_eq!(launch_report("Alpha", Ok(None)), (None, 0));
    }

    /// **A shortcut's `Exec=` line parses back into `--launch <id>`, through
    /// this binary's own argument parser.**
    ///
    /// #90b: nothing pinned this, and the gap was measured. Deleting
    /// `--launch` from [`shortcut_command`] left the whole app suite green
    /// (280 passed), because the core tests pin the `.desktop` **format** while
    /// taking the `Exec` string as a literal parameter — the one string that
    /// has to be right is the one those tests take on trust.
    /// `CreateDesktopShortcut` is the only writer of that string, so nothing
    /// downstream could catch it: the `.desktop` file is written,
    /// `desktop-file-validate` passes, and the menu entry opens the *window*
    /// instead of the game. That is P-71 unmet, in the one place a user is
    /// guaranteed to look.
    ///
    /// # Why the assertion is a parse rather than a `contains`
    ///
    /// `command.contains("--launch")` passes on a command that named the verb
    /// twice, named it with no id, or named the *name* where the id belongs —
    /// and the third of those is a real bug class here, because
    /// [`library_lookup`] takes an id, so a shortcut carrying a name would exit
    /// 1 with "no game with id Steam". Feeding the arguments through [`Cli`] is
    /// the same check a `.desktop` file performs, and it is the parser the
    /// shortcut's command is *for*: a renamed verb in the `Cli` definition goes
    /// red here rather than six months later on a user's desktop.
    ///
    /// `argv[0]` is dropped and replaced, and that is deliberate rather than
    /// sloppy: [`launcher_command`] may return a `shlex.quote`d path containing
    /// a space, which no whitespace split can recover — a defect the reference
    /// has too, recorded on that function. The launcher is asserted separately,
    /// as a prefix, so nothing is left unchecked; what is parsed here is the
    /// argument list, which is what clap reads.
    #[test]
    fn a_shortcut_parses_back_into_the_launch_verb_and_the_games_id() {
        let mut game = Game::new_named("Alpha");
        game.id = "11111111111111111111111111111111".to_string();
        let command = shortcut_command(&game);

        assert!(
            command.starts_with(&launcher_command()),
            "the shortcut must name the launcher first: {command:?}"
        );

        let args: Vec<&str> = command.split_whitespace().skip(1).collect();
        assert!(
            !args.is_empty(),
            "the shortcut passes no arguments: {command:?}"
        );
        let cli = Cli::try_parse_from(std::iter::once("gamehandler").chain(args.iter().copied()))
            .unwrap_or_else(|error| {
                panic!("{command:?} is not a command this binary accepts: {error}")
            });

        assert!(!cli.list, "a shortcut must not ask for the listing");
        assert_eq!(
            cli.launch.as_deref(),
            Some(game.id.as_str()),
            "the shortcut's arguments parsed as launch={:?}; a `.desktop` file \
             with this Exec opens the window instead of the game",
            cli.launch
        );
    }

    /// The shortcut names the game by **id**, not by name — the discriminating
    /// case, since two games can share a name.
    ///
    /// Two identical names with different ids is the only pair that tells the
    /// two implementations apart: a shortcut built from `game.name` produces
    /// the *same* string for both, so equality of the two commands is the
    /// observation. (The parse test above would also catch it, by a different
    /// route; this one is here because its failure names the cause.)
    #[test]
    fn two_games_with_one_name_get_two_shortcuts() {
        let mut first = Game::new_named("Alpha");
        first.id = "11111111111111111111111111111111".to_string();
        let mut second = Game::new_named("Alpha");
        second.id = "22222222222222222222222222222222".to_string();

        assert_ne!(
            shortcut_command(&first),
            shortcut_command(&second),
            "both shortcuts are identical, so the command names something the \
             two games share — the name, which no lookup accepts"
        );
    }

    /// A `kind: linux` game whose executable is a real binary, in its own temp
    /// library, with a runner manager pointed at an empty directory.
    ///
    /// A native title is what makes a launch drivable **in process**:
    /// `build_linux_command` (`launch_opts.rs:781`) is `argv = [exe_path]` plus
    /// the inherited environment, so there is no Wine, no runner lookup and no
    /// prefix — the two things that would otherwise make this a test that needs
    /// a machine set up a particular way. A Windows title would need a Proton
    /// runner and a `pfx`, which is exactly what a unit test must not assume.
    ///
    /// `exe` is a parameter rather than a hard-coded `/bin/true` so the two arms
    /// below (`it exits 0` / `it exits non-zero`) are the same fixture with one
    /// value changed, which is what makes the pair a control for each other.
    ///
    /// The runner manager points at a directory that does not exist, and that is
    /// deliberate: a native title must never consult it, so if a future change
    /// routes `kind: linux` through the runner lookup, this fixture's launch
    /// fails loudly instead of silently finding the developer's own Proton.
    fn native_game_library(label: &str, exe: &str) -> (std::path::PathBuf, Library, RunnerManager) {
        let (_root, mut library) = library_with(label, &[("native", "Alpha")]);
        let game = library.get("native").expect("the fixture just added it");
        let mut game = game.clone();
        game.kind = "linux".to_string();
        game.exe_path = exe.to_string();
        library.update(game).unwrap();
        let runners = RunnerManager::at(
            std::env::temp_dir().join(format!("gh-no-runners-{}", std::process::id())),
        );
        // `library_with` returns the root that owns the temp directory, and it
        // is moved out here as the caller's handle rather than dropped: dropping
        // it would delete the directory `library` still points at.
        let root = library.path().parent().unwrap().to_path_buf();
        (root, library, runners)
    }

    /// **`--launch` on a title that stays up exits 0, and records the game as
    /// played.**
    ///
    /// This is the end-to-end half of T-07, and it exists because of a mutation
    /// that survived: **deleting the whole `mark_played` call left the suite
    /// green**, 298 passed, because every test of this verb stopped at
    /// [`launch_report`] and nothing reached the line between the launch and the
    /// watch. That is the "a symbol nobody reaches can be perfect and dead"
    /// shape, so the seam [`launch_game_at`] exists to make the line reachable
    /// rather than to make the test easier to write.
    ///
    /// It really launches: `/bin/true` is exec'd, `started.failure(6s)` observes
    /// it exit 0, the library is saved, and `ExitCode::SUCCESS` comes back. The
    /// two assertions that matter are the return code — `#31`'s whole subject is
    /// a `--launch` that reported the wrong one — and `last_played`, which is
    /// read back **from the file**, not from the in-memory library, so the
    /// `save()` inside `mark_played` is covered too and not just the field
    /// write.
    ///
    /// `/bin/true` is coreutils and present on every Linux; if it were not, the
    /// spawn would fail and this test would fail loudly with the runner's own
    /// message rather than passing vacuously.
    #[test]
    fn a_real_launch_records_the_game_as_played() {
        let (root, mut library, runners) = native_game_library("launch-ok", "/bin/true");
        let before = library.get("native").unwrap().last_played;
        assert_eq!(before, 0.0, "the fixture should start unplayed");

        let code = launch_game_at(&mut library, &runners, "native");
        assert_eq!(code, ExitCode::SUCCESS, "a title that stayed up exited non-zero");

        // Read back through a **fresh** Library over the same file: an
        // in-memory read would pass on a handler that set the field and never
        // saved, which is the whole of `mark_played`.
        let reread = Library::new_at(Some(root.join("games.json")), 0.0);
        let played = reread.get("native").expect("the game should still be there");
        assert_ne!(
            played.last_played, before,
            "the game was launched and never marked as played"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A title that stops right away exits non-zero, and the record of the
    /// attempt survives it.**
    ///
    /// The control for the test above with one value changed (`/bin/false`
    /// instead of `/bin/true`), so the pair cannot both pass on a function that
    /// ignores the child. It also pins the reference's *effect*: `mark_played`
    /// (`main.py:40`) runs before `started.failure()` (`:42`), so a title that
    /// dies still counts as having been played — the launch did happen.
    ///
    /// # What this pair does *not* catch, stated rather than implied
    ///
    /// **The order itself.** Both arms record the game, so moving the
    /// `mark_played` call to *after* the watch leaves these two green. The
    /// order matters only for a title that takes longer to die than
    /// `LAUNCH_GRACE_SECONDS` costs to expire, and no deterministic test can
    /// produce that without sleeping for six seconds. The order is therefore
    /// recorded from the reference rather than asserted, and this paragraph is
    /// the record — a reader who needs it proved should reach for
    /// `started.child_mut()`, which the port exposes for exactly that kind of
    /// caller.
    ///
    /// **The length of the grace period.** [`launch_grace`] is a one-line
    /// wrapper over `LAUNCH_GRACE_SECONDS`, and both fixtures here exit within
    /// microseconds of the spawn, so a shortened timeout would still observe
    /// them. Its value is pinned by
    /// [`the_grace_period_is_the_references`] rather than by anything below;
    /// what *this* pair pins is that both callers read it through the same
    /// function.
    ///
    /// This paragraph said the value "is pinned by `core`'s own tests" until
    /// `P-46`'s walk measured that claim false — `grep -rn grace
    /// crates/core/src/` finds the constant's definition and its two uses in
    /// `launch.rs` and no test asserting its value. A doc comment resting on a
    /// test that does not exist is the defect class this project keeps writing
    /// about, so it is corrected here and the test it named now exists, one
    /// function below.
    /// **The grace period is the reference's own number.**
    ///
    /// `LAUNCH_GRACE_SECONDS = 6.0` (`runners.py:1295`) is the default
    /// `LaunchedGame.failure` is called with (`:1360`), and the port carries it
    /// as a `core` constant that this file reads through [`launch_grace`]. Two
    /// things could go wrong and neither is visible from a launch test — both
    /// fixtures in this module exit in microseconds, so a shortened grace would
    /// still observe them: the constant could drift from the reference's value,
    /// and [`launch_grace`] could stop being a faithful conversion of it.
    ///
    /// The reference is read off disk rather than transcribed, so this fails on
    /// a Python-side change to the number as well as a Rust-side one. That is
    /// the point: the value is the *reference's*, and the port's job is to
    /// follow it.
    #[test]
    fn the_grace_period_is_the_references() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../gamehandler/runners.py");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
        let line = source
            .lines()
            .find(|line| line.starts_with("LAUNCH_GRACE_SECONDS"))
            .unwrap_or_else(|| panic!("{} no longer declares LAUNCH_GRACE_SECONDS", path.display()));
        let value = line
            .split('=')
            .nth(1)
            .and_then(|value| value.trim().trim_end_matches(';').parse::<f64>().ok())
            .unwrap_or_else(|| panic!("`{line}` is not a float assignment"));
        assert_eq!(
            LAUNCH_GRACE_SECONDS, value,
            "the port's grace period has drifted from `{}`",
            path.display()
        );
        assert_eq!(
            launch_grace(),
            Duration::from_secs_f64(value),
            "`launch_grace` must be that constant and nothing else, or the two \
             callers are not watching for the length the reference does"
        );
    }

    #[test]
    fn a_launch_that_stops_right_away_exits_non_zero_and_still_records_it() {
        let (root, mut library, runners) = native_game_library("launch-fail", "/bin/false");

        let code = launch_game_at(&mut library, &runners, "native");
        assert_ne!(
            code,
            ExitCode::SUCCESS,
            "a title that exited 1 was reported as a successful launch"
        );
        let reread = Library::new_at(Some(root.join("games.json")), 0.0);
        assert_ne!(
            reread.get("native").unwrap().last_played,
            0.0,
            "`mark_played` runs before the grace watch in the reference; a title \
             that died was still launched"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An id the library does not hold exits 1 **before** any launch is
    /// attempted, driven through the same entry point the binary uses.
    ///
    /// The pure [`library_lookup`] test above pins the sentence; this pins that
    /// `launch_game_at` calls it first and does not fall through to a launch.
    #[test]
    fn an_unknown_id_exits_before_launching() {
        let (root, mut library, runners) = native_game_library("launch-missing", "/bin/true");
        let code = launch_game_at(&mut library, &runners, "not-in-this-library");
        assert_ne!(code, ExitCode::SUCCESS);
        assert_eq!(
            library.get("native").unwrap().last_played,
            0.0,
            "an unknown id must not touch the library"
        );
        let _ = std::fs::remove_dir_all(&root);
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
        // `true`, not `false`: `LaunchStarted` emits `true` when close-on-launch
        // is set and `false` when it is not, so `true` is the sample a mutation
        // of that field would have to move. Neither value does anything in this
        // shell — the arm is `App::update`'s — so this sample is here to make the
        // variant constructible, not to be measured.
        Message::SetWindowHidden(_) => ("SetWindowHidden", Message::SetWindowHidden(true)),
        Message::SetColorScheme(_) => ("SetColorScheme", Message::SetColorScheme("light".to_string())),
        // A value *other than the default*: on a default shell `"grid"` and
        // `"name"` write what is already there, which makes a guarded setter
        // and an unwritten one produce the same silence — the defect D-34
        // names, in the samples rather than in the handler.
        Message::SetViewMode(_) => ("SetViewMode", Message::SetViewMode("list".to_string())),
        Message::SetSortMode(_) => ("SetSortMode", Message::SetSortMode("recent".to_string())),
        Message::SetDefaultRunner(_) => ("SetDefaultRunner", Message::SetDefaultRunner("proton-ge".to_string())),
        // A runner no default can already be: `State::installer_runner` is
        // seeded from `settings.default_runner` or from `choices()`' first
        // entry, which is System Wine's id or a Proton version's, never this.
        // A sample equal to the seeded value would write what is already there
        // and read as an unwritten arm — D-34.
        Message::SetInstallRunner(_) => ("SetInstallRunner", Message::SetInstallRunner("GE-Proton9-5".to_string())),
        Message::SetCloseOnLaunch(_) => ("SetCloseOnLaunch", Message::SetCloseOnLaunch(true)),
        Message::SetDefaultToggle { .. } => ("SetDefaultToggle", Message::SetDefaultToggle {
                            name: "mangohud".to_string(),
                            value: true,
                        }),
        Message::SetSearchText(_) => ("SetSearchText", Message::SetSearchText("half".to_string())),
        Message::SetCategoryFilter(_) => ("SetCategoryFilter", Message::SetCategoryFilter("Action".to_string())),
        Message::ClearFilters => ("ClearFilters", Message::ClearFilters),
        // A **named** form, not the empty one. `shell_with_work_to_do` holds an
        // empty template, whose name is `""` and which `GameForm::apply` refuses
        // — so `SaveGameForm(GameForm::default())` writes nothing and returns no
        // task, and the handler would be reported here as an unwritten arm. The
        // sample has to be a value the handler acts on, which is the same rule
        // the search-text and category fixtures state above.
        //
        // This is the one sample in this list that is also state: driving it
        // closes the form, which the arm after it relies on. See
        // `only_the_written_handlers_change_anything` for how the order is held.
        Message::SaveGameForm(_) => ("SaveGameForm", {
                            let mut form = GameForm::new_template(
                                &Settings::default(),
                                "sample-game".to_string(),
                            );
                            form.set_field(crate::state::FormField::Name, "Half-Life 2".to_string());
                            Message::SaveGameForm(form)
                        }),
        // `Category`, whose fixture value is `"Uncategorized"` — writing
        // `"Uncategorized"` again would be a write of what is already there.
        Message::FormFieldChanged { .. } => ("FormFieldChanged", Message::FormFieldChanged {
                            field: crate::state::FormField::Category,
                            value: "Action".to_string(),
                        }),
        // `mangohud`, which the default template seeds `false`, so `true` is a
        // write that shows.
        Message::FormToggleChanged { .. } => ("FormToggleChanged", Message::FormToggleChanged {
                            name: "mangohud".to_string(),
                            value: true,
                        }),
        Message::SetFormLinux(_) => ("SetFormLinux", Message::SetFormLinux(true)),
        // `"g"` is a game the fixture's library holds and it is **Windows**, so
        // all four entry points reach the half that returns a task rather than
        // the guard that toasts: the lookup succeeds and none of them is the
        // Linux refusal. The task is never driven (`observe` reads `units()` and
        // drops it), which is what keeps a fork, an `xdg-open` and a `.desktop`
        // write out of the test process — and it is why the samples can name a
        // real game without any of the four touching the disk.
        Message::LaunchGame(_) => ("LaunchGame", Message::LaunchGame("g".to_string())),
        // `result: Ok(())`, not `Err`: the `Ok` arm is the one that writes state
        // (`mark_played`) *and* returns a task, so it is the arm a regression to
        // `{}` would hide. The `Err` arm's only effect is a toast, which the
        // `LaunchWatchFinished` sample below already covers for the same
        // mechanism.
        Message::LaunchStarted { .. } => ("LaunchStarted", Message::LaunchStarted {
                            game_id: "g".to_string(),
                            result: Ok(()),
                        }),
        // `Some(reason)`, not `None`. `None` is the still-running case and the
        // reference does nothing with it, so a sample of `None` would make this
        // arm indistinguishable from an unwritten one — the D-34 shape, in the
        // sample rather than in the handler. `the_still_running_case_is_silent`
        // is the control arm for the `None` half.
        Message::LaunchWatchFinished { .. } => ("LaunchWatchFinished", Message::LaunchWatchFinished {
                            game_id: "g".to_string(),
                            reason: Some("the runner exited with status 1".to_string()),
                        }),
        Message::RunPrefixTool { .. } => ("RunPrefixTool", Message::RunPrefixTool {
                            game_id: "g".to_string(),
                            tool: PrefixTool::WineCfg,
                        }),
        Message::PrefixToolStarted { .. } => ("PrefixToolStarted", Message::PrefixToolStarted {
                            game_id: "g".to_string(),
                            tool: PrefixTool::Winetricks,
                            result: Ok(()),
                        }),
        Message::OpenPrefixFolder(_) => ("OpenPrefixFolder", Message::OpenPrefixFolder("g".to_string())),
        Message::PrefixFolderOpened { .. } => ("PrefixFolderOpened", Message::PrefixFolderOpened {
                            result: Err("Could not open the prefix folder: permission denied".to_string()),
                        }),
        Message::CreateDesktopShortcut(_) => ("CreateDesktopShortcut", Message::CreateDesktopShortcut("g".to_string())),
        Message::ShortcutCreated { .. } => ("ShortcutCreated", Message::ShortcutCreated {
                            result: Ok(PathBuf::from("/tmp/gamehandler-fixture.desktop")),
                        }),
        // A URL that cannot resolve, so nothing this sample ever reaches can
        // touch the network — and the task is never driven anyway (`observe`
        // reads `units()` and drops it), which is what keeps a browser out of
        // `cargo test`. `.invalid` is reserved for exactly this.
        Message::OpenUrl(_) => ("OpenUrl", Message::OpenUrl("https://example.invalid/credit".to_string())),
        // `Err`, not `Ok`: the `Ok` arm is `cosmic::task::none()` with no state
        // write, which `observe` cannot tell from an unwritten arm — the D-34
        // shape, in the sample. The `Err` arm toasts, and the toast is the
        // change. `the_url_opened_success_arm_is_silent` is the control arm for
        // the other half.
        Message::UrlOpened { .. } => ("UrlOpened", Message::UrlOpened {
                            result: Err("Could not open https://example.invalid/credit: No such file or directory".to_string()),
                        }),
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
        Message::ConfirmRemoveRunner { .. } => ("ConfirmRemoveRunner", Message::ConfirmRemoveRunner {
                            runner_id: "v1.0".to_string(),
                            name: "GE-Proton".to_string(),
                        }),
        Message::RemoveRunnerConfirmed(_) => ("RemoveRunnerConfirmed", Message::RemoveRunnerConfirmed("v1.0".to_string())),
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
        // The failure text is not a placeholder for the same reason the plugin
        // id is not: the handler builds its sentence from
        // `State::running_install`, which the fixture holds, so a message that
        // named an installer's *own* name would be a second source for a fact
        // that has one. The text here is what `_async`'s `fail` passes through.
        Message::EasyInstallFailed { .. } => ("EasyInstallFailed", Message::EasyInstallFailed {
                            message: "the download failed".to_string(),
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
        // A library holding the two ids the samples use, at its **own temp path**.
        //
        // `Shell::new` opens the real one, and a fixture that added a game to it
        // would write the user's `games.json`. `library_with` is not reused here
        // because it keys its directory on the label and the process id alone:
        // this fixture is built by every test that calls this function, so those
        // calls run concurrently and would `remove_dir_all` each other's library
        // mid-save. Measured — that is exactly what the first version did, and it
        // failed ten tests with `left: 1, right: 0` in ones that never touch a
        // form. The counter is what makes the path unique per call.
        //
        // The ids are the samples': `OpenEditGameForm("g")` needs a game to find
        // or its arm is reported as unwritten, and `SaveGameForm`'s update branch
        // needs `"sample-game"` to exist.
        shell.state.library = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "gh-form-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let mut library = Library::new_at(Some(root.join("games.json")), 0.0);
            for (id, name) in [("g", "Fixture"), ("sample-game", "Sample")] {
                let mut game = Game::new_named(name);
                game.id = id.to_string();
                library.add(game).unwrap();
            }
            library
        };
        // The form: a **new template for a game that really is in the library**,
        // which is the fixture every form handler needs.
        //
        // Not `GameForm::default()`, for the same reason the search text is not
        // the sample value: `default()` has `is_new: false` and an empty name, so
        // `SaveGameForm`'s sample would be refused by `apply` and the arm would
        // be reported here as unwritten. The id is the one the library holds, so
        // the sample *updates* rather than adds — which is the branch that leaves
        // `form.game_form` as the sample's own value and lets
        // `only_the_written_handlers_change_anything` drive `SaveGameForm` after
        // the two form-field handlers without either of them hiding the other.
        //
        // `is_new: true` with a library id is not a contradiction: the reference
        // decides add-versus-update from the library (`bridge.py:406`) and only
        // the *title* from `isNew`, which is exactly what this fixture is.
        shell.state.game_form = Some(GameForm {
            game_id: Some("fixture-game".to_string()),
            is_new: true,
            name: "Fixture".to_string(),
            toggles: GameForm::TOGGLE_NAMES
                .iter()
                .map(|name| ((*name).to_string(), false))
                .collect::<std::collections::BTreeMap<_, _>>(),
            ..GameForm::default()
        });
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
        // The easy-install state machine's fixture, and the one place in this
        // function where the state deliberately holds two records the real flow
        // keeps apart.
        //
        // `easy_busy` is set for a reason that is not about observation at all:
        // **with it false, `StartEasyInstall` would start a real install.**
        // `state.runners` points at the user's real runners directory, so on a
        // machine that has proton-ge and a Wine, the arm would create a prefix,
        // spawn the worker and download the vendor's installer from the network
        // — from inside `cargo test`. The guard reads `is_handled`, not branch
        // coverage, so holding the guard is enough to measure the arm, and it
        // measures the *refusal* branch, which is a real one. That is the same
        // trade this fixture makes elsewhere: crafted state so a handler acts,
        // without the handler reaching the outside world.
        shell.state.easy_busy = true;
        // `Some(0.25)` and not `Some(0.5)`, which is the sample
        // `EasyInstallProgress` carries: a handler that wrote what was already
        // there would be reported as an unwritten arm (D-34).
        shell.state.progress = Some(0.25);
        // The in-flight record and the pending record, held together. In the
        // real flow they are mutually exclusive — `easy_install_wizard_finished`
        // moves the record from one to the other — and the fixture holds both
        // because the guard drives every message against **one** shell rather
        // than one per message. Nothing branches on the combination:
        // `EasyInstallWizardFinished` reads `running_install` and
        // `CompleteEasyInstall` reads `easy_pending`, so neither observes the
        // other's field. If a future handler does branch on the pair, this
        // fixture is where the impossible state will show up as a surprise.
        //
        // `installer_id` is `"steam"`, a real catalog id, because
        // `finish_easy_install` resolves it and a fixture naming an id the
        // catalog lacks would drive the failure branch of a working handler.
        shell.state.running_install = Some(PendingInstall {
            installer_id: "steam".to_string(),
            installer_name: "Steam".to_string(),
            prefix: std::env::temp_dir().join("gh-install-fixture-prefix"),
            runner_id: "proton-ge".to_string(),
            game_id: "install-fixture".to_string(),
        });
        // Keyed by the token `CompleteEasyInstall`'s sample uses.
        shell.state.easy_pending.insert(
            "t".to_string(),
            PendingInstall {
                installer_id: "steam".to_string(),
                installer_name: "Steam".to_string(),
                prefix: std::env::temp_dir().join("gh-install-fixture-prefix"),
                runner_id: "proton-ge".to_string(),
                game_id: "install-fixture".to_string(),
            },
        );
        shell
    }

    /// **The set of messages that do anything at all is the set of written
    /// handlers, and nothing else.**
    ///
    /// This is the check that was missing. All five real arms of `update` were
    /// replaced with inert bodies and all five survived, because nothing ever
    /// called them: `App` cannot be built without a display, so no test could.
    /// Moving the dispatcher onto [`Shell`] is what makes the arms callable, and
    /// this drives every message `every_message` knows about through the real
    /// match and requires the set that has an effect to be exactly the written
    /// handlers. Completeness is the macro's job, not a number's: `message_variants!`
    /// generates `every_message` from one list with an exhaustive match and no
    /// wildcard, so a variant with no sample does not compile. That paragraph
    /// used to say "fifty", which was a count of the macro's own list written
    /// down outside it — the same drift that got the count removed from
    /// [`Shell::update`]'s doc comment.
    ///
    /// Both directions are pinned. A handler that regresses to `{}` disappears
    /// from this list; a new handler landing appears in it. Neither can pass
    /// unnoticed, which is what keeps the `T-0x` markers honest.
    ///
    /// # The two exclusions, and the one blind spot
    ///
    /// - **`Quit`** is excluded by name. It is handled by [`App::update`] and
    ///   not by [`Shell::update`], because it closes the framework's window;
    ///   `Shell`'s arm for it is deliberately empty, so counting it here would
    ///   mean counting an empty arm as a handler.
    /// - **`Message::SetWindowHidden`** is excluded by the same rule and for the
    ///   same reason: `App::update` answers it, because only `App` holds the
    ///   window id. T-29 added this arm and this exclusion together — the
    ///   alternative was to put the hide inside `LaunchStarted`'s body, which
    ///   would have needed a `Core` on [`Shell`] and cost every other arm its
    ///   callability from a test.
    /// - **`DismissToast`** is *not* excluded, but it *is* invisible: the arm is
    ///   real (`toasts.remove(id)`) and no test can build an id naming a live
    ///   toast, so it cannot be told apart from `{}` — see
    ///   [`a_test_cannot_observe_which_toast_was_dismissed`], which measures
    ///   that rather than asserting it. The list below therefore names every
    ///   written handler but that one — and the list itself is the count. It
    ///   used to say "fifteen handlers where sixteen bodies are written", which
    ///   was a second number describing the first and went stale at T-11
    ///   without anything failing; the sentence a reviewer trusts to know what
    ///   is live is the list, so there is no longer a number beside it.
    ///
    /// `Message::LaunchWatchTick` is neither excluded nor listed, and that is
    /// the honest place for it: it is measured like every other message, it has
    /// no effect, and it must not. `LaunchWatchTick` reaching this assertion's
    /// `changed` would mean its arm had grown a body — which is the change a
    /// future poll would have to make deliberately.
    ///
    /// [`observe`] counts a returned [`cosmic::Task`] as well as a state
    /// change, so the *other* class of invisible handler — one whose only
    /// effect is the task it returns, which is the shape T-09's `FetchCover`
    /// will take — is caught here rather than declared away.
    #[test]
    fn only_the_written_handlers_change_anything() {
        let mut changed: Vec<&str> = every_message()
            .into_iter()
            // The two `App::update` arms. `Shell::update`'s bodies for them are
            // deliberately empty — both need the framework's window — so they
            // are not part of the claim. See the doc above.
            .filter(|message| {
                !matches!(
                    message,
                    Message::Quit | Message::SetWindowHidden(_)
                )
            })
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
            // T-11's six, plus P-37's two. Live because the Runners page draws
            // the controls that produce them: the family selector, its two
            // callbacks, the Install button, the progress reports, and the
            // Remove button — which now asks first, so the dialog's two halves
            // are here too.
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
            "ConfirmRemoveRunner",
            "RemoveRunnerConfirmed",
            "UninstallRunner",
            // `RunnersRefreshed` is not a seventh page action: nothing draws it
            // and no user sends it. It is the reply `view::runners::refresh`
            // sends itself, so its arm writes the two row bundles and returns no
            // task — which `observe` sees as a state change, and this list
            // therefore has to name. A page whose rows could only arrive from a
            // task *and* whose handler was empty would be a page that never
            // learns what is installed.
            "RunnersRefreshed",
            // T-10's six. Live because the form is a layer over the current
            // page, not a page of its own: the Library's add button and the row
            // menu open it, and the controls it draws produce the other four.
            //
            // `OpenNewGameForm` was the one entry in `dispatch_coverage`'s
            // `KNOWN_DEAD` deferral while its arm was `{}` — a *dispatched* page
            // emitting a message nothing handled. Landing the arm is what
            // retired that entry, and the guard said so by failing on it.
            //
            // `SaveGameForm` reaches its `update` branch here only because
            // `shell_with_work_to_do`'s library already holds the id its sample
            // carries: the `add` branch would `save()` to the path the fixture
            // was built with either way, and a sample naming an id the library
            // does not hold is what once wrote a real `games.json` — see that
            // fixture's doc.
            "OpenNewGameForm",
            "OpenEditGameForm",
            "SaveGameForm",
            "FormFieldChanged",
            "FormToggleChanged",
            "SetFormLinux",
            // T-29's nine: the four entry points and the five replies their
            // deferred halves come back on. Every one of the four is here for
            // the *lookup* and the guards alone — what they return is a task no
            // test drives, so the fork, the `xdg-open` and the `.desktop` write
            // stay out of the test process while the arm that asks for them is
            // still measured.
            //
            // # Three of the four have no producer, and this comment used to
            // # say otherwise (#89)
            //
            // It read "the game form's three buttons produce the rest", and
            // that is false in both of the ways it could be: the wrong
            // *control* and the wrong *file*. Measured against the tree, in the
            // production code only — `view/form.rs` (before its `#[cfg(test)]`
            // module at `:822`, comments stripped) constructs **six** messages
            // (`CloseDialog`, `FetchCoverForForm`, `FormFieldChanged`,
            // `FormToggleChanged`, `SaveGameForm`, `SetFormLinux`) and none of
            // these three; its three buttons are Find cover, Cancel and Save.
            // `view/library.rs` (before its test module at `:455`) constructs
            // **eight** (`ClearFilters`, `LaunchGame`, `NavigateTo`,
            // `OpenNewGameForm`, `SetCategoryFilter`, `SetSearchText`,
            // `SetSortMode`, `SetViewMode`) and none of these three either. A
            // reader sent to `view/form.rs` to find the button would find
            // nothing, so the sentence is corrected rather than kept.
            //
            // Both counts are *constructions*, not mentions, and the difference
            // is measured rather than asserted: a plain `grep Message::` over
            // `form.rs`'s production region returns **eight**, because
            // `PickExeFile` and `PickCoverFile` occur in the module doc's
            // "controls that are not drawn" list (`form.rs:36-37`). Counting a
            // doc link as a producer is the same defect this paragraph is
            // about, one layer down — and it is the trap `dispatch_coverage.rs`
            // names in its own header ("comments and string literals are
            // stripped before any scan"), which is why the count here is taken
            // the way that guard takes it.
            //
            // Where the producer goes is `LibraryPage.qml:320-336`, the
            // reference's **only** call sites for these three: a context menu on
            // a Library row, whose `onTriggered` handlers are `runPrefixTool`,
            // `openPrefix` and `createShortcut`. `view/library.rs` has no menu,
            // so the correct file currently draws no control that emits any of
            // them. **That file is UX's under D-51**, and the menu is T-09's.
            //
            // The three arms are listed anyway, and that is deliberate: a menu
            // item is a button like any other, so each is reachable in exactly
            // the sense this guard asks about, and landing the arm first means
            // the producer arrives to a handler that already works rather than
            // to an `{}` — finding #65's shape. **Nothing else in the suite can
            // see this gap:** `dispatch_coverage.rs` polices the opposite
            // direction (a control that *emits* into an empty arm) and says so
            // in its own header, so a handled-but-unemitted variant is invisible
            // to it. This paragraph is the record; if the menu lands and these
            // lines are still needed, they are stale.
            //
            // `LaunchGame` is the one of the four with a live producer today:
            // `view/widgets.rs`'s card and row take the `on_press` that emits
            // it, and the guard prints that closure — `covered: view/widgets.rs
            // — called from view/library.rs`.
            //
            // `LaunchWatchTick` is deliberately absent: it has an empty arm and
            // does nothing, so it has nothing to be listed for. The doc above
            // says why that is a decision rather than an omission.
            "LaunchGame",
            "LaunchStarted",
            "LaunchWatchFinished",
            "RunPrefixTool",
            "PrefixToolStarted",
            "OpenPrefixFolder",
            "PrefixFolderOpened",
            "CreateDesktopShortcut",
            "ShortcutCreated",
            // P-65's two, and this is the pair that retired `LINKS_OPEN`. The
            // producer is the credits page's twenty-seven `button::link`s —
            // twenty-five `Visit` buttons and the footer's two — which are the
            // only controls in the app that emit `OpenUrl`, and `UrlOpened` is
            // the reply. Both entered this list in the same commit that wired
            // them, because a handler listed here with no producer is the state
            // finding #65 is about.
            "OpenUrl",
            "UrlOpened",
            // T-38's nine. Live because the Installers page draws the search
            // box, the category selector and the Install button that produce the
            // first three, and the other six are the install's own replies.
            //
            // `SetInstallerSearch` and `SetInstallerCategory` are here because
            // their samples write `installer_search`/`installer_category`, and
            // that write is the only part of the arm this guard can see.
            //
            // The half it *cannot* see is the point of those two arms: the
            // catalog the page draws is derived, and an arm that ran the filter
            // and forgot `State::refresh_installers` would be listed here all the
            // same, because the search text still changed. This comment used to
            // claim otherwise — that the refresh "would still be observed here,
            // because `refresh_installers` writes three fields" — and deleting
            // the refresh left this test green, which is how the claim was
            // measured false. `a_filter_rebuilds_the_catalog_the_page_draws` is
            // the assertion that closes it; the arm runs the refresh
            // unconditionally rather than only when `view::installers::update`
            // returns `Some`, because the two filter arms return `Some` and do
            // their work in the state.
            //
            // `StartEasyInstall` is measured at its **first refusal** — the
            // fixture holds `easy_busy` — for the reason written on
            // `shell_with_work_to_do`: the alternative is a real download from
            // inside the test process.
            //
            // The five replies are here because the fixture holds the state they
            // need (`running_install`, `easy_pending`, a progress that is not
            // the sample's), and each of them is a message `Shell::update` must
            // answer or the install's UI is stuck: `EasyInstallWizardFinished`
            // and `CompleteEasyInstall` are the two ways an install finishes,
            // `CancelEasyInstall` the only way out of the not-found state, and
            // `EasyInstallFailed` the only arm that clears the guard when the
            // worker gives up.
            "SetInstallerSearch",
            "SetInstallerCategory",
            // The runner the *next install* uses (D-55). It is here because its
            // handler writes `State::installer_runner`, and that write is the
            // only part of the arm this guard can see — the value reaching the
            // created game is three hops away (`InstallersView::runner_id` →
            // `install_press` → `StartEasyInstall`), and
            // `the_chosen_runner_is_the_one_the_install_press_names` walks it.
            "SetInstallRunner",
            "StartEasyInstall",
            "EasyInstallProgress",
            "EasyInstallWizardFinished",
            "CompleteEasyInstall",
            "CancelEasyInstall",
            "EasyInstallFailed",
            "EasyInstallFinished",
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

    /// **The grace watch reports only when the title died — both directions, on
    /// the same fixture.**
    ///
    /// This is the control arm for [`Message::LaunchWatchFinished`], and it is
    /// needed because that arm has a property no other arm in this file has:
    /// one of its two branches is *supposed* to do nothing. `reason: None` is
    /// the success case — the title was still running when the grace expired —
    /// and the reference does nothing with it (`bridge.py:478-483`; `report` is
    /// called and the watch simply ends).
    ///
    /// The guard above cannot see the difference. `LaunchWatchFinished` is in
    /// its `expected` list because the `Some` branch is effectful, so deleting
    /// the `let Some(reason) = reason else` and always hiding-and-toasting
    /// leaves that guard green while every *successful* launch raises a
    /// spurious "stopped right away" notice. That mutation is what the first
    /// half of this test fails on.
    ///
    /// # Why the macro's own sample cannot cover this
    ///
    /// [`message_variants`]'s sample for this variant carries `Some(..)`, for
    /// the D-34 reason: `None` is silent, so a `None` sample would make a
    /// correct arm indistinguishable from an unwritten one and the guard would
    /// pass either way. That choice is right, and it is precisely why the
    /// *silent* branch needs a test of its own — every instrument in the guard
    /// points at the loud one.
    ///
    /// The fixture is the same one in both halves, the id is one the library
    /// really holds (`"g"`, and `game_name` answers `""` for an id it does not,
    /// so a broken lookup would silently agree with the quiet half), and the
    /// only thing that differs between the two calls is `reason`.
    #[test]
    fn the_grace_watch_reports_only_when_the_title_died() {
        // The success case: still running when `LAUNCH_GRACE_SECONDS` expired.
        let mut running = shell_with_work_to_do();
        let quiet = observe(
            &mut running,
            Message::LaunchWatchFinished {
                game_id: "g".to_string(),
                reason: None,
            },
        );
        assert!(
            !quiet.state_changed && quiet.task_units == 0,
            "a title that was still running produced something: {quiet:?}. \
             `None` is the success case and the reference does nothing with it"
        );
        assert!(
            format!("{:?}", running.state.toasts).contains("num_elems: 0"),
            "a title that stayed up pushed a toast — the mutation this test \
             exists for is an arm that reports unconditionally"
        );

        // The failure case, on a fresh copy of the same fixture, so the two
        // halves differ in `reason` and in nothing else.
        let mut died = shell_with_work_to_do();
        let loud = observe(
            &mut died,
            Message::LaunchWatchFinished {
                game_id: "g".to_string(),
                reason: Some("the runner exited with status 1".to_string()),
            },
        );
        assert!(
            format!("{:?}", died.state.toasts).contains("num_elems: 1"),
            "a title that died must be reported — the reference restores the \
             window and toasts “{{name}}” stopped right away: {{reason}}, and \
             an unreported failure is the whole reason this watch exists"
        );
        assert!(
            loud.task_units >= 2,
            "the failure case batches the window restore (`Task::done`) and the \
             toast (`Toasts::push`'s expiry task), so it must ask the runtime \
             for at least two units; it asked for {}",
            loud.task_units
        );
    }

    /// **`LaunchWatchTick` does nothing, and that is the decision rather than
    /// an omission.**
    ///
    /// The guard above measures this variant and finds no effect, but it cannot
    /// *say* so: an assertion comparing two lists shows what is in them, and
    /// `LaunchWatchTick` is correctly in neither. So the decision
    /// (`architecture.md` §3.3 names one blocking `failure(6.0)` over a poll,
    /// and [`launch_and_watch`] follows it) is pinned here instead.
    ///
    /// The instrument is the same one the guard uses, so "does nothing" means
    /// what it means there — no state change and no task — rather than "the arm
    /// looks empty in the source". This fails the moment the arm grows a body
    /// without the watch growing a producer, which is the shape a
    /// half-migrated poll would take: half a tick loop is worse than none.
    ///
    /// It is not a claim that the variant is unreachable — `view/runners.rs`
    /// constructs it in a test of its own — and it does not lock the design in
    /// place: a future poll renames this test rather than deleting it.
    #[test]
    fn the_watch_tick_has_no_effect() {
        let mut shell = shell_with_work_to_do();
        assert!(
            !is_handled(&mut shell, Message::LaunchWatchTick),
            "`LaunchWatchTick` grew an effect. That is not forbidden — but it \
             means the watch became a poll, and the arm and its producer must \
             then land together, or `dispatch_coverage` sees an emission \
             landing in a body that does something without saying what"
        );
    }

    /// **`Shell`'s `SetWindowHidden` arm is empty, so the guard's exclusion of
    /// it is honest rather than a hiding place.**
    ///
    /// The guard above filters this variant out *by name* — because
    /// [`App::update`] answers it and [`Shell`] has no `Core` to answer it with
    /// — and an exclusion that conceals the thing it excludes is worse than no
    /// exclusion at all: the doc beside the filter says the arm is empty, and
    /// without this nothing checks that it stays so. A body growing here would
    /// be an effect that no instrument in this file measures.
    ///
    /// It asserts *emptiness*, not correctness. What the message must actually
    /// do is recorded where it is done, in [`App::update`]'s own comment, and
    /// is reachable by no test because building a `cosmic::Core` is the wall
    /// #33 was filed about.
    #[test]
    fn shells_window_arm_is_empty_because_app_owns_it() {
        let mut shell = shell_with_work_to_do();
        let effect = observe(&mut shell, Message::SetWindowHidden(true));
        assert!(
            !effect.state_changed && effect.task_units == 0,
            "`Shell` answered `SetWindowHidden` itself: {effect:?}. The guard \
             excludes this variant because `App::update` owns it, so anything \
             done here is done where nothing looks"
        );
    }

    // -----------------------------------------------------------------------
    // T-38: the easy-install flow (P-53…P-59)
    // -----------------------------------------------------------------------

    /// A shell with the *starting* easy-install state and a library at a temp
    /// path.
    ///
    /// [`shell_with_work_to_do`] primes the two install records and the busy
    /// guard so the handler guard can observe `StartEasyInstall` at its first
    /// refusal; these tests need the state a user has before pressing Install,
    /// so they clear what it primed.
    ///
    /// The library is the fixture's own temp one, and that is load-bearing
    /// rather than tidy: [`finish_easy_install`] calls `Library::add`, which
    /// saves, so a shell built over the real `games.json` would write the
    /// running user's library from a test.
    fn shell_for_installs() -> Shell {
        let mut shell = shell_with_work_to_do();
        shell.state.easy_busy = false;
        shell.state.progress = None;
        shell.state.running_install = None;
        shell.state.easy_pending.clear();
        shell
    }

    /// The record [`start_easy_install`] writes, for the tests that need one
    /// already in place.
    ///
    /// `installer_id` is `"steam"`, a real catalog id: every path that consumes
    /// this record resolves the recipe through `installer_by_id`, so a fixture
    /// naming an id the catalog lacks would drive the failure branch of a
    /// handler that is working.
    fn install_record(prefix: &Path, game_id: &str) -> PendingInstall {
        PendingInstall {
            installer_id: "steam".to_string(),
            installer_name: "Steam".to_string(),
            prefix: prefix.to_path_buf(),
            runner_id: "proton-ge".to_string(),
            game_id: game_id.to_string(),
        }
    }

    /// A real directory of this test's own.
    ///
    /// P-57 is a claim about a **path that still exists** — "cancel keeps the
    /// prefix" — so the fixture has to be a directory that can be checked
    /// afterwards, not a name.
    fn install_prefix(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("gh-install-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// **A second install is refused while one is running — P-59's other
    /// half.**
    ///
    /// `InstallersPage.qml:125` is `enabled: !backend.busy`, which disables the
    /// button; this is the guard *behind* the button (`bridge.py:833-835`), and
    /// it is the one that matters for a message that reached `update` without a
    /// press: a re-sent task, a shortcut, or a future caller of
    /// `Message::StartEasyInstall`.
    ///
    /// # The installer id is one the catalog does not hold, on purpose
    ///
    /// The mutation this test exists for is *deleting the busy guard*, and the
    /// assertion it fails on has to be reachable without starting a real
    /// install. With `"steam"` the arm would go on to resolve a runner and, on
    /// a machine that has proton-ge, create a prefix and download a vendor
    /// installer **from inside `cargo test`**. With an id the catalog lacks,
    /// the next statement is `except KeyError: return`'s silence — so the
    /// mutated arm produces nothing at all and this test fails on its first
    /// assertion, deterministically, on every machine.
    #[test]
    fn an_install_in_flight_refuses_a_second_one() {
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;

        let effect = observe(
            &mut shell,
            Message::StartEasyInstall {
                installer_id: "no-such-installer".to_string(),
                runner_id: "proton-ge".to_string(),
            },
        );

        assert!(
            effect.task_units > 0,
            "a refused install said nothing: {effect:?}. The reference answers a \
             second `installEasy` with `notify(\"Another install is already \
             running\")` (`bridge.py:833-835`), and a silence here is a user \
             pressing Install and seeing nothing happen at all"
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("Another install is already running"),
            "the refusal is not the reference's sentence; toasts: {:?}",
            shell.state.toasts
        );
        assert!(
            shell.state.running_install.is_none() && !shell.state.easy_pending.values().any(
                |record| record.game_id == "no-such-installer"
            ),
            "a refused install wrote a record — the guard is supposed to leave \
             the state exactly as it found it"
        );
    }

    /// **An installer id the catalog does not hold is silent, not reported.**
    ///
    /// `bridge.py:836-839` is `except KeyError: return` — the reference says
    /// nothing at all, and the difference from the busy refusal is the point:
    /// the user pressed a button that exists, over a card built from this very
    /// catalog, so an id that does not resolve is a *code* fault and a toast
    /// about it would be a sentence no user can act on.
    ///
    /// It is also the branch that keeps the busy guard's test honest — see that
    /// test — so a mutation that turns this silence into a notice breaks both.
    #[test]
    fn an_installer_id_the_catalog_does_not_hold_is_silent() {
        let mut shell = shell_for_installs();
        let before = format!("{:?}", shell.state);

        let effect = observe(
            &mut shell,
            Message::StartEasyInstall {
                installer_id: "no-such-installer".to_string(),
                runner_id: "proton-ge".to_string(),
            },
        );

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "`except KeyError: return` produced something: {effect:?}"
        );
        assert_eq!(
            format!("{:?}", shell.state),
            before,
            "the whole state moved, not just the part this test names"
        );
    }

    /// **A wizard that found no executable stores the install and leaves the
    /// page busy — both halves of `done(result)`'s else branch.**
    ///
    /// `bridge.py:874-895`: nothing found means `_pending_installs[token] = {…}`
    /// and a notice, and **`_easy_busy` is deliberately left `True`**. That
    /// asymmetry is what `cancelEasyInstall` and `completeEasyInstall` exist to
    /// end, and a port that cleared the guard here would let a second install
    /// start on top of a pending one — P-59's guard, defeated by the only path
    /// with something to lose.
    ///
    /// The suffix is conditional on the return code (`:885-886`), so the same
    /// test drives both codes on two fresh shells: a port that always appended
    /// it, or never, fails on one of them.
    #[test]
    fn a_wizard_that_found_no_executable_stores_the_install_and_stays_busy() {
        for (returncode, suffix) in [(1, " (installer exited 1)"), (0, "")] {
            let prefix = install_prefix(&format!("not-found-{returncode}"));
            let mut shell = shell_for_installs();
            shell.state.easy_busy = true;
            shell.state.running_install = Some(install_record(&prefix, "install-1"));

            let effect = observe(
                &mut shell,
                Message::EasyInstallWizardFinished {
                    found: None,
                    returncode,
                },
            );

            assert!(
                effect.task_units > 0,
                "the user is not told the executable was not found: {effect:?}"
            );
            assert!(
                format!("{:?}", shell.state.toasts).contains(&format!(
                    "Could not find the Steam executable in the prefix{suffix}. Pick it \
                     yourself if the install finished."
                )),
                "the notice is not the reference's sentence for return code \
                 {returncode}; toasts: {:?}",
                shell.state.toasts
            );
            assert!(
                shell.state.easy_pending.contains_key("install-1"),
                "the pending install is not stored, so `completeEasyInstall` and \
                 `cancelEasyInstall` have no token to answer"
            );
            assert!(
                shell.state.running_install.is_none(),
                "the in-flight record outlived the wizard it describes"
            );
            assert!(
                shell.state.easy_busy,
                "the guard was cleared while an install is waiting for the user — \
                 the page becomes pressable again and a second install starts over \
                 a pending one"
            );
            assert!(
                prefix.exists() && shell.state.library.get("install-1").is_none(),
                "P-57: the prefix is kept and **no** library entry is made until \
                 the user says what to run"
            );
        }
    }

    /// **A cancel clears both guards and keeps the prefix — P-57.**
    ///
    /// `bridge.py:937-948`. The guards are cleared *before* the `if pending`
    /// (`:940-943`), which is why this function is the only way out of the busy
    /// state the not-found branch leaves behind; the notice fires only when
    /// there really was a prefix to keep.
    ///
    /// The "keeps the prefix" half is asserted on a directory that exists, not
    /// on the sentence that mentions it: a port that removed the prefix and
    /// printed the same words would pass a text-only check, and deleting a
    /// user's half-finished Wine prefix is the failure the sentence promises
    /// against.
    #[test]
    fn a_cancel_clears_the_guards_and_keeps_the_prefix() {
        let prefix = install_prefix("cancel");
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;
        shell.state.progress = Some(0.4);
        shell.state
            .easy_pending
            .insert("tok".to_string(), install_record(&prefix, "install-2"));

        let effect = observe(&mut shell, Message::CancelEasyInstall("tok".to_string()));

        assert!(effect.task_units > 0, "a cancel said nothing: {effect:?}");
        assert!(
            format!("{:?}", shell.state.toasts)
                .contains("Kept the Steam prefix. Add it later from Add Game if you want."),
            "the kept-prefix notice is not the reference's sentence; toasts: {:?}",
            shell.state.toasts
        );
        assert!(
            !shell.state.easy_busy && shell.state.progress.is_none(),
            "a cancel left the page busy or the bar drawn — P-57's cancel is the \
             only exit from the not-found state, so a guard left on here is a page \
             that can never install anything again"
        );
        assert!(
            shell.state.easy_pending.is_empty(),
            "the pending record survived the cancel it was cancelled by"
        );
        assert!(prefix.exists(), "the prefix was deleted");
    }

    /// **A cancel for a token nobody holds still clears the guards, and says
    /// nothing.**
    ///
    /// The second half of the same reference block, and the one a tidy rewrite
    /// loses: the guards are cleared outside the `if pending`, so this really
    /// does un-busy the page, while the notice is inside it and does not fire.
    /// Asserting the two together is what keeps a mutation from satisfying one
    /// by breaking the other.
    #[test]
    fn a_cancel_for_a_token_nobody_holds_still_clears_the_guards() {
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;
        shell.state.progress = Some(0.4);

        let effect = observe(
            &mut shell,
            Message::CancelEasyInstall("nobody-holds-this".to_string()),
        );

        assert!(
            effect.state_changed && effect.task_units == 0,
            "the guards were not cleared, or a prefixless cancel spoke: {effect:?}"
        );
        assert!(!shell.state.easy_busy && shell.state.progress.is_none());
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 0"),
            "a cancel with nothing to keep promised to keep something; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// **The found branch makes a library entry with the recipe's own fields —
    /// `_finish_easy_install` (`bridge.py:905-919`).**
    ///
    /// Four of the five assertions are about `game_from_install` rather than
    /// about this file, and that is deliberate: the point of the arm is *which*
    /// values reach the library, and an arm that passed the wrong prefix or the
    /// wrong runner would produce an entry that looks fine in a listing and
    /// fails on launch.
    ///
    /// # What is not asserted, because it is not built
    ///
    /// `cover_path`. The reference sets it to the executable's own icon
    /// (`:907-911`) and **this tree has no `exe_icons`** — see
    /// [`finish_easy_install`]. The assertion below pins the empty string the
    /// reference's own `except` clause produces, so the day T-05 lands, this
    /// test fails and says which field changed rather than quietly agreeing
    /// with the new behaviour.
    #[test]
    fn a_wizard_that_found_the_executable_makes_a_library_entry() {
        let prefix = install_prefix("found");
        let exe = prefix.join("drive_c/Steam/steam.exe");
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;
        shell.state.running_install = Some(install_record(&prefix, "install-3"));

        let _ = observe(
            &mut shell,
            Message::EasyInstallWizardFinished {
                found: Some(exe.clone()),
                returncode: 0,
            },
        );

        assert!(
            !shell.state.easy_busy && shell.state.running_install.is_none(),
            "the install's guards outlived the install"
        );
        assert!(
            shell.state.easy_pending.is_empty(),
            "a finished install is not a pending one — the user is not asked for \
             an executable that was already found"
        );
        let game = shell
            .state
            .library
            .get("install-3")
            .expect("the install became a library entry");
        assert_eq!(game.name, "Steam");
        assert_eq!(game.exe_path, exe.to_string_lossy());
        assert_eq!(game.prefix_path, prefix.to_string_lossy());
        assert_eq!(game.runner, "proton-ge");
        assert_eq!(game.kind, "windows");
        assert_eq!(
            game.cover_path, "",
            "P-58's vendor icon: `save_exe_icon` has no port in this tree, so the \
             entry carries the reference's own empty-string fallback. If this \
             fails because T-05 landed, the assertion to change is this one"
        );
    }

    /// **`completeEasyInstall` finishes an install the wizard could not, and
    /// a cancel is what an empty path means.**
    ///
    /// `bridge.py:921-936`. The pop comes first, which is why the cancel this
    /// falls into finds nothing to promise about — the reference's own
    /// asymmetry, and the thing a tidy rewrite would "fix" into a second
    /// kept-prefix notice the user never sees in Python.
    #[test]
    fn a_located_executable_finishes_the_install_and_an_empty_path_cancels_it() {
        let prefix = install_prefix("complete");
        let exe = prefix.join("drive_c/Steam/steam.exe");

        let mut finished = shell_for_installs();
        finished
            .state
            .easy_pending
            .insert("tok".to_string(), install_record(&prefix, "install-4"));
        let _ = observe(
            &mut finished,
            Message::CompleteEasyInstall {
                token: "tok".to_string(),
                path: Some(exe.to_string_lossy().into_owned()),
            },
        );
        assert!(
            finished.state.library.get("install-4").is_some(),
            "the located executable did not become the entry's executable"
        );

        let mut cancelled = shell_for_installs();
        cancelled.state.easy_busy = true;
        cancelled
            .state
            .easy_pending
            .insert("tok".to_string(), install_record(&prefix, "install-5"));
        let _ = observe(
            &mut cancelled,
            Message::CompleteEasyInstall {
                token: "tok".to_string(),
                path: None,
            },
        );
        assert!(
            !cancelled.state.easy_busy && cancelled.state.library.get("install-5").is_none(),
            "an empty path is a cancel: the guards clear and no entry is made"
        );
        assert!(
            format!("{:?}", cancelled.state.toasts).contains("num_elems: 0"),
            "the token was popped before the cancel, so the reference's \
             `cancelEasyInstall` finds nothing and says nothing — a kept-prefix \
             notice here is a sentence Python never prints; toasts: {:?}",
            cancelled.state.toasts
        );
    }

    /// **The install's own reply moves the user to the library and toasts with
    /// a Play action — `Main.qml:154-158`.**
    ///
    /// Two behaviours and one duration, and `observe` can see all three: the
    /// page records (the navigation), the batch's unit count (the toast's
    /// expiry task is a second unit), and the toast's own `Debug`, which carries
    /// both the message and its action's description.
    ///
    /// The reference's comment on the action is the reason it is not optional:
    /// "installing a store launcher is only half the job — the user still has to
    /// open it and sign in, so offer that right here". An entry that lands in
    /// the library and a page that stays where it was is the user having to
    /// find what they just installed.
    ///
    /// The `game_id` is carried into the action's closure, which is why it is a
    /// `String` and not a reference: `Toast::action` takes `'static`. A port
    /// that dropped it would build the button and launch nothing.
    #[test]
    fn the_installed_toast_offers_play_and_moves_to_the_library() {
        let mut shell = shell_for_installs();
        // Somewhere other than the page the message navigates to, so the
        // navigation has something to do.
        let _ = shell.show_page(Page::Settings);

        let effect = observe(
            &mut shell,
            Message::EasyInstallFinished {
                game_id: "install-6".to_string(),
                message: "Installed “Steam”".to_string(),
            },
        );

        assert_eq!(
            shell.state.page,
            Page::Library,
            "`root.showPage(\"library\")` did not happen, so the entry the user \
             just made is on a page they are not looking at"
        );
        assert!(
            shell.pages_agree(),
            "the navigation bypassed `show_page` and left the sidebar out of step"
        );
        // One unit, and it is the toast's expiry rather than the navigation:
        // `show_page` adds `page_entry_task`'s work only when it *arrives*, and
        // only `Page::Runners` has any (it fetches the release list). So the
        // count here measures the toast, and the navigation is measured by the
        // two records this test asserts above — a batch whose other half is
        // `Task::none()` asking for one unit is the correct answer, and an
        // assertion of `>= 2` here would have been a claim about `Page::Library`
        // having entry work it does not have.
        assert!(
            effect.task_units >= 1,
            "the toast's expiry task is missing, so the toast appears and never \
             leaves; the arm asked for {}",
            effect.task_units
        );
        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("Installed “Steam”"),
            "the reference's message is the one `gameInstalled` carried; toasts: \
             {toasts}"
        );
        assert!(
            toasts.contains("Play"),
            "the toast has no action — the reference passes \"Play\" as \
             `showPassiveNotification`'s third argument, and without it the user \
             has to find the entry themselves; toasts: {toasts}"
        );
    }

    /// **The failure branch clears the guard, resets the bar, and says why.**
    ///
    /// `bridge.py:896-900`, reachable here only from
    /// [`Message::EasyInstallFailed`] — the variant `architecture.md` §2.2's set
    /// had no room for. All three parts matter: a page left `busy` cannot start
    /// another install, a bar left drawn claims work that is not happening, and
    /// an unreported failure is a press that did nothing.
    ///
    /// The name in the sentence comes from [`State::running_install`] rather
    /// than from the message, which is why the fixture holds one.
    #[test]
    fn a_failed_install_reports_the_installers_name_and_clears_the_guard() {
        let prefix = install_prefix("failed");
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;
        shell.state.progress = Some(0.3);
        shell.state.running_install = Some(install_record(&prefix, "install-7"));

        let effect = observe(
            &mut shell,
            Message::EasyInstallFailed {
                message: "the download failed".to_string(),
            },
        );

        assert!(effect.task_units > 0, "a failed install said nothing");
        assert!(
            format!("{:?}", shell.state.toasts)
                .contains("Could not install Steam: the download failed"),
            "the notice is not `f\"Could not install {{name}}: {{message}}\"`; \
             toasts: {:?}",
            shell.state.toasts
        );
        assert!(
            !shell.state.easy_busy && shell.state.progress.is_none(),
            "the guard or the bar outlived the failure, so the page cannot be \
             used to try again"
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
    ///
    /// # The library is set, not inherited
    ///
    /// This read `Shell::new()` and asserted *its* library was empty, which is a
    /// claim about the machine the suite runs on rather than about the dispatch
    /// arm: `Shell::new()` opens the real `$XDG_CONFIG_HOME/gamehandler/games.json`,
    /// so the test passed only for a developer who does not use the app and
    /// failed for one who does. It failed here for exactly that reason, after a
    /// sample in this file wrote the real file — the empty library is now given
    /// to the shell the way its sibling test gives one, so what is under test is
    /// the arm and only the arm. The path is never written to, so unlike
    /// `shell_with_work_to_do`'s it needs no per-call counter.
    #[test]
    fn the_library_page_draws_the_library_and_not_the_placeholder() {
        let mut shell = Shell::new();
        shell.state.library = Library::new_at(
            Some(std::env::temp_dir().join("gh-empty-library/games.json")),
            0.0,
        );
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

    /// **The Installers page draws the catalog and not the placeholder — the
    /// page T-38 landed, and the one the arm had to be rewritten for.**
    ///
    /// The counterpart of the Plugins test above. `view::installers`'s own tests
    /// pin the rows, the filters and the guard, and none of them can see whether
    /// [`Shell::view_body`] calls the page at all — which was literally true
    /// until T-38, when the arm was `pending_page(Page::Installers, "T-12")` and
    /// every assertion in that module passed over a page no user could reach.
    ///
    /// # The assertion that is about the *shell* rather than the page
    ///
    /// `shell.state.installer_catalog` is not read from the page: it is the
    /// field `State::refresh_installers` fills and `Shell::new` primes. So
    /// requiring the drawn names to be the whole nine is also the check that the
    /// priming happened — an unprimed shell draws `EMPTY_TEXT` over an empty
    /// list, which looks like a search that matched nothing rather than like a
    /// page nothing filled.
    #[test]
    fn the_installers_page_draws_the_catalog_and_not_the_placeholder() {
        let mut shell = Shell::new();
        let _ = shell.show_page(Page::Installers);
        let drawn = drawn_strings(shell.view_body());

        assert_eq!(
            shell.state.installer_catalog.len(),
            gamehandler_core::installers::installers().len(),
            "`Shell::new` left the primed catalog short of the whole recipe list"
        );
        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::installers::INTRO),
            "the page's own explainer is not drawn; drawn: {drawn:?}"
        );
        for row in &shell.state.installer_catalog {
            assert!(
                drawn.iter().any(|text| text == &row.name),
                "{} is in the catalog but not drawn; drawn: {drawn:?}",
                row.name
            );
        }
        assert!(
            drawn.iter().any(|text| text == "Install"),
            "no card drew an Install button; drawn: {drawn:?}"
        );
        // The tooltip sentence is deliberately **not** asserted here, and the
        // reason is measured rather than assumed: `iced_widget`'s `Tooltip`
        // traverses only its `content` in `operate` (`tooltip.rs:372-383`), so a
        // `drawn_strings` of this page returns the button's "Install" and never
        // the sentence beside it. An assertion here would be a test of
        // `install_tooltip`'s own body, which already has one. See
        // `installer_card` for the note beside the wrap.
        assert!(
            !drawn.iter().any(|text| text.contains("has not been ported yet")),
            "the placeholder is gone from the dispatch arm; drawn: {drawn:?}"
        );
    }

    /// **The filter arms rebuild the catalog, and the guard above could not see
    /// it if they stopped.**
    ///
    /// This test exists because of a mutation that *survived*. The `expected`
    /// list on `only_the_written_handlers_change_anything` carries a sentence
    /// saying `SetInstallerSearch`/`SetInstallerCategory` are listed "for a
    /// reason that is not the filter" — that an arm which ran the filter and
    /// forgot `State::refresh_installers` "would still be observed here, because
    /// `refresh_installers` writes three fields". Deleting the
    /// `self.state.refresh_installers();` line from the arm left that guard
    /// green, so the sentence was false: the two samples change
    /// `installer_search`/`installer_category` on their own, and that write is
    /// what puts them in `changed`. A guard whose stated reason is false is the
    /// defect this repository calls #69, and the repair is the assertion that
    /// was missing rather than a softer sentence.
    ///
    /// # What is asserted
    ///
    /// The catalog after the message is `installer_rows` of the state *after*
    /// the message — which is the whole of what the refresh does. The control is
    /// the first assertion: the needle must really narrow the list, or "the
    /// catalog equals the filtered rows" would be satisfied by a filter that
    /// matched everything and a refresh that did nothing. The count is the
    /// un-primed-and-primed distinction from the page test above, measured here
    /// as a change rather than as a size.
    #[test]
    fn a_filter_rebuilds_the_catalog_the_page_draws() {
        let mut shell = Shell::new();
        let before = shell.state.installer_catalog.len();
        // From the data layer, not a literal: the needle is narrowed by the
        // same function the page filters with, so a catalog that grew a second
        // Steam-matching recipe keeps this test honest.
        let narrowed = crate::view::installers::installer_rows("steam", "");
        assert!(
            narrowed.len() < before,
            "\"steam\" no longer narrows the catalog ({before} rows before, {} after), \
             so the assertion below would hold for an arm that did nothing",
            narrowed.len()
        );

        let _ = shell.update(Message::SetInstallerSearch("steam".to_string()));

        assert_eq!(
            shell.state.installer_catalog,
            crate::view::installers::installer_rows(
                &shell.state.installer_search,
                &shell.state.installer_category
            ),
            "the arm wrote the search text but did not rebuild the catalog from it, \
             so the page still draws the rows it drew before the user typed"
        );
        assert_eq!(
            shell.state.installer_catalog, narrowed,
            "the rebuilt catalog is not the filtered list"
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

    /// A shell with the game form open over `Page::Library`.
    ///
    /// `is_new` picks `newGameTemplate` or `getGame` — the two the message arms
    /// use — because the layer's title and action label are read from it, and a
    /// fixture that only ever opened the add form would let an edit form draw
    /// "Add" forever.
    fn shell_with_form_open(is_new: bool, is_linux: bool) -> Shell {
        let mut shell = shell_with_work_to_do();
        let _ = shell.show_page(Page::Library);
        let mut form = if is_new {
            GameForm::new_template(&shell.state.settings, "layer-game".to_string())
        } else {
            GameForm::from_game(
                shell
                    .state
                    .library
                    .get("g")
                    .expect("the fixture holds `g`"),
            )
        };
        form.is_new = is_new;
        form.is_linux = is_linux;
        shell.state.game_form = Some(form);
        shell
    }

    /// **The form is a layer above the page, and it replaces what is under it.**
    ///
    /// All four directions, because each of them is a way this could look right
    /// and be wrong:
    ///
    /// - the layer draws the form — its title and a row label the page could not
    ///   produce;
    /// - the layer does **not** draw the page, which is what makes it a layer
    ///   rather than a merge and is why `view_with_overlays` returns one or the
    ///   other instead of stacking them;
    /// - with nothing open, the same function is the page, so the fall-through is
    ///   checked rather than assumed;
    /// - and the page's own body does not know about the form at all — a
    ///   `view_body` that had started drawing the layer would make every page test
    ///   above depend on whether a form happened to be open.
    ///
    /// The form's strings are `view::form`'s own constants rather than literals,
    /// so this cannot pass against a page that happens to say "Add Game".
    #[test]
    fn the_form_is_a_layer_over_the_page_it_replaces() {
        let shell = shell_with_form_open(true, false);
        let layer = drawn_strings(shell.view_with_overlays());

        for expected in [
            crate::view::form::TITLE_ADD,
            crate::view::form::LABEL_TYPE,
            crate::view::form::SECTION_ADVANCED,
            crate::view::form::ACTION_CANCEL,
        ] {
            assert!(
                layer.iter().any(|text| text == expected),
                "the open form should draw {expected:?}; drawn: {layer:?}"
            );
        }
        assert!(
            !layer
                .iter()
                .any(|text| text == crate::view::library::ADD_FIRST_GAME),
            "the page is still drawn under the form, so this is a merge and not a layer; \
             drawn: {layer:?}"
        );

        // The page's own body is unchanged by the form being open. Asserted as an
        // *equality* against the same shell with the form closed rather than
        // against a string the page happens to draw: this shell has a search
        // filter on it, so the page's empty state is the filtered one, and naming
        // a string here would be asserting the fixture rather than the property.
        // The property is that `view_body` does not read `game_form` at all.
        let mut closed = shell_with_form_open(true, false);
        closed.state.game_form = None;
        assert_eq!(
            drawn_strings(shell.view_body()),
            drawn_strings(closed.view_body()),
            "`view_body` draws differently depending on whether a layer is open, so \
             every page test above now depends on the form"
        );

        // Nothing open: the same call is the page, with no form in it.
        let fallthrough = drawn_strings(closed.view_with_overlays());
        assert!(
            fallthrough.iter().any(|text| text == crate::view::library::NO_MATCHES_TITLE),
            "the fall-through is not the page; drawn: {fallthrough:?}"
        );
        assert!(!fallthrough.iter().any(|text| text == crate::view::form::TITLE_ADD));
    }

    /// **The edit form draws the edit form**, not the add form with values in it.
    ///
    /// `isNew` drives the title and the confirming action (`GameFormPage.qml:18`,
    /// `:32`) and is *not* derivable from the form having a `game_id` — both forms
    /// have one. A fixture that only built the add form would leave `TITLE_EDIT`
    /// and `ACTION_SAVE` unreachable from any test.
    #[test]
    fn the_edit_form_draws_its_own_title_and_action() {
        let shell = shell_with_form_open(false, false);
        let drawn = drawn_strings(shell.view_with_overlays());

        for expected in [
            crate::view::form::TITLE_EDIT,
            crate::view::form::ACTION_SAVE,
        ] {
            assert!(
                drawn.iter().any(|text| text == expected),
                "the edit form should draw {expected:?}; drawn: {drawn:?}"
            );
        }
        for absent in [crate::view::form::TITLE_ADD, crate::view::form::ACTION_ADD] {
            assert!(
                !drawn.iter().any(|text| text == absent),
                "the edit form drew the add form's {absent:?}; drawn: {drawn:?}"
            );
        }
    }

    /// **Find cover is drawn exactly when its message is handled.**
    ///
    /// [`crate::view::form::COVER_FETCH_MISSING`] is the port's statement that
    /// `FetchCoverForForm`'s arm is still empty, and it is read here rather than
    /// left as prose: the day the arm is written, this test fails until the
    /// constant is flipped, and flipping it puts the button on screen.
    ///
    /// Both directions, so neither "always draw it" nor "never draw it" passes:
    /// the button's label is absent exactly while the constant is `true`, and its
    /// message is handled exactly when the constant is `false`. Today that is
    /// absent-and-unhandled, and the assertion below names which of the two it is
    /// so a failure says what changed rather than only that something did.
    #[test]
    fn the_find_cover_button_is_drawn_iff_its_message_is_handled() {
        let handled = is_handled(
            &mut shell_with_work_to_do(),
            Message::FetchCoverForForm {
                token: 0,
                game_id: "g".to_string(),
                name: "Fixture".to_string(),
                exe: String::new(),
            },
        );
        let drawn = drawn_strings(shell_with_form_open(true, false).view_with_overlays());
        let button_on_screen = drawn.iter().any(|text| text == crate::view::form::FIND_COVER);

        assert_eq!(
            button_on_screen, !crate::view::form::COVER_FETCH_MISSING,
            "the button is drawn from `COVER_FETCH_MISSING` and nothing else"
        );
        assert_eq!(
            handled, !crate::view::form::COVER_FETCH_MISSING,
            "the reference's Find cover (`GameFormPage.qml:151-158`) is on screen only \
             while its message does nothing: button on screen = {button_on_screen}, \
             `FetchCoverForForm` handled = {handled}, `COVER_FETCH_MISSING` = {}. When \
             the arm lands, set the constant to `false` in the same change.",
            crate::view::form::COVER_FETCH_MISSING
        );
        // A *compile-time* assertion, because that is the strongest form this can
        // take: flipping `COVER_FETCH_MISSING` to `false` without drawing the
        // button breaks the build with this message rather than failing one test
        // a reader has to find. Clippy's `assertions_on_constants` is what asks
        // for the const block, and it is right to.
        const {
            assert!(
                crate::view::form::COVER_FETCH_MISSING,
                "this reached the state the constant exists for; delete the note in \
                 `view/form.rs` with it"
            )
        };
    }

    /// **The runner row is hidden exactly when it cannot be drawn disabled.**
    ///
    /// The reference's runner combo is `enabled: !form.isLinux`
    /// (`GameFormPage.qml:175`), libcosmic's `Dropdown` has no disabled state, and
    /// [`crate::view::form::RUNNER_ROW_HIDDEN_FOR_LINUX`] records the departure.
    /// This reads it: the row is on a Windows form and off a Linux one, and when
    /// the constant flips the second assertion is what fails.
    ///
    /// The *value* is still on screen in the other direction, which is why the
    /// Linux half asserts on the row's label rather than on the runner's name.
    #[test]
    fn the_runner_row_is_hidden_exactly_when_it_cannot_be_disabled() {
        let windows = drawn_strings(shell_with_form_open(true, false).view_with_overlays());
        assert!(
            windows.iter().any(|text| text == crate::view::form::LABEL_RUNNER),
            "a Windows game has a runner to choose; drawn: {windows:?}"
        );

        let linux = drawn_strings(shell_with_form_open(true, true).view_with_overlays());
        assert_eq!(
            linux.iter().any(|text| text == crate::view::form::LABEL_RUNNER),
            !crate::view::form::RUNNER_ROW_HIDDEN_FOR_LINUX,
            "a Linux game's runner row: the reference disables it (`:175`), and \
             `RUNNER_ROW_HIDDEN_FOR_LINUX` = {}",
            crate::view::form::RUNNER_ROW_HIDDEN_FOR_LINUX
        );
        // Compile-time, for the reason the Find-cover guard above gives.
        const {
            assert!(
                crate::view::form::RUNNER_ROW_HIDDEN_FOR_LINUX,
                "libcosmic gained a disabled dropdown; draw the row and set the \
                 constant to `false`"
            )
        };

        // Everything else is still drawn on the Linux form — a layer that drew
        // nothing would satisfy the assertion above.
        for expected in [
            crate::view::form::TITLE_ADD,
            crate::view::form::TEXT_ROWS[4].label,
            crate::view::form::SECTION_COMPAT,
        ] {
            assert!(
                linux.iter().any(|text| text == expected),
                "the Linux form should still draw {expected:?}; drawn: {linux:?}"
            );
        }
    }

    /// **The toggles' labels are drawn and are not observable** — the limit
    /// [`the_settings_page_draws_the_settings_and_not_the_placeholder`]
    /// documents, pinned so it cannot quietly stop being true.
    ///
    /// The label it hands the `Toggler` is **one of the form's own subtitles**
    /// rather than a string invented for the test, so what is measured is the
    /// claim `view/form.rs`'s `field_row` note makes about *this* page: that a
    /// switch's subtitle reaches no `Text` operation, which is why the form's
    /// toggle copy rests on [`view::form`]'s table and cannot be asserted from a
    /// render.
    ///
    /// If a future libcosmic makes `Toggler` build a child text widget, this test
    /// fails and that note becomes wrong — which is the point.
    #[test]
    fn the_toggler_labels_do_not_reach_the_text_operation() {
        use cosmic::widget::toggler;
        let subtitle = view::form::LAUNCH_TOGGLES
            .first()
            .expect("the form has launch toggles")
            .subtitle;
        assert!(
            !subtitle.is_empty(),
            "the row picked names no subtitle, so this measures nothing"
        );
        let drawn =
            drawn_strings::<Message>(toggler(true).label(subtitle.to_string()).into());
        assert!(
            drawn.is_empty(),
            "if this now lists {subtitle:?}, `Toggler` gained a child text widget \
             — update the note on the form's `field_row` and on this test's \
             callers. Drawn: {drawn:?}"
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

    /// **`Ctrl+F` shows the Library from wherever the user is** — the half of
    /// the reference's two actions (`Main.qml:126-134`) that can be observed.
    ///
    /// The other half is `focusSearch()`, which returns an `iced::Task`; a task
    /// is a value with no accessor, so no assertion can reach the widget
    /// operation in it and this test does not pretend otherwise. What it does
    /// hold is that the shortcut's navigation is not merely *reachable* — it is
    /// the same `show_page`, so the sidebar moves with it and `pages_agree`
    /// stays true, rather than a second assignment to `state.page` that would
    /// leave the panel highlighting the page the user just left.
    ///
    /// The starting page is *not* Library, deliberately: from Library the
    /// navigation would be a no-op and a broken implementation would pass.
    #[test]
    fn ctrl_f_shows_the_library_from_wherever_the_user_is() {
        let mut shell = Shell::new();
        let _ = shell.update(Message::NavigateTo(Page::Settings));
        assert_eq!(shell.state.page, Page::Settings, "start elsewhere");

        let _ = shell.focus_library_search();

        assert_eq!(shell.state.page, Page::Library, "Ctrl+F shows the Library");
        assert_eq!(
            shell.sidebar_page(),
            Some(Page::Library),
            "and the sidebar follows, or the panel highlights Settings while \
             the body draws the Library"
        );
        assert!(shell.pages_agree());
    }

    /// `CloseDialog` clears all three of the things a dialog can be: the game
    /// form, the pending game delete, and the pending runner removal.
    ///
    /// Asserted on all three, because the arm writes three fields and dropping
    /// any one is invisible in the others' checks.
    #[test]
    fn closing_the_dialog_clears_the_form_and_the_delete_confirmation() {
        let mut shell = shell_with_work_to_do();
        shell.state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });

        let _ = shell.update(Message::CloseDialog);

        assert!(shell.state.game_form.is_none(), "the form must be closed");
        assert!(
            shell.state.confirm_delete.is_none(),
            "a pending delete must not survive the dialog closing"
        );
        assert!(
            shell.state.confirm_remove_runner.is_none(),
            "a pending runner removal must not survive the dialog closing"
        );
    }

    /// **The runner dialog is a modal over the page, not a layer replacing
    /// it.**
    ///
    /// The reference's `removeRunnerDialog` is a `PromptDialog` over the
    /// Runners page: the page stays drawn underneath and the dialog names the
    /// pending removal in its title. Asserted in three directions, because each
    /// is a way this could look right and be wrong:
    ///
    /// - with a pending removal, the overlay draws the title and both actions
    ///   — the `"Remove {name}?"` the button's row names and the fixed Cancel
    ///   and Remove the reference's footer carries;
    /// - the page is still drawn underneath: the popover covers, it does not
    ///   replace, which is the opposite of the game form's layer above (and
    ///   why the form takes precedence when both are open);
    /// - with nothing pending, the same call is the page, with none of the
    ///   dialog's strings in it.
    ///
    /// The title and subtitle are the state's own values rather than literals,
    /// so this cannot pass against a dialog that titles a constant.
    #[test]
    fn the_runner_dialog_is_a_modal_over_the_page_it_names() {
        // `shell_with_work_to_do` holds an open form, and the form takes
        // precedence — so the dialog would wait underneath it and this test
        // would photograph the form. A fresh shell has no form open, which is
        // the state the dialog is drawn in.
        let mut shell = Shell::new();
        let _ = shell.update(Message::ConfirmRemoveRunner {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });
        let pending = shell.state.confirm_remove_runner.clone().unwrap();
        let drawn = drawn_strings(shell.view_with_overlays());

        for expected in [
            pending.title(),
            "Cancel".to_string(),
            "Remove".to_string(),
        ] {
            assert!(
                drawn.iter().any(|text| text == &expected),
                "the open dialog should draw {expected:?}; drawn: {drawn:?}"
            );
        }
        // The subtitle is long; assert it is drawn rather than equal, so a
        // re-wrapping does not fail what is a presence claim.
        assert!(
            drawn.iter().any(|text| text.contains("fall back to System Wine")),
            "the dialog should draw the reference's subtitle; drawn: {drawn:?}"
        );
        // The page underneath: a fresh shell opens on the Library page, and a
        // layer-replacing composition would have removed its strings. The
        // dialog is namespaced to the Runners page only by the state that
        // opens it — see the precedence test below — so what is asserted here
        // is the property, not the page: the body stays drawn under the
        // dialog. Asserted on the module's own constants rather than
        // literals, so a copy change fails in one place rather than here.
        for expected in [
            crate::view::library::NO_GAMES_TITLE,
            crate::view::library::ADD_FIRST_GAME,
        ] {
            assert!(
                drawn.iter().any(|text| text == expected),
                "the page must stay drawn under the modal; {expected:?} missing from {drawn:?}"
            );
        }

        // Nothing pending: the same call is the page, with no dialog in it.
        // A fresh shell draws the unfiltered empty state; assert on the
        // module's own constant rather than a literal, so a copy change fails
        // in one place rather than here.
        let closed = Shell::new();
        let fallthrough = drawn_strings(closed.view_with_overlays());
        assert!(
            !fallthrough.iter().any(|text| text == &pending.title()),
            "no pending removal, so no dialog title; drawn: {fallthrough:?}"
        );
        assert!(
            fallthrough
                .iter()
                .any(|text| text == crate::view::library::NO_GAMES_TITLE),
            "the fall-through is the page; drawn: {fallthrough:?}"
        );
    }

    /// **The form wins when both a form and a runner removal are open.**
    ///
    /// Two dialogs at once would need a stacking order, and the reference has
    /// no such state — each QML layer closes the other. The pending removal
    /// waits underneath the form rather than drawing over it: asserted as the
    /// form's title present and the dialog's title absent, so neither "both
    /// draw" nor "neither draws" passes.
    #[test]
    fn the_form_takes_precedence_over_the_runner_dialog() {
        let mut shell = shell_with_form_open(true, false);
        shell.state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });
        let pending = shell.state.confirm_remove_runner.clone().unwrap();

        let drawn = drawn_strings(shell.view_with_overlays());

        assert!(
            drawn.iter().any(|text| text == crate::view::form::TITLE_ADD),
            "the form must still be drawn; drawn: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text == &pending.title()),
            "the runner dialog must wait under the form, not over it; drawn: {drawn:?}"
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
    /// **T-19's acceptance was that [`PENDING_PAGES`] is empty, and T-38 got
    /// there** — by landing the last page rather than by clearing the table.
    /// This test is the evidence for that ordering: it failed at T-38 with
    /// `Installers is listed in PENDING_PAGES as T-12 but its body does not
    /// draw the placeholder`, and printed the real page's nine cards, until the
    /// entry was deleted. What it asserts now is that no page draws a
    /// placeholder — the same claim, with the list empty.
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
