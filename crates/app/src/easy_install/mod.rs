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
//! The flow deliberately stays one module rather than being cut further: the
//! steps above share a working set (the install record, the download timeout,
//! the token that identifies a run) and threading that through three modules
//! would add a boundary the reference does not have. What *did* move — the
//! same row's second pass — is the pure `Message`/`Task`/file-filter
//! vocabulary the flow speaks, which takes none of that state and now lives in
//! [`messages`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use cosmic::iced::futures::StreamExt;
use cosmic::iced::futures::channel::mpsc::{UnboundedSender, unbounded as unbounded_channel};
use gamehandler_core::installers::{
    Installer, SystemClock, build_installer_command, download_installer_with, game_from_install,
    installer_by_id, prepare_prefix, wait_for_installer, wait_for_prefix_idle,
};
use gamehandler_core::paths;
use gamehandler_core::runners::{RunnerManager, SystemLaunchEnv};

use crate::state::State;

use super::Message;

mod messages;
pub(crate) use messages::*;

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
    // The abort switch the card's Cancel sets (`Message::AbortEasyInstall`,
    // UX-27). The worker holds the other `Arc` and polls it inside the
    // download and between the phases after it; `running_install` and this
    // flag are written together here and cleared together everywhere the
    // install resolves, so `Some` means exactly "a worker is running".
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.easy_cancel = Some(cancel.clone());
    state.running_install = Some(crate::state::PendingInstall {
        installer_id: installer.id.to_string(),
        installer_name: installer.name.to_string(),
        prefix: prefix.clone(),
        runner_id: resolved.clone(),
        game_id: game_id.clone(),
    });
    let runners = state.runner_manager();
    let (sender, receiver) = unbounded_channel::<Message>();
    std::thread::spawn(move || {
        easy_install_worker(
            installer, runners, prefix, resolved, game_id, &sender, &cancel,
        )
    });
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
    game_id: String,
    sender: &UnboundedSender<Message>,
    cancel: &std::sync::atomic::AtomicBool,
) {
    let launch_env = SystemLaunchEnv;
    let cancelled = || cancel.load(std::sync::atomic::Ordering::SeqCst);
    // `fail` goes through `crate::report`: it carries the reason the install
    // failed, and a bare `let _ =` would let that reason vanish with a dropped
    // receiver — a failure with no record at all (BUG-28). `progress` stays a
    // bare send on purpose: it fires once per archive chunk, and a gone
    // receiver logged once per chunk is spam, not a record.
    //
    // `game_id` rides both terminal messages so the shell can tell *this*
    // run's reply from a cancelled predecessor's — an aborted run's late
    // `EasyInstallFailed`/`EasyInstallWizardFinished` must not clear the guard
    // a newer install is holding, and the id is the only thing that separates
    // the two (UX-27).
    let fail = |message: String| {
        crate::report(
            sender,
            Message::EasyInstallFailed {
                game_id: game_id.clone(),
                message,
            },
        );
    };
    let progress = |fraction: f64| {
        // A cancelled install stops reporting: its ticks are ephemeral and the
        // abort has already cleared the bar they would land on — or worse, a
        // successor's bar (`RunnerProgress`'s closure makes the same check and
        // says why there).
        if cancelled() {
            return;
        }
        // A send fails only when the receiver is gone — the window closed or
        // the task was dropped — and a progress fraction has nobody to show.
        let _ = sender.unbounded_send(Message::EasyInstallProgress(fraction as f32));
    };
    let archive = match download_installer_with(
        installer,
        &paths::downloads_dir(),
        Some(&progress),
        INSTALLER_DOWNLOAD_TIMEOUT,
        &crate::http::UreqClient,
        &launch_env,
        &cancelled,
    ) {
        Ok(archive) => archive,
        // A cancel reports itself — the abort already toasted "Cancelled" —
        // and every other failure reports through the gated `fail`, which a
        // stale reply cannot pass either.
        Err(_) if cancelled() => return,
        Err(error) => return fail(error.to_string()),
    };
    if cancelled() {
        return;
    }
    // The byte-measurable phase is over: what remains — the vendor wizard and
    // the settle wait — has no fraction to report. `-1.0` is the reference's
    // own "nothing to show" sentinel (`_set_progress(-1)`), and the page reads
    // it as "indeterminate" rather than a bar frozen at its last tick (UX-27).
    // A bare send for the same reason `progress` is one.
    let _ = sender.unbounded_send(Message::EasyInstallProgress(-1.0));
    // `bridge.py:859-865`, emitted from the worker because it must arrive
    // *after* the download: the point of the sentence is that the vendor's
    // window is about to appear, and a notice that precedes a two-minute
    // download says the opposite. A bare `let _ =` is the honest send here —
    // it fails only when the window is already gone, which is precisely when
    // there is no wizard to announce (BUG-28).
    let _ = sender.unbounded_send(Message::Notify(format!(
        "Launching the {} installer… Finish the vendor wizard, then close it — \
         GameHandler adds it as soon as the install lands.",
        installer.name
    )));
    let runner = runners.get(&runner_id, &launch_env);
    let command = match build_installer_command(&*runner, &prefix, installer, &archive, &launch_env)
    {
        Ok(command) => command,
        Err(_) if cancelled() => return,
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
    let returncode = match run_installer(&command, &cwd, &cancelled) {
        Ok(Some(code)) => code,
        // `None` is the cancelled exit — the wizard was killed, the abort's
        // toast has spoken, and there is nothing left this run could usefully
        // report.
        Ok(None) => return,
        Err(_) if cancelled() => return,
        Err(error) => return fail(error.to_string()),
    };
    // `wait_for_installer(runner, env, prefix, installer.expected_exe)`
    // (`:871`) — P-56's loop: poll for the expected executable, slice the
    // wineserver wait so a leftover store client cannot hide a finished
    // install, and give up at `INSTALL_SETTLE_TIMEOUT_SECONDS` (six hours) —
    // or at `cancelled`, which returns the timeout's `None` one poll early.
    let found = wait_for_installer(
        &*runner,
        &command.env,
        &prefix,
        installer.expected_exe,
        &SystemClock,
        &|runner, env, seconds| wait_for_prefix_idle(runner, env, seconds, &SystemLaunchEnv),
        &cancelled,
    );
    if cancelled() {
        return;
    }
    crate::report(
        sender,
        Message::EasyInstallWizardFinished {
            found,
            returncode,
            game_id,
        },
    );
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
///
/// The poll exists for the Cancel button (UX-27): `status()` waits for the
/// child without looking at anything else, so the loop below checks
/// `cancelled` between `try_wait`s and kills the wizard when it trips — the
/// vendor's own window closes as part of the abort rather than outliving it.
/// `Ok(None)` is that exit: not a code, because the wizard did not have one.
fn run_installer(
    command: &gamehandler_core::runners::Command,
    cwd: &Path,
    cancelled: &dyn Fn() -> bool,
) -> std::io::Result<Option<i32>> {
    let Some((program, arguments)) = command.argv.split_first() else {
        // Unreachable: `build_installer_command` refuses an empty argv with
        // `InstallerError::EmptyCommand` before returning.
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the runner produced no command to run",
        ));
    };
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .env_clear()
        .envs(&command.env)
        .current_dir(cwd)
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status.code().unwrap_or(-1)));
        }
        if cancelled() {
            // `kill` then `wait`, the pair the reaper uses in
            // `core::runners::launch`: the kill asks and the wait reaps, so
            // no zombie outlives the abort. A `kill` failure — the wizard
            // exited in the gap — is ignored for the same reason the poll is
            // a loop: `try_wait` is about to say so anyway.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub(crate) fn easy_install_wizard_finished(
    state: &mut State,
    found: Option<&Path>,
    returncode: i32,
    game_id: &str,
) -> cosmic::app::Task<Message> {
    // A reply whose install is gone — the task was dropped, or a cancel raced
    // it — is dropped rather than interpreted. So is a reply whose `game_id`
    // is not the running install's: that is an aborted predecessor's late
    // report, and letting it resolve would feed the old run's `found` into the
    // new run's record (UX-27). This is the same silence
    // `Message::LaunchWatchFinished` keeps for a game the library no longer
    // holds.
    let Some(record) = state.running_install.clone() else {
        return cosmic::task::none();
    };
    if record.game_id != game_id {
        return cosmic::task::none();
    }
    // The worker's last act was sending this message, so the abort flag it
    // polled is dead — whether the install finishes here or waits on the
    // picker, no worker remains to observe it.
    state.easy_cancel = None;
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
    // Defensive only: `easy_install_wizard_finished` already cleared the flag
    // when the worker exited, which is the only way into `easy_pending`.
    state.easy_cancel = None;
    let Some(record) = record else {
        return cosmic::task::none();
    };
    let text = format!(
        "Kept the {} prefix. Add it later from Add Game if you want.",
        record.installer_name
    );
    state.toast_task(text)
}

/// The Cancel button's message (`Message::AbortEasyInstall`, UX-27).
///
/// This is the control the reference does not have: `cancelEasyInstall` there
/// only answers the locate dialog's reject, after the worker has already
/// exited. Here the worker is still running — mid-download, mid-wizard, or in
/// the settle poll — so the abort is two acts: set the flag the worker polls
/// (the download's sink turns it into `InstallerError::Cancelled` at the next
/// chunk; `run_installer` kills the wizard on it; `wait_for_installer` returns
/// `None` on it), and clear the guards *now* so the page stops waiting on the
/// press rather than on that next boundary.
///
/// The record goes with the guards: `running_install` is what a late
/// `EasyInstallWizardFinished`/`EasyInstallFailed` would resolve against, and
/// dropping it here is what makes those replies land on nothing — the
/// `game_id` gate covers the ordering where a new install has already started.
///
/// The prefix is **kept**, deliberately and stated in the toast: deleting a
/// prefix out from under a dying wizard is a second failure mode, not a
/// cleanup.
pub(crate) fn abort_easy_install(state: &mut State) -> cosmic::app::Task<Message> {
    let Some(flag) = state.easy_cancel.take() else {
        // No worker is polling — the press raced the install's own end, or the
        // install is in the pending-exe phase whose Cancel goes through
        // `CancelEasyInstall` instead. Nothing to set, nothing to say.
        return cosmic::task::none();
    };
    flag.store(true, std::sync::atomic::Ordering::SeqCst);
    let name = state
        .running_install
        .take()
        .map(|record| record.installer_name);
    state.easy_busy = false;
    state.progress = None;
    let text = match name {
        Some(name) => format!(
            "Cancelled the {name} install. The prefix was kept — add it later \
             from Add Game if you want."
        ),
        // The flag existed with no record, which the invariants above say
        // cannot happen — reported anyway rather than unwrapped, because a
        // cancel that says nothing reads as a cancel that did nothing.
        None => "Cancelled the install.".to_string(),
    };
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
        state.easy_cancel = None;
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
    state.easy_cancel = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// UX-27: the wizard's exit code is reported, not turned into an error —
    /// `check=False` in the reference (`bridge.py:870`), so a nonzero exit is
    /// `Ok(Some(code))` and reaches `EasyInstallWizardFinished`.
    #[test]
    fn a_finished_wizard_reports_its_exit_code() {
        let command = gamehandler_core::runners::Command {
            argv: vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            env: std::collections::BTreeMap::new(),
        };
        let code = run_installer(&command, Path::new("/"), &|| false).unwrap();
        assert_eq!(code, Some(7));
    }

    /// UX-27: a flag that trips mid-run kills the vendor's wizard and returns
    /// `Ok(None)` — the "no code" answer the worker reads as "cancelled".
    ///
    /// The child is a real `sleep 60`, and the bound on elapsed *is* the
    /// assertion that it was killed: a `run_installer` that waited the child
    /// out would return in a minute, not in the poll's own granularity. The
    /// flag is flipped from the test thread after a beat, so the abort lands
    /// while the poll loop is genuinely between `try_wait`s.
    #[test]
    fn a_cancelled_wizard_is_killed_not_waited_out() {
        let command = gamehandler_core::runners::Command {
            argv: vec!["sleep".to_string(), "60".to_string()],
            env: std::collections::BTreeMap::new(),
        };
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_flag = flag.clone();
        let started = std::time::Instant::now();
        let worker = std::thread::spawn(move || {
            run_installer(&command, Path::new("/"), &|| {
                worker_flag.load(std::sync::atomic::Ordering::SeqCst)
            })
        });
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let outcome = worker.join().unwrap().unwrap();
        assert_eq!(outcome, None, "a killed wizard has no exit code to report");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the wizard was waited out, not killed: {:?}",
            started.elapsed()
        );
    }
}
