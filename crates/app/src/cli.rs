//! The command line and the startup path — `ARCH-11`'s seam out of `main.rs`.
//!
//! Everything that runs **before** a window exists lives here: the argument
//! parser, the two headless verbs, the display check and the call that starts
//! the event loop. `main.rs` keeps the binary's entry point, which is [`run`]
//! and nothing else, and the whole of the interface behind it.
//!
//! The rule this half is a module of its own for is the one the crate's header
//! states: `--list`, `--launch` and `--version` are parsed and dispatched
//! **before** anything that could create a window, a renderer or an event loop
//! (DECISIONS D-12), because the shortcuts this app writes are already on
//! users' disks and must work on a headless machine. What is above that line
//! needs no window; what is below cannot exist without one.
//!
//! `docs/audit/ARCHITECTURE.md` ARCH-11.

use std::process::ExitCode;

use clap::Parser;
use gamehandler_core::models::{Game, Library};
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};
use gamehandler_core::{APP_NAME, VERSION};

use crate::{App, MIN_WINDOW, launch_grace, launch_process};

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
pub fn run() -> ExitCode {
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
    // An unreadable or unparsable file is the case the criterion above exists
    // for, and the one it was still failing: without this, `--list` printed
    // "the library is empty" with `rc=0` for a file the app could not read, so
    // a script could not tell a full library it failed to open from a build
    // that never opened the file at all. That is the same shape as the stub
    // this function's doc records being deleted. `BUGS.md` BUG-01.
    if library.load_status().is_destructive_to_save_over() {
        return vec![format!(
            "{APP_NAME}: the library file could not be read and was not modified: {}",
            library.path().display()
        )];
    }
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
    let library = Library::new(None);
    for line in list_lines(&library) {
        println!("{line}");
    }
    // A non-zero exit for a library the app could not read: the printed line
    // says so, but `--list`'s contract is a row per game to a script, and a
    // script reads the exit code. Silence-plus-zero is what the deleted stub
    // did, and this is the same failure wearing a better sentence.
    if library.load_status().is_destructive_to_save_over() {
        return ExitCode::FAILURE;
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
/// [`crate::Message::LaunchStarted`]) and this function's `could not launch {name}:
/// {exc}` are two different strings in the reference itself: the GUI's is
/// capitalised with typographic quotes and the CLI's is lowercase with a plain
/// colon (`main.py:38`). They are not unified here, because the CLI's copy is
/// what a `.desktop` shortcut's stderr shows and the tests pin it as such. What
/// the two *do* share is the `{exc}` text, which is why both are built from the
/// error's `Display` rather than from a second rendering of the failure.
pub(crate) fn launch_report(
    name: &str,
    attempt: Result<Option<String>, String>,
) -> (Option<String>, u8) {
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
/// shortcut this app writes invokes — [`crate::shortcut_command`] builds
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
/// [`crate::Message::LaunchStarted`]'s arm making the same call for the same reason
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
pub(crate) fn launch_game(game_id: &str) -> ExitCode {
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
            let _ = writeln!(out, "{APP_NAME}: could not start the interface: {error}");
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
            let settings = cosmic::app::Settings::default()
                .size(cosmic::iced::Size::new(1200.0, 800.0))
                // The reference sets a floor and this port did not, so the
                // window could be dragged to 1x1 and every page became
                // unusable. `UX-04`; the numbers are `Main.qml:12-13`'s.
                .size_limits(
                    cosmic::iced::Limits::NONE
                        .min_width(MIN_WINDOW.0)
                        .min_height(MIN_WINDOW.1),
                );
            // `String` rather than the framework's error type so the seam does
            // not depend on it; the message is all that is used.
            cosmic::app::run::<App>(settings, ()).map_err(|error| error.to_string())
        },
    );

    ExitCode::from(outcome.exit_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Most of these drive the CLI's pure halves directly, so `env` and
    // `native_game_library` below move with them.
    //
    // `library_with` and `ShortcutEnv` do **not**: they are shared with the
    // 130-odd tests that stay in `main.rs`'s test module, and the majority of
    // their callers are there (`library_with` 8 of 13, `ShortcutEnv` 2 of 3).
    // Moving them would have inverted the dependency for the larger group and
    // rewritten call sites in tests this change has no other reason to touch,
    // so they stay where they are and are imported here.
    use crate::tests::{ShortcutEnv, library_with};
    // The two "parses back" tests grade the `Exec=` line this binary writes
    // against the parser above, so they name both sides of that contract.
    use crate::{launcher_command_with, shortcut_command, shortcut_command_with};
    use gamehandler_core::plugins::SystemPluginEnv;

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

    /// `--list` prints one `id\tname` row per game, in the case-insensitive
    /// name order Python's default sort produces.
    #[test]
    fn list_lines_are_the_ids_and_names_in_name_order() {
        let (_root, library) =
            library_with("rows", &[("b", "Beta"), ("a", "Alpha"), ("c", "gamma")]);
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
        let (_root, library) =
            library_with("case", &[("z", "Zebra"), ("a", "apple"), ("m", "Mango")]);
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

    /// **A library the app could not read does not print the empty line.**
    ///
    /// The same criterion as the stub regression below, applied to the case
    /// that was still failing it: a truncated `games.json` printed "the library
    /// is empty", which is byte-identical to a genuinely empty library, and
    /// exited 0 — so a script could not tell a full library this build failed
    /// to open from a build that never opened the file. `BUGS.md` BUG-01.
    ///
    /// The fixture is the exact one the audit executed against the built
    /// binary: the closing brackets cut off a one-entry file.
    #[test]
    fn an_unreadable_library_says_so_and_is_not_reported_as_empty() {
        let root = std::env::temp_dir().join(format!("gh-cli-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("games.json");
        std::fs::write(&path, r#"[{"id": "alpha-1", "name": "Alpha""#).unwrap();

        let library = Library::new_at(Some(path.clone()), 0.0);
        let lines = list_lines(&library);

        assert_eq!(lines.len(), 1, "one diagnostic line, not a row per game");
        assert!(
            lines[0].contains("could not be read"),
            "the line must say the file could not be read: {lines:?}"
        );
        assert!(
            lines[0].contains(&path.display().to_string()),
            "it must name the file, since the user's next move is to go and look \
             at it: {lines:?}"
        );
        assert_ne!(
            lines,
            vec!["GameHandler: the library is empty"],
            "the empty sentence is reserved for a library that is genuinely empty — \
             this is the confusion the earlier stub was deleted for"
        );
        // And the file is untouched, which is the half that matters: telling the
        // user is worthless if the next write eats the data anyway.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"[{"id": "alpha-1", "name": "Alpha""#
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
                Some(
                    "GameHandler: could not launch Alpha: no Proton runner is configured"
                        .to_string()
                ),
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
    /// sloppy: [`launcher_command_with`] may return a `shlex.quote`d path containing
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
            command.starts_with(&launcher_command_with(&SystemPluginEnv)),
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

    /// The Flatpak form parses back into the launch verb and the game's id —
    /// the #90b check for the three-token launcher, which `skip(1)` cannot
    /// reach.
    #[test]
    fn a_flatpak_shortcut_parses_back_into_the_launch_verb_and_the_games_id() {
        let mut game = Game::new_named("Alpha");
        game.id = "11111111111111111111111111111111".to_string();
        let env = ShortcutEnv::new().with_var("FLATPAK_ID", "com.goshapps.GameHandler");
        let command = shortcut_command_with(&game, &env);

        let args: Vec<&str> = command.split_whitespace().skip(3).collect();
        let cli = Cli::try_parse_from(std::iter::once("gamehandler").chain(args.iter().copied()))
            .unwrap_or_else(|error| {
                panic!("{command:?} is not a command this binary accepts: {error}")
            });
        assert_eq!(
            cli.launch.as_deref(),
            Some(game.id.as_str()),
            "the Flatpak shortcut's arguments parsed as launch={:?}",
            cli.launch
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
        assert_eq!(
            code,
            ExitCode::SUCCESS,
            "a title that stayed up exited non-zero"
        );

        // Read back through a **fresh** Library over the same file: an
        // in-memory read would pass on a handler that set the field and never
        // saved, which is the whole of `mark_played`.
        let reread = Library::new_at(Some(root.join("games.json")), 0.0);
        let played = reread
            .get("native")
            .expect("the game should still be there");
        assert_ne!(
            played.last_played, before,
            "the game was launched and never marked as played"
        );
        let _ = std::fs::remove_dir_all(&root);
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
            assert!(!line.ends_with(' '), "{line:?} should not end in a space");
        }
    }

    /// **The window floor this app states is the reference's, and it is actually
    /// applied** — `UX-04`, both halves.
    ///
    /// # Why this reads two files rather than asserting a constant
    ///
    /// There are two ways `UX-04` can come back, and a constant compared against
    /// itself catches neither:
    ///
    /// * the number drifts from the reference's. So the first half parses
    ///   `main.qml`'s `minimumWidth`/`minimumHeight` — the authority the port is
    ///   specified by, still in the tree — and requires [`MIN_WINDOW`] to be
    ///   those values. A transcription verified by eye is verified by the same
    ///   eyes that made the mistake;
    /// * the constant survives and stops being *used*, which is the finding's
    ///   actual shape: the default it replaced was
    ///   `Limits::NONE.min_height(1.0).min_width(1.0)`, and a `size_limits` call
    ///   deleted from the builder would put the app back where it was with this
    ///   test still green. So the second half reads `cli.rs` and requires the
    ///   one construction of `cosmic::app::Settings` in it to carry the call.
    ///
    /// # What this cannot do, so nobody over-trusts it
    ///
    /// `Settings::size_limits` is `pub(crate)` in libcosmic, so the built value
    /// cannot be read back and compared — this checks that the call is written,
    /// not that the framework honoured it. Honouring is
    /// `libcosmic src/app/mod.rs:79-84`, cited in [`MIN_WINDOW`]'s doc, and it
    /// is not something this crate can observe from a unit test. A green here
    /// means "the floor is the reference's and the builder still asks for it".
    #[test]
    fn the_window_floor_is_the_references_and_the_builder_asks_for_it() {
        // ---- half one: the number is the reference's -----------------------
        let text = gamehandler_core::oracle_support::repo_file("gamehandler/qml/Main.qml");
        let qml_value = |key: &str| -> f32 {
            let line = text
                .lines()
                .find(|line| line.trim().starts_with(&format!("{key}:")))
                .unwrap_or_else(|| {
                    panic!(
                        "`{key}:` is not in Main.qml. It is where the reference \
                         states its window floor, and this test exists to keep \
                         this port's floor equal to it. If the reference stopped \
                         stating one, `MIN_WINDOW` has no authority to cite and \
                         this test should be deleted rather than guessed at"
                    )
                });
            let rest = line
                .trim()
                .strip_prefix(&format!("{key}:"))
                .expect("the prefix was just matched")
                .trim();
            rest.parse::<f32>().unwrap_or_else(|err| {
                panic!("`{key}:` in Main.qml is {rest:?}, which is not a number: {err}")
            })
        };
        let raw_width = text
            .lines()
            .find(|line| line.trim().starts_with("minimumWidth:"))
            .expect("the reference states a minimum width");
        assert_eq!(
            qml_value("minimumWidth"),
            MIN_WINDOW.0,
            "the reference's window floor is {}x{} ({raw_width:?}) and this port \
             states {}x{}. `UX-04`'s fix is to match the reference; changing the \
             constant without changing the reading means the port's floor is now \
             a number nobody chose",
            qml_value("minimumWidth"),
            qml_value("minimumHeight"),
            MIN_WINDOW.0,
            MIN_WINDOW.1
        );
        assert_eq!(
            qml_value("minimumHeight"),
            MIN_WINDOW.1,
            "as above, on the other axis"
        );

        // ---- half two: the builder still asks for it -----------------------
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs"),
        )
        .expect("this file must be readable");
        // The one place the app builds the framework's settings, searched in the
        // *non-test* half of the file: this test's own body names the string, so
        // counting over the whole file would count itself and the arithmetic
        // would be about this test rather than about the app. Splitting at the
        // test module is what makes the count a property of the code that runs.
        let tests_at = source
            .find("\nmod tests {")
            .expect("this file has a test module; the split below depends on it");
        let production = &source[..tests_at];
        let constructions = production
            .matches("cosmic::app::Settings::default()")
            .count();
        assert_eq!(
            constructions, 1,
            "this file builds `cosmic::app::Settings` {constructions} times; this \
             test grades the one at the app's startup and would have to name \
             which, rather than assuming there is one"
        );
        let start = production
            .find("cosmic::app::Settings::default()")
            .expect("counted above");
        // The expression runs to the `;` that closes the `let settings = …`, and
        // the comment lines are dropped *before* looking for it: the builder's
        // own comment contains a `;` mid-sentence (`\`UX-04\`; the numbers are
        // …`), so searching the raw text stops inside a comment and reports a
        // construction with no `size_limits` in the part it kept. That is a
        // measurement that reads what is convenient rather than what is there —
        // the finding this whole file is an audit of — so the comments come out
        // first, and the assertion's failure message prints what was kept.
        let expression: String = production[start..]
            .lines()
            .take_while(|line| !line.trim_end().ends_with(';'))
            .chain(
                production[start..]
                    .lines()
                    .skip_while(|line| !line.trim_end().ends_with(';'))
                    .take(1),
            )
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let expression = expression.as_str();
        assert!(
            expression.contains(".size_limits("),
            "the settings construction does not call `.size_limits(…)`, so the \
             window floor is back to the framework's default — \
             `Limits::NONE.min_height(1.0).min_width(1.0)` (`libcosmic \
             src/app/settings.rs:98`) — and the window can be dragged to 1x1 \
             again. That is `UX-04` exactly, and a `MIN_WINDOW` nothing applies \
             is the shape it came back in. The construction: {expression:?}"
        );
        assert!(
            expression.contains("MIN_WINDOW"),
            "the settings construction sets size limits but not from \
             [`MIN_WINDOW`], so the floor and the constant can disagree without \
             this test noticing: {expression:?}"
        );
    }
}
