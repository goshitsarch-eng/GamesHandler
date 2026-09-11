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

use std::process::ExitCode;

use clap::Parser;
use gamehandler_core::{APP_ID, APP_NAME, VERSION};

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

/// The application state.
///
/// T-01 holds only the COSMIC `Core`. The library, settings and view state
/// land in T-07 (`docs/migration/architecture.md` §2.3), which is also where
/// the pages, nav bar and dialogs move in.
pub struct App {
    core: cosmic::Core,
}

/// Messages handled by [`App::update`].
///
/// The variant set is deliberately empty: T-01 ships no interactive widgets.
/// T-07 replaces it with the ~50-variant enum specified in
/// `docs/migration/architecture.md` §2.2. Leaving it uninhabited here keeps
/// `update()` exhaustive by construction, so it cannot acquire the `_ => …`
/// arm that would later swallow a real message.
#[derive(Clone, Debug)]
#[allow(clippy::empty_enums)] // intentional until T-07 fills in the variants
pub enum Message {}

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
        (
            App { core },
            // There is no `Command` in this iced generation; the no-op is
            // `cosmic::task::none()`.
            cosmic::task::none(),
        )
    }

    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        // Exhaustive by construction — no wildcard arm, so adding a variant
        // without handling it is a compile error rather than a silent drop.
        match message {}
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
