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

use cosmic::app::ApplicationExt;
use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc::{UnboundedSender, unbounded as unbounded_channel};
use cosmic::widget::toaster::ToastId;
use cosmic::widget::{icon, nav_bar, toaster};
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
use gamehandler_core::{APP_ID, APP_NAME};

mod easy_install;

// The flow lives in `easy_install.rs` (ARCH-11); this is the crate root's view
// of it. Everything named here is `pub(crate)` and exists for exactly one
// caller in this file: an arm of `Shell::update`, or a test that asserts the
// arm does what it says.
use easy_install::{
    cancel_easy_install, complete_easy_install, cover_choice_message, cover_lookup,
    easy_install_wizard_finished, exe_choice_message, exe_file_filters, image_file_filters,
    installed_play_message, start_easy_install,
};
// The CLI and startup path (ARCH-11). `main` below is the only thing this
// file keeps from that seam, because it is the binary's entry point.
mod cli;
mod http;
mod icons;
mod shortcuts;
mod state;
mod theme;
// The module is declared here, rather than beside its first caller, because a
// declaration is what makes its tests run: without this line `cargo test`
// reports success 44 assertions short of the truth, which is the same defect
// as a check that never executes.
//
// This declaration used to carry a module-level `#[allow(dead_code)]`, with a
// `TODO(T-09)` saying the pages would call the components eventually (D-32).
// ARCH-25 measured what it actually covered: ten items, not the fifteen the
// row named. Nine were genuinely unused — the `RowWindow` helpers, the
// grid-tile cover stack (`cover_tile`, `tile_cover_spec`, `tile_height`,
// `PORTRAIT_RATIO`), the credits title — and one was a field read only by a
// test. The first nine are deleted and the tenth is `#[cfg(test)]`, so lint
// now reaches the whole view layer rather than trusting that it does.
mod view;

pub use state::{
    CoverHit, FormField, FormToken, GameForm, GameId, Page, PendingInstall, PrefixTool,
    ReleasesStatus, State,
};

/// Application entry point.
///
/// Everything behind it — the argument parsing, the headless `--list` and
/// `--launch` verbs, and the startup path that opens the window — is
/// [`cli::run`] (ARCH-11). What is left here is the binary's entry point,
/// which cannot live anywhere else.
fn main() -> ExitCode {
    cli::run()
}

/// The smallest window this app will draw, in logical pixels — `UX-04`.
///
/// The reference states a floor (`gamehandler/qml/Main.qml:12-13`:
/// `minimumWidth: 420; minimumHeight: 480`) and this port stated none, so the
/// framework's default applied verbatim: `Limits::NONE.min_height(1.0)
/// .min_width(1.0)` (`libcosmic src/app/settings.rs:98`), honoured at
/// `src/app/mod.rs:79-84`. The window could be dragged to 1x1 and every page
/// became unusable — and because the value was an implicit framework default
/// rather than a stated choice, nothing in the port read as "we chose no
/// minimum".
///
/// A floor is also the precondition for `UX-08`, `UX-09` and `UX-21`: the three
/// horizontal-overflow findings are only meaningful once there is a width the
/// layout is guaranteed to be handed.
///
/// The numbers are the reference's, not a re-derivation — `the_window_floor_is_
/// the_references` reads them back out of `Main.qml` rather than trusting this
/// constant, so changing either one alone fails a test.
pub const MIN_WINDOW: (f32, f32) = (420.0, 480.0);

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
    /// Focus the Library's search box. `Ctrl+F` — `root.showPage("library")`
    /// plus `root.libraryPage.focusSearch()` (`Main.qml:126-134`) — for the one
    /// state libcosmic's own binding cannot reach.
    ///
    /// `Ctrl+F` is normally answered by the framework and not by this app:
    /// `keyboard_nav::subscription()` matches it and `Cosmic::update` calls
    /// `Application::on_search` (`src/app/cosmic.rs:850`), which is [`App`]'s
    /// reply below. That subscription delivers only events whose status is
    /// `Ignored` (`src/keyboard_nav.rs:20-23`), so it is silent exactly while a
    /// text field has focus — the state `BUG-12` is about — and
    /// `shortcuts::search_the_framework_cannot_reach` answers the key there
    /// instead. Both routes end in the same [`Shell::focus_library_search`], and
    /// they are exclusive by construction, so one press navigates once.
    FocusLibrarySearch,
    /// Open the add-game form, with a fresh template. `newGameTemplate()`.
    OpenNewGameForm,
    /// Open the edit form for an existing game. `getGame(id)` + push.
    OpenEditGameForm(GameId),
    /// Close whatever overlay is open. `layers.pop()`.
    CloseDialog,
    /// Ask before deleting. `removeDialog` (`LibraryPage.qml:346-360`): the
    /// title names the game, Cancel closes, Remove removes the entry and
    /// leaves the prefix and game files on disk.
    ///
    /// This used to say the QML deleted immediately and cite `architecture.md`
    /// §2.5's behaviour-change list. That was read off `bridge.removeGame`
    /// alone; the confirm lives in the QML, which opens the dialog before the
    /// slot ever runs — so asking is parity, not a change, and §2.5's first
    /// item went with the correction.
    ConfirmDeleteGame(GameId),
    /// The user confirmed; do it. `removeGame()` past the dialog.
    DeleteGameConfirmed(GameId),
    /// Open the game's actions layer — the keyboard route to the menu
    /// (**UX-16**). `page.openGameMenu(game, opener)`.
    ///
    /// The reference reaches its `gameMenu` from two controls: a right-button
    /// press on the card or row, and the per-tile "More actions"
    /// `QQC2.ToolButton` (`LibraryPage.qml:209-213`, `:281-284`). Only the first
    /// reached the port, because the toolkit's `ContextMenu` opens from a
    /// pointer button release alone (`src/widget/context_menu.rs:441-460`), and
    /// the menu the QML opens is the route to every per-game action — including
    /// "Remove from library". The library's "More actions" control sends this;
    /// [`Shell::update`]'s arm opens [`State::game_menu`].
    ///
    /// # Why it carries the id rather than a position
    ///
    /// The same reason [`Message::ConfirmDeleteGame`] does: the control is drawn
    /// once per game, from that game's own builder
    /// (`view/widgets.rs::more_actions_button`), so the id is what the control
    /// already has in hand — a position would have to be resolved back to a game
    /// at the moment the message is handled, through a list that re-sorts under
    /// the user (the sort mode is a setting).
    OpenGameMenu(GameId),
    /// Open the reference's `exeDialog` (`GameFormPage.qml:334-347`).
    ///
    /// This used to carry an `ExeField` naming one of four path fields. The
    /// reference has no parameterised chooser — one exe dialog with an inline
    /// handler (`GameFormPage.qml:337-345`) — so U6 deleted the enum with the
    /// false premise and the message is what the dialog is.
    PickExeFile,
    /// The exe chooser closed. `None` means it was cancelled.
    ExeFileChosen(Option<String>),
    /// Open a chooser for a cover image, as opposed to fetching one.
    PickCoverFile,
    /// The cover chooser closed. `None` means it was cancelled.
    CoverFileChosen(Option<String>),
    /// A toast expired or was dismissed.
    DismissToast(ToastId),
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
    /// The Library page was scrolled: how far, and the box it happened in.
    ///
    /// # Why this is a message and not cached geometry
    ///
    /// The Library body builds only a window of its rows, so it has to know
    /// things it cannot compute itself: the scroll offset, and the size of the
    /// viewport it is laying into. Both are properties of the widget tree iced
    /// builds, and neither is readable from inside `view()` — the page is
    /// *inside* the scrollable, and its layout limits are `[0, ∞]` however tall
    /// the window is. iced publishes both through `Scrollable::on_scroll`
    /// (`iced/widget/src/scrollable.rs:183`), which is the only route from that
    /// widget back to this state.
    ///
    /// The variant carries four numbers rather than a
    /// [`Viewport`](cosmic::iced::widget::scrollable::Viewport), because
    /// `Viewport`'s accessors return iced types this crate would then have to
    /// keep in `State` for no benefit: `absolute_offset().y` and the two
    /// dimensions of its two `Rectangle`s are the whole of what the window
    /// computation reads, and naming them here is what makes
    /// `view::library::visible_range` a pure function of plain numbers rather
    /// than of a widget.
    ///
    /// `viewport_width` and `viewport_height` are `0.0` on the first publish,
    /// from a call that has no scrollable state to read yet
    /// (`iced/widget/src/scrollable.rs:1072`, reached from the `Scrollable`'s
    /// `update` on the frame the wrapper was constructed). `visible_range` and
    /// `grid_columns` each read a zero as "not known yet" and fall back to their
    /// own defaults, so a zero never becomes a window of no rows.
    ///
    /// The other direction — a stale non-zero value after the window is resized
    /// — cannot happen for long: iced publishes again on the first frame after
    /// the resize, because `notify_viewport` compares the bounds it was last
    /// told about against the current ones (`:2060-2074`).
    ///
    /// # The handler is a write, and that is the whole of it
    ///
    /// It stores the four numbers and publishes nothing. In particular it does
    /// **not** clamp the offset to the content: scrolling to a requested offset
    /// is what makes the window the right one on the frame the user asks for it,
    /// and the widget clamps only its own drawing. See
    /// `view::library::visible_range`.
    SetLibraryScroll {
        offset: f32,
        viewport_width: f32,
        viewport_height: f32,
        content_height: f32,
    },

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
    /// `reason` has three states, matching `LaunchedGame::failure`:
    /// `Ok(None)` when the title was still running when the grace period
    /// expired, which is the success case — and the reference does nothing at
    /// all with it (`report`, `bridge.py:478-483`), so neither does this.
    /// `Ok(Some(_))` is the runner's failure report, shown as a toast with the
    /// window brought back. `Err(_)` is the watch itself failing — a `waitpid`
    /// error, which the reference's `watch()` lets raise into `_async`'s
    /// catch-all so the bare exception text is notified (`bridge.py:154-161`)
    /// and `report` never runs. It is not folded into `Ok(None)`: a watch that
    /// cannot answer is not a healthy title (BUG-23).
    LaunchWatchFinished {
        game_id: GameId,
        reason: Result<Option<String>, String>,
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
        /// The name the lookup was started with, carried so a **failed** reply
        /// can name the game (`UX-15`). The variant has no game id — the form
        /// may be a template that has none yet — so the name is the only thing
        /// available to prefix the error with, and it is taken from the request
        /// rather than re-read from the form: the form can be edited between
        /// the request and the reply.
        name: String,
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
    ConfirmRemoveRunner { runner_id: String, name: String },
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
        /// What the scan could not look at, carried with the rows it did
        /// produce (`BUG-44`). `None` for the ordinary case; `Some` is
        /// reported as a notice by the arm that receives it, because the
        /// alternative — a runners folder that could not be read rendering as
        /// "no builds installed" — is indistinguishable from the empty library
        /// a user would go and reinstall against.
        problem: Option<String>,
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
    StartEasyInstall {
        installer_id: String,
        runner_id: String,
    },
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
    CompleteEasyInstall { token: String, path: Option<String> },
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
    /// The EasyInstall toast's Play action, which dismisses the toast and then
    /// launches (`BUG-33`). Two fields rather than one because a `Message` is
    /// the only thing `Toast::action`'s closure can return, and the reference's
    /// behaviour is two effects.
    PlayInstalled { game_id: GameId, toast_id: ToastId },

    // ---- Plugins ----------------------------------------------------------
    /// Re-read the plugin list and recompute the rows. `refreshPlugins()`.
    RefreshPlugins,
    /// Install a plugin. `installPlugin()`.
    InstallPlugin(String),
    /// A plugin install finished.
    ///
    /// The error is [`view::plugins::InstallRunError`], not a `String`
    /// (`ARCH-10`): the payload is the one place the install's outcome travels,
    /// and rendering it to text *here* would put the flattening below the
    /// handler that has to act on it. It is only rendered at the toast, one
    /// line before a user reads it.
    PluginInstallFinished {
        plugin_id: String,
        result: Result<bool, view::plugins::InstallRunError>,
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
/// The nav row's glyph, as an embedded [`icons::Icon`] rather than a theme
/// name: the six `icon.name:` values of `Main.qml:67-108`, which
/// `the_shells_icons_are_the_reference_drawers_icons_in_order` still reads
/// out of the QML and compares through `Icon::legacy_name` (a test-only
/// helper: production renders bytes, never names).
fn page_icon(page: Page) -> icons::Icon {
    match page {
        Page::Library => icons::Icon::Games,
        Page::Installers => icons::Icon::Install,
        Page::Runners => icons::Icon::FolderDownload,
        Page::Plugins => icons::Icon::Plugins,
        Page::Credits => icons::Icon::Help,
        Page::Settings => icons::Icon::Configure,
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
            .icon(icon::icon(crate::icons::handle(page_icon(page))))
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
///
/// # A missing page answers `false` rather than panicking (`BUG-29`)
///
/// This used to be `.expect("every Page is in Page::ALL")`, which is a panic in
/// **release** as well as debug — an `expect` is not a `debug_assert!`. Worse,
/// it ran *before* [`Shell::show_page`]'s `debug_assert!(self.pages_agree())`,
/// so the invariant that check exists to report could never observe the case: a
/// `Page` variant absent from [`Page::ALL`] (reachable by adding a variant and
/// leaving the constant alone, which compiles because an array's length is
/// checked against the literal and not against the enum) took the process down
/// on a sidebar click, on a build with `debug_assertions` off.
///
/// `false` is what the function already meant: `activate_position` returning
/// `false` and the lookup failing are the same fact — the model is not showing
/// the page that was asked for — and the caller already treats `false` as an
/// inconsistency to report rather than as a reason to abort. `Page::ALL` is now
/// held complete by `crates/app/tests/page_roster.rs`, at `cargo test` time,
/// naming the variant; this is the release path if that guard is ever bypassed.
fn activate_page(model: &mut nav_bar::Model, page: Page) -> bool {
    let Some(position) = Page::ALL.iter().position(|candidate| *candidate == page) else {
        return false;
    };
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

/// The game-removal dialog over the library it names: `removeDialog`
/// (`LibraryPage.qml:346-360`).
///
/// Same composition as [`remove_runner_dialog`] — [`dialog_over`], not a
/// `popover`, for the reason recorded there — and the same contract: the title
/// names the pending game, Cancel closes without removing, and Remove removes
/// the pending id and then closes. The subtitle is the reference's verbatim: the
/// entry goes, the prefix and game files stay.
///
/// `name` is looked up by the caller, which owns the library; the id fallback
/// never fires through the arms (an unknown id opens nothing), but the dialog
/// must draw *something* for an id with no game, and the id itself is what
/// there is.
fn remove_game_dialog<'a>(
    body: cosmic::Element<'a, Message>,
    name: &str,
    game_id: &str,
) -> cosmic::Element<'a, Message> {
    use cosmic::widget::{button, dialog};
    let popup: cosmic::Element<'a, Message> = dialog()
        .title(format!("Remove “{name}”?"))
        .body(
            "This removes the game from your GameHandler library. Its Wine prefix and game files are left on disk.",
        )
        // The id is what `Message::ConfirmDeleteGame` focuses (**UX-24**), and
        // it is on Cancel rather than on Remove for `view::REMOVE_GAME_CANCEL_ID`'s
        // reason: the prompt must open with the keyboard on the action that
        // keeps the game.
        .secondary_action(
            button::standard("Cancel")
                .id(view::REMOVE_GAME_CANCEL_ID.into())
                .on_press(Message::CloseDialog),
        )
        .primary_action(
            button::destructive("Remove")
                .on_press(Message::DeleteGameConfirmed(game_id.to_string())),
        )
        .into();
    dialog_over(body, popup)
}

/// A game's actions, as a layer over the page it belongs to (**UX-16**).
///
/// # Why a dialog and not the reference's popup menu
///
/// The reference opens a `QQC2.Menu` here (`LibraryPage.qml:209-213` calls
/// `page.openGameMenu`). libcosmic's equivalent is not reachable from outside
/// the crate — see [`view::library::game_menu_actions`] for the measurement, the
/// short version of which is that the widget's `Menu` is `pub(crate)` in a
/// private module (`src/widget/menu.rs:71`, `:81`) and `ContextMenu` opens on a
/// pointer button release alone (`src/widget/context_menu.rs:441-460`). So the
/// app draws the same entries with the widgets it does have, over the same
/// `dialog_over` composition the two prompts use — which is also what gives the
/// layer UX-06's input wall, so the page under it is not live while it is open.
///
/// # The title and the body
///
/// The menu names itself; a dialog has to name what it is showing. The title is
/// the game and the body says what the eight controls are, because the raw
/// alternative — a bare list of verbs under a bare game name — reads as a game
/// detail page rather than as the actions the context menu also offers.
fn game_menu_dialog<'a>(
    body: cosmic::Element<'a, Message>,
    game: &'a Game,
) -> cosmic::Element<'a, Message> {
    use cosmic::widget::{button, dialog};
    let popup: cosmic::Element<'a, Message> = dialog()
        .title(game.name.clone())
        .body("The actions this game's context menu offers, as a list.")
        .control(view::library::game_menu_actions(game))
        .secondary_action(button::standard("Cancel").on_press(Message::CloseDialog))
        .into();
    dialog_over(body, popup)
}

/// The Runners page's removal confirmation, `removeRunnerDialog`
/// (`RunnersPage.qml:257-272`), drawn as a modal over the page body.
///
/// The reference's dialog is a `Kirigami.PromptDialog`: its `title` is
/// `"Remove " + pendingRemove.name + "?"`, its `subtitle` the fixed sentence,
/// and its footer is Cancel plus a custom Remove action — Cancel closes, and
/// Remove sends the row's `runnerId` to `uninstallRunner` and then closes
/// (`RunnersPage.qml:259-271`). The port renders the same three parts through
/// libcosmic's own composition: `cosmic::widget::dialog` builds the titled card
/// (`src/widget/dialog.rs`), and [`dialog_over`] lays it over the body behind an
/// input-blocking scrim.
///
/// This is a free function over `(body, pending)` rather than a method on
/// [`State`] or a page module for the same reason [`Shell::view_body`]'s arms
/// borrow rather than build: the dialog needs the already-built body beneath
/// it, and the page's view does not know a dialog is open. P-37.
///
/// # Why not `popover`, and what the composition is now
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
/// The second version answered that with a `Column` — the dialog in one band,
/// the page in the next — and paid for it with UX-06: in sequence, the page is
/// neither behind the dialog nor blocked by it, so every control on it stayed
/// live under an open destructive prompt. [§ `dialog_over`](dialog_over) has
/// the measurement and the composition that replaces it. What is preserved is
/// the behaviour the reference's dialog promises: the title names the pending
/// removal, both actions are drawn, Cancel closes without removing, and Remove
/// removes the pending id and then closes.
///
/// # The Remove button's message
///
/// Remove sends [`Message::RemoveRunnerConfirmed`], not
/// [`Message::UninstallRunner`] — and the choice is load-bearing rather than
/// nominal: the confirmed arm clears the pending removal before removing, and
/// a button sending the direct route would remove while leaving the dialog
/// open. A built `Button`'s message is opaque (the sources are cited at
/// `installed_card`'s call site), so
/// `the_runner_dialog_is_a_modal_over_the_page_it_names` photographs the
/// *label* "Remove" and cannot tell which of the two variants it carries.
///
/// The source is the only place the choice is written down, so
/// `the_runner_prompt_s_remove_button_sends_the_confirmed_removal_and_not_the_direct_one`
/// — a `#[cfg(test)]` item, named in prose rather than linked — scans this
/// function's body for it. That test's own docblock carries the measurement and
/// the reason the named-helper fix this header used to propose was wrong
/// (ARCH-20).
fn remove_runner_dialog<'a>(
    body: cosmic::Element<'a, Message>,
    pending: &crate::state::PendingRunnerRemoval,
) -> cosmic::Element<'a, Message> {
    use cosmic::widget::{button, dialog};
    let popup: cosmic::Element<'a, Message> = dialog()
        .title(pending.title())
        .body(crate::state::remove_runner_subtitle())
        // Focused when the prompt opens, on the same reasoning as the game
        // prompt's Cancel above — [`view::REMOVE_RUNNER_CANCEL_ID`] (**UX-24**).
        .secondary_action(
            button::standard("Cancel")
                .id(view::REMOVE_RUNNER_CANCEL_ID.into())
                .on_press(Message::CloseDialog),
        )
        .primary_action(
            button::destructive("Remove")
                .on_press(Message::RemoveRunnerConfirmed(pending.runner_id.clone())),
        )
        .into();
    dialog_over(body, popup)
}

/// A confirmation dialog over the page it asks about: the page, the input wall,
/// the dialog — bottom to top (UX-06).
///
/// # Why this is a `Stack` and no longer a `Column`
///
/// The `Column` it replaces put the two parts in *sequence* — the dialog in one
/// band, the page in the next — so the page was neither behind the dialog nor
/// blocked by it, and every control on it stayed live under an open destructive
/// prompt. Measured before the change, at a 420 px window: a click on the
/// library's own "Add your first game" button, while `Remove “GE-Proton9-5”?`
/// was open, still published `OpenNewGameForm`. The composition the finding
/// asks for — an input-blocking surface *between* the two — cannot be written
/// as a `Column` at all: a third child of a `Column` is a third band, not a
/// layer, so a full-size wrapper there would push the dialog off the bottom
/// rather than sit behind it. `Stack` is iced's "content on top of other
/// content" widget and is the composition the three-part description needs.
///
/// What the `Column` was *right* about is kept: this is not a `popover`. The
/// rationale for abandoning `Popover` ([`remove_runner_dialog`]'s header) is
/// that `Popover::operate` skips both halves when `modal && popup.is_some()`,
/// which makes this file's `drawn_strings` instrument photograph an empty tree —
/// and `Stack::operate` traverses *every* child (`iced/widget/src/stack.rs:212-231`),
/// so the dialog, the scrim and the page under them are all still visible to it.
///
/// # What the page's layout does now, stated rather than hidden
///
/// The page is no longer reflowed downward when a dialog opens; it is drawn
/// where it always is and the scrim covers it. That is the reference's own
/// behaviour — `Kirigami.PromptDialog` is a floating `Popup`, not a band above
/// the page (`LibraryPage.qml:346`, `RunnersPage.qml:257`) — and it is a
/// visible change from the previous revision rather than a side effect of it.
fn dialog_over<'a>(
    body: cosmic::Element<'a, Message>,
    popup: cosmic::Element<'a, Message>,
) -> cosmic::Element<'a, Message> {
    use cosmic::iced::Length;
    use cosmic::iced::widget::Stack;
    // Explicitly `Fill` on both axes: a `Stack`'s size is otherwise taken from
    // its base layer's size *hint*, and the page below hands back `Shrink`
    // (`Container::size` reports the length it was given, and these pages set
    // none), which would collapse the stack to the body's intrinsic size.
    Stack::new()
        .width(Length::Fill)
        .height(Length::Fill)
        .push(body)
        .push(dialog_scrim())
        .push(popup)
        .into()
}

/// The dimmed, input-blocking surface a dialog draws over its page (UX-06).
///
/// `mouse_area` is what actually blocks: iced's `MouseArea::update` calls
/// `shell.capture_event()` on a left press **whether or not it carries a
/// message** (`iced/widget/src/mouse_area.rs:466-493` — the `on_press` arm
/// publishes, then both arms fall through to the same capture). So a scrim with
/// no handler at all is an input wall rather than an inert rectangle, and it
/// costs no message: clicking the dimmed page while a dialog is open does
/// nothing, which is what the reference's modal `Popup` does.
///
/// It sits *above* the page and *below* the dialog in the [`dialog_over`]
/// stack, so the dialog's own buttons are reached first —
/// `Stack::update` walks its children back to front and stops at the first one
/// that captures the event (`iced/widget/src/stack.rs:251-266`).
fn dialog_scrim<'a>() -> cosmic::Element<'a, Message> {
    use cosmic::iced::widget::container::Style;
    use cosmic::iced::{Background, Length};
    use cosmic::theme::Container;
    use cosmic::widget::{Space, container, mouse_area};

    mouse_area(
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .class(Container::custom(|theme| {
                // The theme's own `shade` — the colour libcosmic uses for the
                // shadow under a dialog (`src/theme/style/iced.rs:706`) — at
                // half alpha, so the dimming follows the light/dark scheme
                // rather than being a hard-coded black.
                let shade: cosmic::iced::Color = theme.cosmic().shade.into();
                Style {
                    background: Some(Background::Color(cosmic::iced::Color { a: 0.5, ..shade })),
                    ..Default::default()
                }
            })),
    )
    .into()
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
        // `showPage` re-detects the host on arrival (`Main.qml:41-44`), and this
        // arm was missing — `RefreshPlugins` had a handler and no producer
        // anywhere, so installing MangoHud from a terminal and clicking Plugins
        // left the row saying "Install" for the rest of the session. Adding the
        // helper outside the app is the ordinary way to install it, which is why
        // the reference re-detects at all rather than only after its own
        // install. `BUG-08`.
        //
        // Same call the message's handler makes, so there is one implementation
        // of "re-detect" whether it was triggered by arriving or by pressing.
        Page::Plugins => {
            state.refresh_plugins(&gamehandler_core::plugins::SystemPluginEnv);
            cosmic::app::Task::none()
        }
        _ => cosmic::app::Task::none(),
    }
}

impl Shell {
    /// `playGame()` (`bridge.py:461-485`), as a method so the two entry points
    /// that mean it — `Message::LaunchGame` and the EasyInstall toast's Play
    /// action — cannot drift apart.
    ///
    /// The lookup and its sentence are the reference's first two lines: an id
    /// the library does not hold is "Select a game first", not silence — this is
    /// the one entry point of the four that says so.
    ///
    /// Choosing an action closes the layer it was chosen from (**UX-16**): the
    /// layer stands in for the reference's `gameMenu`, and a menu that stayed
    /// open after an item was chosen is the one thing a menu never does.
    fn play_game(&mut self, game_id: &str) -> cosmic::app::Task<Message> {
        self.state.game_menu = None;
        let Some(game) = self.state.library.get(game_id).cloned() else {
            return self.state.toast_task("Select a game first".to_string());
        };
        let runners = self.state.runner_manager();
        // One worker for both messages, so the "Launching…" report cannot
        // overtake the launch that justifies it and the grace cannot begin
        // before the process exists. The shape is
        // `view::runners::install_runner_task`'s — a thread and a channel,
        // because the work is blocking and there is more than one thing to say
        // — and the receiver's drop ends the stream.
        let (sender, receiver) = unbounded_channel::<Message>();
        std::thread::spawn(move || launch_and_watch(&game, &runners, &sender));
        cosmic::app::Task::stream(receiver.map(cosmic::Action::App))
    }

    /// A shell over a real library, settings and runner manager, the way
    /// [`App::init`] builds one — but with no window, so a test can have one.
    ///
    /// The settings are **not** the user's: they load from a fresh temp path,
    /// unique per call, so a mutation arm's save lands in a fixture no other
    /// test shares. The library keeps the real path here (callers that write
    /// it redirect, as `shell_with_work_to_do` does), but every settings arm
    /// saves, so sharing the real file would make each of those tests write
    /// the user's `settings.json` — and read each other's values. The counter
    /// is what keeps parallel calls apart, for the reason that fixture's own
    /// comment measures.
    #[cfg(test)]
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let settings_root = std::env::temp_dir().join(format!(
            "gh-shell-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&settings_root);
        let mut state = State::new(
            Library::new(None),
            Settings::load(Some(settings_root.join("settings.json"))),
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
    ///
    ///   This is a **debug** assertion, so it is absent from a release build —
    ///   and until `BUG-29` the case it names did not reach it even in debug.
    ///   A `Page` variant missing from [`Page::ALL`] made `activate_page`
    ///   `expect`-panic before this line, in every profile.
    ///   `crates/app/tests/page_roster.rs` now holds `Page::ALL` complete at
    ///   test time, which is where a divergence should surface; `activate_page`
    ///   answers `false` for the release path.
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
        // The reference's `showPage` tears the layers down before it pushes the
        // page:
        //
        //     while (pageStack.layers.depth > 1)
        //         pageStack.layers.pop()
        //     pageStack.clear()
        //     pageStack.push(pageFor(name))
        //
        // — `Main.qml:37-45`, and `GameFormPage` is the one thing pushed as a
        // layer (`:47-53`). So navigating away from an open form **closes** it
        // in the reference, and the port did not: `view_with_overlays` returns
        // the form before it consults the page, so the body kept drawing the
        // form while the sidebar moved to the page the user actually chose —
        // the rail and the body disagreeing about which page is showing.
        //
        // This is the right place rather than a `Message::NavigateTo` arm,
        // because the sidebar's own clicks reach here without passing through
        // that message, and the reference's rule lives in `showPage` for the
        // same reason. The form's own Cancel and Save are unaffected: neither
        // navigates, so neither passes through here (`CloseDialog` clears the
        // form directly, and `SaveGameForm` clears it before returning its
        // toast).
        //
        // `confirm_delete` and `confirm_remove_runner` are cleared with it,
        // which is the `pageStack.clear()` half — the reference's dialogs are
        // children of the page it destroys and re-pushes, so their state does
        // not survive a navigation either.
        self.state.clear_overlays();
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
                    covers: &self.state.cover_cache,
                    // The four numbers iced published from the scrollable. The
                    // width is in here rather than read off the window because
                    // it comes from the same source as the rest — the bounds
                    // the scrollable last laid out at — and mixing a measured
                    // width with a published offset is how the two get to
                    // disagree about which frame they describe.
                    scroll: view::library::ScrollGeometry {
                        offset: self.state.library_scroll_offset,
                        viewport_height: self.state.library_scroll_viewport,
                        viewport_width: self.state.library_scroll_width,
                        content_height: self.state.library_scroll_content,
                    },
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
    /// The root element: the page, the toaster, and the live region.
    ///
    /// Split out of [`cosmic::Application::view`] so that a test can build it
    /// without an `App` — the same reason [`Self::view_with_overlays`] exists —
    /// and because the three layers have a fixed order that is worth stating in
    /// one place:
    ///
    /// * Every failure path in `bridge.py` ends in the `notify` signal, so the
    ///   toaster is wrapped around the whole body rather than placed inside a
    ///   page: a toast raised by one page must survive a navigation to another,
    ///   and `Toasts` lives in `State` for exactly that reason. The body itself
    ///   is [`Shell::view_with_overlays`], which falls through to
    ///   [`Shell::view_body`] when nothing is open above it.
    /// * [`view::a11y::live_notice`] wraps that (**UX-14**), publishing the
    ///   message the toaster draws to assistive technology. It goes out here
    ///   rather than inside a page for the same reason the toaster does: the
    ///   message outlives the page that raised it. Its own doc records that
    ///   nothing delivers the node at the pinned rev.
    fn view_root(&self) -> cosmic::Element<'_, Message> {
        view::a11y::live_notice(
            self.state.notice.as_deref(),
            toaster(&self.state.toasts, self.view_with_overlays()),
        )
    }

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
                covers: &self.state.cover_cache,
            });
        }
        let body = self.view_body();
        // The actions layer wraps first, so it is the *lowest* of the three:
        // choosing "Remove from library" from it clears the field and opens the
        // delete prompt, and the two therefore do not normally coexist — the
        // order here is the backstop, not the mechanism. An id the library no
        // longer holds draws nothing: the layer is a list of that game's
        // actions, and there are none for a game that is not there (the same
        // silence `Message::OpenGameMenu` keeps on the way in).
        let body = match self
            .state
            .game_menu
            .as_ref()
            .and_then(|id| self.state.library.get(id))
        {
            Some(game) => game_menu_dialog(body, game),
            None => body,
        };
        // The game dialog wraps next, so the runner dialog — when both are
        // somehow pending — draws outermost. Both pending at once takes
        // opening one dialog from inside another's page, which the mutual
        // clear in the two confirm arms already prevents.
        let body = match &self.state.confirm_delete {
            Some(id) => {
                let name = self
                    .state
                    .library
                    .get(id)
                    .map(|game| game.name.clone())
                    .unwrap_or_else(|| id.clone());
                remove_game_dialog(body, &name, id)
            }
            None => body,
        };
        match &self.state.confirm_remove_runner {
            Some(pending) => remove_runner_dialog(body, pending),
            None => body,
        }
    }

    /// Escape cancels whichever confirmation is open, and reports whether
    /// there was one (UX-07).
    ///
    /// # Why this is on `Shell` and not the body of `App::on_escape`
    ///
    /// The same reason [`Shell::focus_library_search`] is: `App` cannot be built
    /// off a display, so a dismissal written there would be unreachable from a
    /// test. Here the effect is a `State` write, which a test can read back.
    ///
    /// # What the return value is for, and what it does not reach
    ///
    /// It is the branch the finding names — "returning `Task::none()` when
    /// neither is set" — made observable. A `Task` has no accessor (the same
    /// gap [`Shell::focus_library_search`] records), so the `Task` half of that
    /// sentence is not testable from here and the *decision* is what is: a
    /// `bool` either dialog was open. Both of `App::on_escape`'s arms return
    /// `Task::none()` anyway — clearing two `Option` fields is not work a task
    /// can do — so the flag is a record for the test rather than a branch the
    /// caller takes.
    ///
    /// # Why the game form is not included
    ///
    /// [`Message::CloseDialog`] clears the form as well, and this deliberately
    /// does not: the two dialogs are `Kirigami.PromptDialog`s, whose Qt default
    /// `closePolicy` is `Popup.CloseOnEscape` (`LibraryPage.qml:346`,
    /// `RunnersPage.qml:257` — neither sets it, so both take the default), while
    /// the form is a pushed `Kirigami.Page` with no Escape binding anywhere in
    /// the QML. Escape dismissing a destructive prompt and Escape silently
    /// discarding a half-filled form are different promises, and only the first
    /// is the reference's.
    fn dismiss_dialogs(&mut self) -> bool {
        let had = self.state.confirm_delete.is_some()
            || self.state.confirm_remove_runner.is_some()
            // Esc closes the actions layer too (**UX-16**), and it is the
            // reference's own binding rather than an addition to it: the QML's
            // `QQC2.Menu` closes on Escape, and the toolkit's context menu takes
            // `close_on_escape: true` at `src/widget/context_menu.rs:34` for the
            // pointer-opened path this one replaces.
            || self.state.game_menu.is_some();
        self.state.confirm_delete = None;
        self.state.confirm_remove_runner = None;
        self.state.game_menu = None;
        had
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
            // `Ctrl+F` while a text field has focus. The same navigation as
            // `App::on_search`, through the same function, because the two are
            // one accelerator: the framework answers it when the event is
            // `Ignored` and `shortcuts` answers it when it is not, and both
            // end here. See [`Message::FocusLibrarySearch`].
            Message::FocusLibrarySearch => {
                return self.focus_library_search();
            }
            // `newGameTemplate()` (`bridge.py:382-399`): a fresh id, the stored
            // default runner, and the fifteen toggles from
            // `Settings::default_*` where a setting exists and the `Game`
            // dataclass default where one does not. `GameForm::new_template` is
            // that whole function, tested without a display.
            Message::OpenNewGameForm => {
                let id = gamehandler_core::models::new_id();
                self.state.game_form = Some(GameForm::new_template(&self.state.settings, id));
                self.state.confirm_delete = None;
            }
            // `getGame(id)` + push (`bridge.py:356-379`). An id the library does
            // not hold opens nothing rather than an empty form: the reference's
            // `getGame` returns `{}` and the QML's `gameData` would then be a map
            // whose every read falls through to its `|| ""`, which draws a blank
            // Add form under an "Edit Game" title. Doing nothing is the honest
            // version of that.
            Message::OpenEditGameForm(game_id) => {
                // Choosing an action closes the layer it was chosen from
                // (**UX-16**): the layer stands in for the reference's
                // `gameMenu`, and a menu that stayed open after an item was
                // chosen is the one thing a menu never does.
                self.state.game_menu = None;
                if let Some(game) = self.state.library.get(&game_id) {
                    self.state.game_form = Some(GameForm::from_game(game));
                    self.state.confirm_delete = None;
                }
            }
            Message::CloseDialog => {
                self.state.game_form = None;
                self.state.confirm_delete = None;
                self.state.confirm_remove_runner = None;
                // The actions layer is one of the things "whatever overlay is
                // open" names (**UX-16**); its Cancel button sends this.
                self.state.game_menu = None;
            }
            // `removeDialog.open()` (`LibraryPage.qml:346-360`): the pending id
            // the dialog draws. An id the library does not hold opens nothing
            // — the reference's `removeGame` returns early on it
            // (`bridge.py:449-451`), and there is no dialog for a game that is
            // not there. Opening one dialog closes the other: the port draws a
            // single modal layer, and two pending removals would nest.
            Message::ConfirmDeleteGame(game_id) => {
                // Choosing an action closes the layer it was chosen from
                // (**UX-16**): the layer stands in for the reference's
                // `gameMenu`, and a menu that stayed open after an item was
                // chosen is the one thing a menu never does.
                self.state.game_menu = None;
                if self.state.library.get(&game_id).is_some() {
                    self.state.confirm_delete = Some(game_id);
                    self.state.confirm_remove_runner = None;
                    // **UX-24**: the prompt opens with the keyboard on Cancel.
                    // The state above put the layer in the tree, so the focus
                    // issued here has a widget to land on; the arm is a
                    // fall-through when the id is unknown precisely so that a
                    // focus is never asked for a dialog nothing drew.
                    return cosmic::iced::widget::operation::focus(view::REMOVE_GAME_CANCEL_ID);
                }
            }
            // `removeGame()` past the dialog: the entry goes, the prefix and
            // game files stay on disk, and the toast names what left. The
            // pending id clears first, so the dialog closes even when the
            // removal fails — the P-37 shape, where the confirmed arm clears
            // before removing. An unknown id is silence past the clear.
            Message::DeleteGameConfirmed(game_id) => {
                self.state.confirm_delete = None;
                if let Some(name) = self
                    .state
                    .library
                    .get(&game_id)
                    .map(|game| game.name.clone())
                {
                    match self.state.library.remove(&game_id) {
                        Ok(()) => {
                            return self.state.toast_task(format!("Removed “{name}”"));
                        }
                        Err(error) => {
                            return self
                                .state
                                .toast_task(format!("Could not remove “{name}”: {error}"));
                        }
                    }
                }
            }
            // The keyboard route to the same actions the context menu holds
            // (**UX-16**). An id the library does not hold opens nothing, for
            // [`Message::ConfirmDeleteGame`]'s reason: there are no actions for
            // a game that is not there.
            //
            // The task focuses the layer's first control. Without it the layer
            // would open with the focus still on the "More actions" button that
            // opened it, and the keyboard user's next Tab would walk the rest of
            // the page before reaching the actions — the layer would be reachable
            // and still not usable. The id is the view's, for
            // [`Shell::focus_library_search`]'s reason: `view/library.rs` builds
            // the widget that carries it, so the two cannot drift into a focus
            // that names a control nothing draws. A `Task` has no accessor for
            // the operation it carries — `units()` is the count and nothing more
            // (`iced/runtime/src/task.rs:282`) — so what the layer's test reads is
            // the state this writes plus the fact that work was asked for.
            Message::OpenGameMenu(game_id) => {
                if self.state.library.get(&game_id).is_some() {
                    self.state.game_menu = Some(game_id.clone());
                    return cosmic::iced::widget::operation::focus(
                        view::library::game_menu_item_id(&game_id, 0),
                    );
                }
            }
            // `exeDialog` (`GameFormPage.qml:334-347`): "Select an executable"
            // with the exe-then-all filters, answered by `ExeFileChosen`.
            Message::PickExeFile => {
                return cosmic::app::Task::perform(
                    async move {
                        use cosmic::dialog::file_chooser::open;
                        let filters = exe_file_filters();
                        let mut dialog = open::Dialog::new()
                            .title("Select an executable")
                            .current_filter(filters[0].clone());
                        for filter in filters {
                            dialog = dialog.filter(filter);
                        }
                        let answer = dialog
                            .open_file()
                            .await
                            .map(|response| response.url().clone());
                        exe_choice_message(answer)
                    },
                    cosmic::Action::App,
                );
            }
            // `exeDialog.onAccepted` (`GameFormPage.qml:337-345`): the path
            // into the field, and the basename-minus-extension into the name
            // when the name is blank (P-20's second clause). A cancel — or a
            // closed form — writes nothing: the QML has no `onRejected`.
            Message::ExeFileChosen(path) => {
                let Some(path) = path.filter(|path| !path.is_empty()) else {
                    return cosmic::task::none();
                };
                let Some(form) = self.state.game_form.as_mut() else {
                    return cosmic::task::none();
                };
                form.set_field(FormField::ExePath, path.clone());
                if form.name.trim().is_empty() {
                    // `base.substring(0, dot)` for `dot > 0`, else `base`:
                    // `file_stem` agrees on every shape, including a leading
                    // dot (`.profile` is kept whole) and no extension. A path
                    // with no file name at all leaves the blank name blank,
                    // which is setting it to `""` with fewer steps.
                    if let Some(stem) = Path::new(&path).file_stem() {
                        form.set_field(FormField::Name, stem.to_string_lossy().into_owned());
                    }
                }
            }
            // `coverDialog` (`GameFormPage.qml:349-358`): "Select a cover
            // image" with the images filter, answered by `CoverFileChosen`.
            Message::PickCoverFile => {
                return cosmic::app::Task::perform(
                    async move {
                        use cosmic::dialog::file_chooser::open;
                        let filters = image_file_filters();
                        let mut dialog = open::Dialog::new()
                            .title("Select a cover image")
                            .current_filter(filters[0].clone());
                        for filter in filters {
                            dialog = dialog.filter(filter);
                        }
                        let answer = dialog
                            .open_file()
                            .await
                            .map(|response| response.url().clone());
                        cover_choice_message(answer)
                    },
                    cosmic::Action::App,
                );
            }
            // `coverDialog.onAccepted` plus `importCustomCover`
            // (`bridge.py:589-600`): the copy into the covers dir, the path
            // into the form, and the reference's notice on either outcome. A
            // cancel writes nothing; without an open form there is nowhere to
            // put the path, so the copy is not even attempted — and a form
            // without an id names no file, so that is silence too (both
            // constructors set the id, which is why this is a guard rather
            // than a behaviour).
            //
            // The success assignment is read, not tested: it writes the real
            // covers directory, which the suite must not touch. The failure
            // half is pinned below (a missing source fails before any write,
            // so that test is hermetic); the copy itself is core's
            // `copy_custom_cover`, tested there.
            Message::CoverFileChosen(path) => return cover_file_chosen(&mut self.state, path),
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
            // `crates/core/src/settings.rs:155-157`) **and then applies it**.
            // Storing is the guard;
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
                    // The save goes only on a change — the reference's own
                    // early return (`bridge.py:198`) — while the apply stays
                    // unconditional: the theme is a libcosmic global, not a
                    // field, so idempotent re-application is the arm's
                    // standing behaviour and a failed save must not skip it.
                    let changed = self.state.settings.color_scheme != value;
                    self.state.settings.color_scheme = value;
                    let mut tasks = Vec::new();
                    if changed && let Some(task) = self.state.save_settings_or_toast() {
                        tasks.push(task);
                    }
                    // Applies the value **stored**, not the one received: they
                    // are equal here (the guard just checked), and reading it
                    // back is what keeps the two from being able to diverge.
                    tasks.push(theme::apply(&self.state.settings.color_scheme));
                    // One task, not a batch of one: the success path returns
                    // exactly what it returned before the save existed.
                    return match tasks.len() {
                        1 => tasks.pop().expect("the apply task was just pushed"),
                        _ => cosmic::app::Task::batch(tasks),
                    };
                }
            }
            // `_set_view_mode` (`bridge.py:210-214`) and `_set_sort_mode`
            // (`223-228`) both *ignore* a value outside the allowed set rather
            // than storing it. The load path already does this
            // (`Settings::from_dict`'s view- and sort-mode folds,
            // `crates/core/src/settings.rs:158-163`); the message path did not, so a stale UI
            // could write a mode the code does not handle and the next start
            // would silently fold it back — the same defect D-34 names, on the
            // other side of the file.
            Message::SetViewMode(value) => {
                if VIEW_MODES.contains(&value.as_str()) && self.state.settings.view_mode != value {
                    self.state.settings.view_mode = value;
                    if let Some(task) = self.state.save_settings_or_toast() {
                        return task;
                    }
                }
            }
            Message::SetSortMode(value) => {
                if SORT_MODES.contains(&value.as_str()) && self.state.settings.sort_mode != value {
                    self.state.settings.sort_mode = value;
                    if let Some(task) = self.state.save_settings_or_toast() {
                        return task;
                    }
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
                    if let Some(task) = self.state.save_settings_or_toast() {
                        return task;
                    }
                }
            }
            // `_set_close_on_launch` (`bridge.py:247-250`): `bool(value)` then a
            // compare, so writing the stored value is a no-op rather than a
            // redundant save.
            Message::SetCloseOnLaunch(value) => {
                if value != self.state.settings.close_on_launch {
                    self.state.settings.close_on_launch = value;
                    if let Some(task) = self.state.save_settings_or_toast() {
                        return task;
                    }
                }
            }
            // `setDefaultToggle` (`bridge.py:264-269`): the name must be one of
            // the defaulted toggles, and the value must differ — both checked in
            // one place, `view::settings::set_toggle`, so the page's read and the
            // message path's write cannot disagree about which field a key means.
            // An unknown name is ignored, which is the reference's behaviour and
            // the reason `set_toggle` returns a `bool` instead of asserting.
            Message::SetDefaultToggle { name, value } => {
                if view::settings::set_toggle(&mut self.state.settings, &name, value)
                    && let Some(task) = self.state.save_settings_or_toast()
                {
                    return task;
                }
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
            // PERF-03: the offset and the box the page is drawn in. A plain
            // write — see the variant's docs for why there is no task here and
            // no clamping.
            //
            // The four are stored as given, including zeros, which is what the
            // pre-layout publish carries. Folding a zero to a default *here*
            // would be a second place that knows what a zero means;
            // `view::library::visible_range` is where that is decided and where
            // it is tested.
            Message::SetLibraryScroll {
                offset,
                viewport_width,
                viewport_height,
                content_height,
            } => {
                self.state.library_scroll_offset = offset;
                self.state.library_scroll_width = viewport_width;
                self.state.library_scroll_viewport = viewport_height;
                self.state.library_scroll_content = content_height;
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
            // `Library::add`/`update` return `PersistenceError` (`ARCH-10`) and
            // a write failure is not the user's to fix, so it is reported
            // rather than raised, the same way every other store in this shell
            // reports one. The variant is rendered, not discarded: its
            // `Display` names the file and the step that failed, which is what
            // a bare `io::Error` did not.
            Message::SaveGameForm(form) => return save_game_form(&mut self.state, form),
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
                return self.play_game(&game_id);
            }
            // The EasyInstall toast's Play action (`BUG-33`).
            //
            // QQC2's notification action **dismisses and then runs**; here the
            // action was `Message::LaunchGame` alone, so the game launched and a
            // "Installed … ▶ Play" toast sat over the Library the user had just
            // been moved to, for up to `Duration::Long`. The closure is handed a
            // `ToastId` and this is what to do with it.
            //
            // A variant rather than a `Task::batch` inside the action closure:
            // the closure returns one `Message`, so "dismiss, then launch" has
            // to be one message that means both. The dismissal is first and
            // unconditional — a launch that fails still leaves the toast gone,
            // which is what the reference does, because QQC2 dismissed it
            // before the slot ran.
            Message::PlayInstalled { game_id, toast_id } => {
                self.state.toasts.remove(toast_id);
                return self.play_game(&game_id);
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
                    tasks.push(
                        self.state
                            .toast_task(format!("Could not save “{name}”: {error}")),
                    );
                }
                tasks.push(self.state.toast_task(format!("Launching “{name}”…")));
                if self.state.settings.close_on_launch {
                    tasks.push(hide_window(true));
                }
                return cosmic::app::Task::batch(tasks);
            }
            // `report()` (`bridge.py:478-483`) for the two `Ok` arms: `None` is
            // the success case and the reference does nothing with it; `Some`
            // is an error nobody can see if close-on-launch hid the window, so
            // it is brought back before the notice is shown. `Err` is the
            // `watch()` half of `bridge.py` raising into `_async`'s catch-all
            // (`:154-161`): `fail` is unset there, so the bare error text is
            // notified and `report` — the show-window, the "stopped right
            // away" wording — never runs.
            Message::LaunchWatchFinished { game_id, reason } => match reason {
                Ok(None) => {}
                Err(message) => return self.state.toast_task(message),
                Ok(Some(reason)) => {
                    let name = self.state.game_name(&game_id);
                    return cosmic::app::Task::batch([
                        hide_window(false),
                        self.state
                            .toast_task(format!("“{name}” stopped right away: {reason}")),
                    ]);
                }
            },
            // `runPrefixTool()` (`bridge.py:487-500`). An id the library does
            // not hold returns silently, which is the reference's own first two
            // lines — unlike `playGame`, there is no "Select a game first" here.
            Message::RunPrefixTool { game_id, tool } => {
                return run_prefix_tool(&mut self.state, game_id, tool);
            }
            // `Opening {tool} for “{name}”` on success; `str(exc)` on failure,
            // which is the reference's bare exception text with no prefix
            // (`bridge.py:495-499`). `tool` is spelled the way the reference
            // spells it — the `winecfg` / `winetricks` string the QML sends.
            Message::PrefixToolStarted {
                game_id,
                tool,
                result,
            } => {
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
                // Choosing an action closes the layer it was chosen from
                // (**UX-16**): the layer stands in for the reference's
                // `gameMenu`, and a menu that stayed open after an item was
                // chosen is the one thing a menu never does.
                self.state.game_menu = None;
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return cosmic::task::none();
                };
                if game.is_linux() {
                    return self
                        .state
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
            // A silent success, and it is the reference's: `openPrefix`
            // (`bridge.py:509-517`) prints nothing once the directory exists and
            // the file manager has been asked to open it. Since `BUG-04` the
            // *asking* can fail (`xdg-open` missing), and that failure is not
            // silent — it takes the `Err` arm below.
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
                    async move {
                        Message::UrlOpened {
                            result: open_url(&url),
                        }
                    },
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
                // Choosing an action closes the layer it was chosen from
                // (**UX-16**): the layer stands in for the reference's
                // `gameMenu`, and a menu that stayed open after an item was
                // chosen is the one thing a menu never does.
                self.state.game_menu = None;
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
            // `fetchCover` (`bridge.py:537-560`). Silent for an unknown id —
            // the reference returns before spawning — and `""` for the exe of
            // a Linux game, which has none. The lookup runs on a worker
            // (D-48); the reply re-checks the game for the same reason
            // `bridge.py:547-549` does.
            Message::FetchCover(game_id) => {
                // Choosing an action closes the layer it was chosen from
                // (**UX-16**): the layer stands in for the reference's
                // `gameMenu`, and a menu that stayed open after an item was
                // chosen is the one thing a menu never does.
                self.state.game_menu = None;
                let Some(game) = self.state.library.get(&game_id).cloned() else {
                    return cosmic::task::none();
                };
                let name = game.name.clone();
                let exe = (!game.is_linux() && !game.exe_path.is_empty())
                    .then(|| PathBuf::from(&game.exe_path));
                return cosmic::app::Task::perform(
                    async move {
                        let result = cover_lookup(&name, &game_id, exe.as_deref());
                        Message::CoverFetchFinished { game_id, result }
                    },
                    cosmic::Action::App,
                );
            }
            // The `done` of `fetchCover` (`bridge.py:546-558`): re-check the
            // game, write what the hit carries, persist, and name the source.
            // A failure toasts the raw message — `_async`'s default `fail`
            // (`bridge.py:161`) — and a failed *save* is reported rather than
            // raised, the divergence `Message::LaunchStarted` records for
            // `mark_played`: the reference lets `done`'s exception escape to
            // the Qt loop, which is not a behaviour to reproduce.
            Message::CoverFetchFinished { game_id, result } => {
                return cover_fetch_finished(&mut self.state, game_id, result);
            }
            // `fetchCoverForForm` (`bridge.py:562-588`). The token is
            // allocated here, at emission — `State::next_form_cover_token`,
            // the same shape `runner_rows_token` uses — and the carried token
            // is ignored: the view cannot bump (it holds no `&mut State`), so
            // it sends `0` and the arm mints the real one. The `exe` is
            // stripped but not URL-decoded: `as_local_path`'s decoding is the
            // chooser's job (T-15), and the form has no chooser until U6, so
            // what reaches here is typed text, not a `file://` URL.
            Message::FetchCoverForForm {
                token: _token,
                game_id,
                name,
                exe,
            } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return self.state.toast_task("Enter a game name first".to_string());
                }
                let token = self.state.next_form_cover_token();
                let looked_up = name.clone();
                let exe = (!exe.trim().is_empty()).then(|| PathBuf::from(exe.trim()));
                let looking = self
                    .state
                    .toast_task(format!("Looking for artwork for “{name}”…"));
                let fetch = cosmic::app::Task::perform(
                    async move {
                        let result = cover_lookup(&name, &game_id, exe.as_deref());
                        Message::FormCoverFetchFinished {
                            token,
                            name: looked_up,
                            result,
                        }
                    },
                    cosmic::Action::App,
                );
                return cosmic::app::Task::batch([looking, fetch]);
            }
            // `coverFetched`'s half of `fetchCoverForForm` (`bridge.py:571-585`)
            // plus the QML that answers it (`GameFormPage.qml:57-68`). A reply
            // whose token is not the form's current one is a late answer
            // about an older lookup and is dropped — the counter is this
            // port's form of the QML's `token !== gameId` check, per lookup
            // rather than per form. The notice fires whether or not a form is
            // still open, as the reference's `notify` does; the fields are
            // written only while one is.
            Message::FormCoverFetchFinished {
                token,
                name,
                result,
            } => {
                if token != self.state.form_cover_token {
                    return cosmic::task::none();
                }
                // The same prefix as the library reply above (`UX-15`), with the
                // name carried on the message rather than re-read: there may be
                // no form open at all, and the one that was open can have been
                // edited since the request.
                let Ok(hit) = result else {
                    return self.state.toast_task(format!(
                        "Could not fetch a cover for “{name}”: {error}",
                        error = result.unwrap_err()
                    ));
                };
                // Same rewrite as `CoverFetchFinished` above, and the same
                // reason for the `forget`: the file is written by the lookup at a
                // path derived from the game id, so looking a second time
                // replaces the bytes this cache already answered for.
                self.state
                    .cover_cache
                    .forget(&hit.cover_path.to_string_lossy());
                if let Some(form) = self.state.game_form.as_mut() {
                    form.cover_path = hit.cover_path.to_string_lossy().into_owned();
                    if hit.appid != 0 {
                        form.steam_appid = hit.appid.to_string();
                    }
                    if !hit.category.is_empty()
                        && hit.category != "Uncategorized"
                        && form.category.trim() == "Uncategorized"
                    {
                        form.category = hit.category.clone();
                    }
                }
                return self.state.toast_task(format!(
                    "Cover found via {}: {}",
                    hit.origin_label(),
                    hit.name
                ));
            }

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
            message @ (Message::FetchReleases { .. }
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
            // # Dismiss-then-run, as QQC2 does it
            //
            // Clicking the toast's action in QQC2 **dismisses the
            // notification** and then runs it (`Main.qml:158-161`). The port's
            // action used to be `Message::LaunchGame` alone, so the game
            // launched and the toast stayed until its own duration expired, up
            // to `State::TOAST_DURATION` later. It is now
            // [`Message::PlayInstalled`], which carries the `ToastId` the
            // action closure is handed alongside the game id — two fields
            // rather than one because a `Message` is the only thing
            // `Toast::action`'s closure can return, and the dismissal and the
            // launch are two effects (`BUG-33`).
            //
            // The dismissal is first and unconditional: a launch that fails
            // still leaves the toast gone, which is what QQC2 does.
            Message::EasyInstallFinished { game_id, message } => {
                let navigate = self.show_page(Page::Library);
                // The one site whose duration was already `Long` before UX-14:
                // `"long"` is QQC2's own word in the reference
                // (`Main.qml:158`), and `Duration::Long` is the toolkit's
                // spelling of the same idea, not a number chosen here. It is now
                // the app's [`State::TOAST_DURATION`] as well, so this site and
                // the other 38 agree.
                self.state.note(&message);
                let toast = self
                    .state
                    .toasts
                    .push(
                        toaster::Toast::new(message)
                            .duration(State::TOAST_DURATION)
                            .action("Play".to_string(), move |toast_id| {
                                installed_play_message(&game_id, toast_id)
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
            // The page owns its arms (ARCH-23) — the same shape
            // [`view::installers::update`] and [`view::runners::update`] use,
            // and the reason given in `view::plugins::update`'s header.
            // `update` is total in the sense that matters here: it answers
            // `None` for every message it does not own, so the arms below
            // cannot be silently missed.
            Message::RefreshPlugins
            | Message::InstallPlugin(_)
            | Message::PluginInstallFinished { .. } => {
                if let Some(task) = view::plugins::update(&mut self.state, &message) {
                    return task;
                }
            }

            // ---- Internal plumbing -----------------------------------------
            // The `notify` signal. Every failure path in `bridge.py` ends here,
            // so this is the first handler worth having: it makes the others
            // implementable by reporting rather than by silently doing nothing.
            Message::Notify(text) => {
                // Through [`State::toast_task`] rather than a second `push`, so
                // that this route and the `update`-arm route cannot come apart:
                // the duration (**UX-14**) and the copy the live region
                // announces are set there, and a `push` written out again here
                // would be a third place to forget one of them. It is the same
                // `Toasts::push` and the same `Task` mapping the comment above
                // describes — `Task::none()` here would push the toast and never
                // expire it.
                return self.state.toast_task(text);
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

// ---------------------------------------------------------------------------
// The dispatcher's long arms, as functions (ARCH-12)
// ---------------------------------------------------------------------------

// `Shell::update` is one 968-line `match` over 59 arms. Its length is mostly a
// few long arms rather than deeply nested logic, and the exhaustiveness check
// the match buys is worth keeping — so the match stays, and the four arms that
// were longest became the functions below. Each takes `&mut State` because
// `State` is what the arm actually manipulates; `Shell` owns only the state, so
// handing it the state is the whole of what the arm needed from `self`.
//
// The file already did this for `open_prefix_folder`/`open_url`, and these
// follow that shape rather than inventing one.
//
// The arm that remains is one line and names the function, so a reader
// following a variant from `enum Message` still lands on the dispatch first.

fn save_game_form(state: &mut State, form: GameForm) -> cosmic::app::Task<Message> {
    // The reference's add-vs-update test (`bridge.py:406`): whether the
    // library already holds this id, not what the form says it is.
    let existing = state
        .library
        .get(form.game_id.as_deref().unwrap_or_default());
    match form.apply(existing) {
        Err(message) => state.toast_task(message),
        Ok(game) => {
            let is_new = existing.is_none();
            let name = game.name.clone();
            // `if not game.cover_path: self.fetchCover(game.id)`
            // (`bridge.py:444-445`): a save without artwork starts
            // a lookup. Captured before the store moves the game;
            // the notice below is the batched toast, and the fetch
            // is the same task `Message::FetchCover` builds.
            let fetch = game.cover_path.is_empty().then(|| {
                let game_id = game.id.clone();
                let name = game.name.clone();
                let exe = (!game.is_linux() && !game.exe_path.is_empty())
                    .then(|| PathBuf::from(&game.exe_path));
                cosmic::app::Task::perform(
                    async move {
                        let result = cover_lookup(&name, &game_id, exe.as_deref());
                        Message::CoverFetchFinished { game_id, result }
                    },
                    cosmic::Action::App,
                )
            });
            let stored = if is_new {
                state.library.add(game)
            } else {
                state.library.update(game)
            };
            if let Err(error) = stored {
                return state.toast_task(format!("Could not save “{name}”: {error}"));
            }
            // The two sentences are the reference's, em dashes and
            // all (`bridge.py:439`, `:442`).
            state.game_form = None;
            let notice = if is_new {
                format!("Added “{name}”")
            } else {
                format!("Updated “{name}”")
            };
            let notice = state.toast_task(notice);
            match fetch {
                Some(fetch) => cosmic::app::Task::batch([notice, fetch]),
                None => notice,
            }
        }
    }
}

fn cover_fetch_finished(
    state: &mut State,
    game_id: GameId,
    result: Result<CoverHit, String>,
) -> cosmic::app::Task<Message> {
    // `UX-15`: the bare string is `str(exc)` — the reference's own
    // default `fail` (`bridge.py:161`) — and the port keeps the raw
    // message, because that is what `NOTIFY_VOICES` pins. What it
    // adds is the context the reference has in its *own* voice for
    // every sibling failure (`bridge.py:444`'s save, `:474`'s add):
    // which game, and that a cover lookup is what failed. The
    // alternative — prefixing inside `_async`'s port, which is
    // every `Err` arm — would rewrite 39 messages the oracle pins.
    let Ok(hit) = result else {
        return state.toast_task(format!(
            "Could not fetch a cover for “{named}”: {error}",
            named = state
                .library
                .get(&game_id)
                .map(|game| game.name.clone())
                .unwrap_or_else(|| game_id.clone()),
            error = result.unwrap_err()
        ));
    };
    let Some(mut game) = state.library.get(&game_id).cloned() else {
        return cosmic::task::none();
    };
    // The lookup wrote the cover to a path in `covers_dir()`, derived
    // from the game id — so a *second* fetch for the same game
    // rewrites the file this cache is already holding. See
    // `CoverFileChosen` above for the full argument.
    state.cover_cache.forget(&hit.cover_path.to_string_lossy());
    game.cover_path = hit.cover_path.to_string_lossy().into_owned();
    if hit.appid != 0 {
        game.steam_appid = hit.appid;
    }
    if game.display_category() == "Uncategorized" && !hit.category.is_empty() {
        game.category = hit.category.clone();
    }
    let name = game.name.clone();
    if let Err(error) = state.library.update(game) {
        return state.toast_task(format!("Could not save “{name}”: {error}"));
    }
    state.toast_task(format!(
        "Cover set from {}: {}",
        hit.origin_label(),
        hit.name
    ))
}

fn cover_file_chosen(state: &mut State, path: Option<String>) -> cosmic::app::Task<Message> {
    let Some(path) = path.filter(|path| !path.is_empty()) else {
        return cosmic::task::none();
    };
    let Some(game_id) = state
        .game_form
        .as_ref()
        .and_then(|form| form.game_id.clone())
    else {
        return cosmic::task::none();
    };
    match gamehandler_core::covers::copy_custom_cover(
        Path::new(&path),
        &game_id,
        &gamehandler_core::paths::covers_dir(),
    ) {
        Ok(destination) => {
            // PERF-01/PERF-02: the copy just wrote bytes at
            // `destination`, which is `covers_dir()/<game_id>.<ext>`
            // (`copy_custom_cover`) — a path this cache may already
            // hold the *previous* cover's answer and pixels for,
            // because the same game picking a second custom cover
            // lands on the same name. Dropping the entry is what
            // makes the new file visible; without it the picker
            // would keep drawing the old cover until a restart, which
            // is the staleness `cover_cache`'s module docs admit and
            // this is the line that bounds it.
            state.cover_cache.forget(&destination.to_string_lossy());
            if let Some(form) = state.game_form.as_mut() {
                form.cover_path = destination.to_string_lossy().into_owned();
            }
            state.toast_task("Custom cover added".to_string())
        }
        Err(error) => state.toast_task(format!("Could not copy cover: {error}")),
    }
}

fn run_prefix_tool(
    state: &mut State,
    game_id: GameId,
    tool: PrefixTool,
) -> cosmic::app::Task<Message> {
    // Choosing an action closes the layer it was chosen from
    // (**UX-16**): the layer stands in for the reference's
    // `gameMenu`, and a menu that stayed open after an item was
    // chosen is the one thing a menu never does.
    state.game_menu = None;
    let Some(game) = state.library.get(&game_id).cloned() else {
        return cosmic::task::none();
    };
    if game.is_linux() {
        return state.toast_task("Prefix tools are only available for Windows games".to_string());
    }
    let runners = state.runner_manager();
    cosmic::app::Task::perform(
        async move {
            let result = start_prefix_tool(&game, &runners, tool);
            Message::PrefixToolStarted {
                game_id: game.id.clone(),
                tool,
                result,
            }
        },
        cosmic::Action::App,
    )
}

impl State {
    /// The duration every toast this app raises is given.
    ///
    /// **UX-14.** The toolkit's default is `Duration::Short`, 5000 ms
    /// (`src/widget/toaster/mod.rs:69-86`), and a failure that is gone in five
    /// seconds is one a user who looked away never learns about. `Long` is
    /// 15 000 ms and is the toolkit's own name for "this one matters": it is
    /// what the reference reaches for when it has something the user must act
    /// on — `showPassiveNotification(message, "long", "Play", …)` at
    /// `Main.qml:158` — and the port already had it at that one site
    /// ([`Message::EasyInstallFinished`]) before this.
    ///
    /// # Why every toast and not just the failures
    ///
    /// The reference's channel has *one* duration: `backend.notify` reaches
    /// `root.showPassiveNotification(message)` with no severity at all
    /// (`Main.qml:151`), and the failure/information distinction the audit
    /// draws is one the QML does not make. Splitting it here would be a
    /// divergence invented to satisfy a finding, and it would put a
    /// per-message judgement at 38 push sites where nothing today knows which
    /// is which.
    ///
    /// # What this does not fix
    ///
    /// A user who is away for longer than fifteen seconds still misses the
    /// message, and that residue is real: the audit's own recommendation for it
    /// is a *persistent line on the page that failed*, which is not what this
    /// is. The audit cites `view/form.rs:503-515` as the pattern to copy and
    /// that citation is wrong — at `532f1c0`, the commit the audit was written
    /// against, those lines are `can_save` and `save_message`, a pair of pure
    /// predicates, and `view/form.rs` renders no error text at all. There is no
    /// such pattern in this repository to reuse, so building one is a page-level
    /// design decision rather than a fix to the toaster, and it is named here
    /// as the remaining gap rather than half-built.
    const TOAST_DURATION: cosmic::widget::toaster::Duration =
        cosmic::widget::toaster::Duration::Long;

    /// A toast at [`State::TOAST_DURATION`], as the [`Task`](cosmic::Task) the
    /// application trait wants.
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
        self.notice = Some(text.clone());
        self.toasts
            .push(cosmic::widget::toaster::Toast::new(text).duration(Self::TOAST_DURATION))
            .map(cosmic::Action::App)
    }

    /// The message for the live region, for a toast pushed by hand.
    ///
    /// ⚠️ Every site that calls `toasts.push` directly has to call this with the
    /// same string, because `Toast`'s message cannot be read back out
    /// ([`State::notice`]). There is exactly **one** such site —
    /// [`Message::EasyInstallFinished`] — and it is the site that needs the
    /// toast's own builder rather than the generic one, because its toast carries
    /// a "Play" action built from the game id. Every other report in the app goes
    /// through [`State::toast_task`], which sets this itself.
    /// `the_hand_pushed_installed_toast_is_announced_too` is what keeps that
    /// count honest.
    fn note(&mut self, text: &str) {
        self.notice = Some(text.to_string());
    }

    /// Persist the settings an `update` arm just mutated: `_save_settings`
    /// (`bridge.py:190-192`), minus the signal Qt needs and Elm does not.
    ///
    /// `None` on success — the running app already renders from memory, so a
    /// good save has nothing to say. A failure is reported rather than
    /// raised, the way every other store in this shell reports one
    /// (`SaveGameForm`, `LaunchStarted`): the reference lets the slot raise,
    /// which in Qt means a traceback and no notice, and a traceback the user
    /// never sees is the worse half of that choice.
    fn save_settings_or_toast(&mut self) -> Option<cosmic::app::Task<Message>> {
        match self.settings.save() {
            Ok(()) => None,
            Err(error) => Some(self.toast_task(format!("Could not save settings: {error}"))),
        }
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
    launch::launch(
        game,
        runners,
        &SystemLaunchEnv,
        &NetpathsShares::new(&SystemEnv),
    )
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
/// # Why this does not call [`cli::launch_report`]
///
/// The CLI's [`cli::launch_game`] and this function drive the same two steps and
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
    // [`launch_grace`]. The `Err` arm is the wait itself failing, which
    // `bridge.py`'s `watch()` lets raise; it travels as its own value so the
    // handler can report it the way `_async` does rather than as silence.
    let reason = started
        .failure(launch_grace())
        .map_err(|error| error.to_string());
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
fn start_prefix_tool(game: &Game, runners: &RunnerManager, tool: PrefixTool) -> Result<(), String> {
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

/// The command that opens the prefix folder in the user's file manager.
///
/// Split out for the same reason [`open_url_command`] is: the argument list is
/// then *readable*, rather than trusted to be a path and not something a shell
/// would split.
fn prefix_folder_command(target: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(target);
    command
}

/// Start a detached helper and report only the failure that means it never
/// started.
///
/// Both `xdg-open` call sites need exactly this, and they had drifted into two
/// different answers for the same failure — [`open_url`] reported it and
/// [`open_prefix_folder`] discarded it, which is `BUG-04`. Sharing the spawn
/// makes the divergence impossible to reintroduce, and it makes the failure
/// **drivable from a test**: the program name is a parameter, so a test can
/// point it at something that does not exist instead of asserting a string it
/// made up.
///
/// The three `Stdio::null()`s are the point of the call rather than incidental:
/// this process must not hold the helper's pipes open, because a browser or a
/// file manager that inherits them outlives the app that started it. The child
/// is deliberately left unreaped for the same reason — a browser window is not
/// something to wait for.
///
/// `what` is the thing that could not be opened, already phrased as the object
/// of the sentence: `"the prefix folder"`, or a URL.
fn spawn_detached(command: &mut std::process::Command, what: &str) -> Result<(), String> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_child| ())
        .map_err(|error| format!("Could not open {what}: {error}"))
}

/// `openPrefix`'s body from the prefix resolution down (`bridge.py:509-517`).
///
/// The one failure the reference itself reports is the directory creation — it
/// prints `Could not open the prefix folder: {exc}` and returns *without*
/// opening anything, because a file manager pointed at a path that does not
/// exist is a worse answer than a sentence.
///
/// `QDesktopServices.openUrl`'s return value is discarded in the reference
/// (`:517`), and for a while this discarded `xdg-open`'s too, on the argument
/// that a discarded value is what fidelity requires. That argument only holds
/// where the two are the same kind of thing, and they are not: Qt's `openUrl`
/// returning `false` means the platform was asked and declined, while
/// `Command::spawn` failing means the helper was **never started at all** —
/// `ENOENT`, `xdg-open` not on `PATH`, which is a minimal flatpak's and a
/// container's normal condition. Discarding that made the menu item do nothing
/// and say nothing, and it contradicted [`open_url`] thirty lines below, which
/// maps the identical failure to a message that gets toasted. See `BUG-04`.
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
    spawn_detached(&mut prefix_folder_command(&resolved), "the prefix folder")
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

/// Whether `url` may be handed to the desktop's URL dispatcher (SEC-07).
///
/// **`xdg-open` dispatches on the scheme**, so the sink is where the invariant
/// has to hold. Every producer reachable today is a compile-time `https://`
/// constant — `Credit::url` in the credit catalogue, `releases_url()` and
/// `homepage()` in the runner family catalogue — and that is checked by tests.
/// But the payload is a `Message::OpenUrl(String)`, and a `String` is a
/// `String`: the day a URL that came out of `games.json`, `settings.json`, a
/// manifest or a network response reaches this arm, `file://` and whatever
/// handler claims it become reachable with no other change anywhere. The
/// argument list above already stops `;`, `&` and backticks from being anything
/// but characters — this stops the *scheme* from choosing the program.
///
/// Strict and case-sensitive on purpose: RFC 3986 makes schemes
/// case-insensitive, so `HTTPS://…` is a URL this refuses. Every producer is a
/// lowercase constant, and refusing something a producer cannot currently
/// write is the direction a gate should fail in. If a future caller needs a
/// scheme this refuses, that is a deliberate edit here rather than an accident
/// at a call site.
fn is_openable_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty())
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
/// container's. `spawn_detached` is where the failure is turned into a sentence;
/// `a_detached_spawn_that_cannot_start_says_so` drives it with a program that
/// does not exist instead of asserting a string the test invented.
///
/// The gate is first, before the command is even built (SEC-07): a URL that is
/// not `https://` never reaches `open_url_command`, so there is no ordering in
/// which a refused scheme is still spawned. The refusal names the scheme it
/// refused rather than the URL, because the URL is the untrusted half.
fn open_url(url: &str) -> Result<(), String> {
    if !is_openable_url(url) {
        return Err(format!(
            "Refused to open a link that is not https: {}",
            url.split(':').next().unwrap_or("no scheme")
        ));
    }
    spawn_detached(&mut open_url_command(url), url)
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
    shortcut_command_with(game, &SystemPluginEnv)
}

/// [`shortcut_command`] with the environment injected, so a test can be inside
/// a sandbox this process is not in.
fn shortcut_command_with(game: &Game, env: &dyn PluginEnv) -> String {
    format!("{} --launch {}", launcher_command_with(env), game.id)
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
///
/// The environment is injected so a test can be inside a sandbox this process
/// is not in — which is where the first branch below matters. Inside the
/// sandbox the `PATH` lookup is a trap: it resolves to
/// `/app/bin/gamehandler`, a path that exists only in the mount namespace,
/// while the `.desktop` file is written to the host's
/// `~/.local/share/applications` — so the menu entry dangles. T-19 watched
/// exactly that file get written. `flatpak run` plus the app id is the command
/// a host menu can actually run, and it is this branch rather than a
/// post-write repair because the `PATH` form is never right in here.
fn launcher_command_with(env: &dyn PluginEnv) -> String {
    if gamehandler_core::plugins::in_flatpak(env) {
        return format!("flatpak run {APP_ID}");
    }
    let found = env.which("gamehandler");
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
/// reachable from [`launcher_command_with`] — it calls this only for a value
/// containing a space, and a space is not in the safe set — so the other two are
/// here because they are what the function *is*, not because a caller needs
/// them.
fn shlex_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let safe = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || character == '_' || "@%+=:,./-".contains(character)
    });
    if safe {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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
        // The only loads *here*, and neither is fallible: `Settings.load` and
        // `Library.load` degrade to defaults rather than raising (D-20). They
        // are not the only reads at startup — see `PERFORMANCE.md` PERF-04.
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

    /// P-68, all four accelerators `Main.qml:121-143` binds, from anywhere in
    /// the window.
    ///
    /// The subscription itself is [`shortcuts::subscription`], and it moves
    /// there rather than staying a body here for the reason this doc used to
    /// claim the opposite: a `Subscription` **does** have an accessor.
    /// `into_recipes` and `Recipe::stream` are public (`iced/src/advanced.rs`),
    /// so `shortcuts`'s tests hand the subscription a real runtime event and
    /// read the messages back — including the status a focused `text_input`
    /// leaves, measured rather than assumed. A body here could not be reached
    /// from a test at all: `App` cannot be built off a display, and
    /// `Application::subscription` takes `&self`.
    ///
    /// What this line still owns is the wiring, and it is one call: the state
    /// machine's dispatcher is [`Shell::update`], and the keyboard plumbing is
    /// `shortcuts`.
    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        shortcuts::subscription()
    }

    /// `Ctrl+F` — libcosmic's own `Action::Search`, emitted by
    /// `keyboard_nav::subscription()` (`src/keyboard_nav.rs:50-55`) and routed
    /// here by `Cosmic::update` (`src/app/cosmic.rs:850`).
    ///
    /// **This is one of the two routes `Ctrl+F` takes, and the quieter one.**
    /// The framework's subscription is gated on the event being `Ignored`
    /// (`src/keyboard_nav.rs:20-23`), so this hook never runs while a text field
    /// has the focus; that half arrives as [`Message::FocusLibrarySearch`] and
    /// its arm in [`Shell::update`] calls the same
    /// [`Shell::focus_library_search`] this does. Both halves fire on the same
    /// press exactly once, because the gates are complementary.
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
    /// being liberal in this one case. `shortcuts`' half inherits the same
    /// predicate deliberately, so the key does not change meaning with the
    /// focus. See [`shortcuts::shortcut_for`] for the exact-modifier guard the
    /// other three *do* use, and why the two differ.
    fn on_search(&mut self) -> cosmic::app::Task<Self::Message> {
        self.shell.focus_library_search()
    }

    /// Escape — UX-07, and the reason `App` implements this hook at all.
    ///
    /// libcosmic dispatches the key here (`src/app/cosmic.rs:849`, the
    /// `keyboard_nav::Action::Escape` arm of the framework's Tab/Escape
    /// subscription), and the default it replaces returns `Task::none()` without
    /// looking at anything (`src/app/mod.rs:436-439`) — so before this, Escape
    /// on an open "Remove …?" prompt did nothing whatsoever. The delegation is
    /// [`Shell::dismiss_dialogs`], which records why it is a `Shell` method and
    /// which state it clears.
    ///
    /// `Task::none()` is returned rather than anything else because clearing the
    /// two pending fields *is* the whole effect: no focus, window or I/O
    /// operation is involved, and the next frame draws the page with no dialog
    /// over it.
    fn on_escape(&mut self) -> cosmic::app::Task<Self::Message> {
        self.shell.dismiss_dialogs();
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
        self.shell.view_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Four names the flow exposes only to this module — two called from
    // assertions, two named only inside intra-doc links in the comments below.
    // Imported here rather than at the crate root: `production_src` in
    // `tests/dispatch_coverage.rs` cuts `main.rs` at its first top-level
    // `#[cfg(test)]` item, so a gated `use` there would truncate the guards'
    // view of every page dispatch in the file. It did, and the guard said so.
    #[cfg(test)]
    use super::easy_install::{easy_install_cover, locate_message};
    use gamehandler_core::models::{Game, LoadStatus};

    /// A library at a temp path, populated with `(id, name)` games.
    ///
    /// Built through `Library::add`, which saves, so the games are read back
    /// from a real file rather than injected — a listing that worked only on an
    /// in-memory library would pass here and print nothing for a user.
    pub(crate) fn library_with(
        label: &str,
        games: &[(&str, &str)],
    ) -> (std::path::PathBuf, Library) {
        let root = std::env::temp_dir().join(format!("gh-cli-{label}-{}", std::process::id()));
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

    /// A [`PluginEnv`] for shortcut tests: a `PATH` lookup, a file set, and
    /// variables, each empty unless the test puts something in.
    pub(crate) struct ShortcutEnv {
        which: std::collections::BTreeMap<String, String>,
        files: Vec<String>,
        vars: std::collections::BTreeMap<String, String>,
    }

    impl ShortcutEnv {
        pub(crate) fn new() -> Self {
            Self {
                which: std::collections::BTreeMap::new(),
                files: Vec::new(),
                vars: std::collections::BTreeMap::new(),
            }
        }

        pub(crate) fn with_which(mut self, name: &str, path: &str) -> Self {
            self.which.insert(name.to_string(), path.to_string());
            self
        }

        pub(crate) fn with_file(mut self, path: &str) -> Self {
            self.files.push(path.to_string());
            self
        }

        pub(crate) fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), value.to_string());
            self
        }
    }

    impl paths::Env for ShortcutEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    impl PluginEnv for ShortcutEnv {
        fn which(&self, name: &str) -> Option<std::path::PathBuf> {
            self.which.get(name).map(std::path::PathBuf::from)
        }

        fn exists(&self, path: &str) -> bool {
            self.files.iter().any(|known| known == path)
        }

        fn euid(&self) -> Option<u32> {
            None
        }
    }

    /// Inside the sandbox the shortcut names `flatpak run`, not the binary's
    /// own path — the `PATH` lookup resolves to `/app/bin/gamehandler`, which
    /// exists only in the mount namespace, while the `.desktop` file is
    /// written to the host. T-19 watched that dangling `Exec=` get written;
    /// the matrix row this closes is T-07's caveat (c).
    #[test]
    fn a_shortcut_written_in_the_sandbox_names_flatpak_run() {
        let mut game = Game::new_named("Alpha");
        game.id = "11111111111111111111111111111111".to_string();
        // Both Flatpak markers at once, and a `PATH` that would resolve to
        // the sandbox binary if the lookup ran: the branch must win over the
        // lookup rather than consult it.
        let env = ShortcutEnv::new()
            .with_var("FLATPAK_ID", "com.goshapps.GameHandler")
            .with_file("/.flatpak-info")
            .with_which("gamehandler", "/app/bin/gamehandler");

        assert_eq!(
            shortcut_command_with(&game, &env),
            format!("flatpak run {APP_ID} --launch {}", game.id),
        );
    }

    /// Outside the sandbox the `PATH` lookup still rules: a host install
    /// keeps the binary's own path.
    #[test]
    fn a_shortcut_written_on_the_host_keeps_the_path_lookup() {
        let mut game = Game::new_named("Alpha");
        game.id = "11111111111111111111111111111111".to_string();
        let env = ShortcutEnv::new().with_which("gamehandler", "/usr/bin/gamehandler");

        assert_eq!(
            shortcut_command_with(&game, &env),
            format!("/usr/bin/gamehandler --launch {}", game.id),
        );
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
        let source = gamehandler_core::oracle_support::repo_file("gamehandler/runners.py");
        let line = source
            .lines()
            .find(|line| line.starts_with("LAUNCH_GRACE_SECONDS"))
            .unwrap_or_else(|| {
                panic!(
                    "gamehandler/runners.py no longer declares LAUNCH_GRACE_SECONDS; \
                     it is the reference this constant was transcribed from, so its \
                     absence means this test has no oracle left and should be deleted \
                     rather than repointed"
                )
            });
        let value = line
            .split('=')
            .nth(1)
            .and_then(|value| value.trim().trim_end_matches(';').parse::<f64>().ok())
            .unwrap_or_else(|| panic!("`{line}` is not a float assignment"));
        assert_eq!(
            LAUNCH_GRACE_SECONDS, value,
            "the port's grace period has drifted from `gamehandler/runners.py`"
        );
        assert_eq!(
            launch_grace(),
            Duration::from_secs_f64(value),
            "`launch_grace` must be that constant and nothing else, or the two \
             callers are not watching for the length the reference does"
        );
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

        let expected: Vec<(&str, Page)> =
            Page::ALL.iter().map(|page| (page.label(), *page)).collect();
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
        let mut seen: Vec<icons::Icon> = Vec::new();
        for page in Page::ALL {
            let icon = page_icon(page);
            assert!(
                !seen.contains(&icon),
                "{icon:?} is used by {page:?} and by an earlier page; the \
                 reference gives each page its own"
            );
            seen.push(icon);
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
        Message::FocusLibrarySearch => ("FocusLibrarySearch", Message::FocusLibrarySearch),
        Message::OpenNewGameForm => ("OpenNewGameForm", Message::OpenNewGameForm),
        Message::OpenEditGameForm(_) => ("OpenEditGameForm", Message::OpenEditGameForm("g".to_string())),
        Message::CloseDialog => ("CloseDialog", Message::CloseDialog),
        // `"sample-game"`, not `"g"`: the fixture presets the pending delete to
        // `Some("g")` (so `CloseDialog`'s sample has a delete to clear), and a
        // sample writing what is already there would be reported as an
        // unwritten arm (D-34) — which is exactly what `"g"` did.
        Message::ConfirmDeleteGame(_) => ("ConfirmDeleteGame", Message::ConfirmDeleteGame("sample-game".to_string())),
        Message::DeleteGameConfirmed(_) => ("DeleteGameConfirmed", Message::DeleteGameConfirmed("g".to_string())),
        // `"sample-game"`, for `ConfirmDeleteGame`'s reason exactly: the fixture
        // presets the pending delete to `Some("g")`, and an id the library does
        // not hold opens nothing — so `"g"` would be reported as an unwritten
        // arm by its own guard. The fixture's library holds this one.
        Message::OpenGameMenu(_) => ("OpenGameMenu", Message::OpenGameMenu("sample-game".to_string())),
        Message::PickExeFile => ("PickExeFile", Message::PickExeFile),
        Message::ExeFileChosen(_) => ("ExeFileChosen", Message::ExeFileChosen(Some("/tmp/g.exe".to_string()))),
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
        // All four **non-zero**, and the four are all different from each other.
        // A fresh shell starts at `0.0` on all four (`State::new`), which is the
        // value the pre-layout publish carries, so a sample of zeros would write
        // what is already there and be reported as an unwritten arm (D-34) — and
        // four *equal* non-zero values would still be one field's worth of
        // evidence, because a handler that assigned one of them to all four
        // would look the same. These are the four the `Covers` fixture below
        // measures against, in `view::library`'s units: an offset past the fold,
        // and a 1200×800 window.
        Message::SetLibraryScroll { .. } => ("SetLibraryScroll", Message::SetLibraryScroll {
                            offset: 640.0,
                            viewport_width: 1200.0,
                            viewport_height: 800.0,
                            content_height: 4200.0,
                        }),
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
        // `"install-6"` rather than a placeholder, per the rule stated above
        // `InstallPlugin`'s sample: `play_game` answers an id the library does
        // not hold with a "Select a game first" toast, so a sample like `"g"`
        // would drive the refusal branch and report a working arm as a stub.
        // The id need not resolve for *this* macro — it only pairs names with
        // patterns — but the same samples are driven through `Shell::update`
        // elsewhere, and a sample has to be a value its handler acts on.
        Message::PlayInstalled { .. } => ("PlayInstalled", Message::PlayInstalled {
                            game_id: "install-6".to_string(),
                            toast_id: ToastId::default(),
                        }),
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
                            reason: Ok(Some("the runner exited with status 1".to_string())),
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
        // `token: 0`, not `1`: the guard drives every sample against a fresh
        // shell, whose `form_cover_token` is still `0` — a reply token of `1`
        // would be dropped as a stale answer about an older lookup, and the
        // arm reported as unwritten. (The `FetchCoverForForm` sample beside it
        // carries `1` too, but that arm mints its own token and ignores the
        // carried one.)
        Message::FormCoverFetchFinished { .. } => ("FormCoverFetchFinished", Message::FormCoverFetchFinished {
                            token: 0,
                            name: "Hades".to_string(),
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
                            // `None`, not a message: the sample exists to make
                            // the arm *write* something observable, and a notice
                            // is a side effect rather than a field this arm
                            // writes. The reporting half is covered by
                            // `a_runners_folder_that_cannot_be_read_is_reported`.
                            problem: None,
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
    /// - **`PlayInstalled`** is listed, and it is listed for its *second* half
    ///   only. Its body is `toasts.remove(id)` — invisible, for the reason the
    ///   `DismissToast` bullet gives — followed by the launch, which is what
    ///   changes the state here: the fixture's library holds no `install-6`, so
    ///   the launch path answers with its "Select a game first" toast. The row
    ///   is `BUG-33`, and the arm would leave this list the moment its launch
    ///   half were dropped, which is the failure mode that fix invites.
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
            .filter(|message| !matches!(message, Message::Quit | Message::SetWindowHidden(_)))
            .filter(|message| {
                let mut shell = shell_with_work_to_do();
                is_handled(&mut shell, message.clone())
            })
            .map(|message| variant_name(&message))
            .collect();

        let mut expected: Vec<&str> = vec![
            "NavigateTo",
            // `Ctrl+F` when the framework's own binding is silent — a text field
            // has the focus. Live because `shortcuts` emits it from the shell's
            // keyboard subscription and the arm navigates; see
            // [`Message::FocusLibrarySearch`] for why it is not simply bound to
            // `on_search`'s path as well.
            "FocusLibrarySearch",
            "CloseDialog",
            // U3's pair. Live because the library's context menu sends the
            // first and the dialog it opens sends the second; the unknown-id
            // silence in both arms is what the samples' held id keeps out of
            // reach here (see the removal tests for the unheld half).
            "ConfirmDeleteGame",
            "DeleteGameConfirmed",
            // UX-16's route to the same menu. Live because the library's "More
            // actions" control sends it and the arm opens the layer — the
            // unknown-id silence is what the sample's held id keeps out of
            // reach here (see the layer tests for the unheld half).
            "OpenGameMenu",
            // U5's four. Live because the library's context menu sends
            // `FetchCover` (U2's entry point), the form's Find-cover button
            // sends `FetchCoverForForm`, and each request has its reply. The
            // stale-token silence in the form reply is what the sample's
            // token keeps out of reach here (see the cover tests for the
            // stale half).
            "FetchCover",
            "CoverFetchFinished",
            "FetchCoverForForm",
            "FormCoverFetchFinished",
            // U6's four. Live because the form draws both browse buttons (F4,
            // F8): the exe row's opens the exe dialog and the cover row's the
            // image dialog, and each has its reply. The cancelled half of
            // both replies is what the samples' `Some` keeps out of reach
            // here (see the picker tests for the `None` half).
            "PickExeFile",
            "ExeFileChosen",
            // BUG-33's. Listed for its launch half; its dismissal half is
            // invisible for the reason `DismissToast`'s bullet gives.
            "PlayInstalled",
            "PickCoverFile",
            "CoverFileChosen",
            "Notify",
            // T-09's five. They are live because the Library page needs them:
            // the search box, the category filter, the button that clears both
            // at once, and the two settings the toolbar's selectors write.
            "SetSearchText",
            "SetCategoryFilter",
            "ClearFilters",
            "SetViewMode",
            "SetSortMode",
            // PERF-03's one. Live because the Library page's scrollable
            // publishes its viewport through `on_scroll` on every scroll, which
            // is the only route from that widget to the window the body builds.
            // It is in this list rather than treated as bookkeeping for the
            // reason the list exists: the four floats are what the window is
            // computed from, and an arm that stopped writing them would show a
            // page frozen at whatever offset it last saw.
            "SetLibraryScroll",
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
            // # All four have producers now; this comment used to say three do
            // # not (#89, retired by U2)
            //
            // It read "the game form's three buttons produce the rest" — wrong
            // control, wrong file, measured against the tree — and then, for
            // the corrected paragraph, that `view/library.rs` has no menu, so
            // the three prefix/shortcut arms were handled-but-unemitted, with
            // the stale condition spelled out: "if the menu lands and these
            // lines are still needed, they are stale." The context menu landed
            // in U2 and sends all three, plus `LaunchGame`, `OpenEditGameForm`,
            // `FetchCover` and `ConfirmDeleteGame`, so the condition fired and
            // the paragraph went with it. The history stays in git: the defect
            // it recorded — counting a doc link as a producer — is still the
            // trap `dispatch_coverage.rs` names in its own header.
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
            changed, expected,
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
                reason: Ok(None),
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
                reason: Ok(Some("the runner exited with status 1".to_string())),
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

    /// **A watch that *failed* — rather than observing a death — toasts the raw
    /// error and does not touch the window.**
    ///
    /// `reason` is a `Result` because [`LaunchedGame::failure`]'s `try_wait`
    /// can itself fail (BUG-23): that is a different event from "the title
    /// exited", and folding it into the same arm would both misdescribe the
    /// toast — nothing stopped, the *watch* broke — and wrongly restore a
    /// window the reference only restores for a real death. The `Err` arm
    /// therefore skips `game_name` and `hide_window` entirely: it pushes the
    /// bare error text, which is why this test's fixture can use a title the
    /// library does not hold — a lookup the arm performs is a bug this catches.
    ///
    /// The task count is the instrument: the death case batches two units
    /// (window restore plus toast expiry), the still-running case batches
    /// none, and this case must land exactly on one — the toast's expiry task
    /// alone. A mutation that routes `Err` through the `Some` body (window
    /// restore included) fails the upper bound; one that treats it as `None`
    /// fails the toast assertion.
    #[test]
    fn a_failed_watch_toasts_the_error_without_restoring_the_window() {
        let mut shell = shell_with_work_to_do();
        let effect = observe(
            &mut shell,
            Message::LaunchWatchFinished {
                game_id: "not-a-real-id".to_string(),
                reason: Err("failed to wait for the runner: ESRCH".to_string()),
            },
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 1"),
            "a failed watch must still surface — the error text is the only \
             record the user gets that the launch could not be watched at all"
        );
        assert_eq!(
            effect.task_units, 1,
            "the watch-failure arm must push the toast and nothing else: \
             restoring the window is for a title that actually died, and \
             `Task::none` would mean the error was silently swallowed"
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
    /// rather than tidy: [`easy_install::finish_easy_install`] calls `Library::add`, which
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
            shell.state.running_install.is_none()
                && !shell
                    .state
                    .easy_pending
                    .values()
                    .any(|record| record.game_id == "no-such-installer"),
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

    /// The not-found branch opens the locate dialog alongside the notice:
    /// two units of work, the toast and the chooser. Before U4 this arm
    /// returned the toast alone, and the busy guard it holds had no producer
    /// for either message that clears it — the page wedged (P-57). The count
    /// is what pins the chooser's existence: the dialog assembly itself is
    /// read, not tested (see [`easy_install::locate_exe_task`]), but a branch that stopped
    /// opening it would return one unit and fail here.
    #[test]
    fn the_not_found_branch_opens_the_locate_dialog() {
        let prefix = install_prefix("not-found-opens");
        let mut shell = shell_for_installs();
        shell.state.easy_busy = true;
        shell.state.running_install = Some(install_record(&prefix, "install-1"));

        let effect = observe(
            &mut shell,
            Message::EasyInstallWizardFinished {
                found: None,
                returncode: 0,
            },
        );

        assert!(
            effect.task_units >= 2,
            "the notice without the chooser is the wedge: {effect:?}"
        );
    }

    /// The locate dialog's filters are the reference's `nameFilters`
    /// (`Main.qml:185`): executables first, then everything.
    #[test]
    fn the_locate_dialog_filters_executables_first_then_everything() {
        let filters = exe_file_filters();

        assert_eq!(filters.len(), 2, "the reference lists exactly two filters");
        assert_eq!(filters[0].label(), "Windows executables");
        assert_eq!(filters[0].pattern_filters(), ["*.exe", "*.EXE"]);
        assert_eq!(filters[1].label(), "All files");
        assert_eq!(filters[1].pattern_filters(), ["*"]);
    }

    /// A located file completes the install with its path: the portal's
    /// `file://` URL decoded the way `as_local_path` decodes the reference's
    /// `selectedFile` (`bridge.py:926`).
    #[test]
    fn a_located_file_completes_the_install_with_its_decoded_path() {
        let url = url::Url::parse("file:///tmp/My%20Game/setup.exe").unwrap();

        let message = locate_message("install-1".to_string(), Ok(url));

        assert!(
            matches!(&message, Message::CompleteEasyInstall { token, path }
                if token == "install-1" && path.as_deref() == Some("/tmp/My Game/setup.exe")),
            "a chosen file must finish the install from its decoded path: {message:?}"
        );
    }

    /// A locate answer with no local path cancels: the `None` takes the same
    /// path `as_local_path` of nothing takes in the reference
    /// (`bridge.py:926-928`).
    #[test]
    fn a_locate_answer_with_no_local_path_cancels() {
        let url = url::Url::parse("https://example.invalid/setup.exe").unwrap();

        let message = locate_message("install-1".to_string(), Ok(url));

        assert!(
            matches!(&message, Message::CompleteEasyInstall { token, path }
                if token == "install-1" && path.is_none()),
            "a URL with no local path must reach the cancel path, not a //host path: {message:?}"
        );
    }

    /// Rejecting the locate dialog cancels the install: the wedge's way out.
    #[test]
    fn rejecting_the_locate_dialog_cancels_the_install() {
        let message = locate_message(
            "install-1".to_string(),
            Err(cosmic::dialog::file_chooser::Error::Cancelled),
        );

        assert!(
            matches!(&message, Message::CancelEasyInstall(token) if token == "install-1"),
            "a rejection must release the busy guard: {message:?}"
        );
    }

    /// A locate dialog that errors — the portal failing, not the user
    /// refusing — still cancels the install. Both must release the busy
    /// guard, and the kept-prefix notice is honest either way, because
    /// nothing deleted the prefix (see [`locate_message`]).
    #[test]
    fn a_locate_dialog_that_errors_cancels_the_install() {
        let message = locate_message(
            "install-1".to_string(),
            Err(cosmic::dialog::file_chooser::Error::UrlAbsolute),
        );

        assert!(
            matches!(&message, Message::CancelEasyInstall(token) if token == "install-1"),
            "a portal failure must release the busy guard like a rejection: {message:?}"
        );
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
        shell
            .state
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
    /// `cover_path` pins the empty string — and that is still the right value
    /// here, for a narrower reason than it used to be. The reference sets it
    /// to the executable's own icon (`:907-911`), and this tree *has*
    /// `exe_icons` now (T-05 landed as A1/A2, U5 wired it in
    /// [`easy_install::finish_easy_install`]) — but the fixture's executable is a path with
    /// no icon behind it, so the wiring takes the reference's own `except`
    /// fallback. The `""` below is that fallback, not a missing port; the
    /// seam's own test is
    /// `an_install_whose_executable_has_no_icon_keeps_no_cover`.
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

    /// **`BUG-33`: the toast's Play action dismisses the toast, and then
    /// launches.**
    ///
    /// QQC2's notification action removes the notification and *then* runs its
    /// slot (`showPassiveNotification`'s `function()`, `Main.qml:158-161`). The
    /// port's action was `Message::LaunchGame` alone, so a game launched and an
    /// "Installed … ▶ Play" toast stayed over the Library the user had just been
    /// moved to, for up to `State::TOAST_DURATION`.
    ///
    /// **What is asserted here is the launch half only**, and that is not
    /// laziness: the dismissal half is not observable from a test, for the
    /// reason [`a_test_cannot_observe_which_toast_was_dismissed`] exists to
    /// record. The arm calls `self.state.toasts.remove(toast_id)`, the only
    /// `ToastId` a test can build is `Default::default()` whose key names no
    /// slot, and `Toasts` exposes neither the slot map nor its queue. So the
    /// removal below is a no-op against the live toast, on the assertion that
    /// the toast is *still* there afterwards — which is `DismissToast`'s own
    /// measured gap, hit by a second handler.
    ///
    /// What the launch half buys: it fails if the arm ever becomes
    /// dismissal-only, which is the mistake this fix invites (removing the
    /// toast is the visible half and the tempting place to stop).
    #[test]
    fn the_installed_toasts_play_action_launches_the_game() {
        let mut shell = shell_for_installs();
        let game_id = "install-6".to_string();
        let mut game = gamehandler_core::models::Game::new_named("Steam");
        game.id = game_id.clone();
        // Saved into the fixture's own temp library, as the doc on
        // `shell_for_installs` requires: a shell over the real `games.json`
        // would write the running user's library from a test.
        shell.state.library.add(game).unwrap();

        // A live toast, so the null-key no-op below is not "nothing to remove".
        let _ = shell.update(Message::Notify("Installed “Steam”".to_string()));
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 1"),
            "the fixture should have exactly one live toast"
        );

        let effect = observe(
            &mut shell,
            Message::PlayInstalled {
                game_id: game_id.clone(),
                toast_id: ToastId::default(),
            },
        );
        assert!(
            effect.task_units > 0,
            "the Play action has to *play*: an arm that only dismisses the toast \
             is a Play button that deletes itself, and the reference launches \
             (`Main.qml:159-161`)"
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("num_elems: 1"),
            "the toast survives the dismissal here, and that is the recorded \
             gap rather than a passing test: see \
             `a_test_cannot_observe_which_toast_was_dismissed`. If this ever \
             fails, `Toasts` grew an accessor and the removal can be asserted \
             for real"
        );
    }

    /// **The action the toast carries is `PlayInstalled`, not `LaunchGame`.**
    ///
    /// This is the killing test for `BUG-33`. The launch arm above can be right
    /// while nothing reaches it, and that is exactly how the defect existed:
    /// the toast's closure built `LaunchGame`, so the game launched and the
    /// toast stayed. `installed_play_message` is the one place that mapping
    /// lives, and the `Play` button is otherwise undrivable — `Toast` keeps its
    /// action private with no accessor — so this pins the value the button
    /// carries.
    #[test]
    fn the_installed_toasts_action_carries_the_toast_id() {
        let id = ToastId::default();
        match installed_play_message("install-6", id) {
            Message::PlayInstalled { game_id, toast_id } => {
                assert_eq!(game_id, "install-6");
                assert_eq!(toast_id, id);
            }
            other => panic!(
                "the Play action must dismiss the toast it was pressed on; \
                 {other:?} launches and leaves a stale toast over the Library, \
                 which is the defect this closes"
            ),
        }
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
        assert_eq!(
            shell.state.library.len(),
            0,
            "this fixture is the empty library"
        );
        let drawn = crate::view::testkit::drawn_strings(shell.view_body());

        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::library::NO_GAMES_TITLE),
            "an empty library draws the empty state; drawn: {drawn:?}"
        );
        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::library::ADD_FIRST_GAME),
            "and the button out of it; drawn: {drawn:?}"
        );
        assert!(
            !drawn
                .iter()
                .any(|text| text.contains("has not been ported yet")),
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

        let drawn = crate::view::testkit::drawn_strings(shell.view_body());
        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::library::NO_MATCHES_TITLE),
            "a library with a game in it whose search matches nothing says so; \
             drawn: {drawn:?}"
        );
        assert!(
            !drawn
                .iter()
                .any(|text| text == crate::view::library::NO_GAMES_TITLE),
            "and must not claim the library is empty; drawn: {drawn:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **An out-of-set view mode or sort mode is ignored, not stored.**
    ///
    /// `bridge.py:210-214` and `223-228` both *ignore* an unrecognised value,
    /// and the load path already folds one (`Settings::from_dict`,
    /// `crates/core/src/settings.rs:158-163`) — so the
    /// message path was the one place the check was missing, and a stale UI
    /// could write a mode the code does not handle.
    ///
    /// The in-set half is asserted too, because "ignores everything" would
    /// satisfy the first half alone.
    #[test]
    fn an_out_of_set_view_mode_or_sort_mode_is_ignored() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::SetViewMode("nonsense".to_string()));
        assert!(
            !effect.state_changed,
            "an unrecognised view mode must not be stored"
        );
        assert_eq!(shell.state.settings.view_mode, "grid");

        let effect = observe(&mut shell, Message::SetViewMode("list".to_string()));
        assert!(effect.state_changed, "a known view mode must be stored");
        assert_eq!(shell.state.settings.view_mode, "list");

        let effect = observe(&mut shell, Message::SetSortMode("nonsense".to_string()));
        assert!(
            !effect.state_changed,
            "an unrecognised sort mode must not be stored"
        );
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
        assert!(
            !shell.state.search_text.is_empty(),
            "the fixture is filtering"
        );
        assert_ne!(
            shell.state.category_filter, "All",
            "the fixture is filtering"
        );

        observe(&mut shell, Message::ClearFilters);

        assert!(shell.state.search_text.is_empty());
        assert_eq!(
            shell.state.category_filter,
            crate::view::library::ALL_CATEGORIES
        );
    }

    /// `bridge.py:290-293`: an empty category folds to "All" rather than being
    /// stored as the empty string, which is what makes "no filter" have one
    /// spelling instead of two.
    #[test]
    fn an_empty_category_filter_folds_to_all() {
        let mut shell = shell_with_work_to_do();
        observe(&mut shell, Message::SetCategoryFilter(String::new()));
        assert_eq!(
            shell.state.category_filter,
            crate::view::library::ALL_CATEGORIES
        );
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
        let drawn = crate::view::testkit::drawn_strings(shell.view_body());

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
            !drawn
                .iter()
                .any(|text| text.contains("has not been ported yet")),
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
        let drawn = crate::view::testkit::drawn_strings(shell.view_body());

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
            !drawn
                .iter()
                .any(|text| text.contains("has not been ported yet")),
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
        let drawn = crate::view::testkit::drawn_strings(shell.view_body());

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
            !drawn
                .iter()
                .any(|text| text.contains("has not been ported yet")),
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
        let drawn = crate::view::testkit::drawn_strings(shell.view_body());

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
        assert!(
            !sections.is_empty(),
            "the catalogue is empty; this checks nothing"
        );
        for section in sections {
            assert!(
                drawn.iter().any(|text| text == section.title),
                "section {} is in the catalogue but not drawn; drawn: {drawn:?}",
                section.id
            );
        }

        assert!(
            !drawn
                .iter()
                .any(|text| text.contains("has not been ported yet")),
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
            GameForm::from_game(shell.state.library.get("g").expect("the fixture holds `g`"))
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
        let layer = crate::view::testkit::drawn_strings(shell.view_with_overlays());

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
            crate::view::testkit::drawn_strings(shell.view_body()),
            crate::view::testkit::drawn_strings(closed.view_body()),
            "`view_body` draws differently depending on whether a layer is open, so \
             every page test above now depends on the form"
        );

        // Nothing open: the same call is the page, with no form in it.
        let fallthrough = crate::view::testkit::drawn_strings(closed.view_with_overlays());
        assert!(
            fallthrough
                .iter()
                .any(|text| text == crate::view::library::NO_MATCHES_TITLE),
            "the fall-through is not the page; drawn: {fallthrough:?}"
        );
        assert!(
            !fallthrough
                .iter()
                .any(|text| text == crate::view::form::TITLE_ADD)
        );
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
        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

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

    /// **Find cover is drawn, and its message is handled.**
    ///
    /// This used to be the iff the missing fetch needed: the button's label
    /// absent exactly while `COVER_FETCH_MISSING` was `true`, and its message
    /// handled exactly when the constant was `false` — plus a compile-time
    /// assertion that broke the build the day the constant flipped. U5 wrote
    /// the arm, drew the button unconditionally, and deleted the constant with
    /// its note in `view/form.rs`; what remains is the conjunction, so neither
    /// "drawn but dead" nor "handled but unreachable" passes.
    #[test]
    fn the_find_cover_button_is_drawn_and_its_message_is_handled() {
        let handled = is_handled(
            &mut shell_with_work_to_do(),
            Message::FetchCoverForForm {
                token: 0,
                game_id: "g".to_string(),
                name: "Fixture".to_string(),
                exe: String::new(),
            },
        );
        let drawn = crate::view::testkit::drawn_strings(
            shell_with_form_open(true, false).view_with_overlays(),
        );
        let button_on_screen = drawn
            .iter()
            .any(|text| text == crate::view::form::FIND_COVER);

        assert!(
            button_on_screen,
            "the reference's Find cover (`GameFormPage.qml:151-158`) is not on screen"
        );
        assert!(
            handled,
            "the Find cover button is drawn but its message does nothing"
        );
    }

    /// Fetching a cover for a game the library does not hold is silence: the
    /// reference returns before spawning (`bridge.py:539-540`), so there is no
    /// lookup to answer and nothing to say.
    #[test]
    fn fetching_a_cover_for_an_unknown_game_is_silence() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::FetchCover("absent".to_string()));

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "an unknown id must not start a lookup: {effect:?}"
        );
    }

    /// Fetching a cover for a held game starts the lookup. The lookup itself
    /// is a worker task no test drives (it reaches the network); the reply
    /// tests below cover everything the task's answer can do.
    #[test]
    fn fetching_a_cover_for_a_held_game_starts_a_lookup() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::FetchCover("g".to_string()));

        assert!(
            effect.task_units > 0,
            "a held id must return the lookup task: {effect:?}"
        );
    }

    /// A finished fetch writes what the hit carries — cover, app id, and the
    /// category when the game has none — persists, and names the source
    /// (`bridge.py:546-558`). The category write is P-25: Steam's genre
    /// auto-categorises a game the user never categorised.
    #[test]
    fn a_finished_fetch_writes_the_cover_and_names_its_source() {
        let mut shell = shell_with_work_to_do();
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g".to_string(),
                result: Ok(hit),
            },
        );

        let game = shell.state.library.get("g").expect("the fixture holds `g`");
        assert_eq!(game.cover_path, "/tmp/hl.jpg");
        assert_eq!(game.steam_appid, 70);
        assert_eq!(game.category, "Action");
        assert!(
            format!("{:?}", shell.state.toasts).contains("Cover set from Steam: Half-Life"),
            "the toast must name the source and the hit; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// A finished fetch does not recategorise a game the user categorised:
    /// the auto-categorise fires only for `Uncategorized` (`bridge.py:552`).
    #[test]
    fn a_finished_fetch_leaves_a_categorised_game_alone() {
        let mut shell = shell_with_work_to_do();
        let mut categorised = shell
            .state
            .library
            .get("g")
            .expect("the fixture holds `g`")
            .clone();
        categorised.category = "Puzzle".to_string();
        shell.state.library.update(categorised).unwrap();
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g".to_string(),
                result: Ok(hit),
            },
        );

        let game = shell.state.library.get("g").expect("the fixture holds `g`");
        assert_eq!(game.cover_path, "/tmp/hl.jpg", "the cover is still written");
        assert_eq!(game.category, "Puzzle", "the user's category must win");
    }

    /// A finished fetch for a game that left while the lookup ran is
    /// silence: the re-check (`bridge.py:547-549`) finds nothing to write to.
    #[test]
    fn a_finished_fetch_for_a_missing_game_is_silence() {
        let mut shell = shell_with_work_to_do();
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let effect = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "absent".to_string(),
                result: Ok(hit),
            },
        );

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "a game that left must not be written back: {effect:?}"
        );
    }

    /// A failed fetch reports the raw message: `_async`'s default `fail`
    /// notifies `str(exc)` verbatim (`bridge.py:157-161`), and the reply arm
    /// toasts the string untouched.
    #[test]
    fn a_failed_fetch_reports_the_raw_message() {
        let mut shell = shell_with_work_to_do();

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g".to_string(),
                result: Err("No Steam cover found for \u{201c}Zork\u{201d}".to_string()),
            },
        );

        assert!(
            format!("{:?}", shell.state.toasts)
                .contains("No Steam cover found for \u{201c}Zork\u{201d}"),
            "the failure must surface verbatim; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// A hit with no app id — the exe icon, which carries `appid=0`
    /// (`covers.py:324`) — leaves the stored id alone (`bridge.py:550-551`).
    #[test]
    fn a_finished_fetch_with_no_appid_keeps_the_old_one() {
        let mut shell = shell_with_work_to_do();
        let mut identified = shell
            .state
            .library
            .get("g")
            .expect("the fixture holds `g`")
            .clone();
        identified.steam_appid = 999;
        shell.state.library.update(identified).unwrap();
        let hit = CoverHit::from_steam(
            0,
            "Setup".to_string(),
            "Uncategorized".to_string(),
            PathBuf::from("/tmp/setup.ico"),
            "C:\\setup.exe".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g".to_string(),
                result: Ok(hit),
            },
        );

        assert_eq!(
            shell
                .state
                .library
                .get("g")
                .expect("the fixture holds `g`")
                .steam_appid,
            999,
            "an appid of zero must not overwrite a stored one"
        );
    }

    /// An icon hit says where it came from: `origin_label`'s "the app icon"
    /// in the toast's source slot.
    #[test]
    fn a_finished_fetch_from_the_app_icon_says_so() {
        let mut shell = shell_with_work_to_do();
        let hit = CoverHit {
            appid: 0,
            name: "Setup".to_string(),
            category: "Uncategorized".to_string(),
            cover_path: PathBuf::from("/tmp/setup.ico"),
            source_url: "C:\\setup.exe".to_string(),
            source: gamehandler_core::covers::ICON_SOURCE.to_string(),
        };

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g".to_string(),
                result: Ok(hit),
            },
        );

        assert!(
            format!("{:?}", shell.state.toasts).contains("Cover set from the app icon: Setup"),
            "the toast must name the icon source; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// A finished fetch the disk refuses to persist is reported, not raised:
    /// the reference lets `done`'s exception escape to the Qt loop, which is
    /// not a behaviour to reproduce (see `CoverFetchFinished`).
    #[test]
    fn a_finished_fetch_that_cannot_persist_is_reported() {
        let stale =
            std::env::temp_dir().join(format!("gh-cli-u5-cover-fail-{}", std::process::id()));
        let _ = std::fs::remove_file(&stale);
        let (root, library) = library_with("u5-cover-fail", &[("g1", "Hades")]);
        let mut shell = Shell::new();
        shell.state.library = library;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::write(&root, b"a file, not a directory").unwrap();
        let hit = CoverHit::from_steam(
            70,
            "Hades".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hades.jpg"),
            "https://example.invalid/hades.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g1".to_string(),
                result: Ok(hit),
            },
        );

        assert!(
            format!("{:?}", shell.state.toasts).contains("Could not save \u{201c}Hades\u{201d}"),
            "the save failure must surface with the game's name; toasts: {:?}",
            shell.state.toasts
        );
        std::fs::remove_file(&root).unwrap();
    }

    /// **Boundary: `core`'s typed persistence error survives to the toast**
    /// (`ARCH-10`).
    ///
    /// The app layer is where a `core` error becomes the text a user reads, and
    /// this is the test for that boundary. It asserts on the two facts the
    /// [`PersistenceError`](gamehandler_core::json::PersistenceError) variant
    /// carries and the `io::Error` it replaced did not — the **path** and the
    /// **step** that failed. The OS text alone for the destination below is
    /// `Not a directory (os error 20)`, which names neither, so a boundary that
    /// flattened the error back to a `String` would print exactly that and fail
    /// both assertions here.
    ///
    /// The failure is the filesystem's and is produced without an unwritable
    /// uid: a regular file stands where the library's directory has to be, so
    /// `create_dir_all` fails with `NotADirectory` for root as much as for
    /// anyone else.
    #[test]
    fn a_failed_write_reaches_the_toast_naming_the_path_and_the_step() {
        let (root, library) = library_with("arch10-write", &[("g1", "Hades")]);
        let target = library.path().to_path_buf();
        let mut shell = Shell::new();
        shell.state.library = library;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::write(&root, b"a file, not a directory").unwrap();
        let hit = CoverHit::from_steam(
            70,
            "Hades".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hades.jpg"),
            "https://example.invalid/hades.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "g1".to_string(),
                result: Ok(hit),
            },
        );

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("Could not save \u{201c}Hades\u{201d}"),
            "the save failure must still surface with the game's name; toasts: {toasts}"
        );
        assert!(
            toasts.contains("could not create the directory holding"),
            "the toast must name the step that failed, not just the OS error the \
             step returned; toasts: {toasts}"
        );
        assert!(
            toasts.contains(&target.display().to_string()),
            "the toast must name the file it could not write, which the `io::Error` \
             this replaced never did; toasts: {toasts}"
        );
        std::fs::remove_file(&root).unwrap();
    }

    /// **The other half of the same boundary** (`ARCH-10`, `ARCH-01`): a write
    /// the gate refused is reported as a refusal, not as a failed write.
    ///
    /// The two are different problems with different fixes — one is the disk's,
    /// one is the file's — and before the typed error they were the same type
    /// differing only by an `ErrorKind` no caller read. This test is the
    /// anti-vacuity arm of the one above: a boundary that reported *every*
    /// persistence failure as "could not create the directory holding …" would
    /// pass that test and fail this one, because the gate refuses before the
    /// filesystem is touched at all.
    #[test]
    fn a_refused_write_reaches_the_toast_as_a_refusal_not_a_write_failure() {
        let mut shell = shell_with_work_to_do();
        let target = shell.state.library.path().to_path_buf();
        // A torn file — the exact shape `models`' `an_unparsable_library_is_
        // reported_and_never_overwritten` pins — then a re-read, which is what
        // arms the gate.
        std::fs::write(&target, r#"[{"id": "g1", "name": "Hades""#).unwrap();
        shell.state.library.load();
        assert_eq!(
            shell.state.library.load_status(),
            LoadStatus::Unparsable,
            "the fixture has to arm the gate, or this test observes the wrong branch"
        );

        let mut form = GameForm::new_template(&shell.state.settings, "refused".to_string());
        form.set_field(crate::state::FormField::Name, "Refused".to_string());
        let _ = observe(&mut shell, Message::SaveGameForm(form));

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("has not been overwritten"),
            "the refusal has to tell the user their file was left alone; toasts: {toasts}"
        );
        for step in [
            "could not create the directory holding",
            "could not write the temporary file for",
            "could not move the temporary file onto",
        ] {
            assert!(
                !toasts.contains(step),
                "the gate refused before the filesystem was touched, so the toast must \
                 not report a write failure ({step:?}); toasts: {toasts}"
            );
        }
        assert!(
            !toasts.contains("Added"),
            "a refused save must not also announce that it saved; toasts: {toasts}"
        );
    }

    /// The form's lookup without a name is refused before anything is minted:
    /// no token, no task, just the reference's sentence (`bridge.py:565-567`).
    #[test]
    fn asking_the_form_to_fetch_without_a_name_is_refused() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(
            &mut shell,
            Message::FetchCoverForForm {
                token: 0,
                game_id: "g".to_string(),
                name: "   ".to_string(),
                exe: String::new(),
            },
        );

        assert_eq!(
            shell.state.form_cover_token, 0,
            "a refused lookup must not mint a token"
        );
        assert!(
            effect.task_units > 0
                && format!("{:?}", shell.state.toasts).contains("Enter a game name first"),
            "the refusal must be the reference's sentence: {effect:?}"
        );
    }

    /// The form's lookup mints a token, announces itself, and starts the
    /// fetch: the notice plus the task (`bridge.py:568` and the `_async`).
    /// The carried token is ignored — the view sends `0` and the arm mints
    /// the real one — so sending `99` still yields `1`.
    #[test]
    fn asking_the_form_to_fetch_mints_a_token_and_announces_the_lookup() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(
            &mut shell,
            Message::FetchCoverForForm {
                token: 99,
                game_id: "g".to_string(),
                name: "Half-Life 2".to_string(),
                exe: String::new(),
            },
        );

        assert_eq!(
            shell.state.form_cover_token, 1,
            "the arm must mint the lookup's token, ignoring the carried one"
        );
        assert!(
            effect.task_units >= 2,
            "the notice without the fetch is half the request: {effect:?}"
        );
        assert!(
            format!("{:?}", shell.state.toasts)
                .contains("Looking for artwork for \u{201c}Half-Life 2\u{201d}\u{2026}"),
            "the announcement must be the reference's sentence; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// A form reply for a superseded lookup is dropped: its token is not the
    /// form's current one, so it is a late answer about an older world.
    #[test]
    fn a_stale_form_reply_is_dropped() {
        let mut shell = shell_with_work_to_do();
        shell.state.form_cover_token = 2;
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let effect = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 1,
                name: "Hades".to_string(),
                result: Ok(hit),
            },
        );

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "a stale reply must change nothing and say nothing: {effect:?}"
        );
    }

    /// A current form reply writes the form — cover, app id, and the category
    /// when the form's is `Uncategorized` — and names the source
    /// (`bridge.py:571-585`, `GameFormPage.qml:57-68`).
    #[test]
    fn a_form_reply_writes_the_form_and_names_its_source() {
        let mut shell = shell_with_work_to_do();
        shell
            .state
            .game_form
            .as_mut()
            .expect("the fixture opens a form")
            .category = "Uncategorized".to_string();
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Ok(hit),
            },
        );

        let form = shell.state.game_form.as_ref().expect("the form stays open");
        assert_eq!(form.cover_path, "/tmp/hl.jpg");
        assert_eq!(form.steam_appid, "70");
        assert_eq!(form.category, "Action");
        assert!(
            format!("{:?}", shell.state.toasts).contains("Cover found via Steam: Half-Life"),
            "the toast must name the source and the hit; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// A form reply does not recategorise a form the user categorised, and an
    /// `Uncategorized` hit categorises nothing: the QML's two guards
    /// (`GameFormPage.qml:62-64`).
    #[test]
    fn a_form_reply_keeps_a_categorised_form_and_an_empty_hit_category() {
        let mut shell = shell_with_work_to_do();
        shell
            .state
            .game_form
            .as_mut()
            .expect("the fixture opens a form")
            .category = "Puzzle".to_string();
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Ok(hit),
            },
        );

        assert_eq!(
            shell
                .state
                .game_form
                .as_ref()
                .expect("the form stays open")
                .category,
            "Puzzle",
            "the user's category must win over the hit's"
        );

        let mut shell = shell_with_work_to_do();
        shell
            .state
            .game_form
            .as_mut()
            .expect("the fixture opens a form")
            .category = "Uncategorized".to_string();
        let blank = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Uncategorized".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Ok(blank),
            },
        );

        assert_eq!(
            shell
                .state
                .game_form
                .as_ref()
                .expect("the form stays open")
                .category,
            "Uncategorized",
            "an `Uncategorized` hit must not write its own blank"
        );
    }

    /// A form reply with no app id leaves the form's alone: the QML's
    /// `if (hit.steamAppid)` (`GameFormPage.qml:61`).
    #[test]
    fn a_form_reply_with_no_appid_keeps_the_old_one() {
        let mut shell = shell_with_work_to_do();
        let form = shell
            .state
            .game_form
            .as_mut()
            .expect("the fixture opens a form");
        form.steam_appid = "999".to_string();
        let hit = CoverHit::from_steam(
            0,
            "Setup".to_string(),
            "Uncategorized".to_string(),
            PathBuf::from("/tmp/setup.ico"),
            "C:\\setup.exe".to_string(),
        );

        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Ok(hit),
            },
        );

        assert_eq!(
            shell
                .state
                .game_form
                .as_ref()
                .expect("the form stays open")
                .steam_appid,
            "999",
            "an appid of zero must not overwrite a stored one"
        );
    }

    /// A form reply after the form closed still notifies: the reference emits
    /// `notify` whether or not the page is there to answer `coverFetched`
    /// (`bridge.py:585`), and only the field writes need the form.
    #[test]
    fn a_form_reply_after_the_form_closed_still_notifies() {
        let mut shell = shell_with_work_to_do();
        shell.state.game_form = None;
        let hit = CoverHit::from_steam(
            70,
            "Half-Life".to_string(),
            "Action".to_string(),
            PathBuf::from("/tmp/hl.jpg"),
            "https://example.invalid/hl.jpg".to_string(),
        );

        let effect = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Ok(hit),
            },
        );

        assert!(
            effect.task_units > 0
                && format!("{:?}", shell.state.toasts).contains("Cover found via Steam: Half-Life"),
            "the notice must fire with no form to write: {effect:?}"
        );
    }

    /// A failed form lookup names the game **and** keeps the raw message.
    ///
    /// Both halves are asserted, and the second is the one that keeps the port
    /// honest about `UX-15`: the row is that the *context* was missing, not that
    /// the message was unfit to show. `NOTIFY_VOICES` pins the reference's
    /// default `fail` (`bridge.py:161`) as the raw `str(exc)`, so a fix that
    /// replaced the message with a friendlier one would put the port and the
    /// reference's own voice at odds while making this test pass.
    #[test]
    fn a_failed_form_lookup_names_the_game_and_keeps_the_message() {
        let mut shell = shell_with_work_to_do();

        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: 0,
                name: "Hades".to_string(),
                result: Err("connection refused".to_string()),
            },
        );

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("connection refused"),
            "the failure must still surface verbatim; toasts: {toasts}"
        );
        assert!(
            toasts.contains("Hades"),
            "and it must say which game it was about — a bare `connection \
             refused` is the whole of UX-15; toasts: {toasts}"
        );
        assert!(
            toasts.contains("cover"),
            "the user cannot tell a cover lookup failed as opposed to a save or \
             a launch; toasts: {toasts}"
        );
    }

    /// The name on a failed reply is the one the **request** carried.
    ///
    /// `UX-15`'s library half names the game by looking the id up again, which
    /// is right there — the id is the key. The form has no id to look up, so
    /// the name rides on the message. That makes the two halves diverge in a
    /// way a reader would not guess from either alone, which is why it is
    /// pinned: a reply for a form that has since been renamed must report the
    /// name the lookup was for, not the one now in the box.
    #[test]
    fn a_failed_form_reply_names_the_game_the_request_was_for() {
        let mut shell = shell_with_work_to_do();
        let mut form = GameForm::new_template(&shell.state.settings, "coverless".to_string());
        form.set_field(crate::state::FormField::Name, "Renamed Since".to_string());
        let _ = observe(&mut shell, Message::OpenNewGameForm);
        shell.state.game_form = Some(form);

        let live_token = shell.state.form_cover_token;
        let _ = observe(
            &mut shell,
            Message::FormCoverFetchFinished {
                token: live_token,
                name: "The Name Asked For".to_string(),
                result: Err("connection refused".to_string()),
            },
        );

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("The Name Asked For"),
            "the reply must name the request's game; toasts: {toasts}"
        );
        assert!(
            !toasts.contains("Renamed Since"),
            "and must not re-read a form that has been edited since the lookup \
             started — that would report a failure about a game nobody asked \
             about; toasts: {toasts}"
        );
    }

    /// `UX-15`'s library half: a failed cover fetch names the game.
    ///
    /// The id is the message's own, so this half does not need the name carried
    /// — and that is the point of the assertion: with the game in the library,
    /// the failure names it, and the assertion would still hold if the arm read
    /// the name from anywhere else. The sibling test below is what stops that.
    #[test]
    fn a_failed_library_cover_fetch_names_the_game() {
        let mut shell = shell_with_work_to_do();
        let mut game = gamehandler_core::models::Game::new_named("Hades");
        game.id = "cover-1".to_string();
        shell.state.library.add(game).unwrap();

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "cover-1".to_string(),
                result: Err("connection refused".to_string()),
            },
        );

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(toasts.contains("connection refused"), "toasts: {toasts}");
        assert!(
            toasts.contains("Hades"),
            "the failure must name the game; toasts: {toasts}"
        );
        assert!(
            toasts.contains("cover"),
            "and say that a cover lookup is what failed; toasts: {toasts}"
        );
    }

    /// A failed fetch for an id the library does not hold still says something.
    ///
    /// The lookup re-checks the game before writing (`bridge.py:547-549`), so an
    /// id can be missing here — deleted between the request and the reply. That
    /// path must not panic on a `None` unwrap and must not produce an empty
    /// prefix that reads as a formatting bug.
    #[test]
    fn a_failed_cover_fetch_for_an_unknown_game_still_reads() {
        let mut shell = shell_with_work_to_do();

        let _ = observe(
            &mut shell,
            Message::CoverFetchFinished {
                game_id: "deleted-while-fetching".to_string(),
                result: Err("connection refused".to_string()),
            },
        );

        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("deleted-while-fetching"),
            "an id with no game must fall back to the id rather than an empty \
             name: a toast reading `Could not fetch a cover for “”:` is a \
             formatting bug the user sees; toasts: {toasts}"
        );
        assert!(toasts.contains("connection refused"), "toasts: {toasts}");
    }

    /// Saving without a cover starts a lookup alongside the notice (P-31):
    /// `if not game.cover_path: self.fetchCover(game.id)` (`bridge.py:444`).
    #[test]
    fn saving_without_a_cover_starts_a_lookup() {
        let mut shell = shell_with_work_to_do();
        let mut form = GameForm::new_template(&shell.state.settings, "coverless".to_string());
        form.set_field(crate::state::FormField::Name, "Coverless".to_string());

        let effect = observe(&mut shell, Message::SaveGameForm(form));

        assert!(
            effect.task_units >= 2,
            "the notice without the fetch is half the save: {effect:?}"
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("Added \u{201c}Coverless\u{201d}"),
            "the notice must still fire; toasts: {:?}",
            shell.state.toasts
        );
    }

    /// Saving with a cover starts no lookup: the notice is the whole answer.
    #[test]
    fn saving_with_a_cover_starts_no_lookup() {
        let mut shell = shell_with_work_to_do();
        let mut form = GameForm::new_template(&shell.state.settings, "covered".to_string());
        form.set_field(crate::state::FormField::Name, "Covered".to_string());
        form.set_field(
            crate::state::FormField::CoverPath,
            "/tmp/covered.jpg".to_string(),
        );

        let effect = observe(&mut shell, Message::SaveGameForm(form));

        assert_eq!(
            effect.task_units, 1,
            "a covered save must return the notice alone: {effect:?}"
        );
    }

    /// An install whose executable carries no icon keeps no cover: the
    /// reference's own `except` fallback (`bridge.py:910-911`), which is what
    /// a missing file reaches. The success half is `core`'s
    /// `save_exe_icon_to`, tested there (see [`easy_install_cover`]).
    #[test]
    fn an_install_whose_executable_has_no_icon_keeps_no_cover() {
        let missing =
            std::env::temp_dir().join(format!("gh-no-such-exe-{}-{}", std::process::id(), "u5"));
        let _ = std::fs::remove_file(&missing);

        assert_eq!(
            easy_install_cover(&missing, "install-9"),
            "",
            "an unreadable executable must fall back to the empty string"
        );
        assert!(
            !missing.exists(),
            "the probe must not create the file it failed to read"
        );
    }

    /// Picking an exe opens the chooser: the request's whole effect is the
    /// task, which the portal answers.
    #[test]
    fn picking_an_exe_opens_the_chooser() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::PickExeFile);

        assert!(
            effect.task_units > 0,
            "the exe row's browse button must open the dialog: {effect:?}"
        );
    }

    /// A chosen exe fills the field and, when the name is blank, auto-fills it
    /// from the basename-minus-extension (`GameFormPage.qml:339-344`, P-20).
    #[test]
    fn a_chosen_exe_fills_the_field_and_autofills_a_blank_name() {
        let mut shell = shell_with_work_to_do();
        shell
            .state
            .game_form
            .as_mut()
            .expect("the fixture opens a form")
            .name = String::new();

        let _ = observe(
            &mut shell,
            Message::ExeFileChosen(Some("/tmp/Setup/setup.exe".to_string())),
        );

        let form = shell.state.game_form.as_ref().expect("the form stays open");
        assert_eq!(form.exe_path, "/tmp/Setup/setup.exe");
        assert_eq!(form.name, "setup");
    }

    /// A chosen exe does not touch a name the user typed: the autofill fires
    /// only for a blank name (`nameField.text.trim() === ""`).
    #[test]
    fn a_chosen_exe_keeps_a_typed_name() {
        let mut shell = shell_with_work_to_do();

        let _ = observe(
            &mut shell,
            Message::ExeFileChosen(Some("/tmp/Setup/setup.exe".to_string())),
        );

        let form = shell.state.game_form.as_ref().expect("the form stays open");
        assert_eq!(form.exe_path, "/tmp/Setup/setup.exe");
        assert_eq!(
            form.name, "Fixture",
            "the typed name must win over the basename"
        );
    }

    /// The autofill agrees with the QML on the edge shapes: `dot > 0` keeps a
    /// leading-dot basename whole, and a basename with no dot is kept whole
    /// too (`base.substring(0, dot)` else `base`).
    #[test]
    fn autofill_agrees_with_the_qml_on_edge_shapes() {
        for (chosen, named) in [("/tmp/x/.profile", ".profile"), ("/tmp/x/README", "README")] {
            let mut shell = shell_with_work_to_do();
            shell
                .state
                .game_form
                .as_mut()
                .expect("the fixture opens a form")
                .name = String::new();

            let _ = observe(&mut shell, Message::ExeFileChosen(Some(chosen.to_string())));

            assert_eq!(
                shell
                    .state
                    .game_form
                    .as_ref()
                    .expect("the form stays open")
                    .name,
                named,
                "choosing {chosen} must name the game {named}"
            );
        }
    }

    /// A cancelled exe choice writes nothing: the QML's exe dialog has no
    /// `onRejected`, so rejecting leaves the field — and the name — as they
    /// were. The empty string is the same silence, for a mapping that hands
    /// one over.
    #[test]
    fn a_cancelled_exe_choice_writes_nothing() {
        for path in [None, Some(String::new())] {
            let mut shell = shell_with_work_to_do();

            let effect = observe(&mut shell, Message::ExeFileChosen(path));

            assert!(
                !effect.state_changed && effect.task_units == 0,
                "a cancel must change nothing and say nothing: {effect:?}"
            );
        }
    }

    /// A chosen exe with no open form is silence: there is no field to write
    /// it into.
    #[test]
    fn a_chosen_exe_with_no_open_form_is_silence() {
        let mut shell = shell_with_work_to_do();
        shell.state.game_form = None;

        let effect = observe(
            &mut shell,
            Message::ExeFileChosen(Some("/tmp/Setup/setup.exe".to_string())),
        );

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "with no form open the choice has nowhere to go: {effect:?}"
        );
    }

    /// Picking a cover opens the chooser, like picking an exe.
    #[test]
    fn picking_a_cover_opens_the_chooser() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::PickCoverFile);

        assert!(
            effect.task_units > 0,
            "the cover row's browse button must open the dialog: {effect:?}"
        );
    }

    /// A cancelled cover choice writes nothing, for the same absent
    /// `onRejected` as the exe dialog.
    #[test]
    fn a_cancelled_cover_choice_writes_nothing() {
        let mut shell = shell_with_work_to_do();

        let effect = observe(&mut shell, Message::CoverFileChosen(None));

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "a cancel must change nothing and say nothing: {effect:?}"
        );
    }

    /// A cover choice for a source that is not there is reported with the
    /// reference's sentence (`bridge.py:595-597`). Hermetic: the copy refuses
    /// a missing source before any write, so this test touches no covers
    /// directory. (The success assignment is read, not tested — see
    /// `CoverFileChosen`.)
    #[test]
    fn a_cover_choice_for_a_missing_source_is_reported() {
        let mut shell = shell_with_work_to_do();

        let _ = observe(
            &mut shell,
            Message::CoverFileChosen(Some("/tmp/gh-no-such-cover.png".to_string())),
        );

        assert!(
            format!("{:?}", shell.state.toasts).contains("Could not copy cover"),
            "the failed import must surface; toasts: {:?}",
            shell.state.toasts
        );
        assert!(
            shell
                .state
                .game_form
                .as_ref()
                .expect("the form stays open")
                .cover_path
                .is_empty(),
            "a failed import must not write the field"
        );
    }

    /// A cover choice with no open form attempts no copy: without a form
    /// there is nowhere to put the path, so the arm returns before the
    /// filesystem is even consulted — no toast either.
    #[test]
    fn a_cover_choice_with_no_open_form_attempts_no_copy() {
        let mut shell = shell_with_work_to_do();
        shell.state.game_form = None;

        let effect = observe(
            &mut shell,
            Message::CoverFileChosen(Some("/tmp/gh-no-such-cover.png".to_string())),
        );

        assert!(
            !effect.state_changed && effect.task_units == 0,
            "with no form open the choice must not even be attempted: {effect:?}"
        );
    }

    /// The exe choice resolves the URL the share-aware way: `file://` is
    /// unwrapped and decoded, and an unmounted share keeps its verbatim
    /// fallback (`as_local_path`, P-21's browse half). Cancel and portal
    /// failure alike become `None`.
    #[test]
    fn the_exe_choice_resolves_the_url_the_share_aware_way() {
        let file = url::Url::parse("file:///tmp/My%20Game/setup.exe").unwrap();
        assert!(
            matches!(&exe_choice_message(Ok(file)), Message::ExeFileChosen(path)
                if path.as_deref() == Some("/tmp/My Game/setup.exe")),
            "a `file://` choice must arrive decoded"
        );

        let share = url::Url::parse("smb://example.invalid/share/setup.exe").unwrap();
        assert!(
            matches!(&exe_choice_message(Ok(share)), Message::ExeFileChosen(path)
                if path.as_deref() == Some("smb://example.invalid/share/setup.exe")),
            "an unmounted share must keep its verbatim fallback, not collapse to a cancel"
        );

        assert!(
            matches!(
                &exe_choice_message(Err(cosmic::dialog::file_chooser::Error::Cancelled)),
                Message::ExeFileChosen(path) if path.is_none()
            ),
            "a rejection must reach the silent half"
        );
    }

    /// The cover choice resolves like the exe choice: the reference runs both
    /// through `as_local_path`.
    #[test]
    fn the_cover_choice_resolves_like_the_exe_choice() {
        let file = url::Url::parse("file:///tmp/My%20Cover.png").unwrap();
        assert!(
            matches!(&cover_choice_message(Ok(file)), Message::CoverFileChosen(path)
                if path.as_deref() == Some("/tmp/My Cover.png")),
            "a `file://` choice must arrive decoded"
        );

        assert!(
            matches!(
                &cover_choice_message(Err(cosmic::dialog::file_chooser::Error::Cancelled)),
                Message::CoverFileChosen(path) if path.is_none()
            ),
            "a rejection must reach the silent half"
        );
    }

    /// The image filter is the reference's one `nameFilters` row
    /// (`GameFormPage.qml:352`): images only, no "All files".
    #[test]
    fn the_image_filter_lists_images_and_no_all_files_row() {
        let filters = image_file_filters();

        assert_eq!(filters.len(), 1, "the reference lists exactly one filter");
        assert_eq!(filters[0].label(), "Images");
        assert_eq!(
            filters[0].pattern_filters(),
            ["*.png", "*.jpg", "*.jpeg", "*.webp"]
        );
    }

    /// Only the executable row carries a browse press, and it is the exe
    /// dialog's (F4). The button-to-message edge itself is read, not tested
    /// (see `text_control`); this pins the table half — which row holds which
    /// press — so a press that moved rows fails here.
    ///
    /// The hint is asserted here too, and it is the other half of UX-12: the
    /// press and the hint live on one value ([`crate::view::form::BrowseButton`]),
    /// so a table that moved the press without the sentence would fail the
    /// second assertion rather than build a button named after another row's
    /// subject.
    #[test]
    fn only_the_exe_row_carries_a_browse_press() {
        let pressed: Vec<(&str, &crate::Message)> = crate::view::form::TEXT_ROWS
            .iter()
            .filter_map(|row| row.browse.as_ref().map(|browse| (row.id, &browse.press)))
            .collect();

        // `Message` is not `PartialEq`, so the pair is matched rather than
        // compared: one row, the exe row, carrying the exe dialog's press.
        assert!(
            matches!(pressed.as_slice(), [(id, crate::Message::PickExeFile)] if *id == "exeField"),
            "exactly the exe row must carry exactly the exe dialog's press: {pressed:?}"
        );

        let hints: Vec<(&str, &str)> = crate::view::form::TEXT_ROWS
            .iter()
            .filter_map(|row| row.browse.as_ref().map(|browse| (row.id, browse.hint)))
            .collect();
        assert_eq!(
            hints,
            [("exeField", crate::view::form::BROWSE_EXE_HINT)],
            "the exe row's browse button must hint the executable, and only it"
        );
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
        let windows = crate::view::testkit::drawn_strings(
            shell_with_form_open(true, false).view_with_overlays(),
        );
        assert!(
            windows
                .iter()
                .any(|text| text == crate::view::form::LABEL_RUNNER),
            "a Windows game has a runner to choose; drawn: {windows:?}"
        );

        let linux = crate::view::testkit::drawn_strings(
            shell_with_form_open(true, true).view_with_overlays(),
        );
        assert_eq!(
            linux
                .iter()
                .any(|text| text == crate::view::form::LABEL_RUNNER),
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
        let drawn = crate::view::testkit::drawn_strings::<Message>(
            toggler(true).label(subtitle.to_string()).into(),
        );
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
        assert!(
            !effect.state_changed,
            "an unrecognised scheme must not be stored"
        );
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
    /// Setting the color scheme returns the task that applies it — on a
    /// change and on a repeat alike, because the apply is unconditional (see
    /// the arm: a stored-but-unapplied scheme is the bug that "works" until a
    /// restart). Mutation F in `theme`'s honesty list was "the arm stores the
    /// value and returns no task"; this is the test that kills it as stated.
    /// What it does not pin is the task's *content* — which scheme the apply
    /// carries is as unreadable as G, and the list says so.
    #[test]
    fn setting_the_color_scheme_returns_the_apply_task() {
        let mut shell = shell_with_work_to_do();
        let current = shell.state.settings.color_scheme.clone();
        let other = if current == "light" { "dark" } else { "light" }.to_string();

        let changed = observe(&mut shell, Message::SetColorScheme(other.clone()));
        assert_eq!(shell.state.settings.color_scheme, other);
        assert!(
            changed.task_units > 0,
            "storing without applying is the bug: {changed:?}"
        );

        let repeated = observe(&mut shell, Message::SetColorScheme(other));
        assert!(
            repeated.task_units > 0,
            "the apply is unconditional — a repeat must still apply: {repeated:?}"
        );
    }

    #[test]
    fn an_empty_default_runner_is_ignored() {
        let mut shell = shell_with_work_to_do();
        let original = shell.state.settings.default_runner.clone();

        let effect = observe(&mut shell, Message::SetDefaultRunner(String::new()));
        assert!(
            !effect.state_changed,
            "an empty runner id must not be stored"
        );
        assert_eq!(shell.state.settings.default_runner, original);

        let effect = observe(&mut shell, Message::SetDefaultRunner(original.clone()));
        assert!(
            !effect.state_changed,
            "writing the stored runner must report no change"
        );

        let effect = observe(
            &mut shell,
            Message::SetDefaultRunner("GE-Proton9-1".to_string()),
        );
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

        let effect = observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "mangohud".to_string(),
                value: true,
            },
        );
        assert!(
            effect.state_changed,
            "mangohud defaults to false, so this is a change"
        );
        assert!(shell.state.settings.default_mangohud);

        // Not "the compare fired": indistinguishable from no write. See above.
        let effect = observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "mangohud".to_string(),
                value: true,
            },
        );
        assert!(
            !effect.state_changed,
            "the state is unchanged by a repeated write"
        );

        let effect = observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "wayland".to_string(),
                value: true,
            },
        );
        assert!(
            !effect.state_changed,
            "`wayland` has no `default_wayland` field and must not be written"
        );

        // The other direction of the compare: `esync` is true by default.
        let effect = observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "esync".to_string(),
                value: false,
            },
        );
        assert!(
            effect.state_changed,
            "esync defaults to true, so turning it off is a change"
        );
        assert!(!shell.state.settings.default_esync);
    }

    /// **Every settings mutation persists to `settings.json`** — P-03, P-66 and
    /// the settings-write half of P-73. Each of the six mutation arms is
    /// driven through the real dispatcher and the file is reloaded after
    /// each, so deleting any arm's save call reddens exactly its assertion.
    ///
    /// The shell's settings path is a per-call temp file (`Shell::new`), so
    /// reloading it observes the arm and only the arm — no other test shares
    /// the file, and the user's settings are never touched.
    #[test]
    fn every_settings_mutation_persists_to_the_settings_file() {
        let mut shell = shell_with_work_to_do();
        let path = shell.state.settings.path().to_path_buf();
        let reloaded = || Settings::load(Some(path.clone()));

        observe(&mut shell, Message::SetColorScheme("light".to_string()));
        assert_eq!(reloaded().color_scheme, "light");

        observe(&mut shell, Message::SetViewMode("list".to_string()));
        assert_eq!(reloaded().view_mode, "list");

        observe(&mut shell, Message::SetSortMode("recent".to_string()));
        assert_eq!(reloaded().sort_mode, "recent");

        observe(
            &mut shell,
            Message::SetDefaultRunner("GE-Proton9-1".to_string()),
        );
        assert_eq!(reloaded().default_runner, "GE-Proton9-1");

        observe(&mut shell, Message::SetCloseOnLaunch(true));
        assert!(reloaded().close_on_launch);

        observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "mangohud".to_string(),
                value: true,
            },
        );
        assert!(reloaded().default_mangohud);

        // The other direction of a toggle, off a default-true field.
        observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "esync".to_string(),
                value: false,
            },
        );
        assert!(!reloaded().default_esync);
    }

    /// **Unchanged and invalid settings writes touch no file** — the
    /// reference's early returns (`bridge.py:198, 211, 222, 236, 248, 267`).
    ///
    /// This is the half `observe` cannot see: writing the stored value leaves
    /// the state's `Debug` byte-identical whether or not the arm compares
    /// first, which is the honest gap the toggle and close-on-launch tests
    /// above document. The filesystem can see it — a save would create the
    /// file — so a shell that only ever repeats the defaults must have no
    /// settings file at all.
    #[test]
    fn unchanged_and_invalid_settings_writes_touch_no_file() {
        let mut shell = shell_with_work_to_do();
        let path = shell.state.settings.path().to_path_buf();
        assert!(!path.exists(), "the fixture starts with no file");

        // Every default, repeated: scheme, view, sort, runner, close flag,
        // one default-false toggle and one default-true toggle.
        observe(&mut shell, Message::SetColorScheme("dark".to_string()));
        observe(&mut shell, Message::SetViewMode("grid".to_string()));
        observe(&mut shell, Message::SetSortMode("name".to_string()));
        let runner = shell.state.settings.default_runner.clone();
        observe(&mut shell, Message::SetDefaultRunner(runner));
        observe(&mut shell, Message::SetCloseOnLaunch(false));
        observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "mangohud".to_string(),
                value: false,
            },
        );
        observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "esync".to_string(),
                value: true,
            },
        );
        // Invalid values are ignored, not stored and not saved.
        observe(&mut shell, Message::SetColorScheme("neon".to_string()));
        observe(&mut shell, Message::SetViewMode("masonry".to_string()));
        observe(&mut shell, Message::SetSortMode("chaos".to_string()));
        observe(&mut shell, Message::SetDefaultRunner(String::new()));
        observe(
            &mut shell,
            Message::SetDefaultToggle {
                name: "wayland".to_string(),
                value: true,
            },
        );
        assert!(
            !path.exists(),
            "nothing changed, so nothing was saved — yet {path:?} exists"
        );
    }

    /// **A failed settings save is reported, not raised** — the
    /// `SaveGameForm`/`LaunchStarted` shape applied to the sixth store.
    ///
    /// The settings path sits under a regular file, so creating the
    /// directory fails and the save with it. Memory still updates — the
    /// store is not the value — and the arm answers with the toast. The
    /// text is read out of the `Debug` of the toaster, which derives it
    /// today; if libcosmic ever stops, this fails to compile rather than
    /// silently asserting nothing.
    #[test]
    fn a_failed_settings_save_is_reported_not_raised() {
        let mut shell = shell_with_work_to_do();
        // Inside the shell's own fixture directory, which `Shell::new` made
        // unique per call: no counter, no clock, no sharing.
        let dir = shell.state.settings.path().to_path_buf();
        let dir = dir
            .parent()
            .expect("the fixture path has a parent")
            .to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"a file, not a directory").unwrap();
        shell.state.settings = Settings::load(Some(blocker.join("settings.json")));

        let effect = observe(&mut shell, Message::SetViewMode("list".to_string()));
        assert_eq!(
            shell.state.settings.view_mode, "list",
            "memory updates even when the disk refuses"
        );
        assert!(
            effect.task_units > 0,
            "the failure must schedule its report: {effect:?}"
        );
        let toasts = format!("{:?}", shell.state.toasts);
        assert!(
            toasts.contains("Could not save settings"),
            "the report names the store: {toasts}"
        );
        assert!(
            !blocker.join("settings.json").exists(),
            "no file was written through the blocker"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(
            !effect.state_changed,
            "the state is unchanged by a repeated write"
        );

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
        assert_eq!(
            shell.state.page,
            Page::Library,
            "the reference's start page"
        );

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

    /// **Navigating away closes the game form.**
    ///
    /// `BUG-46`: `view_with_overlays` returns the form *before* it consults the
    /// page, so with a form open the body kept drawing the form while the
    /// sidebar moved to the page the user had chosen — the rail and the body
    /// disagreeing about which page is showing. The reference closes it
    /// structurally: `showPage` pops every layer and clears the page stack
    /// (`Main.qml:37-45`), and the form is the one thing pushed as a layer
    /// (`:47-53`).
    ///
    /// The whole overlay set is asserted, not just the form — see
    /// [`State::clear_overlays`]. Both navigation routes are driven, because
    /// they reach `show_page` by different paths and a fix in the
    /// `Message::NavigateTo` arm alone would leave `Ctrl+F` broken; that arm is
    /// what the row recommended, and it is the wrong place for exactly this
    /// reason (the sidebar's own clicks never pass through it either).
    #[test]
    fn navigating_away_closes_the_layers_the_page_was_covering() {
        // `Ctrl+F` — `App::on_search` → `focus_library_search` → `show_page`.
        let mut shell = shell_with_work_to_do();
        let _ = shell.show_page(Page::Settings);
        shell.state.game_form = Some(GameForm::new_template(
            &shell.state.settings,
            "x".to_string(),
        ));
        shell.state.confirm_delete = Some("g".to_string());
        assert!(
            shell.state.game_form.is_some(),
            "the fixture must start with a form open, or nothing below is \
             measuring the navigation"
        );
        let _ = shell.focus_library_search();
        assert!(
            shell.state.game_form.is_none(),
            "`Ctrl+F` navigated to the Library and left the form over it"
        );
        assert!(shell.state.confirm_delete.is_none());
        assert_eq!(shell.state.page, Page::Library);
        assert!(shell.pages_agree());

        // `Ctrl+,` and every other non-sidebar route — `Message::NavigateTo`.
        let mut shell = shell_with_work_to_do();
        let _ = shell.show_page(Page::Library);
        shell.state.game_form = Some(GameForm::new_template(
            &shell.state.settings,
            "x".to_string(),
        ));
        let _ = shell.update(Message::NavigateTo(Page::Settings));
        assert!(
            shell.state.game_form.is_none(),
            "`Message::NavigateTo` left the form over the page it navigated to"
        );
        assert_eq!(shell.state.page, Page::Settings);

        // And the form's own Cancel is *not* a navigation: a `CloseDialog` on a
        // shell that is already on the right page must still be what closes it,
        // rather than this test passing because everything closes everything.
        let mut shell = shell_with_work_to_do();
        let _ = shell.show_page(Page::Library);
        shell.state.game_form = Some(GameForm::new_template(
            &shell.state.settings,
            "x".to_string(),
        ));
        let _ = shell.update(Message::CloseDialog);
        assert!(
            shell.state.game_form.is_none(),
            "the form's own Cancel must keep working"
        );
    }

    /// **`clear_overlays` names every overlay the shell can be showing.**
    ///
    /// The fix for `BUG-46` is a list, and a list is the thing that goes stale:
    /// a fourth modal added later and left out of `clear_overlays` would survive
    /// a navigation exactly as the form did, and the navigation test above
    /// would not notice, because it only looks at the fields it knows about.
    ///
    /// So this holds the method's body against the fields it is supposed to
    /// cover, read from the source. A new `Option<…>` overlay field on `State`
    /// that is not in the list fails here — the same shape as the notify-voice
    /// guard, and unlike that one the haystack is the *method* rather than the
    /// table, so it cannot be satisfied by the thing it is checking for.
    #[test]
    fn clear_overlays_names_every_overlay_field_on_state() {
        let source = include_str!("state.rs");
        let body = source
            .split("pub fn clear_overlays(&mut self) {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("`clear_overlays` must exist in state.rs");
        for field in ["game_form", "confirm_delete", "confirm_remove_runner"] {
            assert!(
                body.contains(field),
                "`clear_overlays` does not clear `{field}`; overlays that are \
                 not cleared here outlive the page they cover, which is BUG-46"
            );
        }
        // The list is not empty-for-the-wrong-reason: it must also *assign*
        // `None`, not merely mention the names.
        assert_eq!(
            body.matches("= None;").count(),
            3,
            "each overlay must be set to `None`, not just named: {body}"
        );
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

    /// **A click on the page under an open dialog reaches nothing (UX-06).**
    ///
    /// The dialogs used to be composed as a `Column` — the dialog in one band,
    /// the page in the next — so the page was neither behind the dialog nor
    /// blocked by it and every control on it stayed live under an open
    /// destructive prompt. [`dialog_over`] replaces that with a `Stack`: the page,
    /// an input-blocking [`dialog_scrim`], the dialog. This is the measurement
    /// the finding was written from and the one its fix has to move.
    ///
    /// # Why the click is real, and why the control matters
    ///
    /// `captured` is not the signal: `MouseArea::update` captures a left press
    /// whether or not it carries a message (`iced/widget/src/mouse_area.rs:466`),
    /// so the scrim captures in the *fixed* case and the page's own button
    /// captures in the broken one. What discriminates is the **message** — the
    /// page's own button publishes [`Message::OpenNewGameForm`], and the broken
    /// composition published it under an open "Remove …?" prompt.
    ///
    /// The first click is the control, and it is what keeps the second from being
    /// vacuous: a point that reaches no control at all would pass the "nothing
    /// published" assertion in any composition, including the broken one. It also
    /// pins the point to the *page's* button — the same string the closed
    /// composition draws — so the second click is on the same pixel.
    ///
    /// The third click is the other half: the scrim must block the page without
    /// sealing off the dialog, so the dialog's own Cancel still publishes.
    #[test]
    fn a_click_on_the_page_under_an_open_dialog_reaches_nothing() {
        let window = cosmic::iced::Size::new(420.0, 800.0);
        let target = crate::view::library::ADD_FIRST_GAME;

        // The control: nothing open, the page's own button, a real press.
        let shell = Shell::new();
        let closed = laid_out_texts(&mut shell.view_with_overlays(), window);
        let point = closed
            .iter()
            .find(|(text, _)| text == target)
            .map(|(_, bounds)| bounds.center())
            .unwrap_or_else(|| {
                panic!(
                    "a fresh shell opens on the Library page, whose empty state draws \
                     {target:?}; the page drew {:?} instead",
                    closed.iter().map(|(text, _)| text).collect::<Vec<_>>()
                )
            });
        let (messages, captured) = click_at(&mut shell.view_with_overlays(), window, point);
        assert!(
            captured,
            "a press at {point:?} — the centre of the page's own {target:?} button — \
             was captured by nothing, so this point is not on the button and the \
             assertion below would pass without touching the page at all"
        );
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::OpenNewGameForm)),
            "with no dialog open, the press must publish `OpenNewGameForm`: without \
             this the check below cannot tell a blocked page from a point that \
             never reached a control"
        );

        // The finding: the same point, with a destructive prompt open.
        let mut shell = Shell::new();
        let _ = shell.update(Message::ConfirmRemoveRunner {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });
        let open = laid_out_texts(&mut shell.view_with_overlays(), window);
        let under = open
            .iter()
            .find(|(text, _)| text == target)
            .map(|(_, bounds)| bounds.center())
            .unwrap_or_else(|| {
                panic!(
                    "the page must stay drawn under the dialog rather than be \
                     replaced by it, so {target:?} must still be in the tree; the \
                     overlay drew {:?}",
                    open.iter().map(|(text, _)| text).collect::<Vec<_>>()
                )
            });
        assert_eq!(
            under, point,
            "the open dialog must not reflow the page: the same control is at \
             {under:?} under the prompt and {point:?} without it, so the press \
             below would be measuring a different pixel than the control"
        );
        let (messages, _) = click_at(&mut shell.view_with_overlays(), window, under);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::OpenNewGameForm)),
            "a press on the page's own {target:?} button, with a removal prompt \
             open over it, published `OpenNewGameForm` — that is UX-06 back: the \
             prompt is not modal and the page under it is still live. Messages: \
             {messages:?}"
        );

        // And the dialog is not sealed off by its own scrim.
        let cancel = open
            .iter()
            .find(|(text, _)| text == "Cancel")
            .map(|(_, bounds)| bounds.center())
            .expect("the dialog draws Cancel");
        let (messages, _) = click_at(&mut shell.view_with_overlays(), window, cancel);
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::CloseDialog)),
            "the press at {cancel:?} — the centre of the dialog's own Cancel \
             button — published {messages:?} rather than `CloseDialog`, so the \
             scrim is in front of the dialog as well as behind it"
        );
    }

    /// **Escape cancels whichever confirmation is open, and leaves the form
    /// alone (UX-07).**
    ///
    /// The state half of the finding: `Message::CloseDialog` clears three things
    /// and Escape clears the two dialogs, which is the reference's own split
    /// (`Kirigami.PromptDialog`'s default `closePolicy` is `CloseOnEscape`;
    /// the form is a pushed `Page` with no Escape binding in the QML).
    /// [`Shell::dismiss_dialogs`] is that decision as a `Shell` method, so it can
    /// be measured here rather than only in a running application.
    ///
    /// The return value is asserted because it is the finding's own branch —
    /// "`Task::none()` when neither is set" — made observable: a `Task` has no
    /// accessor, so the *decision* behind it is what a test can read.
    ///
    /// The wiring — that `App::on_escape` is what calls this — is a separate
    /// test, [`the_escape_hook_is_wired_to_the_dismissal`], because `App` cannot
    /// be built off a display.
    #[test]
    fn escape_dismisses_a_pending_confirmation() {
        let mut shell = Shell::new();
        assert!(
            !shell.dismiss_dialogs(),
            "nothing was pending, so Escape must report that it dismissed nothing"
        );

        // Each of the two dialogs on its own, so dropping either half of the
        // method is visible in the other's check.
        shell.state.confirm_delete = Some("a-game".to_string());
        assert!(
            shell.dismiss_dialogs(),
            "a pending game delete is open, so Escape dismissed something"
        );
        assert!(
            shell.state.confirm_delete.is_none(),
            "Escape must clear the pending game delete"
        );

        shell.state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });
        assert!(
            shell.dismiss_dialogs(),
            "a pending runner removal is open, so Escape dismissed something"
        );
        assert!(
            shell.state.confirm_remove_runner.is_none(),
            "Escape must clear the pending runner removal"
        );

        // The actions layer (**UX-16**), which is the third thing Escape
        // dismisses. The reference binds it too: the QML's `QQC2.Menu` closes on
        // Escape, and the toolkit's pointer-opened context menu takes
        // `close_on_escape: true` (`src/widget/context_menu.rs:34`).
        shell.state.game_menu = Some("a-game".to_string());
        assert!(
            shell.dismiss_dialogs(),
            "an open actions layer is something Escape dismissed"
        );
        assert!(
            shell.state.game_menu.is_none(),
            "Escape must clear the actions layer as well as the two prompts"
        );

        // Both at once — the ordering backstop in `view_with_overlays` says this
        // state is reachable, so neither field may be left behind.
        shell.state.confirm_delete = Some("a-game".to_string());
        shell.state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });
        assert!(shell.dismiss_dialogs());
        assert!(
            shell.state.confirm_delete.is_none() && shell.state.confirm_remove_runner.is_none(),
            "both were open, so both must go: delete {:?}, runner {:?}",
            shell.state.confirm_delete,
            shell.state.confirm_remove_runner
        );

        // And the form is deliberately *not* part of this. `CloseDialog` clears
        // it; Escape must not, or a half-filled form would be discarded by a key
        // the reference binds only to its prompts.
        let mut with_form = shell_with_work_to_do();
        assert!(
            with_form.state.game_form.is_some(),
            "this control needs an open form to be a control at all"
        );
        let _ = with_form.dismiss_dialogs();
        assert!(
            with_form.state.game_form.is_some(),
            "Escape is the prompts' key, not the form's: the form must survive it"
        );
    }

    /// **Escape reaches [`Shell::dismiss_dialogs`] at all (UX-07).**
    ///
    /// The finding is that `App` never implemented `on_escape`, so the framework
    /// took its default — `Task::none()` without looking at anything
    /// (`libcosmic src/app/mod.rs:436-439`) — and Escape on an open prompt did
    /// nothing. The test above proves the dismissal works; it would pass just as
    /// well with the hook still missing, because `App` needs a window and no test
    /// can call it. So this half is read out of the source, exactly as
    /// [`the_window_floor_is_the_references_and_the_builder_asks_for_it`] reads
    /// the settings construction out of this same file.
    ///
    /// It is a weaker instrument than a call, and it is the strongest one
    /// available: a `Task` has no accessor, so the wiring cannot be observed any
    /// other way. What it grades is that the hook exists and delegates.
    #[test]
    fn the_escape_hook_is_wired_to_the_dismissal() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("this file must be readable");
        // The non-test half: this test's own body names the string, so searching
        // the whole file would find a match in the search itself.
        let tests_at = source
            .find("\nmod tests {")
            .expect("this file has a test module; the split below depends on it");
        let production = &source[..tests_at];

        let hooks = production.matches("fn on_escape(&mut self)").count();
        assert_eq!(
            hooks, 1,
            "this file implements `on_escape` {hooks} times; libcosmic calls it \
             once per Escape and this test grades the one, rather than assuming \
             there is one"
        );
        let start = production
            .find("fn on_escape(&mut self)")
            .expect("counted above");
        // The body runs to the closing brace at the function's own indent, with
        // the comments dropped first: the doc comment above the function names
        // `dismiss_dialogs` too, and a search that kept the prose would pass on a
        // hook whose body does nothing.
        let body: String = production[start..]
            .lines()
            .take_while(|line| line.trim() != "}")
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("dismiss_dialogs"),
            "`App::on_escape` does not call `Shell::dismiss_dialogs`, so Escape is \
             back to libcosmic's default — `Task::none()` and no look at anything \
             (`libcosmic src/app/mod.rs:436-439`) — and neither prompt closes on \
             it. That is UX-07 exactly. The body: {body:?}"
        );
        assert!(
            body.contains("cosmic::task::none()"),
            "`on_escape` returns nothing at all, so the extraction above found a \
             signature rather than a body and the check before this one is about \
             an empty string: {body:?}"
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
    /// **Each dialog carries its own doc comment, and the comment is about
    /// that dialog.** ARCH-22.
    ///
    /// The three dialog builders had one 107-line doc comment between them:
    /// `page_entry_task`'s block ran into `remove_runner_dialog`'s with no
    /// blank `///`, and that block in turn ran into `remove_game_dialog`'s. All
    /// of it therefore attached to the last function in the chain, and the two
    /// dialogs it was written for had no documentation at all — `rustdoc` showed
    /// one enormous block above `remove_game_dialog` and nothing above the
    /// others. Reading the file bottom-up, the mistake is invisible; the giveaway
    /// is a `///` line whose sentence has no antecedent, which is exactly what a
    /// reader skims past.
    ///
    /// # What this grades, and why it is a source scan
    ///
    /// A doc comment is not reachable at run time — no `Operation` arm carries
    /// one and this crate's traversal instruments photograph widgets, not
    /// attributes — so the source is the only place to look. Reading it is a
    /// weaker instrument than a call and it is the strongest one available.
    ///
    /// The claim graded is the one ARCH-22 was actually about, and it is a
    /// *two-way* claim, because the defect had two halves: every dialog has a
    /// comment (no orphans), **and** each comment names its own subject (no
    /// mis-attachment). A weaker test — "there is a `///` line above each `fn`"
    /// — passes on the broken file, since there was a `///` line above
    /// `remove_game_dialog`; it was 107 lines long and described something else.
    #[test]
    fn every_dialog_docblock_names_the_dialog_it_sits_above() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("this file must be readable");
        let tests_at = source
            .find("\nmod tests {")
            .expect("this file has a test module; the split below depends on it");
        let production = &source[..tests_at];

        // The dialogs this file draws, and the phrase each one's own doc must
        // contain. The phrase is the dialog's QML name, which is what the
        // reference calls it and what a reader arrives looking for.
        // Bare identifiers, not the backticked spellings: the third is written
        // `` `page.openGameMenu` ``, so requiring the backticked form would fail
        // on a doc that does name it — which is how this list was written the
        // first time, and the failure was the list's rather than the file's.
        let dialogs: [(&str, &str); 3] = [
            ("remove_game_dialog", "removeDialog"),
            ("remove_runner_dialog", "removeRunnerDialog"),
            ("game_menu_dialog", "openGameMenu"),
        ];

        for (function, phrase) in dialogs {
            let definition = format!("\nfn {function}<'a>(");
            let at = production.find(&definition).unwrap_or_else(|| {
                panic!(
                    "`{function}` is not defined at column 0 in this file, so this \
                     scan found nothing to grade — the scan is stale rather than \
                     the documentation being right"
                )
            });

            // The doc comment is the unbroken run of `///` lines directly above
            // the definition, which is the region rustdoc itself reads.
            let above = &production[..at];
            let mut block: Vec<&str> = Vec::new();
            for line in above.lines().rev() {
                if line.starts_with("///") {
                    block.push(line);
                } else if line.trim().is_empty() && block.is_empty() {
                    // A blank between the item and its doc would mean there is
                    // none; keep walking until something non-blank appears.
                    continue;
                } else {
                    break;
                }
            }
            block.reverse();

            assert!(
                !block.is_empty(),
                "`{function}` has no doc comment — the ARCH-22 defect, where the \
                 block describing it was attached to the function below"
            );

            let text = block.join("\n");
            assert!(
                text.contains(phrase),
                "`{function}`'s doc comment ({n} lines) never names {phrase}, so it \
                 is not this dialog's comment. ARCH-22: the give-away is a `///` \
                 line whose sentence has no antecedent.\n\n{text}",
                n = block.len()
            );

            // The orphan half: a doc comment this long above a dialog means a
            // neighbouring block has been swallowed into it, which is how the
            // 107-line block happened. 60 is well above the longest legitimate
            // dialog comment here and well below the merged one.
            assert!(
                block.len() < 60,
                "`{function}`'s doc comment is {} lines, which is longer than a \
                 dialog block and is what ARCH-22 looked like — check the function \
                 above it has not lost its own doc to this one",
                block.len()
            );
        }
    }

    /// The runner prompt's Remove button carries a message no render can see, so
    /// this reads the source to pin which one — the gap `remove_runner_dialog`'s
    /// header records, measured rather than assumed (ARCH-20).
    ///
    /// # Why a source scan is the instrument
    ///
    /// A built `Button`'s message is opaque to every instrument this file has —
    /// the sources are at `installed_card`'s call site, and they are the
    /// reason `the_runner_dialog_is_a_modal_over_the_page_it_names` photographs
    /// the *label* "Remove" and cannot tell [`Message::RemoveRunnerConfirmed`]
    /// from [`Message::UninstallRunner`]. Measured at HEAD: swapping the two in
    /// the dialog's `primary_action` leaves all 503 tests green. The source is
    /// the only place the choice is written down, so reading it is a weaker
    /// instrument than a call and it is the strongest one available — the same
    /// argument `every_dialog_docblock_names_the_dialog_it_sits_above` makes.
    ///
    /// # Why not the helper the header used to propose
    ///
    /// The header said the fix was "the same one the codebase already uses for
    /// it: a named press helper carrying the value, as `remove_press` does".
    /// That helper's own doc (`runners.rs:795-798`) says it does **not** close
    /// its call site — `installed_card` could stop calling it and build the
    /// message itself, "and that mutation survives", the third of three such
    /// gaps it names. A helper makes the value readable; it does not make the
    /// *call* unswappable. So the header was proposing a fix the codebase has
    /// already measured as insufficient for this exact gap class, which is why
    /// this scan exists instead.
    ///
    /// # What this does not cover
    ///
    /// The game dialog's Remove button, which has the same shape one function
    /// up. It is not covered because the mutation above was measured against the
    /// runner prompt and the game prompt's twin has not been measured — a scan
    /// written for the unmeasured one would be a check whose coverage is an
    /// assumption. Named here so the omission is a stated scope rather than a
    /// gap a reader has to find.
    #[test]
    fn the_runner_prompt_s_remove_button_sends_the_confirmed_removal_and_not_the_direct_one() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("this file must be readable");
        let tests_at = source
            .find("\nmod tests {")
            .expect("this file has a test module; the split below depends on it");
        let production = &source[..tests_at];

        // Column 0, as `fn`, which is how this file's dialog scans address a
        // definition — the indentation is the assertion that it is a top-level
        // function rather than a nested one that happens to share the name.
        let at = production.find("\nfn remove_runner_dialog<'a>(").expect(
            "`remove_runner_dialog` is not defined at column 0 in this file, so \
                 this scan found nothing to grade — the scan is stale rather than \
                 the button being right",
        );

        // The dialog's own body: up to its closing brace at column 0. Reading the
        // whole file instead would let the game dialog's identical button satisfy
        // this one, which is the failure mode a length-free scan has. The nested
        // braces are indented, so the first `\n}` is this function's own end.
        let rest = &production[at + 1..];
        let body = match rest.find("\n}\n") {
            Some(end) => &rest[..end],
            None => rest,
        };

        assert!(
            body.contains("fn remove_runner_dialog") && body.contains(".into();"),
            "the dialog's body was cut before it was read, so this scan is grading \
             the wrong region: {body}"
        );

        // Anti-vacuity, in the shape the rest of this audit uses: the body must
        // contain the button this test is about, or its silence below is the
        // silence of an empty haystack.
        let remove = body
            .find("button::destructive(\"Remove\")")
            .unwrap_or_else(|| {
                panic!(
                    "the runner prompt no longer draws a destructive \"Remove\" button, \
                 so this scan has no subject: {body}"
                )
            });
        let after = &body[remove..];
        let press = after.find(".on_press(").expect(
            "the runner prompt's Remove button has no `.on_press`, so it cannot \
             send a message at all and the ARCH-20 gap is closed by the button \
             being inert rather than by the message being right",
        ) + ".on_press".len();

        // The argument, by matching parentheses rather than by looking for a
        // newline: `on_press` here takes a single expression, and a mutation that
        // nested one would otherwise truncate the window at its inner close — the
        // first draft of this scan looked for `)\n` and read `.on_press(`'s own
        // nested call as the boundary, which the `))` in it then tripped over.
        let arguments = &after[press..];
        let mut depth = 0usize;
        let mut end = None;
        for (offset, ch) in arguments.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let call = match end {
            Some(end) => &arguments[..=end],
            None => panic!(
                "the Remove button's `on_press` never closes its parentheses, so \
                 this scan cannot tell what it carries: {arguments}"
            ),
        };
        assert!(
            call.starts_with('(') && call.ends_with(')'),
            "the paren walk over the Remove button's `on_press` did not bracket the \
             call it reads: {call}"
        );

        assert!(
            call.contains("Message::RemoveRunnerConfirmed("),
            "the runner prompt's Remove button does not send \
             `Message::RemoveRunnerConfirmed`. The confirmed arm clears the pending \
             removal *before* removing; the direct route removes while leaving the \
             dialog open. This is the choice `remove_runner_dialog`'s header calls \
             load-bearing, and it is unobservable from any render — which is why \
             the header recorded the gap and why this scan reads the source \
             instead. Found: {call}"
        );

        // The negative half, so the assertion above is not satisfied by a call
        // that carries both: this file has no reason for the direct variant at
        // this button, and a `match` or a builder would put it here.
        assert!(
            !call.contains("Message::UninstallRunner("),
            "the runner prompt's Remove button sends `Message::UninstallRunner` as \
             well as the confirmed route, so which one it carries depends on the \
             expression rather than on the source a reader is looking at: {call}"
        );
    }

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
        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

        for expected in [pending.title(), "Cancel".to_string(), "Remove".to_string()] {
            assert!(
                drawn.iter().any(|text| text == &expected),
                "the open dialog should draw {expected:?}; drawn: {drawn:?}"
            );
        }
        // The subtitle is long; assert it is drawn rather than equal, so a
        // re-wrapping does not fail what is a presence claim.
        assert!(
            drawn
                .iter()
                .any(|text| text.contains("fall back to System Wine")),
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
        let fallthrough = crate::view::testkit::drawn_strings(closed.view_with_overlays());
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

    /// Whether `inner` is drawn inside `outer`, both being laid-out rectangles.
    fn contains(outer: cosmic::iced::Rectangle, inner: cosmic::iced::Rectangle) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.width <= outer.x + outer.width
            && inner.y + inner.height <= outer.y + outer.height
    }

    /// The focus `id` names lands on the control labelled `on` and not on the
    /// one labelled `not_on` — asserted on the view rather than on the builder.
    ///
    /// This is the **UX-24** check, in one place because both destructive
    /// prompts make the same three claims and a second copy of them would be a
    /// second thing to forget to update. In order:
    ///
    /// - the id names a widget the laid-out tree actually draws, **exactly
    ///   once** — a focus that names nothing, or that names two controls, is
    ///   not a focus;
    /// - the control it is on contains the `on` label's bounds — read off the
    ///   layout, so moving the `.id(...)` call from `secondary_action` to
    ///   `primary_action` fails here even though the id is still drawn once;
    /// - it does not contain `not_on`'s. A focus on the destructive action is
    ///   the failure the row names, and it satisfies everything above it.
    ///
    /// `what` names the prompt, so a failure says which one moved.
    fn assert_focus_lands_on<M: Clone + 'static>(
        view: cosmic::Element<'_, M>,
        id: &str,
        on: &str,
        not_on: &str,
        what: &str,
    ) {
        let mut view = view;
        let seen = crate::view::testkit::traversal(&mut view);
        let ids = crate::view::testkit::ids(&seen);
        let wanted = cosmic::widget::Id::from(id.to_string());
        assert_eq!(
            ids.iter().filter(|drawn| **drawn == wanted).count(),
            1,
            "{what}: the focused id must name a widget the tree draws, once; ids: {ids:?}"
        );

        let landing = seen
            .iter()
            .find(|node| node.id.as_ref() == Some(&wanted))
            .expect("the id was counted just above, so a node carries it")
            .bounds;
        let label = |text: &str| {
            seen.iter()
                .find(|node| node.text.as_deref() == Some(text))
                .unwrap_or_else(|| panic!("{what}: {text:?} is not drawn; seen: {seen:?}"))
                .bounds
        };
        assert!(
            contains(landing, label(on)),
            "{what}: the prompt must open with the keyboard on {on}: id bounds \
             {landing:?}, {on} label bounds {:?}",
            label(on)
        );
        assert!(
            !contains(landing, label(not_on)),
            "{what}: the prompt must not open with the keyboard on the \
             destructive {not_on}: id bounds {landing:?}, {not_on} label bounds {:?}",
            label(not_on)
        );
    }

    /// **The remove-game prompt opens with the keyboard on Cancel, not on
    /// Remove (UX-24).**
    ///
    /// The finding's consequence is stated the other way round from how it
    /// reads. The prompt is drawn *over* a page that is still in the tree, so
    /// the Delete control that raised it is still a registered focusable and
    /// keeps the focus it had; the next Tab walks the rest of the page before
    /// reaching the two buttons being asked about, and at the moment of
    /// decision the destructive one is indistinguishable from the safe one.
    /// Nothing in the toolkit decides otherwise: libcosmic's `dialog()` builds
    /// its two actions with the default `Id::unique()` and mentions `focus`
    /// nowhere (`a401af8 src/widget/dialog.rs`), so the choice is the port's to
    /// make and `view::REMOVE_GAME_CANCEL_ID` records why it is Cancel.
    ///
    /// The arm's `units()` is the request; `assert_focus_lands_on` is the
    /// target. Neither implies the other, which is why both are here.
    ///
    /// The unknown-id half is the same guard the dialog already had
    /// (`Message::ConfirmDeleteGame`'s doc): no game, no dialog, and therefore
    /// no focus asked for a dialog nothing drew.
    #[test]
    fn the_remove_game_prompt_opens_with_the_keyboard_on_cancel() {
        let (_root, library) = library_with("u24-focus", &[("g1", "Hades")]);
        let mut shell = Shell::new();
        shell.state.library = library;

        assert!(
            shell
                .update(Message::ConfirmDeleteGame("g1".to_string()))
                .units()
                > 0,
            "opening the prompt must ask the runtime to move the focus into it, \
             or the keyboard stays on the Delete control that raised it"
        );
        assert_focus_lands_on(
            shell.view_with_overlays(),
            crate::view::REMOVE_GAME_CANCEL_ID,
            "Cancel",
            "Remove",
            "the remove-game prompt",
        );

        // An id the library does not hold opens nothing, so nothing is focused.
        let mut empty = Shell::new();
        assert_eq!(
            empty
                .update(Message::ConfirmDeleteGame("nobody".to_string()))
                .units(),
            0,
            "there is no prompt for a game that is not there, so there is no \
             focus to ask for either"
        );
    }

    /// **The runner-removal prompt opens with the keyboard on Cancel (UX-24).**
    ///
    /// [`the_remove_game_prompt_opens_with_the_keyboard_on_cancel`]'s claims for
    /// the second destructive prompt. It is not a copy of that test for a
    /// different reason than the copy would cost: this prompt's arm lives in
    /// `view::runners::update`, so the `units()` half here also pins that the
    /// page hands the operation back — an arm that answered `Task::none()` would
    /// leave the dialog opening unfocused and this test is the only one that
    /// looks.
    #[test]
    fn the_runner_removal_prompt_opens_with_the_keyboard_on_cancel() {
        let mut shell = Shell::new();
        assert!(
            shell
                .update(Message::ConfirmRemoveRunner {
                    runner_id: "GE-Proton9-5".to_string(),
                    name: "GE-Proton9-5".to_string(),
                })
                .units()
                > 0,
            "the runner page's own arm must hand back the focus operation, not \
             `Task::none()`"
        );
        assert_focus_lands_on(
            shell.view_with_overlays(),
            crate::view::REMOVE_RUNNER_CANCEL_ID,
            "Cancel",
            "Remove",
            "the runner-removal prompt",
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

        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

        assert!(
            drawn
                .iter()
                .any(|text| text == crate::view::form::TITLE_ADD),
            "the form must still be drawn; drawn: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text == &pending.title()),
            "the runner dialog must wait under the form, not over it; drawn: {drawn:?}"
        );
    }

    /// Deleting a held game parks the id in `confirm_delete`; the dialog the
    /// user answers is drawn from that pending value, looked up at draw time.
    #[test]
    fn confirming_delete_for_a_held_game_sets_the_pending_delete() {
        let mut shell = shell_with_work_to_do();
        // The fixture presets the pending delete; clear it so this asserts the
        // write, not the preset.
        shell.state.confirm_delete = None;

        let _ = shell.update(Message::ConfirmDeleteGame("g".to_string()));

        assert_eq!(
            shell.state.confirm_delete.as_deref(),
            Some("g"),
            "`ConfirmDeleteGame` must park the id so the dialog has something to ask about"
        );
        assert!(
            shell.state.library.get("g").is_some(),
            "asking must not remove: the game stays until the user confirms"
        );
    }

    /// Deleting an id the library does not hold opens nothing: there is no
    /// entry to name in the dialog, so the arm stays silent rather than ask
    /// about a game that is already gone.
    #[test]
    fn confirming_delete_for_an_unheld_game_opens_nothing() {
        let mut shell = shell_with_work_to_do();
        shell.state.confirm_delete = None;

        let _ = shell.update(Message::ConfirmDeleteGame("absent".to_string()));

        assert!(
            shell.state.confirm_delete.is_none(),
            "an unheld id must not open the dialog; nothing is pending"
        );
    }

    /// **UX-16: the "More actions" control's message opens the actions layer,
    /// and an id the library does not hold opens nothing.**
    ///
    /// The state half of the keyboard route. Its *widget* half — that the
    /// control is a Tab stop and that Enter on it publishes this message — is
    /// [`view::widgets`]'s
    /// `the_more_actions_control_is_the_second_tab_stop_and_presses_the_callers_message`,
    /// and the layer's own half is
    /// [`view::library::tests::every_action_in_the_layer_sends_what_its_menu_item_sends`].
    /// This is the arm the whole route lands in: the reference's `openGameMenu`
    /// (`LibraryPage.qml:209-213`), as a `Message`.
    #[test]
    fn opening_the_actions_layer_holds_the_game_and_an_unknown_id_opens_nothing() {
        let mut shell = shell_with_work_to_do();
        assert!(
            shell.state.game_menu.is_none(),
            "a fresh shell has no layer open"
        );

        // An unknown id first, so the assertion is that nothing opened rather
        // than that nothing *changed*: the fixture holds `"g"` and
        // `"sample-game"`, and this is neither.
        let _ = shell.update(Message::OpenGameMenu("absent".to_string()));
        assert!(
            shell.state.game_menu.is_none(),
            "an id the library does not hold has no actions to list, so it opens nothing"
        );

        let _ = shell.update(Message::OpenGameMenu("sample-game".to_string()));
        assert_eq!(
            shell.state.game_menu.as_deref(),
            Some("sample-game"),
            "the layer must remember *which* game's actions it is showing — the \
             focus request and the layer's contents both read it back"
        );

        // And an unknown id arriving while one is open leaves the open one
        // alone: the arm's silence is the same on the way in as on the way out.
        let _ = shell.update(Message::OpenGameMenu("absent".to_string()));
        assert_eq!(
            shell.state.game_menu.as_deref(),
            Some("sample-game"),
            "an unknown id must not close a layer that is already open"
        );
    }

    /// **Choosing any action from the layer closes it** — all eight, derived
    /// from the spec rather than listed here.
    ///
    /// A menu that stayed open after its item was chosen is the one thing a menu
    /// never does, and the eight messages come from
    /// [`view::library::game_menu_spec`] through
    /// [`view::library::game_menu_message`], so this walks the real menu rather
    /// than a transcription of it. Each message is driven on a shell whose layer
    /// is open for the game that sent it; the returned tasks are dropped exactly
    /// as `every_message` drops them, which is also what keeps the four that
    /// would spawn something (`LaunchGame`, `RunPrefixTool`, `OpenPrefixFolder`,
    /// `CreateDesktopShortcut`) from reaching the machine — every one of them
    /// does its work inside `Task::perform`.
    #[test]
    fn choosing_any_action_from_the_layer_closes_it() {
        let mut shell = shell_with_work_to_do();
        let mut game = Game::new_named("Sample");
        game.id = "sample-game".to_string();

        let actions: Vec<(view::library::GameMenuKind, Message)> =
            view::library::game_menu_spec(&game)
                .into_iter()
                .filter_map(|entry| match entry {
                    view::library::MenuEntry::Divider => None,
                    view::library::MenuEntry::Item { action, .. } => {
                        Some((action, view::library::game_menu_message(&game.id, action)))
                    }
                })
                .collect();
        assert_eq!(
            actions.len(),
            8,
            "the fixture is the reference's eight entries"
        );

        for (kind, action) in actions {
            shell.state.game_menu = Some(game.id.clone());
            let _ = shell.update(action);
            assert!(
                shell.state.game_menu.is_none(),
                "choosing {kind:?} left the actions layer open over the page"
            );
        }
    }

    /// The delete dialog is a modal over the page the user was on: it names
    /// the entry, says what survives (covers, saves stay on disk), keeps the
    /// keyboard escape (Cancel), and leaves the page — here the other game —
    /// drawn beneath it.
    #[test]
    fn the_delete_dialog_is_a_modal_over_the_page_it_names() {
        let (_root, library) = library_with("u3-photo", &[("g1", "Hades"), ("g2", "Celeste")]);
        let mut shell = Shell::new();
        shell.state.library = library;

        let _ = shell.update(Message::ConfirmDeleteGame("g1".to_string()));
        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

        assert!(
            drawn
                .iter()
                .any(|text| text == "Remove \u{201c}Hades\u{201d}?"),
            "the dialog must name the entry it is about to remove; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text.contains("left on disk")),
            "the dialog must say what is kept (covers, saves); drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == "Cancel"),
            "Cancel is the keyboard escape; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == "Remove"),
            "Remove is the destructive confirm; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == "Celeste"),
            "the page stays drawn beneath the modal; drawn: {drawn:?}"
        );

        // Nothing pending: the same call is the page, with no dialog in it.
        shell.state.confirm_delete = None;
        let fallthrough = crate::view::testkit::drawn_strings(shell.view_with_overlays());
        assert!(
            !fallthrough
                .iter()
                .any(|text| text == "Remove \u{201c}Hades\u{201d}?"),
            "no pending delete, so no dialog title; drawn: {fallthrough:?}"
        );
        assert!(
            fallthrough.iter().any(|text| text == "Hades"),
            "the fall-through is the page, both games drawn; drawn: {fallthrough:?}"
        );
    }

    /// **The actions layer is drawn, over the page, with the game's own eight
    /// actions in it (UX-16).**
    ///
    /// The wiring half: every other test of this finding passes with the layer
    /// never rendered at all — the state is written, the messages are sent, and
    /// a `view_with_overlays` that ignored `game_menu` would leave all of them
    /// green. This is the one that reads the built view.
    ///
    /// The labels are the menu's own, because the layer is built from
    /// [`view::library::game_menu_spec`]; the page beneath is asserted to still
    /// be drawn, for [`the_delete_dialog_is_a_modal_over_the_page_it_names`]'s
    /// reason — the layer is a modal over the page, not a replacement for it.
    #[test]
    fn the_actions_layer_is_drawn_over_the_page_with_the_games_own_actions() {
        let (_root, library) = library_with("u16-actions", &[("g1", "Hades"), ("g2", "Celeste")]);
        let mut shell = Shell::new();
        shell.state.library = library;
        shell.state.page = Page::Library;

        let without = crate::view::testkit::drawn_strings(shell.view_with_overlays());
        assert!(
            !without.iter().any(|text| text == "Remove from library"),
            "no layer is open, so no game's actions are drawn; drawn: {without:?}"
        );

        // The arm asks the runtime for work — the focus operation that puts the
        // keyboard on the layer's first control. What a `Task` can be asked is
        // its *unit count* and nothing else (`iced/runtime/src/task.rs:282`), so
        // this is the strongest available statement that the request is there:
        // it does not say *which* control, and the id it names is pinned
        // instead by
        // `view::library::tests::every_action_in_the_layer_is_a_named_control_and_the_disabled_ones_say_so`.
        assert!(
            shell
                .update(Message::OpenGameMenu("g1".to_string()))
                .units()
                > 0,
            "opening the layer must ask the runtime to move the focus into it, \
             or the keyboard user's next Tab walks the page first"
        );
        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

        let mut expected = vec![
            "Remove from library".to_string(),
            "Create desktop shortcut".to_string(),
            "Find cover art".to_string(),
        ];
        expected.sort();
        let mut drawn_actions: Vec<String> = drawn
            .iter()
            .filter(|text| expected.contains(text))
            .cloned()
            .collect();
        drawn_actions.sort();
        assert_eq!(
            drawn_actions, expected,
            "the layer must draw the menu's own entries; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == "Cancel"),
            "Cancel is the layer's keyboard escape; drawn: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|text| text == "Celeste"),
            "the page stays drawn beneath the layer; drawn: {drawn:?}"
        );

        // A game that vanished between opening and drawing draws no layer: the
        // list is that game's actions, and there are none for a game that is
        // not there.
        shell.state.game_menu = Some("missing".to_string());
        let vanished = crate::view::testkit::drawn_strings(shell.view_with_overlays());
        assert!(
            !vanished.iter().any(|text| text == "Remove from library"),
            "a layer for an absent game must draw nothing; drawn: {vanished:?}"
        );
    }

    /// A delete dialog for a game that vanished between asking and drawing
    /// names the id: the fallback of the name lookup in `view_with_overlays`,
    /// which unreachable-through-the-arms states still have to draw.
    #[test]
    fn a_delete_dialog_for_a_missing_game_names_the_id() {
        let (_root, library) = library_with("u3-missing", &[("g1", "Hades")]);
        let mut shell = Shell::new();
        shell.state.library = library;
        shell.state.confirm_delete = Some("missing".to_string());

        let drawn = crate::view::testkit::drawn_strings(shell.view_with_overlays());

        assert!(
            drawn
                .iter()
                .any(|text| text == "Remove \u{201c}missing\u{201d}?"),
            "the fallback title must carry the raw id; drawn: {drawn:?}"
        );
    }

    /// Confirming removes the entry from memory and from disk, closes the
    /// dialog, and names the removed game in the toast.
    #[test]
    fn confirming_the_delete_removes_the_entry_and_names_it() {
        let (root, library) = library_with("u3-confirm", &[("g1", "Hades"), ("g2", "Celeste")]);
        let mut shell = Shell::new();
        shell.state.library = library;
        let _ = shell.update(Message::ConfirmDeleteGame("g1".to_string()));
        assert_eq!(shell.state.confirm_delete.as_deref(), Some("g1"));

        let _ = shell.update(Message::DeleteGameConfirmed("g1".to_string()));

        assert!(
            shell.state.library.get("g1").is_none(),
            "the confirmed entry must leave the library"
        );
        assert!(
            shell.state.library.get("g2").is_some(),
            "the entry the user did not name must stay"
        );
        assert!(
            shell.state.confirm_delete.is_none(),
            "the dialog must close once it has acted"
        );
        assert!(
            Library::new_at(Some(root.join("games.json")), 0.0)
                .get("g1")
                .is_none(),
            "removal must persist: a reload must not resurrect the entry"
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("Removed \u{201c}Hades\u{201d}"),
            "the toast must name the removed game"
        );
    }

    /// Closing the dialog without confirming keeps the game: Cancel and Escape
    /// are pure dismissal. (The clearing half — `CloseDialog` dropping the
    /// pending delete — is `closing_the_dialog_clears_the_form_and_the_delete_confirmation`'s;
    /// this is the keeps-half.)
    #[test]
    fn cancel_keeps_the_game() {
        let mut shell = shell_with_work_to_do();
        let _ = shell.update(Message::ConfirmDeleteGame("g".to_string()));

        let _ = shell.update(Message::CloseDialog);

        assert!(
            shell.state.library.get("g").is_some(),
            "dismissal must not remove: the game stays"
        );
    }

    /// Confirming a delete for an id the library does not hold is silence past
    /// clearing the pending delete: the arm removes by the message's id (the
    /// P-37 shape — the confirmed message carries the authority, looked up at
    /// confirm time), so an unknown id finds nothing and reports nothing. A
    /// stale second press, after the first already removed the game, lands here
    /// too.
    #[test]
    fn confirming_a_delete_for_an_unknown_id_is_silence() {
        let mut shell = shell_with_work_to_do();

        let _ = shell.update(Message::DeleteGameConfirmed("absent".to_string()));

        assert!(
            shell.state.library.get("g").is_some(),
            "an unknown id must not remove the game"
        );
        assert!(
            shell.state.confirm_delete.is_none(),
            "the dialog must close even when there was nothing to remove"
        );
    }

    /// The two confirmations are one seat: opening either closes the other, so
    /// the user is never asked two destructive questions at once and the two
    /// dialogs can never stack.
    #[test]
    fn opening_one_dialog_closes_the_other() {
        let mut shell = shell_with_work_to_do();
        let _ = shell.update(Message::ConfirmDeleteGame("g".to_string()));

        let _ = shell.update(Message::ConfirmRemoveRunner {
            runner_id: "GE-Proton9-5".to_string(),
            name: "GE-Proton9-5".to_string(),
        });

        assert!(
            shell.state.confirm_delete.is_none(),
            "opening the runner dialog must close the delete dialog"
        );
        assert!(
            shell.state.confirm_remove_runner.is_some(),
            "the runner dialog must be pending after it opens"
        );

        let _ = shell.update(Message::ConfirmDeleteGame("g".to_string()));

        assert_eq!(
            shell.state.confirm_delete.as_deref(),
            Some("g"),
            "the delete dialog must be pending after it reopens"
        );
        assert!(
            shell.state.confirm_remove_runner.is_none(),
            "opening the delete dialog must close the runner dialog"
        );
    }

    /// A removal the disk refuses is reported, not dropped. The entry still
    /// leaves memory — the reference deletes before it saves
    /// (`models.py:173-176`) with no rollback, and this port keeps that order —
    /// but unlike the reference, whose save exception escapes the slot
    /// unreported, the port toasts what happened. The library path is blocked
    /// by a file where its directory was, so the save fails deterministically.
    #[test]
    fn a_failed_removal_is_reported() {
        // A stale blocker from an interrupted run would break `library_with`
        // (`create_dir_all` on a file), so clear it before building.
        let stale = std::env::temp_dir().join(format!("gh-cli-u3-fail-{}", std::process::id()));
        let _ = std::fs::remove_file(&stale);
        let (root, library) = library_with("u3-fail", &[("g1", "Hades")]);
        let mut shell = Shell::new();
        shell.state.library = library;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::write(&root, b"a file, not a directory").unwrap();

        let _ = shell.update(Message::ConfirmDeleteGame("g1".to_string()));
        let _ = shell.update(Message::DeleteGameConfirmed("g1".to_string()));

        assert!(
            shell.state.library.get("g1").is_none(),
            "the entry leaves memory even when the save fails: delete-before-save \
             with no rollback is the reference's order (`models.py:173-176`)"
        );
        assert!(
            format!("{:?}", shell.state.toasts).contains("Could not remove \u{201c}Hades\u{201d}"),
            "the toast must name the game the disk refused to forget"
        );
        assert!(
            shell.state.confirm_delete.is_none(),
            "the dialog must close once it has answered, even on failure"
        );
        std::fs::remove_file(&root).unwrap();
    }

    /// The text the app would announce, for a shell that has just raised
    /// something.
    ///
    /// Panics rather than returning an `Option`, because UX-14's whole claim is
    /// that a message *is* announced: a helper that let the caller decide what to
    /// do about `None` would let every test below pass on an app that announces
    /// nothing.
    fn announced(shell: &Shell) -> String {
        shell
            .state
            .notice
            .clone()
            .expect("UX-14: a message reached the toaster and nothing reached the live region")
    }

    /// The toasts' own `Debug` rendering.
    ///
    /// The only route to what is inside them: `Toast`'s `message` and `duration`
    /// are private with no getters and `Toasts` exposes neither its `SlotMap` nor
    /// its queue (`libcosmic src/widget/toaster/mod.rs:114-160`). Existing tests
    /// in this file already read the store this way — `notifying_pushes_a_toast`
    /// asserts on `num_elems` — so this is the same instrument, not a new one.
    /// What it reads is the stored toast's own field, which is why a reverted
    /// duration shows up here as `duration: Short`.
    fn toast_store(shell: &Shell) -> String {
        format!("{:?}", shell.state.toasts)
    }

    /// **UX-14: a message is announced to assistive technology, and the toast
    /// carrying it lasts long enough to be read.**
    ///
    /// Both halves of the finding in one measurement, because either alone is a
    /// fix that does not fix it: a message that is announced for five seconds is
    /// announced to a user who is still reading it, and a fifteen-second toast
    /// nobody announces is still invisible.
    ///
    /// `Notify` is the route every failure path in `bridge.py` ends at
    /// (`Message::Notify`'s own doc), so it is the funnel the forty-odd report
    /// sites take; the two hand-written routes are covered by the two tests
    /// below.
    #[test]
    fn a_toast_is_announced_and_lasts_long_enough_to_be_read() {
        let mut shell = Shell::new();
        assert!(
            shell.state.notice.is_none(),
            "a shell that has said nothing must have nothing to announce"
        );

        let effect = observe(&mut shell, Message::Notify("the launch failed".to_string()));

        assert_eq!(
            announced(&shell),
            "the launch failed",
            "the live region must carry the message the toast carries, word for \
             word — a second sentence here would be a second thing the app says"
        );
        let toasts = toast_store(&shell);
        assert!(
            toasts.contains("the launch failed"),
            "the announcement and the toast must be the same message; toasts: \
             {toasts}"
        );
        assert!(
            toasts.contains("duration: Long"),
            "the toolkit's default is `Duration::Short`, 5000 ms \
             (`src/widget/toaster/mod.rs:69-86`), which is shorter than a \
             sentence takes to read and shorter than a reader takes to reach the \
             window; `Long` is 15 000 ms. Stored toasts: {toasts}"
        );
        assert!(
            !toasts.contains("duration: Short"),
            "one toast cannot be both: {toasts}"
        );
        assert!(
            effect.task_units > 0,
            "`Toasts::push` returns the task that schedules the expiry, and this \
             route must still return it"
        );
    }

    /// **The one toast the app pushes by hand is announced too.**
    ///
    /// `Message::EasyInstallFinished` cannot go through `State::toast_task`: its
    /// toast carries a "Play" action built from the game id, so it needs
    /// `Toasts::push`'s builder rather than the generic one. That is exactly the
    /// site a fix written only into `toast_task` would miss — the installed
    /// notice is the one the user must act on — so it is asserted separately
    /// rather than assumed from the test above.
    #[test]
    fn the_hand_pushed_installed_toast_is_announced_too() {
        let mut shell = shell_for_installs();

        let _ = observe(
            &mut shell,
            Message::EasyInstallFinished {
                game_id: "install-6".to_string(),
                message: "Installed “Steam”".to_string(),
            },
        );

        assert_eq!(
            announced(&shell),
            "Installed “Steam”",
            "this toast has its own `push` call, so it has its own announcement \
             to forget"
        );
        assert!(
            toast_store(&shell).contains("duration: Long"),
            "and its own duration: {}",
            toast_store(&shell)
        );
    }

    /// **The install flow's two other notices are announced**, so the fix is a
    /// funnel rather than three edits that happen to agree today.
    ///
    /// `InstallPlugin` and `PluginInstallFinished` were two separate
    /// `toasts.push` calls before UX-14 and now both go through
    /// `State::toast_task`. This drives the first through the real dispatcher;
    /// the id is read from the catalogue rather than written here, and the
    /// `assert!` on `install_plan` is what stops the test passing vacuously on
    /// an id no plugin answers to.
    #[test]
    fn the_plugin_notices_are_announced() {
        // `install_plan` is the guard the `update` arm itself uses, so an id it
        // does not know is an arm that pushes nothing — and a test written
        // against such an id would assert nothing while looking like it did.
        let (expected, _) = crate::view::plugins::install_plan("mangohud")
            .expect("the sample id must name a plugin in the catalogue");

        let mut shell = Shell::new();
        let _ = observe(&mut shell, Message::InstallPlugin("mangohud".to_string()));

        assert_eq!(announced(&shell), expected);
        assert!(
            toast_store(&shell).contains(&expected),
            "toasts: {}",
            toast_store(&shell)
        );
        assert!(
            toast_store(&shell).contains("duration: Long"),
            "toasts: {}",
            toast_store(&shell)
        );
    }

    /// **The app's own root publishes the last message as an assertive alert.**
    ///
    /// The row's verification is "verify the node list from a built toaster", and
    /// this is that: [`Shell::view_root`] is the element `Application::view`
    /// returns, so the nodes read off it are the nodes the app publishes, with
    /// the toolkit's real `Toaster` in the middle.
    ///
    /// What it finds is one node, and that is the finding rather than a mistake
    /// in the test: libcosmic's `Toaster` implements no `a11y_nodes` at all, so
    /// the whole body under it publishes nothing and the notice lands as the
    /// tree's root (`the_toolkits_toaster_publishes_no_node_at_all` measures
    /// that directly, on a toaster with no app around it). The app cannot repair
    /// it from outside — see the residue on `view::a11y::LiveNotice`.
    #[test]
    fn the_apps_root_publishes_the_last_message_as_an_assertive_alert() {
        use iced_accessibility::accesskit::{Live, Role};

        let mut shell = Shell::new();
        let _ = observe(&mut shell, Message::Notify("the launch failed".to_string()));

        let mut root = shell.view_root();
        let nodes = crate::view::a11y::harness::published(&mut root);

        let notice = nodes
            .iter()
            .find(|node| {
                node.id
                    == iced_accessibility::A11yId::Widget(cosmic::iced::advanced::widget::Id::from(
                        view::a11y::NOTICE_ID,
                    ))
            })
            .unwrap_or_else(|| {
                panic!(
                    "the app publishes no node for the live region, so a screen \
                     reader is told nothing: {nodes:#?}"
                )
            });

        assert_eq!(notice.role, Role::Alert);
        assert_eq!(notice.label.as_deref(), Some("the launch failed"));
        assert_eq!(notice.live, Some(Live::Assertive));
        assert_eq!(
            nodes.len(),
            1,
            "libcosmic's `Toaster` publishes nothing, so everything under it is \
             swallowed and the notice is the entire tree; if this count has \
             grown, the toolkit grew an `a11y_nodes` and the residue recorded on \
             `view::a11y::LiveNotice` can be narrowed: {nodes:#?}"
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
        stub.insert()
            .text(Page::Library.label())
            .data(Page::Library);
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
        assert_eq!(
            shell.state.page,
            Page::Library,
            "the shell starts on Library"
        );

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

    /// **Arriving at the Plugins page re-detects the host; an empty row list
    /// does not stay empty.**
    ///
    /// `BUG-08` is a *missing producer*: `RefreshPlugins` had a handler and the
    /// page had a Refresh control, but nothing emitted it on arrival, and
    /// `showPage` re-detects the host (`Main.qml:41-44`). So the one workflow the
    /// reference designed this for — install MangoHud from a terminal, then click
    /// Plugins — left the row saying "Install" for the rest of the session.
    ///
    /// The observable is the state the page draws, not the returned task: this
    /// arm's task is `Task::none()`, so a test that asserted on `task_units` would
    /// pass on a handler that did nothing at all. The rows are *emptied* first for
    /// the reason `shell_with_work_to_do` empties them — a shell that already had
    /// them would make this a write of what was already there and hide a missing
    /// refresh behind an unchanged comparison.
    #[test]
    fn arriving_at_the_plugins_page_re_detects_the_host() {
        let mut shell = Shell::new();
        shell.state.plugins.clear();
        shell.state.plugins_intro.clear();
        // A page the shell is not on, so the arrival below is a real one.
        shell.state.page = Page::Library;

        let _ = shell.show_page(Page::Plugins);

        assert_eq!(
            shell.state.plugins.len(),
            gamehandler_core::plugins::PLUGINS.len(),
            "arriving at Plugins did not re-detect the host, so a helper \
             installed outside the app stays invisible for the session"
        );
        assert!(
            !shell.state.plugins_intro.is_empty(),
            "the intro sentence is part of the same refresh and was left empty"
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
            let drawn = crate::view::testkit::drawn_strings(shell.view_body());

            let says_pending = drawn
                .iter()
                .any(|text| text.contains("has not been ported yet"));

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

    /// The strings a real element hands the operation traversal, **each with the
    /// rectangle the framework laid it out in at a stated window size**.
    ///
    /// [`drawn_strings`] is the same walk under [`layout::Limits::MAX`], which is
    /// an infinite window. That is right for every question it was written for —
    /// which strings are drawn — and wrong for a layout one: a control that
    /// overflows a 420 px window is handed all the room it wants and measures as
    /// fitting, so an assertion that something "fits" made through it cannot
    /// fail. `UX-06` is a layout finding — *where* a click lands — so the window
    /// size is named here and the bounds are the framework's own.
    fn laid_out_texts<M: Clone + 'static>(
        element: &mut cosmic::Element<'_, M>,
        window: cosmic::iced::Size,
    ) -> Vec<(String, cosmic::iced::Rectangle)> {
        use cosmic::iced::advanced::widget::Operation;
        use cosmic::iced::advanced::{Layout, layout::Limits};

        #[derive(Default)]
        struct Texts(Vec<(String, cosmic::iced::Rectangle)>);
        impl Operation for Texts {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn text(
                &mut self,
                _id: Option<&cosmic::widget::Id>,
                bounds: cosmic::iced::Rectangle,
                text: &str,
            ) {
                self.0.push((text.to_string(), bounds));
            }
        }

        let (mut tree, _) = crate::view::a11y::harness::built(element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &crate::view::a11y::harness::renderer(),
            &Limits::new(cosmic::iced::Size::ZERO, window),
        );
        let mut texts = Texts::default();
        element.as_widget_mut().operate(
            &mut tree,
            Layout::new(&node),
            &crate::view::a11y::harness::renderer(),
            &mut texts,
        );
        texts.0
    }

    /// A left click — press *and* release — at `position`, and every message the
    /// tree published while handling it.
    ///
    /// # Why this exists rather than a call into the harness
    ///
    /// [`crate::view::a11y::harness`] dispatches events with
    /// [`mouse::Cursor::Unavailable`], which is right for its own questions (what
    /// a page publishes, which nodes carry an id) and cannot click anything: a
    /// widget's hit test is `cursor.position_over(bounds)`, and a cursor with no
    /// position is over nothing. `UX-06` is precisely the question of which
    /// widget a click reaches, so it needs a cursor at a real point.
    ///
    /// The tree, the layout and the renderer are the real ones for the same
    /// reason: a click asserted against a layout nobody drew is a claim about
    /// arithmetic rather than about the page.
    fn click_at<M: Clone + 'static>(
        element: &mut cosmic::Element<'_, M>,
        window: cosmic::iced::Size,
        position: cosmic::iced::Point,
    ) -> (Vec<M>, bool) {
        use cosmic::iced::advanced::widget::Tree;
        use cosmic::iced::advanced::{Layout, layout::Limits};
        use cosmic::iced::mouse;

        let renderer = crate::view::a11y::harness::renderer();
        let mut tree = Tree::new(element.as_widget());
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(cosmic::iced::Size::ZERO, window),
        );
        let mut messages: Vec<M> = Vec::new();
        let mut captured = false;
        let mut clipboard = cosmic::iced::advanced::clipboard::Null;
        for event in [
            cosmic::iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            cosmic::iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ] {
            let mut shell = cosmic::iced::advanced::Shell::new(&mut messages);
            element.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                mouse::Cursor::Available(position),
                &renderer,
                &mut clipboard,
                &mut shell,
                &cosmic::iced::Rectangle::INFINITE,
            );
            captured |= shell.is_event_captured();
        }
        (messages, captured)
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
        let text = gamehandler_core::oracle_support::repo_file("gamehandler/qml/Main.qml");

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

    /// Every `notify.emit` in `bridge.py`, and the port fragment that voices
    /// it: P-69's audit, kept as data rather than prose.
    ///
    /// The bridge fragment is a distinctive core of the call (not the whole
    /// sentence: `{game.name}` reads `{name}` here, and pinning whole
    /// sentences would turn every rewording into a row edit). The port
    /// fragment is the sentence's spelling under `crates/`. Two rows voice a
    /// mechanism rather than a sentence: the `_async` default fail (161),
    /// which every `Err` reply arm embodies by toasting the raw message, and
    /// the prefix tool's bare `str(exc)` (499), which its reply arm passes
    /// through untouched.
    ///
    /// **The (161) row's needle is a spelling, and it went stale once.** It read
    /// `toast_task(result.unwrap_err())`, which was how the two cover arms
    /// happened to be written; `UX-15` gave those two arms a context prefix, so
    /// that exact text stopped existing and this test went red — correctly, and
    /// for the wrong reason: what the row asserts is that *some* port path
    /// surfaces a raw error verbatim, which is still true and is what the pass
    /// samples at `state.rs:2500`. Anchoring a rule to a spelling that one
    /// reviewer's style happened to produce is the fragility, so the needle is
    /// the more common production form. Both readings have a cost: the old
    /// needle was a stronger claim about those two arms specifically, the new
    /// one is a weaker claim about the mechanism and does not fail when a single
    /// arm stops doing it.
    const NOTIFY_VOICES: [(&str, &str); 39] = [
        (
            "notify.emit(message)",
            "Err(message) => self.state.toast_task(message)",
        ),
        ("A game needs a name", "A game needs a name"),
        ("Added “{game.name}”", "Added “{name}”"),
        ("Updated “{game.name}”", "Updated “{name}”"),
        ("Removed “{game.name}”", "Removed “{name}”"),
        ("Select a game first", "Select a game first"),
        ("Could not launch “{game.name}”", "Could not launch “{}”"),
        ("Launching “{game.name}”…", "Launching “{name}”…"),
        (
            "stopped right away: {reason}",
            "stopped right away: {reason}",
        ),
        (
            "Prefix tools are only available for Windows games",
            "Prefix tools are only available for Windows games",
        ),
        ("notify.emit(str(exc))", "Err(message) => message"),
        ("Opening {tool} for", "Opening {} for"),
        (
            "Linux games do not use a Wine prefix",
            "Linux games do not use a Wine prefix",
        ),
        (
            "Could not open the prefix folder",
            "Could not open the prefix folder",
        ),
        (
            "Could not create the shortcut",
            "Could not create the shortcut",
        ),
        ("Shortcut created at {path}", "Shortcut created at {}"),
        ("Cover set from {hit.origin_label}", "Cover set from {}"),
        ("Enter a game name first", "Enter a game name first"),
        ("Looking for artwork for", "Looking for artwork for"),
        ("Cover found via {hit.origin_label}", "Cover found via {}"),
        ("Could not copy cover", "Could not copy cover"),
        ("Custom cover added", "Custom cover added"),
        ("Downloading {release.tag}", "Downloading {tag}…"),
        (
            "Installed {release.tag}. You can now choose",
            "Installed {tag}. You can now choose",
        ),
        ("Failed to install {release.tag}", "Failed to install {tag}"),
        (
            "Could not remove {runner_id}",
            "Could not remove {runner_id}",
        ),
        ("Removed {runner_id}", "Removed {runner_id}"),
        (
            "Another install is already running",
            "Another install is already running",
        ),
        (
            "is not available. Download a runner first",
            "is not available. Download a runner first",
        ),
        (
            "Could not create a prefix for",
            "Could not create a prefix for",
        ),
        ("Downloading {installer.name}", "Downloading {}…"),
        (
            "Launching the {installer.name} installer",
            "Launching the {} installer",
        ),
        (
            "Could not find the {installer.name} executable",
            "Could not find the {} executable",
        ),
        (
            "Could not install {installer.name}",
            "Could not install {}: {message}",
        ),
        (
            "Kept the {pending['installer'].name} prefix",
            "Kept the {} prefix",
        ),
        ("Installing {plugin.name}", "Installing {name}…"),
        ("{plugin.name} is installed", "{name} is installed"),
        (
            "did not install. The command is shown",
            "did not install. The command is shown",
        ),
        (
            "Could not install {plugin.name}",
            "Could not install {name}: {error}",
        ),
    ];

    /// The `notify.emit(` call sites of `bridge.py`, each call as one string.
    /// Paren-balanced across lines, because five of the calls span several;
    /// none of them nests a paren inside a string, which the length bound
    /// below would catch by swallowing the file.
    fn notify_calls(bridge: &str) -> Vec<String> {
        let mut calls = Vec::new();
        let mut rest = bridge;
        while let Some(at) = rest.find("notify.emit(") {
            let mut depth = 0;
            let mut end = None;
            for (i, ch) in rest[at..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(at + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end.expect("an unterminated notify.emit call");
            calls.push(rest[at..end].to_string());
            rest = &rest[end..];
        }
        calls
    }

    /// `source` as chars, with every comment blanked to spaces and — when
    /// `blank_strings` — every string and char literal with it. Offsets are
    /// preserved, so a range found in one pass indexes the other; the two-pass
    /// use is [`without_test_code`].
    ///
    /// The same lexer as `crate::view::form.rs`'s, and for the same reason: the
    /// cut below is a *brace* walk, and a `{` or `}` inside a string literal or
    /// a comment is not a brace. `char_literal_at`'s rule about lifetimes is
    /// load-bearing here too — every view signature in this crate is
    /// `Element<'a, Message>`, and a lexer that reads that `'` as a literal
    /// opener blanks the rest of the line and silently loses whatever was on it.
    fn lex(source: &str, blank_strings: bool) -> Vec<char> {
        let chars: Vec<char> = source.chars().collect();
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
    /// Transcribed from `crate::view::form.rs`, which is where the rule and its
    /// measured failure were worked out; the note there is the record.
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

    /// The first `needle` at or after `from`.
    fn find_chars(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (from..=haystack.len() - needle.len())
            .find(|start| haystack[*start..*start + needle.len()] == *needle)
    }

    /// The index of the brace that closes the one at `open`.
    ///
    /// All three bracket kinds share a depth, as `view/form.rs`'s `matching`
    /// does: a `)` inside a `[...]` or a `{...}` must not close an earlier
    /// `(`. Strings and comments are not consulted — the caller passes text
    /// that [`lex`] has already handled.
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

    /// The ranges of this file's **test code**, as `(start, end)` char indices:
    /// every `#[cfg(test)]` module body, and every `#[test]` item.
    ///
    /// Two markers, because this tree spells test code both ways.
    /// `#[cfg(test)] mod tests { … }` is the common one, and
    /// `crates/core/src/hash.rs:106` is a bare `#[test] fn` with no attribute
    /// covering it — `#[test]` carries its own `cfg(test)` for the item it is
    /// on, so that file is test code from that line down and the module rule
    /// never sees it. Missing it is not cosmetic: the assertion in
    /// `every_reference_notify_has_a_port_voice` requires the haystack to
    /// contain no `#[test]`, and it is what fails when a spelling goes
    /// unhandled — which is how `pub(crate) mod tests` and this one were both
    /// found.
    fn test_code_ranges(chars: &[char]) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        for (marker, opener) in [
            (
                "#[cfg(test)]",
                module_body_open as fn(&[char], usize) -> Option<usize>,
            ),
            ("#[test]", braced_item_open),
        ] {
            let marker: Vec<char> = marker.chars().collect();
            let mut from = 0usize;
            while let Some(start) = find_chars(chars, &marker, from) {
                let after = start + marker.len();
                match opener(chars, after) {
                    Some(open) => {
                        let end = matching(chars, open).unwrap_or(chars.len());
                        ranges.push((start, end));
                        from = end;
                    }
                    None => from = after,
                }
            }
        }
        ranges.sort_unstable();
        ranges
    }

    /// The index of the `{` that opens the body of the module declared at or
    /// after `from`, or `None` when the item there is not a module with a braced
    /// body.
    ///
    /// The **visibility is skipped rather than assumed absent**, and that is the
    /// one place this differs from `view/form.rs`'s function of the same name,
    /// which checks the three characters after the attribute against `mod`.
    /// `crates/core/src/paths.rs:214` is `pub(crate) mod tests {` — measured, the
    /// copied rule reads that as not-a-module and leaves the whole module in the
    /// haystack, which is the same defect one spelling over. A `;`-declared
    /// module (`#[cfg(test)] mod oracle_tests;`) returns `None` too: its body is
    /// another file, `test_only_modules` is what removes it, and cutting at the
    /// next `{` in this file would swallow whatever item followed.
    fn module_body_open(chars: &[char], from: usize) -> Option<usize> {
        let mut at = from;
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        for keyword in ["pub(crate) mod ", "pub(super) mod ", "pub mod ", "mod "] {
            if !starts_with_at(chars, at, keyword) {
                continue;
            }
            let mut scan = at + keyword.chars().count();
            while scan < chars.len()
                && chars[scan] != '{'
                && chars[scan] != ';'
                && chars[scan] != '\n'
            {
                scan += 1;
            }
            return (chars.get(scan) == Some(&'{')).then_some(scan);
        }
        None
    }

    /// The index of the `{` that opens the first braced item at or after `from`
    /// — the body of a `#[test] fn`.
    ///
    /// The `;` guard is what keeps it from reaching past a test item into
    /// whatever the next braced item happens to be: `#[test]` is always on a
    /// function here, and a `;` before any `{` means this attribute is not on
    /// one and nothing should be cut.
    fn braced_item_open(chars: &[char], from: usize) -> Option<usize> {
        let mut at = from;
        while at < chars.len() {
            match chars[at] {
                '{' => return Some(at),
                ';' => return None,
                _ => at += 1,
            }
        }
        None
    }

    /// Whether `needle` begins at char index `at`.
    fn starts_with_at(chars: &[char], at: usize, needle: &str) -> bool {
        let needle: Vec<char> = needle.chars().collect();
        chars.len() >= at + needle.len() && chars[at..at + needle.len()] == needle[..]
    }

    /// One file's **production** text: its `#[cfg(test)] mod` blocks blanked.
    ///
    /// This is the repair for BUG-18. The guard below asks
    /// `port.contains(voice)` for every notice the reference emits, and
    /// `crates/app/src/main.rs` was in the haystack *with this test module in
    /// it* — whose `NOTIFY_VOICES` table is a list of exactly those voices. So
    /// the table satisfied every lookup for itself, and deleting the port of any
    /// notice left the assertion green. A haystack that contains its own needle
    /// is not a check. `crates/app/tests/dispatch_coverage.rs`'s `production_src`
    /// makes the same cut for the same reason, and states it as this project's
    /// dominant defect class.
    ///
    /// Two passes over one lexer, as `view/form.rs` does it: the *structural*
    /// pass blanks strings too, because the braces must be counted without the
    /// ones inside literals; the *readable* pass keeps them, because the voices
    /// **are** string literals and a haystack with the strings blanked would
    /// find nothing at all. Both are the same length, so a range computed on one
    /// indexes the other.
    fn without_test_code(source: &str) -> String {
        let structural = lex(source, true);
        let readable = lex(source, false);
        let mut readable = readable;
        for (start, end) in test_code_ranges(&structural) {
            for character in &mut readable[start..end] {
                if *character != '\n' {
                    *character = ' ';
                }
            }
        }
        readable.into_iter().collect()
    }

    /// The module names `dir`'s own module file declares under `#[cfg(test)]`
    /// with a `;` — files that are test code from their first line, wherever the
    /// file itself carries no attribute.
    ///
    /// `crates/core/src/oracle_tests.rs` is the live case: `lib.rs:56-57` says
    /// `#[cfg(test)] mod oracle_tests;`, so the file is reached through `mod`
    /// and never through an attribute of its own. Without this it would be
    /// walked like production code — its 31 `#[test]`s and all their fixture
    /// strings back in the haystack, which is the same defect one file over.
    ///
    /// Only the two files that declare modules in this tree are read
    /// (`lib.rs`/`main.rs`/`mod.rs`); a module declared from anywhere else would
    /// go unnoticed, which is named rather than implied.
    fn test_only_modules(dir: &std::path::Path) -> Vec<String> {
        let mut names = Vec::new();
        for candidate in ["mod.rs", "lib.rs", "main.rs"] {
            let path = dir.join(candidate);
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{} should be readable: {err}", path.display()));
            let chars = lex(&text, true);
            let marker: Vec<char> = "#[cfg(test)]".chars().collect();
            let mut from = 0usize;
            while let Some(start) = find_chars(&chars, &marker, from) {
                let after = start + marker.len();
                let rest: String = chars[after..].iter().take(64).collect();
                if let Some(rest) = rest.trim_start().strip_prefix("mod ") {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    // `;` and not `{`: a braced module is cut by
                    // `test_module_ranges`, a `;` one lives in another file.
                    if !name.is_empty() && rest[name.len()..].trim_start().starts_with(';') {
                        names.push(name);
                    }
                }
                from = after;
            }
        }
        names
    }

    /// Every Rust source file under `crates/` that holds **production** code,
    /// concatenated: the haystack the port fragments are read out of.
    fn read_crates(root: &std::path::Path) -> String {
        fn visit(dir: &std::path::Path, out: &mut String) {
            let test_only = test_only_modules(dir);
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|err| panic!("{} should be readable: {err}", dir.display()));
            for entry in entries {
                let path = entry
                    .unwrap_or_else(|err| panic!("a dir entry should be readable: {err}"))
                    .path();
                if path.is_dir() {
                    visit(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let stem = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or_default();
                    if test_only.iter().any(|name| name == stem) {
                        continue;
                    }
                    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| {
                        panic!("{} should be readable: {err}", path.display())
                    });
                    out.push_str(&without_test_code(&text));
                    out.push('\n');
                }
            }
        }
        let mut out = String::new();
        visit(&root.join("crates/app/src"), &mut out);
        visit(&root.join("crates/core/src"), &mut out);
        out
    }

    /// **Every `notify.emit` in `bridge.py` has a voice in this tree.**
    ///
    /// Three directions, so the mapping fails loudly in each: a call no row
    /// voices is a reference notice the port never says (port it and add the
    /// row); a row matching no call is dead (the reference moved on, delete
    /// it); a port fragment found nowhere is a regression (the sentence left
    /// the tree). The call count itself is pinned: an upstream addition or
    /// removal changes it, and the failure names the call that no longer
    /// maps.
    ///
    /// What a row claims is "this notice is voiced *here*"; the arm tests
    /// claim it is voiced *right*. A row's port fragment surviving only in a
    /// comment while the arm is deleted would pass here — and fail the arm
    /// test that asserts the toast — which is why the two halves stay
    /// separate.
    ///
    /// **The haystack is production code only, and that is asserted** (BUG-18).
    /// It used to be `read_crates`' raw concatenation, which included this very
    /// test module — so `port.contains(voice)` was satisfied by the
    /// `NOTIFY_VOICES` table above, and deleting the port of any notice left
    /// this green. The two assertions after `port` are the instrument checking
    /// itself: the table and the test attributes must both be *absent* from what
    /// it searches, or the search proves nothing. They are not decoration — a
    /// cut that stops one item early puts the needle straight back, and this is
    /// what fails when it does.
    #[test]
    fn every_reference_notify_has_a_port_voice() {
        // `root` stays for `read_crates`, which walks the *source tree* rather
        // than reading one reference file; the `bridge.py` read goes through the
        // shared helper like every other reference read (`ARCH-15`).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bridge = gamehandler_core::oracle_support::repo_file("gamehandler/bridge.py");
        let calls = notify_calls(&bridge);
        assert!(
            calls.iter().all(|call| call.len() < 500),
            "a call swallowed the file: the paren walk met a paren inside a string"
        );
        assert_eq!(
            calls.len(),
            NOTIFY_VOICES.len(),
            "bridge.py gained or lost a notify.emit; the table maps {} — add or delete the \
             row. Calls: {calls:?}",
            NOTIFY_VOICES.len()
        );
        let port = read_crates(&root);

        // The instrument, checked before it is used. `NOTIFY_VOICES` is the
        // needle: it is a list of every voice, so a haystack that still holds
        // it satisfies every lookup below by itself. A `#[test]` in the
        // haystack means the cut did not reach a test module — this file's own
        // ends the file, so one that survives it is a *second* module the walk
        // missed. Both are hard errors rather than warnings: an assertion whose
        // haystack contains its needle cannot fail, which is worse than no
        // assertion at all.
        assert!(
            !port.contains("NOTIFY_VOICES"),
            "read_crates' haystack still contains this test module's own NOTIFY_VOICES \
             table, so every `port.contains(voice)` below is satisfied by the table its \
             own rows are listed in and cannot fail (BUG-18). The `#[cfg(test)] mod` cut \
             in `without_test_code` did not reach it."
        );
        assert!(
            !port.contains("#[test]"),
            "read_crates' haystack still contains a `#[test]` attribute, so a test \
             module survived the cut — and the strings those tests assert on are in the \
             haystack the production lookup below is made against, which is the defect \
             per file rather than per module (BUG-18). Check `test_only_modules`, which \
             is what removes a file reached through `#[cfg(test)] mod name;`."
        );

        for (site, voice) in NOTIFY_VOICES {
            assert!(
                calls.iter().any(|call| call.contains(site)),
                "no notify.emit contains {site:?}: the row is dead, delete it"
            );
            assert!(
                port.contains(voice),
                "nothing under crates/ says {voice:?}: the voice for {site:?} left the tree"
            );
        }
        for call in &calls {
            assert!(
                NOTIFY_VOICES.iter().any(|(site, _)| call.contains(site)),
                "no row voices this notify.emit — port it and add the row: {call:?}"
            );
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
        let text = gamehandler_core::oracle_support::repo_file("gamehandler/qml/Main.qml");

        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("icon.name:"))
            .filter_map(|rest| rest.trim().strip_prefix('"'))
            .filter_map(|rest| rest.split_once('"').map(|(name, _)| name))
            .collect();

        let ours: Vec<&str> = Page::ALL
            .iter()
            .map(|page| page_icon(*page).legacy_name())
            .collect();
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

    /// The shared detached spawn reports the failure that means the helper never
    /// started, and does so with a real `io::Error` rather than a string the
    /// test chose.
    ///
    /// This is the assertion `BUG-04` was missing: before it, the prefix
    /// folder's spawn discarded its `Result` outright, and the doc comment that
    /// described this area named a test that had never been written — the same
    /// defect class as a check that passes without looking.
    #[test]
    fn a_detached_spawn_that_cannot_start_says_so() {
        let mut missing = std::process::Command::new("/nonexistent/xdg-open-for-test");
        let error = spawn_detached(&mut missing, "the prefix folder")
            .expect_err("a program that does not exist cannot be spawned");
        assert!(
            error.starts_with("Could not open the prefix folder: "),
            "the message must name what failed: {error:?}"
        );
        // And it carries the OS's own reason, not a placeholder.
        assert_eq!(
            error,
            "Could not open the prefix folder: No such file or directory (os error 2)"
        );
    }

    /// **SEC-07: the gate refuses every scheme but `https`, and the app's own
    /// links are all `https`.**
    ///
    /// Both halves matter. The refusals are what makes the sink local — a
    /// `file://` URL reaching this point must not reach `xdg-open`, whatever
    /// produced it. The acceptances are the control: a predicate that refused
    /// *everything* would satisfy every refusal here while breaking every link
    /// in the application, so the last block drives the real catalogues.
    #[test]
    fn only_an_https_url_may_be_handed_to_the_desktop() {
        for refused in [
            // The scheme the row names, and the one that would open a local
            // file or directory in whatever handler claims it.
            "file:///etc/passwd",
            "file:///home",
            // Plaintext, and no scheme at all.
            "http://example.invalid/",
            "example.invalid/x",
            // A scheme that has no business being dispatched, and the
            // empty-string edge a naive `starts_with` would let through.
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "",
            // Uppercase: RFC 3986 makes schemes case-insensitive, so this is a
            // real URL this gate refuses. Deliberate — every producer is a
            // lowercase constant, and the doc says so.
            "HTTPS://example.invalid/",
            // The scheme with nothing after it, which is not a URL.
            "https://",
        ] {
            assert!(
                !is_openable_url(refused),
                "{refused:?} must not be handed to xdg-open: it dispatches on \
                 the scheme, so this is what chooses the program"
            );
        }

        // The control. Every URL the application can actually produce, read
        // from the catalogues rather than written here, so this fails if a
        // future constant introduces a scheme the gate refuses.
        let mut seen = 0usize;
        for section in crate::view::credits::credit_sections() {
            for credit in section.entries {
                assert!(
                    is_openable_url(credit.url),
                    "the credit catalogue's {credit_url:?} would be refused by \
                     this gate, so its link is dead",
                    credit_url = credit.url
                );
                seen += 1;
            }
        }
        for family in gamehandler_core::runners::families::RUNNER_FAMILIES {
            for url in [family.releases_url(), family.homepage()] {
                assert!(
                    is_openable_url(&url),
                    "the {family_id} family's {url:?} would be refused by this \
                     gate, so its link is dead",
                    family_id = family.id
                );
                seen += 1;
            }
        }
        assert!(
            seen > 0,
            "no URL was checked, so the acceptances above proved nothing — the \
             catalogues this reads must not be empty"
        );
    }

    /// The gate runs **before** the command exists, which is the ordering the
    /// row is about.
    ///
    /// A refusal that happened after `spawn` would be a report about something
    /// that already ran. The two outcomes are distinguishable from outside
    /// without inspecting internals: a spawn of `xdg-open` either succeeds
    /// (`Ok(())`) or fails with the OS's reason, and neither of those starts
    /// with the refusal's own sentence. So `Err` carrying that sentence is only
    /// reachable when `open_url_command` was never called.
    #[test]
    fn a_refused_scheme_never_reaches_the_spawn() {
        let error =
            open_url("file:///etc/passwd").expect_err("a file:// URL must be refused, not opened");
        assert!(
            error.starts_with("Refused to open a link that is not https: "),
            "the failure must be the gate's refusal and not a spawn failure, \
             which is what proves nothing was spawned: {error:?}"
        );
        // It names the scheme rather than echoing the URL, which is the
        // untrusted half. `file` here, not the path.
        assert!(
            error.ends_with("file"),
            "the refusal names the scheme it refused: {error:?}"
        );
        assert!(
            !error.contains("passwd"),
            "the refusal must not echo the URL it refused: {error:?}"
        );
    }
}
