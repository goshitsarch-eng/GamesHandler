//! The easy-install flow (`bridge.py`'s `installEasy` and the wizard behind it).
//!
//! Split out of `main.rs` under audit row **ARCH-11**, which observed that the
//! file was four files sharing one scope. This half was the clearest case: it
//! is a self-contained flow — pick an installer, download it, verify it, run
//! it, locate the executable it installed, fetch a cover, save the game — and
//! its only contact with the rest of the app is [`Message`] and [`State`].
//!
//! Nothing here changed in the move except its path. The line numbers this
//! module's doc comments cite are the *Python* application's, which is the
//! reference this is a port of, and they were never `main.rs`'s.
//!
//! The region deliberately stays one module rather than being cut further: the
//! steps above share a working set (the install record, the download timeout,
//! the token that identifies a run) and threading that through three modules
//! would add a boundary the reference does not have.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc::{UnboundedSender, unbounded as unbounded_channel};
use cosmic::widget::toaster::ToastId;
use gamehandler_core::installers::{
    Installer, SystemClock, build_installer_command, download_installer, game_from_install,
    installer_by_id, prepare_prefix, wait_for_installer, wait_for_prefix_idle,
};
use gamehandler_core::netpaths::as_local_path;
use gamehandler_core::paths;
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};

use crate::state::{CoverHit, State};

use super::Message;

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
pub(crate) fn start_easy_install(
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
            let text = format!(
                "{} is not available. Download a runner first.",
                runner.name()
            );
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
    let command = match build_installer_command(&*runner, &prefix, installer, &archive, &launch_env)
    {
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
        &|runner, env, seconds| wait_for_prefix_idle(runner, env, seconds, &SystemLaunchEnv),
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
/// The reference's `locateDialog` (`Main.qml:182-189`), as a portal chooser
/// task: `easyInstallNeedsExe`'s `FileDialog` with its two `nameFilters`,
/// accepted into [`Message::CompleteEasyInstall`] and rejected into
/// [`Message::CancelEasyInstall`].
///
/// Opened by [`easy_install_wizard_finished`]'s not-found branch, which is
/// what makes the busy state it leaves behind escapable: before this task
/// existed nothing produced either message, so a wizard that closed without
/// installing wedged the page forever (P-57).
///
/// # The start folder is not ported
///
/// `onEasyInstallNeedsExe` points the dialog at the prefix's `drive_c`
/// (`Main.qml:166`), and libcosmic's `open::Dialog` carries a `directory`
/// field for exactly that — but marks it `dead_code` because ashpd does not
/// expose it yet, and the portal request builder never sends it. So the
/// chooser opens wherever the portal opens, and this function does not take a
/// start folder it would silently drop.
///
/// # What is tested, and what is read
///
/// The filters ([`exe_file_filters`]) and the answer mapping
/// ([`locate_message`]) are pure and pinned below. The assembly — the title,
/// the `open_file` call itself — is read, not tested: `Dialog` keeps its
/// fields private and driving the portal needs a session bus. That is the
/// same wall `theme::apply`'s call sites stand behind, and it is named for
/// the same reason.
pub(crate) fn locate_exe_task(token: String, installer_name: String) -> cosmic::app::Task<Message> {
    use cosmic::dialog::file_chooser::open;
    cosmic::app::Task::perform(
        async move {
            let filters = exe_file_filters();
            let mut dialog = open::Dialog::new()
                .title(format!("Locate {installer_name}"))
                .current_filter(filters[0].clone());
            for filter in filters {
                dialog = dialog.filter(filter);
            }
            let answer = dialog
                .open_file()
                .await
                .map(|response| response.url().clone());
            locate_message(token, answer)
        },
        cosmic::Action::App,
    )
}

/// The reference's `nameFilters` (`Main.qml:185`): executables first, then
/// everything.
///
/// A named function rather than two literals at the call site so the patterns
/// are readable back — `Dialog` keeps its filters private, so this is the
/// half of the chooser a test can pin. The `*.EXE` second glob is the
/// reference's own case belt-and-braces, kept rather than assumed redundant.
pub(crate) fn exe_file_filters() -> Vec<cosmic::dialog::file_chooser::FileFilter> {
    use cosmic::dialog::file_chooser::FileFilter;
    vec![
        FileFilter::new("Windows executables")
            .glob("*.exe")
            .glob("*.EXE"),
        FileFilter::new("All files").glob("*"),
    ]
}

/// The pure half of the locate reply: the chooser's answer as the message the
/// shell already handles.
///
/// A chosen file becomes the path [`complete_easy_install`] finishes from —
/// `Url::to_file_path` is the `as_local_path` the reference applies on the
/// way (`bridge.py:926`), decoding the percent-escapes the portal leaves in.
/// A URL with no local path becomes the `None` that takes the cancel path,
/// exactly as `as_local_path` of nothing does there.
///
/// Any error — the user's cancel and the portal's own failure alike — becomes
/// [`Message::CancelEasyInstall`]. The two are indistinguishable for state
/// purposes: both must release the busy guard the not-found branch holds, and
/// the kept-prefix notice the cancel path toasts is honest in both cases,
/// because nothing deleted the prefix.
pub(crate) fn locate_message(
    token: String,
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    match answer {
        Ok(url) => {
            let path = url
                .to_file_path()
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
            Message::CompleteEasyInstall { token, path }
        }
        Err(_) => Message::CancelEasyInstall(token),
    }
}

/// `fetch_cover`'s own default (`covers.py:359`): twenty seconds for the
/// whole lookup, Steam and icon alike. One constant because the reference has
/// one default; a lookup that needs more patience than a store search is a
/// lookup that has already failed.
const COVER_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The worker half of both cover lookups: `fetch_cover` over the production
/// client, failing to the rendered string.
///
/// Called inside `Task::perform`'s future, like `start_prefix_tool` and
/// `open_prefix_folder` — the blocking client needs no async adaptation
/// because the future runs on a worker (D-48). The `Err` is a `String`
/// because `_async` turns whatever the work raised into one before handing it
/// to `fail` (`bridge.py:157`), and the default `fail` notifies it verbatim —
/// so the reply arms toast the string untouched.
pub(crate) fn cover_lookup(
    name: &str,
    game_id: &str,
    exe: Option<&Path>,
) -> Result<CoverHit, String> {
    gamehandler_core::covers::fetch_cover(
        &crate::http::UreqClient,
        name,
        game_id,
        exe,
        &gamehandler_core::paths::covers_dir(),
        COVER_FETCH_TIMEOUT,
    )
    .map_err(|error| error.to_string())
}

/// The cover a finished easy install carries:
/// `game.cover_path = str(save_exe_icon(exe_path, game_id))`, with `""` when
/// that raises (`bridge.py:907-911`) — a store launcher is not a Steam
/// product, so the executable the vendor just installed carries the right
/// artwork already.
///
/// # What is tested, and what is read
///
/// The `""` half is pinned below (an exe with no icon keeps the empty
/// string). The success half — bytes in, `.ico` path out — is `core`'s
/// `save_exe_icon_to`, tested there against PEs its own builders synthesise;
/// rebuilding that fixture here would duplicate a binary-format builder
/// across crates, so the three-line seam is read.
pub(crate) fn easy_install_cover(executable: &Path, game_id: &str) -> String {
    gamehandler_core::covers::save_exe_icon_to(
        executable,
        game_id,
        &gamehandler_core::paths::covers_dir(),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_default()
}

/// The reference's cover `nameFilters` (`GameFormPage.qml:352`): images only.
/// No "All files" row — the QML lists exactly one filter, and a cover that is
/// not an image is not a cover.
pub(crate) fn image_file_filters() -> Vec<cosmic::dialog::file_chooser::FileFilter> {
    use cosmic::dialog::file_chooser::FileFilter;
    vec![
        FileFilter::new("Images")
            .glob("*.png")
            .glob("*.jpg")
            .glob("*.jpeg")
            .glob("*.webp"),
    ]
}

/// The pure half of the exe reply: the chooser's answer as the local path
/// `urlToLocalFile` hands the form (`bridge.py:601-603`, P-21's browse half).
/// Cancel and portal failure alike become `None` — the reference's exe dialog
/// has no `onRejected` at all, so rejecting leaves the field as it was.
///
/// This runs the URL string through [`as_local_path`] rather than
/// `Url::to_file_path` (which is what the locate reply uses): the reference
/// runs the raw `selectedFile` through `as_local_path`, so a share URL keeps
/// its verbatim fallback instead of collapsing to a cancel.
pub(crate) fn exe_choice_message(
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    Message::ExeFileChosen(answer.ok().map(|url| as_local_path(url.as_str())))
}

/// The pure half of the cover reply: same URL handling as the exe choice —
/// the reference runs both through `as_local_path` (`importCustomCover` does
/// it on the way in, `bridge.py:593`) — and the same silent cancel.
pub(crate) fn cover_choice_message(
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    Message::CoverFileChosen(answer.ok().map(|url| as_local_path(url.as_str())))
}

/// The installed toast's Play action, as a value: `showPassiveNotification`'s
/// `function() { backend.playGame(gameId) }` (`Main.qml:159-161`).
///
/// A named function rather than an inline closure so the mapping is readable
/// back — `Toast` keeps its action private with no accessor, so a test cannot
/// drive the button; this pins the value the button *would* carry. What this
/// does not close: the arm could stop calling this and build the message
/// itself — the same call-site gap `remove_press`'s doc names.
pub(crate) fn installed_play_message(game_id: &str, toast_id: ToastId) -> Message {
    Message::PlayInstalled {
        game_id: game_id.to_string(),
        toast_id,
    }
}

pub(crate) fn easy_install_wizard_finished(
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
        let token = record.game_id.clone();
        // The chooser is half the not-found branch, not an accessory: without
        // it the busy guard has no producer for either message that clears
        // it, and the page stays disabled (P-57).
        let locate = locate_exe_task(token.clone(), record.installer_name.clone());
        // Keyed by the game id, which is what the reference uses as the token
        // (`bridge.py:888-889`).
        state.easy_pending.insert(token, record);
        return cosmic::app::Task::batch([state.toast_task(text), locate]);
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
pub(crate) fn complete_easy_install(
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
pub(crate) fn cancel_easy_install(state: &mut State, token: &str) -> cosmic::app::Task<Message> {
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
/// The entry's cover is the executable's own icon
/// ([`easy_install_cover`], `bridge.py:907-911`) — which used to be the
/// paragraph this one replaces: "`save_exe_icon` has no counterpart in this
/// tree ... the port that closes it is `core::exe_icons`, which task T-05
/// owns." T-05 landed as A1/A2 and U5 wired it here, so an install whose
/// executable carries an icon now gets it as its cover, and an iconless one
/// keeps the reference's own `""` fallback.
pub(crate) fn finish_easy_install(
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
        return state.toast_task(format!(
            "Could not install {}: {message}",
            record.installer_name
        ));
    };
    let mut game = game_from_install(
        installer,
        executable,
        &record.prefix,
        &record.runner_id,
        Some(&record.game_id),
    );
    // The icon is extracted to a path derived from the game id
    // (`easy_install_cover`), so a re-install over an id the library already
    // holds replaces bytes this cache may already have answered for. Dropped
    // for the same reason as `CoverFileChosen`'s copy.
    let cover = easy_install_cover(executable, &game.id);
    state.cover_cache.forget(&cover);
    game.cover_path = cover;
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
