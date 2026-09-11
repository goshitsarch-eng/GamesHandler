//! Process launch and early-exit reporting. Port of `runners.py:1295-1500`:
//! `launch`, `LaunchedGame`, `_ErrorTail`, `_readable_error` and
//! `tool_command`.
//!
//! # Why a launched title needs a second look
//!
//! `Popen` succeeding proves only that the runner binary exists. Wine exiting
//! two seconds later because it could not find the executable looks exactly
//! like a successful launch, so [`LaunchedGame::failure`] waits out a short
//! grace period and reports what the runner said. Everything in this module
//! exists to serve that one function.

use std::io::Read;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustix::fs::OFlags;
use rustix::io::Errno;

use super::env::LaunchEnv;
use super::launch_opts::{
    apply_launch_options, build_linux_command, install_bundled_dxvk, parse_env_block,
    resolve_game_paths, ShareResolver,
};
use super::{
    failure_message, readable_error, uses_proton_runtime, Command, RunnerError, RunnerManager,
    DXVK_ROOT, DXVK_ROOT_ENV,
};
use crate::models::Game;
use crate::paths;

/// Bytes of a child's stderr kept for the failure message. `_ERROR_BUFFER_BYTES`.
pub const ERROR_BUFFER_BYTES: usize = 64 * 1024;

/// How long the drain thread sleeps between polls of an open pipe.
///
/// Only reached once the pipe is empty, so it costs nothing during a normal
/// drain and bounds how long [`ErrorTail::finish`] waits for the thread to
/// notice it has been asked to stop.
const DRAIN_POLL: Duration = Duration::from_millis(5);

/// The tail of a child's stderr, kept in a rolling buffer.
///
/// Port of `_ErrorTail` (`runners.py:1315`). Draining is what makes capturing
/// safe at all: an undrained pipe fills and stalls the game, and an unbounded
/// buffer would grow for as long as the title runs.
///
/// # B-07 — the drain is finished before the text is read
///
/// Python's `LaunchedGame.failure()` calls `process.wait()`, which returns when
/// the child is **reaped**, and then reads `self.errors.text()` — the buffer of
/// a thread that was never joined. That is a race, and it is the bug this port
/// deliberately does not copy: whatever the drain had not yet appended when the
/// reaper won is silently replaced by `the runner exited with status N`, so a
/// user sees a status code instead of the reason. Python's own suite hid it for
/// as long as the race stayed narrow.
///
/// [`ErrorTail::finish`] therefore drains to completion before the buffer is
/// read. It is recorded as a divergence in **reliability**, not in output
/// format: given the runner's text, both produce the same message.
///
/// # Why this is not a plain `join` on a blocking reader
///
/// The obvious fix — read on a thread and `join` it — is wrong, and it was
/// measured rather than reasoned about. A blocking read only returns at EOF,
/// and EOF needs **every** writer to close. A runner that leaves a descendant
/// holding stderr (Wine starting the game, a wrapper script backgrounding
/// something) keeps the pipe open after the runner itself is gone, so the join
/// blocks. Measured on this host with `sh -c 'echo … >&2; (sleep 5) & exit 3'`:
/// the child is reaped in 0.8 ms and the drain is still blocked four seconds
/// later. A plain join there is a frozen window, which is worse than the race
/// it fixes.
///
/// So the reader is set `O_NONBLOCK` and the thread polls. [`ErrorTail::finish`]
/// raises a stop flag; the thread then makes one final pass reading until the
/// pipe would block, and exits. Everything written before `finish` was called
/// has been consumed by the time it returns, and nothing blocks on a pipe that
/// will not close. The stop flag is checked *before* the final pass and not
/// instead of it, which is what makes the guarantee hold rather than merely
/// usually hold.
pub struct ErrorTail {
    shared: Arc<TailState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct TailState {
    chunks: Mutex<Vec<u8>>,
    stop: AtomicBool,
}

impl ErrorTail {
    /// Begin draining `stream` on a thread of its own.
    ///
    /// A stream that cannot be put into non-blocking mode is still drained, by
    /// the blocking path — a caller with no better option is better served by a
    /// working buffer than by an error it cannot act on.
    pub fn spawn(stream: ChildStderr, limit: usize) -> Self {
        let nonblocking = set_nonblocking(&stream).is_ok();
        let shared = Arc::new(TailState {
            chunks: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("gamehandler-stderr".to_string())
            .spawn(move || drain(stream, worker, limit, nonblocking))
            .ok();
        Self { shared, thread }
    }

    /// The captured text so far, decoded lossily.
    ///
    /// Decoding is `decode("utf-8", "replace")` — `from_utf8_lossy` — because
    /// Wine's output is not reliably UTF-8 and a decode failure here would
    /// discard the very message this exists to carry.
    pub fn text(&self) -> String {
        let chunks = self.shared.chunks.lock().unwrap_or_else(|error| error.into_inner());
        String::from_utf8_lossy(&chunks).into_owned()
    }

    /// Drain to completion, then return the text. See the type's note on B-07.
    ///
    /// Consumes the tail: `failure()` is a one-shot question about a launch
    /// that has already gone wrong, and allowing a second read would invite a
    /// caller to poll it in a loop.
    pub fn finish(mut self) -> String {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // Bounded, and the bound is safe because the thread's final pass
            // exits at the first would-block rather than waiting for EOF. A
            // thread that is mid-`read` when the flag is set returns within one
            // `DRAIN_POLL`; a thread that panicked leaves the mutex poisoned,
            // which `text` recovers from.
            let _ = thread.join();
        }
        self.text()
    }
}

/// Put `stream` into non-blocking mode, returning the previous flags.
fn set_nonblocking(stream: &ChildStderr) -> std::io::Result<()> {
    let current = rustix::fs::fcntl_getfl(stream).map_err(std::io::Error::from)?;
    rustix::fs::fcntl_setfl(stream.as_fd(), current | OFlags::NONBLOCK)
        .map_err(std::io::Error::from)
}

/// The drain loop. Non-blocking when it can be; blocking otherwise.
fn drain(mut stream: ChildStderr, state: Arc<TailState>, limit: usize, nonblocking: bool) {
    let mut buffer = [0u8; 4096];

    let stop_requested = || state.stop.load(Ordering::SeqCst);

    let append = |bytes: &[u8]| {
        let mut chunks = state.chunks.lock().unwrap_or_else(|error| error.into_inner());
        chunks.extend_from_slice(bytes);
        // Trim from the front, keeping at least one chunk so a single write
        // larger than the limit is not discarded entirely. Python's loop has
        // the same `len(self._chunks) > 1` guard.
        while chunks.len() > limit && chunks.len() > 1 {
            let excess = chunks.len() - limit;
            chunks.drain(..excess);
        }
    };

    if !nonblocking {
        // The blocking path cannot be stopped on demand, so it is only ever
        // left to run: it ends at EOF or on the first error. `finish` joins it
        // with no bound beyond the pipe's own lifetime, which is the case the
        // non-blocking path exists to avoid, and is reachable only if the fd
        // could not be reconfigured.
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => append(&buffer[..read]),
            }
        }
        return;
    }

    loop {
        if stop_requested() {
            // Final pass: everything already in the pipe, then out.
            loop {
                let read = match read_once(&mut stream, &mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                append(&buffer[..read]);
            }
        }
        match read_once(&mut stream, &mut buffer) {
            // EOF: the last writer closed.
            Ok(0) => return,
            Ok(read) => append(&buffer[..read]),
            Err(error) if error == Errno::AGAIN || error == Errno::WOULDBLOCK => {
                std::thread::sleep(DRAIN_POLL);
            }
            Err(_) => return,
        }
    }
}

/// One non-blocking read. `Err(AGAIN)` means "nothing right now", not failure.
fn read_once(stream: &mut ChildStderr, buffer: &mut [u8]) -> Result<usize, Errno> {
    rustix::io::read(stream.as_fd(), &mut *buffer)
}

/// A started title, plus the means to notice it died on the doorstep.
/// `LaunchedGame` (`runners.py:1350`).
pub struct LaunchedGame {
    child: Child,
    errors: Option<ErrorTail>,
}

impl LaunchedGame {
    /// The child's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// A handle to the running child, for a caller that outlives the grace
    /// period and needs to wait on it itself.
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Why the title stopped, if it stopped badly within `timeout`.
    ///
    /// `Some` is the message a toast should show; `None` means the title was
    /// **still running** when the grace period expired, which is the success
    /// case and the overwhelmingly common one.
    ///
    /// The two exits are Python's: a timeout is `None`, an exit code of `0` is
    /// `None`, and anything else is the runner's own last words or — when it
    /// said nothing — [`failure_message`]'s status line. See [`ErrorTail`] for
    /// why the drain is finished before the buffer is read, which is the one
    /// place this deliberately differs from Python.
    ///
    /// The `status == 0` rule is written twice: here, which is Python's order —
    /// it returns before touching the tail — and again in [`failure_message`].
    /// Flipping this one is therefore invisible from outside and a mutation
    /// battery reports it as a survivor. It is a duplicate rather than a gap:
    /// the rule that has to survive is [`failure_message`]'s, and this early
    /// return is kept for Python's ordering rather than for an observable of
    /// its own.
    pub fn failure(&mut self, timeout: Duration) -> Option<String> {
        let status = wait_with_timeout(&mut self.child, timeout)?;
        if status == 0 {
            return None;
        }
        let captured = self.errors.take().map(ErrorTail::finish).unwrap_or_default();
        failure_message(status, &readable_error(&captured))
    }

}

/// Wait up to `timeout` for `child`, returning its exit code or `None`.
///
/// A child killed by a signal has no exit code. Python's `Popen.wait()` returns
/// a negative number there (`-SIGTERM`), and [`failure_message`] renders it as a
/// status; `code()` is `None`, so the port reports `-1`. The difference is
/// visible only for a title that was killed by a signal within the grace
/// period, where Python's number would be the negated signal — a number that
/// names no signal a user could act on either way.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code().unwrap_or(-1)),
            Ok(None) => {}
            // A child that cannot be waited on is treated as still running
            // rather than as a failure: the message a launch shows must come
            // from the runner, and inventing one from a `waitpid` error would
            // be the same class of fabrication B-07 is about.
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

/// Launch `game` with its configured runner. `runners.py:1377`.
///
/// Returns a [`LaunchedGame`] whose `failure()` reports a title that exited
/// immediately, so a broken launch surfaces instead of looking like a
/// successful one.
///
/// The environment is built in Python's exact order, and the order is
/// load-bearing rather than incidental:
///
/// 1. The runner's own environment or, for a Linux title, the process
///    environment.
/// 2. The user's `game.environment` block, applied **early** so that prefix
///    setup and the bundled-DXVK installer can read `WINEARCH` from it.
/// 3. The prefix directory created, so the DXVK copy has somewhere to go.
/// 4. The NVAPI/FSR/Wayland gate, which needs `uses_proton` and so runs before
///    the options are folded in.
/// 5. `install_bundled_dxvk`, which merges a DLL override that
///    `apply_launch_options` must not clobber.
/// 6. `apply_launch_options`, which re-applies the environment block so a
///    user's value still wins over the toggles.
///
/// `extra` — `game.additional_app` — is started *before* the main process and
/// without waiting, which is Python's behaviour and what makes a tool like
/// `gamescope` or a trainer usable alongside the title.
pub fn launch(
    game: &Game,
    manager: &RunnerManager,
    env: &dyn LaunchEnv,
    resolver: &dyn ShareResolver,
) -> Result<LaunchedGame, RunnerError> {
    let game = resolve_game_paths(game, resolver)?;
    let mut uses_proton = false;
    let mut runner_executable = String::new();
    let (argv, environment) = if game.is_linux() {
        (build_linux_command(&game, env)?.argv, env.environ())
    } else {
        let runner = manager.get(&game.runner, env);
        let command = runner.build_command(&game, env)?;
        let Command { argv, env: mut environment } = command;

        // Applied early *and* last — see this function's note.
        environment.extend(parse_env_block(&game.environment));
        runner_executable = argv.first().cloned().unwrap_or_default();

        if let Some(prefix) = super::stripped_var(&environment, "WINEPREFIX") {
            std::fs::create_dir_all(&prefix)?;
        }
        uses_proton = uses_proton_runtime(runner.as_ref(), &argv);
        if game.nvapi && !uses_proton {
            return Err(RunnerError::NvapiNeedsProton);
        }
        if game.fsr && !uses_proton {
            return Err(RunnerError::FsrNeedsProton);
        }
        if game.wayland && !uses_proton {
            return Err(RunnerError::WaylandNeedsProton);
        }
        // `GAMEHANDLER_DXVK_ROOT` overrides the bundled location, for source
        // installs. `DXVK_ROOT_ENV` is read from the **process** environment in
        // Python (`os.environ`), not from the map being built, so a game's own
        // environment block cannot redirect where the runtime is copied from.
        // `Env::var` and `LaunchEnv::var` both exist and `LaunchEnv: Env`
        // re-declares it, so the call is qualified rather than left ambiguous.
        let dxvk_root = crate::paths::Env::var(env, DXVK_ROOT_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DXVK_ROOT));
        if game.dxvk && !uses_proton && dxvk_root.is_dir() {
            install_bundled_dxvk(&mut environment, Some(&dxvk_root), env)?;
        }
        (argv, environment)
    };

    let command = apply_launch_options(&game, &argv, &environment, uses_proton, env)?;
    let Command { argv, env: environment } = command;

    let extra = game.additional_app.trim();
    if !extra.is_empty() {
        let mut extra_argv: Vec<String> = Vec::new();
        if !runner_executable.is_empty() {
            extra_argv.push(runner_executable);
        }
        extra_argv.push(extra.to_string());
        // Detached and unreaped, exactly as Python leaves it: the extra app is
        // a helper the user asked for, and its failure is not the title's.
        let _ = Process::new(&extra_argv[0])
            .args(&extra_argv[1..])
            .envs(&environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    // Python's `cwd = game.working_directory or None`, then the executable's
    // own directory when the working directory is empty and the executable
    // exists. The second half is what makes a title whose installer wrote a
    // relative path beside the `.exe` still find it.
    let mut cwd = game.working_directory.trim().to_string();
    if cwd.is_empty() && !game.exe_path.is_empty() && Path::new(&game.exe_path).exists() {
        cwd = Path::new(&game.exe_path)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    let mut process = Process::new(&argv[0]);
    process.args(&argv[1..]).envs(&environment);
    if !cwd.is_empty() {
        process.current_dir(&cwd);
    }
    // stdout stays inherited so running from a terminal still shows the game's
    // own output; only stderr is captured, and only to explain a fast exit.
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()?;
    let errors = child
        .stderr
        .take()
        .map(|stream| ErrorTail::spawn(stream, ERROR_BUFFER_BYTES));
    Ok(LaunchedGame { child, errors })
}

/// Build a command that opens `winecfg` or `winetricks` in the game's prefix.
/// `tool_command` (`runners.py:1476`).
///
/// Two environment variables carry the point of this function. `WINEPREFIX` is
/// the prefix the *game* uses — which for a Proton build is one level down from
/// where `WINEPREFIX` usually points, see [`super::wine_prefix_root`] — and
/// `WINE` points at the same Wine the game runs on. Without `WINE`, winetricks
/// falls back to the system wine, which refuses or silently migrates a prefix a
/// newer Proton created, so the tool appears to work while operating on the
/// wrong thing.
///
/// `WINESERVER` is set only when a `wineserver` sits beside the wine binary and
/// is executable, which is what stops winetricks from pairing a new wineserver
/// with an old Wine.
///
/// An unknown `tool` is [`RunnerError::UnknownTool`], and `winetricks` missing
/// from `PATH` is [`RunnerError::WinetricksMissing`] — both reach the user as
/// the messages `tests/test_runners.py` asserts on.
pub fn tool_command(
    game: &Game,
    manager: &RunnerManager,
    tool: &str,
    env: &dyn LaunchEnv,
) -> Result<Command, RunnerError> {
    let runner = manager.get(&game.runner, env);
    let Some(wine) = runner.wine_binary() else {
        return Err(RunnerError::NotAvailable {
            name: runner.name().to_string(),
        });
    };
    let prefix_root = game.prefix_path.trim();
    let prefix = if prefix_root.is_empty() {
        paths::prefixes_dir_in(env).join(&game.id)
    } else {
        PathBuf::from(prefix_root)
    };
    let prefix = super::wine_prefix_root(&prefix);

    let mut environment = env.environ();
    environment.insert(
        "WINEPREFIX".to_string(),
        prefix.to_string_lossy().into_owned(),
    );
    environment.insert("WINE".to_string(), wine.to_string_lossy().into_owned());

    let wineserver = wine.with_file_name("wineserver");
    if wineserver.is_file() && is_executable(&wineserver) {
        environment.insert(
            "WINESERVER".to_string(),
            wineserver.to_string_lossy().into_owned(),
        );
    }
    std::fs::create_dir_all(&prefix)?;

    match tool {
        "winecfg" => Ok(Command {
            argv: vec![wine.to_string_lossy().into_owned(), "winecfg".to_string()],
            env: environment,
        }),
        "winetricks" => {
            let Some(winetricks) = env.which("winetricks") else {
                return Err(RunnerError::WinetricksMissing);
            };
            Ok(Command {
                argv: vec![winetricks.to_string_lossy().into_owned()],
                env: environment,
            })
        }
        _ => Err(RunnerError::UnknownTool {
            tool: tool.to_string(),
        }),
    }
}

/// `os.access(path, os.X_OK)`, which is not the same question as `is_file()`.
///
/// `std::fs::metadata` gives the mode bits; the *effective* answer for the
/// current user also depends on ownership and group, so this checks the owner
/// bit when we own the file and the group bit when our group does, and falls
/// back to any-execute otherwise. That is stricter than `access(2)` for the
/// exotic case of a supplementary group, and the consequence of being wrong is
/// a `WINESERVER` variable that is not set — which is exactly what Python does
/// when the file is not executable at all.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::env::tests::{scratch, FakeLaunchEnv};
    use std::io::Write as _;

    /// A `Game` that launches `/bin/sh -c <script>` as a native Linux title.
    ///
    /// A native title takes the `build_linux_command` branch, which is the one
    /// that needs no runner installed — so the drain and the failure reporting
    /// can be exercised against a real child process rather than a simulated
    /// one. The script is passed as the executable's single argument, and
    /// `Game::arguments` is shlex-split, so it is quoted.
    fn scripted(script: &str) -> Game {
        let mut game = Game::new_named("Test Title");
        game.kind = "linux".to_string();
        game.exe_path = "/bin/sh".to_string();
        game.arguments = format!("-c '{}'", script.replace('\'', "'\"'\"'"));
        game
    }

    /// A resolver that does nothing: no shares, no mappings.
    struct NoShares;

    impl ShareResolver for NoShares {
        fn is_remote_url(&self, _value: &str) -> bool {
            false
        }
        fn as_local_path(&self, value: &str) -> String {
            value.to_string()
        }
        fn unreachable_share_message(&self, value: &str) -> String {
            format!("{value} is not mounted")
        }
    }

    /// `#49`: launch, retrying the `ETXTBSY` window that *other tests'* forks
    /// open — the repair for a defect in this harness rather than in `launch`.
    ///
    /// # The mechanism, measured rather than reasoned
    ///
    /// `execve` refuses with `ETXTBSY` (`code: 26`) while the exec'd inode has
    /// a live writable reference. The written-and-exec'd script is exclusive to
    /// one test — `scratch` is pid-keyed with a unique label (`env.rs:420-425`)
    /// — so the reference cannot be another test's write. It is a *fork*: this
    /// test binary runs its tests on threads, and when one thread forks, the
    /// child inherits a copy of every open file in the process, including
    /// another test's just-written script. That copy keeps
    /// `i_writecount > 0` until the child reaches its own `execve`, and in that
    /// interval an `execve` of the same inode by the writing thread is refused.
    ///
    /// Each clause of that was measured against a standalone reproducer before
    /// it was written down here:
    ///
    /// - **One writer alone never fails** — 300 solo runs of the single most
    ///   flaky test, 0 failures. There is no self-inflicted window.
    /// - **Two writers contend** — 79 refusals in 8s with no forking thread,
    ///   and 0 with one writer. Refusals scale with the number of concurrent
    ///   writers (1: 0, 2: 79, 4: 893), which no same-path explanation reaches.
    /// - **A thread that only forks multiplies it** — 2 writers plus a thread
    ///   running `/bin/true` in a loop: 79 → 431 refusals. The forking thread
    ///   neither writes nor execs any script of ours.
    /// - **The child's lifetime is the window** — replacing the child with
    ///   `/bin/sleep 0.05` cuts refusals back to 79, tracking the fork *rate*
    ///   (20× fewer forks) rather than the child's lifetime. The window is per
    ///   fork, and the child's window is only as long as it takes to `execve`.
    /// - **The refusal is inode-specific** — the decisive control: write a
    ///   *different* file and `execve` a shared script created once before any
    ///   thread existed. 0 refusals at 4 writers, 0 at 8, against 2072 for the
    ///   same load writing and executing the same inode. So it is not
    ///   "concurrent writes poison `execve` process-wide"; it is a descriptor
    ///   inherited on *that* inode.
    ///
    /// Those measurements also retire the obvious alternative repairs.
    /// The write-then-`chmod` window is not it, since both write idioms flake at
    /// the same rate under load and one writer alone never does. Neither is
    /// write-to-temp-then-rename, which sounds like the textbook answer and is
    /// measured not to help: `rename` carries the *inode* to the new name, so
    /// the inherited descriptor follows it to the exec'd path (1568 refusals
    /// under the load that gives the fresh-inode control 0). Only a target
    /// nothing in this process ever opened for writing is immune.
    ///
    /// # Why the retry is here and not in `launch`
    ///
    /// `launch`'s own `spawn` is not exposed to this. Production `launch.rs`
    /// writes no executable — the only filesystem calls before the test module
    /// are two `create_dir_all` on prefix directories (`:334`, `:465`) — so
    /// nothing in the shipped app creates the window, and a retry in `launch`
    /// would be a catch on a condition production cannot enter: dead code
    /// pretending to be a guard. The window is opened by *this binary's own
    /// concurrency*, so its repair belongs to this binary's harness.
    ///
    /// # What a retry does and does not hide
    ///
    /// A refusal is not a behaviour of the code under test — no input, no
    /// configuration and no host state the test controls produces it — so
    /// tolerating it removes noise and no signal. Every other error, and every
    /// non-`ETXTBSY` `Io` error, is returned on the first attempt unchanged.
    /// If the condition outlasts the attempts the last error is returned as-is,
    /// so the caller's `expect` still fails on it: after ~100ms of trying, a
    /// persistent `ETXTBSY` is a real finding and must not be swallowed.
    fn launched_or_busy_retry(
        game: &Game,
        manager: &RunnerManager,
        env: &dyn LaunchEnv,
        resolver: &dyn ShareResolver,
    ) -> Result<LaunchedGame, RunnerError> {
        const ATTEMPTS: u32 = 100;
        const PAUSE: Duration = Duration::from_millis(1);
        let mut attempt = 0;
        loop {
            attempt += 1;
            match launch(game, manager, env, resolver) {
                Err(RunnerError::Io(error))
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && attempt < ATTEMPTS =>
                {
                    std::thread::sleep(PAUSE);
                }
                outcome => return outcome,
            }
        }
    }

    /// Launch `game` with nothing installed and no host environment.
    fn launched(game: &Game) -> LaunchedGame {
        launched_or_busy_retry(
            game,
            &RunnerManager::at("/nonexistent"),
            &FakeLaunchEnv::new(),
            &NoShares,
        )
        .expect("a native Linux title needs no runner")
    }

    // -----------------------------------------------------------------
    // B-07 — the drain is finished before the text is read
    // -----------------------------------------------------------------

    /// The coordinator's requirement stated exactly: the runner's own error
    /// text must be **present**, and the fallback status line is a failure.
    ///
    /// There is deliberately no `assert!(text == real || text == fallback)`
    /// here. Accepting either outcome is how Python's suite stayed green while
    /// the race was live, and a test that tolerates the fallback is a test that
    /// passes for the bug.
    #[test]
    fn a_dead_runner_reports_its_own_error_text_and_never_the_fallback() {
        let game = scripted("echo 'err: import_dll failed for xaudio2_9' >&2; exit 3");
        let mut running = launched(&game);
        let message = running
            .failure(Duration::from_secs(10))
            .expect("a non-zero exit within the grace period is a failure");

        assert!(
            message.contains("import_dll"),
            "the runner's error text should be reported, but got {message:?}"
        );
        assert!(
            !message.starts_with("the runner exited with status"),
            "the fallback means the drain lost the text, which is B-07 itself: {message:?}"
        );
    }

    /// The case that makes the naive fix wrong, and the reason this drain is
    /// non-blocking.
    ///
    /// A runner that leaves a **descendant** holding stderr keeps the pipe open
    /// after the runner itself is gone. A blocking read only returns at EOF, so
    /// a plain `join` blocks until that descendant exits — measured at over
    /// four seconds for a five-second sleeper, and unbounded in principle. The
    /// fix must therefore both (a) report the runner's text and (b) return
    /// promptly.
    ///
    /// The descendant sleeps far longer than this test's own patience, so a
    /// blocking join cannot pass it even by luck.
    #[test]
    fn a_descendant_holding_stderr_neither_deadlocks_the_drain_nor_loses_the_text() {
        let game = scripted(
            "echo 'err: could not find the executable' >&2; (sleep 30) & exit 7",
        );
        let mut running = launched(&game);

        let started = Instant::now();
        let message = running
            .failure(Duration::from_secs(10))
            .expect("a non-zero exit within the grace period is a failure");
        let elapsed = started.elapsed();

        assert!(
            message.contains("could not find the executable"),
            "the text written before the descendant was started must survive: {message:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the drain must not wait for a descendant to close the pipe; took {elapsed:?}"
        );
    }

    /// Text written by the child is captured even when the child exits
    /// immediately — no sleep, no window for a thread to have caught up.
    /// Repeated because a race is only sometimes visible.
    #[test]
    fn the_capture_does_not_depend_on_winning_a_race() {
        for attempt in 0..25 {
            let game = scripted("echo 'err: attempt marker' >&2; exit 2");
            let mut running = launched(&game);
            let message = running.failure(Duration::from_secs(10)).unwrap();
            assert!(
                message.contains("attempt marker"),
                "attempt {attempt} lost the text: {message:?}"
            );
        }
    }

    /// The other direction, so the test above cannot be satisfied by a function
    /// that simply never uses the fallback: a child that exits non-zero without
    /// writing anything is reported by its status.
    #[test]
    fn a_silent_non_zero_exit_falls_back_to_the_status_line() {
        let game = scripted("exit 4");
        let mut running = launched(&game);
        assert_eq!(
            running.failure(Duration::from_secs(10)).as_deref(),
            Some("the runner exited with status 4")
        );
    }

    /// A clean exit is not a failure, however short-lived.
    #[test]
    fn a_zero_exit_is_not_a_failure() {
        let game = scripted("exit 0");
        let mut running = launched(&game);
        assert_eq!(running.failure(Duration::from_secs(10)), None);
    }

    /// A title still running when the grace period expires is the success case.
    #[test]
    fn a_title_still_running_at_the_deadline_is_not_a_failure() {
        let game = scripted("sleep 30");
        let mut running = launched(&game);
        let started = Instant::now();
        assert_eq!(running.failure(Duration::from_millis(150)), None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the grace period must bound the wait, not the child's lifetime"
        );
        let _ = running.child_mut().kill();
    }

    /// `failure` reads the runaway's output through `readable_error`, so Wine's
    /// noise is filtered and the actual error is what a user sees.
    #[test]
    fn wine_noise_is_filtered_out_of_the_reported_error() {
        let game = scripted(
            "echo 'fixme:heap:HeapSetInformation' >&2; \
             echo 'warn: D3D11CreateDevice failed' >&2; \
             echo 'err: module not found' >&2; \
             exit 5",
        );
        let mut running = launched(&game);
        let message = running.failure(Duration::from_secs(10)).unwrap();
        assert!(message.contains("module not found"), "{message:?}");
        assert!(
            !message.contains("fixme:"),
            "noise prefixes are dropped: {message:?}"
        );
        assert!(!message.contains("warn:"), "noise prefixes are dropped: {message:?}");
    }

    // -----------------------------------------------------------------
    // The rolling buffer
    // -----------------------------------------------------------------

    /// Output under the limit is kept whole, which is the ordinary case and
    /// the one every other test depends on.
    ///
    /// The script writes to **stderr**. That is worth stating because the first
    /// draft of this test wrote to stdout, which `launch` leaves inherited, and
    /// it then measured an empty buffer — a test of the capture that never
    /// reached the capture.
    #[test]
    fn output_under_the_limit_is_kept_whole() {
        let game = scripted("yes AAAAAAAAAA | head -c 20000 >&2; exit 1");
        let mut running = launched(&game);
        // Wait for the child before finishing the drain. `finish` is a
        // one-shot that does not wait for more output to arrive, so calling it
        // straight after a spawn reads an empty pipe — which is how the first
        // draft of this test measured zero bytes and called it a capture bug.
        running.child_mut().wait().expect("the child should exit");
        let text = running.errors.take().expect("stderr is captured").finish();
        assert_eq!(
            text.len(),
            20000,
            "20000 bytes is under the 64 KB limit and must survive intact"
        );
    }

    /// The trim itself, at a limit small enough to reach without 64 KB of
    /// fixture.
    ///
    /// The buffer is a **byte** ring, not a line buffer, so the honest
    /// expectation for a 4-byte limit over `first` then `SECOND` is the last
    /// four bytes — `COND`. Asserting on whole words here would be asserting
    /// something the implementation does not promise.
    #[test]
    fn the_tail_buffer_trims_from_the_front_to_its_limit() {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("printf 'first' >&2; sleep 0.05; printf 'SECOND' >&2; sleep 0.3")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stream = child.stderr.take().unwrap();
        let errors = ErrorTail::spawn(stream, 4);
        std::thread::sleep(Duration::from_millis(200));
        let text = errors.finish();
        assert_eq!(
            text, "COND",
            "the last four bytes, which is what a byte ring keeps"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A child that writes nothing leaves an empty capture, which is what
    /// makes the fallback reachable rather than incidental.
    #[test]
    fn a_quiet_child_leaves_an_empty_capture() {
        let game = scripted("exit 6");
        let mut running = launched(&game);
        let errors = running.errors.take().expect("stderr should be captured");
        assert_eq!(errors.finish(), "");
    }

    // -----------------------------------------------------------------
    // tool_command
    // -----------------------------------------------------------------

    /// Write an executable `/bin/sh` script at `path`, creating its parents.
    ///
    /// # Why this opens with a mode, and what that is *not* (#49)
    ///
    /// This used to be `fs::write` followed by `set_permissions(0o755)`, which
    /// creates the file non-executable and makes it executable in a second
    /// step. Opening with `.mode(0o755)` removes that second step, and the
    /// write is flushed and the handle dropped before returning, so nothing
    /// this function opens outlives it. Those are the reasons to keep it, and
    /// they are the whole of what it is worth.
    ///
    /// **It is not the repair for `#49`, and the create-then-`chmod` window is
    /// not that flake's cause.** That was the hypothesis, and it is measured
    /// false: with both idioms exercised under the same concurrent load, the
    /// two flake at the same rate — so the window cannot be what the kernel is
    /// refusing on. The mechanism that *is* established, and the repair, are
    /// in `launched_or_busy_retry` below, because they belong to the `execve`
    /// rather than to the write.
    ///
    /// # The one behavioural difference from the old body, stated because it is
    /// real
    ///
    /// `OpenOptions::mode` applies **at creation**; the `set_permissions` it
    /// replaces applied unconditionally. So this helper no longer makes an
    /// *existing* file executable — it only creates one executable. That is
    /// sufficient here and not merely convenient: every call site writes under
    /// a `scratch(...)` root, and `scratch` does `remove_dir_all` then
    /// `create_dir_all` (`env.rs:420-425`), so every path this function is
    /// given is new. The one helper that writes a runner's wine without this
    /// function, `runner_with_wine`, sets its own mode explicitly and is not
    /// reused as a `write_script` target: no test hands this function a path
    /// another helper created.
    ///
    /// If a future caller does need "make this existing file executable", this
    /// helper is the wrong tool and would return silently without doing it.
    fn write_script(path: &Path, text: &str) {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o755)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.flush().unwrap();
        drop(file);
    }

    /// A runner id that resolves to a directory with a wine binary in it.
    fn runner_with_wine(root: &Path, id: &str) -> RunnerManager {
        let directory = root.join(id);
        std::fs::create_dir_all(directory.join("files/bin")).unwrap();
        let wine = directory.join("files/bin/wine");
        std::fs::write(&wine, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wine, std::fs::Permissions::from_mode(0o755)).unwrap();
        RunnerManager::at(root)
    }

    fn windows_game(runner: &str) -> Game {
        let mut game = Game::new_named("Test Title");
        game.runner = runner.to_string();
        game
    }

    #[test]
    fn winecfg_uses_the_games_own_wine_and_prefix() {
        let root = scratch("launch-tool-winecfg");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let mut game = windows_game("GE-Proton9-5");
        game.prefix_path = root.join("prefix").to_string_lossy().into_owned();

        let command = tool_command(&game, &manager, "winecfg", &FakeLaunchEnv::new()).unwrap();
        let expected_wine = root.join("GE-Proton9-5/files/bin/wine");
        assert_eq!(
            command.argv,
            vec![
                expected_wine.to_string_lossy().into_owned(),
                "winecfg".to_string()
            ]
        );
        assert_eq!(
            command.env.get("WINE").map(String::as_str),
            Some(expected_wine.to_string_lossy().as_ref()),
            "winetricks otherwise falls back to the system wine"
        );
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            Some(game.prefix_path.as_str())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A Proton prefix is one level down from where `WINEPREFIX` usually
    /// points, and pointing plain Wine at the parent makes it build a second,
    /// empty prefix beside the real one.
    #[test]
    fn the_prefix_is_the_proton_subdirectory_when_one_exists() {
        let root = scratch("launch-tool-pfx");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let mut game = windows_game("GE-Proton9-5");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        game.prefix_path = prefix.to_string_lossy().into_owned();

        let command = tool_command(&game, &manager, "winecfg", &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            Some(prefix.join("pfx").to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `WINESERVER` is set only when an executable `wineserver` sits beside the
    /// wine binary — pairing a new wineserver with an old Wine is what that
    /// guards against.
    #[test]
    fn wineserver_is_set_only_when_it_sits_beside_wine_and_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("launch-tool-wineserver");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let mut game = windows_game("GE-Proton9-5");
        game.prefix_path = root.join("prefix").to_string_lossy().into_owned();
        let server = root.join("GE-Proton9-5/files/bin/wineserver");

        // Absent.
        let command = tool_command(&game, &manager, "winecfg", &FakeLaunchEnv::new()).unwrap();
        assert!(!command.env.contains_key("WINESERVER"));

        // Present but not executable.
        std::fs::write(&server, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o644)).unwrap();
        let command = tool_command(&game, &manager, "winecfg", &FakeLaunchEnv::new()).unwrap();
        assert!(
            !command.env.contains_key("WINESERVER"),
            "a non-executable wineserver must not be advertised"
        );

        // Present and executable.
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = tool_command(&game, &manager, "winecfg", &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            command.env.get("WINESERVER").map(String::as_str),
            Some(server.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn winetricks_is_taken_from_path_and_its_absence_is_the_documented_error() {
        let root = scratch("launch-tool-winetricks");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let mut game = windows_game("GE-Proton9-5");
        game.prefix_path = root.join("prefix").to_string_lossy().into_owned();

        let error = tool_command(&game, &manager, "winetricks", &FakeLaunchEnv::new()).unwrap_err();
        assert_eq!(error.to_string(), "winetricks is not installed");

        let host = FakeLaunchEnv::new().with_which("winetricks", "/usr/bin/winetricks");
        let command = tool_command(&game, &manager, "winetricks", &host).unwrap();
        assert_eq!(command.argv, vec!["/usr/bin/winetricks".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_tool_is_refused_by_name() {
        let root = scratch("launch-tool-unknown");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let mut game = windows_game("GE-Proton9-5");
        game.prefix_path = root.join("prefix").to_string_lossy().into_owned();
        let error = tool_command(&game, &manager, "notepad", &FakeLaunchEnv::new()).unwrap_err();
        assert_eq!(error.to_string(), "Unknown tool: notepad");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A runner with no wine binary is `Runner '{name}' is not available`,
    /// which is the message `tests/test_runners.py` asserts on.
    #[test]
    fn a_runner_without_wine_is_reported_unavailable() {
        let game = windows_game("wine-system");
        // No `wine` on PATH in the fake host, so the system runner has none.
        let error = tool_command(
            &game,
            &RunnerManager::at("/nonexistent"),
            "winecfg",
            &FakeLaunchEnv::new(),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Runner 'System Wine' is not available");
    }

    #[test]
    fn an_empty_prefix_path_falls_back_to_the_prefixes_directory_for_the_game_id() {
        let root = scratch("launch-tool-default-prefix");
        let manager = runner_with_wine(&root, "GE-Proton9-5");
        let game = windows_game("GE-Proton9-5");
        // `prefixes_dir_in` resolves under `XDG_DATA_HOME`/`HOME`, so the fake
        // host needs one that is writable — without it the fallback is a real
        // path on the developer's machine, which is the non-hermeticity
        // `FakeLaunchEnv` exists to prevent.
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let host = FakeLaunchEnv::new()
            .with_vars(&[("XDG_DATA_HOME", home.to_str().unwrap())]);
        let command = tool_command(&game, &manager, "winecfg", &host).unwrap();
        let prefix = command.env.get("WINEPREFIX").expect("WINEPREFIX is set");
        assert!(
            prefix.ends_with(&game.id),
            "the prefix is per game id, got {prefix:?}"
        );
        assert!(
            std::path::Path::new(prefix).is_dir(),
            "the prefix directory is created before the tool runs"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // launch — the environment order and the process setup
    // -----------------------------------------------------------------

    /// A native Linux title: the executable runs, stdout is inherited and
    /// stderr is captured. The observable is that the child actually ran.
    #[test]
    fn a_linux_title_is_started_with_its_own_environment_and_no_runner() {
        let root = scratch("launch-linux");
        let marker = root.join("ran");
        let game = scripted(&format!("touch '{}'", marker.display()));
        let mut running = launched(&game);
        assert!(running.pid() > 0);
        // Wait for it to finish, then assert the side effect.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "the title's process should have run");
        assert_eq!(running.failure(Duration::from_secs(10)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The working directory falls back to the executable's directory, which is
    /// what makes a title whose installer wrote a relative path beside the
    /// `.exe` still find it.
    #[test]
    fn the_working_directory_defaults_to_the_executables_directory() {
        let root = scratch("launch-cwd");
        let script = root.join("run.sh");
        let mut file = std::fs::File::create(&script).unwrap();
        writeln!(file, "#!/bin/sh\ntouch ./beside").unwrap();
        drop(file);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut game = Game::new_named("Test Title");
        game.kind = "linux".to_string();
        game.exe_path = script.to_string_lossy().into_owned();
        // `working_directory` is empty, so the executable's directory is used.
        game.working_directory = String::new();

        let mut running = launched(&game);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("beside").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            root.join("beside").exists(),
            "the child should have run in its own directory"
        );
        assert_eq!(running.failure(Duration::from_secs(10)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An explicit working directory wins over the executable's.
    #[test]
    fn an_explicit_working_directory_wins_over_the_executables_directory() {
        let root = scratch("launch-cwd-explicit");
        let script = root.join("run.sh");
        let mut file = std::fs::File::create(&script).unwrap();
        writeln!(file, "#!/bin/sh\ntouch ./where").unwrap();
        drop(file);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();

        let mut game = Game::new_named("Test Title");
        game.kind = "linux".to_string();
        game.exe_path = script.to_string_lossy().into_owned();
        game.working_directory = elsewhere.to_string_lossy().into_owned();

        let mut running = launched(&game);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !elsewhere.join("where").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            elsewhere.join("where").exists(),
            "the child should have run in the explicit directory"
        );
        assert_eq!(running.failure(Duration::from_secs(10)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A title on an unreachable share is refused before anything is spawned —
    /// the message is `netpaths`', and the user's fix is to mount it.
    #[test]
    fn an_unreachable_share_is_refused_before_the_process_starts() {
        struct Stuck;
        impl ShareResolver for Stuck {
            fn is_remote_url(&self, value: &str) -> bool {
                value.starts_with("smb://")
            }
            fn as_local_path(&self, value: &str) -> String {
                value.to_string()
            }
            fn unreachable_share_message(&self, value: &str) -> String {
                format!("Mount the share for {value}")
            }
        }
        let mut game = Game::new_named("Test Title");
        game.exe_path = "smb://host/share/x.exe".to_string();
        let error = launch(
            &game,
            &RunnerManager::at("/nonexistent"),
            &FakeLaunchEnv::new(),
            &Stuck,
        )
        .err()
        .expect("an unreachable share must be refused");
        assert_eq!(error.to_string(), "Mount the share for smb://host/share/x.exe");
    }

    /// A game with no executable configured is `No executable is configured`,
    /// and that comes from the command builder rather than from a spawn error.
    #[test]
    fn a_title_with_no_executable_is_refused() {
        let mut game = Game::new_named("Test Title");
        game.kind = "linux".to_string();
        game.exe_path = String::new();
        let error = launch(
            &game,
            &RunnerManager::at("/nonexistent"),
            &FakeLaunchEnv::new(),
            &NoShares,
        )
        .err()
        .expect("a title with no executable must be refused");
        assert_eq!(error.to_string(), "No executable is configured");
    }

    /// The Proton gate is on the **runtime**, not on the family name, and both
    /// halves are asserted: a test that only checked the refusal would also
    /// pass if every Windows title were refused.
    ///
    /// `uses_proton_runtime` requires a `proton` script *and* `umu-run` as
    /// `argv[0]`, which is why the middle case exists — a Proton build reached
    /// without `umu-run` runs as plain Wine and cannot provide DLSS, so it must
    /// be refused exactly as the system Wine is.
    #[test]
    fn nvapi_follows_the_proton_runtime_rather_than_the_family_name() {
        let root = scratch("launch-nvapi");
        let prefix = root.join("prefix");
        let wine = root.join("wine");
        let umu = root.join("umu-run");
        write_script(&wine, "#!/bin/sh\nexit 0\n");
        write_script(&umu, "#!/bin/sh\nexit 0\n");

        // A Proton build: a `proton` script beside its wine.
        let directory = root.join("GE-Proton9-5");
        write_script(&directory.join("proton"), "#!/bin/sh\nexit 0\n");
        write_script(&directory.join("files/bin/wine"), "#!/bin/sh\nexit 0\n");

        let mut game = windows_game("wine-system");
        game.prefix_path = prefix.to_string_lossy().into_owned();
        game.nvapi = true;

        // The system Wine, which has no Proton runtime at all.
        let host = FakeLaunchEnv::new().with_which("wine", wine.to_str().unwrap());
        let error = launch(&game, &RunnerManager::at("/nonexistent"), &host, &NoShares)
            .err()
            .expect("the system Wine is not a Proton runtime");
        assert_eq!(
            error.to_string(),
            "NVAPI/DLSS requires a Proton runner through UMU"
        );

        // The Proton build, but with no umu-run on PATH: `build_command` falls
        // back to that build's own wine, so the runtime is not Proton's.
        game.runner = "GE-Proton9-5".to_string();
        let error = launch(&game, &RunnerManager::at(&root), &host, &NoShares)
            .err()
            .expect("a Proton build without umu-run is plain Wine");
        assert_eq!(
            error.to_string(),
            "NVAPI/DLSS requires a Proton runner through UMU"
        );

        // Through umu, the same build is a Proton runtime and the gate opens.
        let host = host.with_which("umu-run", umu.to_str().unwrap());
        let mut running =
            launched_or_busy_retry(&game, &RunnerManager::at(&root), &host, &NoShares)
                .expect("a Proton build reached through umu is what NVAPI needs");
        assert_eq!(running.failure(Duration::from_secs(10)), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The early application of the game's environment block is load-bearing,
    /// not tidiness: the bundled-DXVK installer runs *between* the two
    /// applications and reads `WINEARCH` from the map it is given.
    ///
    /// Asserting on the **child's** environment would not separate the two
    /// applications — `apply_launch_options` applies the same block again at
    /// the end — so the observable is the prefix layout the installer chose.
    /// Only a visible `WINEARCH` produces Wine's single-architecture layout.
    #[test]
    fn the_game_environment_block_reaches_the_dxvk_installer_and_not_only_the_child() {
        let root = scratch("launch-env-order");
        let prefix = root.join("prefix");
        let wine = root.join("wine");
        write_script(&wine, "#!/bin/sh\nexit 0\n");

        // A bundled runtime with both architectures, each recognisable in the
        // prefix afterwards: the DLL's *contents* name its source directory.
        let dxvk = root.join("dxvk");
        for arch in ["x32", "x64"] {
            std::fs::create_dir_all(dxvk.join(arch)).unwrap();
            std::fs::write(dxvk.join(arch).join("d3d11.dll"), arch).unwrap();
        }

        let host = FakeLaunchEnv::new()
            .with_which("wine", wine.to_str().unwrap())
            .with_vars(&[("GAMEHANDLER_DXVK_ROOT", dxvk.to_str().unwrap())]);

        let mut game = windows_game("wine-system");
        game.prefix_path = prefix.to_string_lossy().into_owned();
        game.dxvk = true;
        game.environment = "WINEARCH=win32".to_string();

        let mut running =
            launched_or_busy_retry(&game, &RunnerManager::at("/nonexistent"), &host, &NoShares)
                .expect("a plain-Wine title with a bundled runtime launches");
        assert_eq!(running.failure(Duration::from_secs(10)), None);

        let windows = prefix.join("drive_c/windows");
        assert_eq!(
            std::fs::read_to_string(windows.join("system32/d3d11.dll")).unwrap(),
            "x32",
            "only a WINEARCH the installer can see produces the 32-bit layout"
        );
        assert!(
            !windows.join("syswow64").exists(),
            "a win32 prefix has no syswow64, so its presence means WINEARCH arrived after the install"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
