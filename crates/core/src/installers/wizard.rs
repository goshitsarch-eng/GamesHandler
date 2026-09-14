//! The wizard half of the easy installer (`installers.py:299-408`).
//!
//! The `wineserver` lookup, the idle wait and the wizard poll. See
//! [`crate::installers`] for the catalog, the error type and the seams.

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use crate::installers::command::{
    INSTALL_FLUSH_SECONDS, INSTALL_HANDOFF_SECONDS, INSTALL_POLL_SECONDS,
    INSTALL_SETTLE_TIMEOUT_SECONDS, INSTALL_WAIT_EVIDENCE_SECONDS, INSTALL_WIZARD_SECONDS,
    find_prefix_exe,
};
use crate::installers::download::spawn_retrying;
use crate::runners::{LaunchEnv, Runner, wine_prefix_root};

/// The clock and the sleep the wizard poll runs on
/// (`sleep=time.sleep, clock=time.monotonic`, `installers.py:360-361`).
///
/// Injected rather than read, which is what makes the poll testable at all: the
/// behaviour worth pinning is *when* a wizard is given up on, and every arm of
/// it is measured in tens of minutes. The reference's own suite drives it on a
/// virtual clock for exactly that reason, and the alternative — a test that
/// really sleeps for six hours, or one that asserts on a shorter constant it
/// just changed — is either impossible or vacuous.
///
/// `now` need not be an absolute time: only differences are used, so the origin
/// is arbitrary. [`SystemClock`] uses process start, which is monotonic and
/// therefore immune to the system clock being set.
pub trait InstallClock {
    /// `time.monotonic()` — seconds from an arbitrary origin, never going back.
    fn now(&self) -> f64;

    /// `time.sleep(seconds)`.
    fn sleep(&self, seconds: f64);
}

/// The real clock and the real sleep.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

/// The origin [`SystemClock::now`] counts from.
static CLOCK_ORIGIN: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

impl InstallClock for SystemClock {
    fn now(&self) -> f64 {
        CLOCK_ORIGIN.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        if seconds > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }
}

/// `shutil.which`'s executable test, for the sibling lookup below.
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// The `wineserver` that goes with `runner`, if one can be found
/// (`installers.py:302-309`).
///
/// Beside the runner's own `wine` first — a Proton build ships a matching
/// `wineserver` in the same `bin` directory and that is the one whose protocol
/// version matches the prefix — and `PATH` second.
///
/// `launch_env` is the seam `shutil.which` was: the reference's own tests patch
/// `shutil.which` here, and a [`LaunchEnv`] is where that lookup already lives
/// in this port.
pub fn wineserver_binary(runner: &dyn Runner, launch_env: &dyn LaunchEnv) -> Option<PathBuf> {
    if let Some(wine) = runner.wine_binary() {
        let sibling = wine.with_file_name("wineserver");
        if is_executable_file(&sibling) {
            return Some(sibling);
        }
    }
    launch_env.which("wineserver")
}

/// Block until nothing is running in the prefix any more
/// (`installers.py:312-342`).
///
/// `wineserver -w` returns once the server owning `WINEPREFIX` has no processes
/// left, which is the only reliable "the wizard is closed" signal when the
/// installer that was started has already handed off and exited.
///
/// Returns `false` for every way of not knowing: no `wineserver`, no
/// `WINEPREFIX` to wait on, a server that could not be started, and a server
/// that outlived `timeout_seconds`. That last one is not an error — it is the
/// case the caller handles, because it means "still busy" and the poll slices
/// these waits precisely so a leftover store client cannot hide an install that
/// has already finished.
///
/// `timeout_seconds` is whole seconds because the reference's `subprocess.run`
/// is given `int`-truncated values (`:378`).
pub fn wait_for_prefix_idle(
    runner: &dyn Runner,
    env: &std::collections::BTreeMap<String, String>,
    timeout_seconds: u64,
    launch_env: &dyn LaunchEnv,
) -> bool {
    let Some(server) = wineserver_binary(runner, launch_env) else {
        return false;
    };
    let Some(prefix) = env.get("WINEPREFIX") else {
        return false;
    };
    let mut waiting_env = env.clone();
    // A Proton install put its registry under `pfx`; that is the prefix whose
    // server is worth waiting on.
    waiting_env.insert(
        "WINEPREFIX".to_string(),
        wine_prefix_root(Path::new(prefix))
            .to_string_lossy()
            .into_owned(),
    );

    let argv = vec![server.to_string_lossy().into_owned(), "-w".to_string()];
    run_with_env(&argv, &waiting_env, Duration::from_secs(timeout_seconds))
}

/// Run `argv` with `env`, discarding its output, and report whether it exited
/// within `timeout` (`subprocess.run(..., check=False, timeout=…,
/// stdout=DEVNULL, stderr=DEVNULL)`).
///
/// Any exit status counts as success — `check=False` — and the reference
/// catches `OSError` and `SubprocessError` and returns `false`, which is what
/// both of the failure arms here do.
fn run_with_env(
    argv: &[String],
    env: &std::collections::BTreeMap<String, String>,
    timeout: Duration,
) -> bool {
    use std::process::Stdio;

    const POLL: Duration = Duration::from_millis(20);

    let Some((program, arguments)) = argv.split_first() else {
        return false;
    };
    let Ok(mut child) = spawn_retrying(|| {
        ProcessCommand::new(program)
            .args(arguments)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }) else {
        return false;
    };

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

/// The idle-wait seam [`wait_for_installer`] takes.
///
/// [`wait_for_prefix_idle`] in production, a stub in the tests. A named type
/// rather than the spelled-out `&dyn Fn(…)` for the same reason
/// [`crate::runners::proton::HttpClient`]'s callbacks are: the signature is the
/// documentation of the seam.
pub type IdleWait<'a> =
    &'a dyn Fn(&dyn Runner, &std::collections::BTreeMap<String, String>, u64) -> bool;

/// Wait out a vendor wizard and return the executable it installed
/// (`installers.py:360-408`).
///
/// # Why the executable is polled for, and not the process
///
/// The process GameHandler started is usually just the bootstrapper. Battle.net,
/// the EA App, GOG Galaxy and Discord all unpack a payload, start the real
/// installer as a separate process, and exit within seconds — so waiting on that
/// first process means scanning the prefix while the user is still on the
/// wizard's first page, which is why an install that plainly succeeded used to
/// end in "could not find the executable".
///
/// # The shape of the loop
///
/// Poll for the expected executable the whole time, and *slice* the wineserver
/// wait so a leftover store client cannot hide an install that has already
/// finished. Once a wineserver has actually waited — or the busy waits add up
/// to the same evidence — the prefix is known idle and only the wizard's last
/// writes are still in flight, so the deadline is pulled in to
/// [`INSTALL_FLUSH_SECONDS`]; otherwise the window has to cover a person
/// reading and clicking through a wizard, which is
/// [`INSTALL_WIZARD_SECONDS`].
///
/// # Why `waited >= INSTALL_WAIT_EVIDENCE_SECONDS` is kept although a slice is
/// at most [`INSTALL_POLL_SECONDS`]
///
/// A single `wait_for_prefix_idle` call is bounded by `slice_timeout`, which is
/// `max(1, int(min(INSTALL_POLL_SECONDS, remaining)))` — one or two seconds —
/// so `waited` cannot exceed about two and the first clause of `confirmed` looks
/// unreachable. It is kept verbatim because it is *nearly* unreachable rather
/// than provably dead: `waited` is wall time around the whole call, which
/// includes spawning a process and waiting for the poll interval, so on a loaded
/// machine or with a `wineserver` that ignores its bound it can cross five
/// seconds. Dropping it would be a silent divergence from the reference on a
/// path neither suite reaches, which is exactly the kind of change that is
/// invisible until it matters.
///
/// `idle` is [`wait_for_prefix_idle`] in production and a stub in the tests —
/// the parameter exists because the reference's suite monkey-patches that name
/// (`tests/test_installers.py:462`), and there is no monkey-patching in Rust.
///
/// `cancelled` is the same kind of seam for a caller with a Cancel button
/// (UX-27): it is polled at the top of every pass — so the worst wait after an
/// abort is one [`INSTALL_POLL_SECONDS`] slice plus the slice's own `idle`
/// call — and answering `true` returns `None`. A cancel and a timeout share
/// the `None` deliberately: the caller that set the flag knows which one it
/// was, and no consumer of this result should treat them differently.
pub fn wait_for_installer(
    runner: &dyn Runner,
    env: &std::collections::BTreeMap<String, String>,
    prefix: &Path,
    expected: &[&str],
    clock: &dyn InstallClock,
    idle: IdleWait<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Option<PathBuf> {
    if cancelled() {
        return None;
    }
    clock.sleep(INSTALL_HANDOFF_SECONDS);
    let started = clock.now();
    let mut busy_wait = 0.0f64;
    let mut deadline = started + INSTALL_SETTLE_TIMEOUT_SECONDS;
    loop {
        if cancelled() {
            return None;
        }
        if let Some(found) = find_prefix_exe(prefix, expected) {
            return Some(found);
        }
        let now = clock.now();
        if now >= deadline {
            return None;
        }
        let remaining = deadline - now;
        // `max(1, int(min(INSTALL_POLL_SECONDS, remaining)))` — `as u64`
        // truncates toward zero and saturates, which is what `int()` does for
        // the positive values that reach here.
        let slice_timeout = (INSTALL_POLL_SECONDS.min(remaining) as u64).max(1);
        let wait_started = clock.now();
        let was_idle = idle(runner, env, slice_timeout);
        let waited = clock.now() - wait_started;
        if was_idle {
            let confirmed = waited >= INSTALL_WAIT_EVIDENCE_SECONDS
                || busy_wait >= INSTALL_WAIT_EVIDENCE_SECONDS;
            let extra = if confirmed {
                INSTALL_FLUSH_SECONDS
            } else {
                INSTALL_WIZARD_SECONDS
            };
            deadline = deadline.min(clock.now() + extra);
        } else {
            busy_wait += waited;
        }
        let leftover = INSTALL_POLL_SECONDS - waited;
        if leftover > 0.0 {
            let now = clock.now();
            if now < deadline {
                clock.sleep(leftover.min(deadline - now));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installers::download::{RunFailure, SPAWN_RETRIES, run_capturing};
    use crate::installers::tests_support::*;
    use crate::installers::{download::SPAWN_RETRIES_TAKEN, installer_by_id};
    use crate::runners::env::tests::FakeLaunchEnv;

    // The wizard poll — InstallerHandoffTests
    // (tests/test_installers.py:421-528)
    // -----------------------------------------------------------------------

    /// A clock that only moves when something sleeps or waits on it.
    ///
    /// The reference's suite does the same with `sleep=` and `clock=`
    /// arguments, because every arm of the poll is measured in tens of minutes
    /// and none of them can be reached by waiting.
    #[derive(Default)]
    struct VirtualClock {
        now: std::cell::Cell<f64>,
    }

    impl InstallClock for VirtualClock {
        fn now(&self) -> f64 {
            self.now.get()
        }
        fn sleep(&self, seconds: f64) {
            self.now.set(self.now.get() + seconds);
        }
    }

    /// One poll run: the found path, the virtual time it ended at, and the
    /// slice timeouts the idle wait was asked for.
    struct PollRun {
        found: Option<PathBuf>,
        elapsed: f64,
        idle_timeouts: Vec<u64>,
    }

    /// Drive [`wait_for_installer`] on the virtual clock
    /// (`InstallerHandoffTests._run`).
    ///
    /// `wait_seconds` is how long the wineserver appears to busy-wait before
    /// reporting idle; `install_after` is the virtual moment the wizard writes
    /// its executable. The stub honours `timeout` the way the reference's does,
    /// so a leftover store client cannot hide an already-written launcher.
    fn drive_poll(prefix: &Path, wait_seconds: f64, install_after: Option<f64>) -> PollRun {
        let clock = VirtualClock::default();
        let remaining = std::cell::Cell::new(wait_seconds);
        let idle_timeouts = std::cell::RefCell::new(Vec::new());
        let write_target = prefix.join("drive_c/Program Files (x86)/Steam/steam.exe");

        let maybe_install = || {
            if install_after.is_some_and(|after| clock.now() >= after) && !write_target.exists() {
                touch(&write_target);
            }
        };

        let idle = |_runner: &dyn Runner,
                    _env: &std::collections::BTreeMap<String, String>,
                    timeout: u64| {
            idle_timeouts.borrow_mut().push(timeout);
            let step = remaining.get().min(timeout as f64);
            remaining.set(remaining.get() - step);
            clock.now.set(clock.now.get() + step);
            maybe_install();
            remaining.get() <= 0.0
        };

        let found = wait_for_installer(
            &wine(),
            &std::collections::BTreeMap::new(),
            prefix,
            installer_by_id("steam").unwrap().expected_exe,
            &clock,
            &idle,
            &|| false,
        );
        PollRun {
            found,
            elapsed: clock.now.get(),
            idle_timeouts: idle_timeouts.into_inner(),
        }
    }

    /// A prefix with the Wine layout and no launcher in it yet.
    fn empty_prefix(label: &str) -> Scratch {
        let scratch = Scratch::new(label);
        std::fs::create_dir_all(scratch.path().join("drive_c")).unwrap();
        scratch
    }

    /// `test_polls_until_the_wizard_writes_the_executable`
    /// (`tests/test_installers.py:470-473`).
    #[test]
    fn the_poll_waits_until_the_wizard_writes_the_executable() {
        let prefix = empty_prefix("poll-late");
        let run = drive_poll(prefix.path(), 0.0, Some(60.0));
        let found = run.found.expect("the wizard's executable was never found");
        assert!(found.is_file());
        assert!(run.elapsed >= 60.0);
    }

    /// `test_gives_up_at_the_deadline_instead_of_polling_forever`
    /// (`:475-478`).
    #[test]
    fn the_poll_gives_up_at_the_deadline() {
        let prefix = empty_prefix("poll-deadline");
        let run = drive_poll(prefix.path(), 0.0, None);
        assert!(run.found.is_none());
        assert!(run.elapsed < 60.0 * 60.0, "the wizard window is bounded");
    }

    /// UX-27: a flag already set returns `None` before even the handoff sleep
    /// runs — the cheapest of the poll's three cancel points.
    ///
    /// `unreachable!` on the idle wait is the other half of the assertion:
    /// a poll that cancelled *and then* waited on wineserver would fail loud,
    /// not just late.
    #[test]
    fn a_cancelled_poll_returns_before_the_handoff() {
        let prefix = empty_prefix("poll-cancel");
        let clock = VirtualClock::default();
        let found = wait_for_installer(
            &wine(),
            &std::collections::BTreeMap::new(),
            prefix.path(),
            installer_by_id("steam").unwrap().expected_exe,
            &clock,
            &|_, _, _| unreachable!("a cancelled poll never waits on wineserver"),
            &|| true,
        );
        assert!(found.is_none());
        assert_eq!(clock.now.get(), 0.0, "the handoff sleep ran anyway");
    }

    /// UX-27: a flag that flips while the poll is running ends it at the next
    /// loop boundary — `None`, the same answer the deadline gives, just early.
    ///
    /// The flag is flipped inside the idle stub so the cancel lands at a real
    /// mid-loop moment rather than at a point of the test's choosing, and the
    /// elapsed bound is what separates "stopped early" from "ran out".
    #[test]
    fn a_cancel_during_the_poll_ends_it_at_the_next_boundary() {
        let prefix = empty_prefix("poll-cancel-mid");
        let clock = VirtualClock::default();
        let flag = std::cell::Cell::new(false);
        let idle = |_runner: &dyn Runner,
                    _env: &std::collections::BTreeMap<String, String>,
                    _timeout: u64| {
            flag.set(true);
            false
        };
        let found = wait_for_installer(
            &wine(),
            &std::collections::BTreeMap::new(),
            prefix.path(),
            installer_by_id("steam").unwrap().expected_exe,
            &clock,
            &idle,
            &|| flag.get(),
        );
        assert!(found.is_none(), "a cancelled poll reports no executable");
        assert!(
            clock.now.get() < INSTALL_SETTLE_TIMEOUT_SECONDS,
            "the poll ran to the deadline after being cancelled: {}",
            clock.now.get()
        );
    }

    /// `test_a_wineserver_that_really_waited_shortens_the_window` (`:480-483`)
    /// and `test_a_wineserver_that_returned_at_once_proves_nothing`
    /// (`:485-492`).
    ///
    /// One test, two arms, because they are the two directions of one branch:
    /// with `wait_seconds = 30` the busy waits accumulate past
    /// [`INSTALL_WAIT_EVIDENCE_SECONDS`], the prefix is known idle, and the
    /// deadline is pulled in to [`INSTALL_FLUSH_SECONDS`] — so the run ends in
    /// well under a minute. With `wait_seconds = 0` the wait proves nothing (a
    /// Proton install's wineserver runs inside umu's container where a host one
    /// cannot attach to it), so the window stays at
    /// [`INSTALL_WIZARD_SECONDS`] and the run is minutes long.
    ///
    /// This is the test that pins the *reason* `INSTALL_WAIT_EVIDENCE_SECONDS`
    /// exists, and a port that treated an instant answer as "the wizard has
    /// finished" would pass the first arm and fail the second.
    #[test]
    fn a_wineserver_wait_that_proves_something_shortens_the_window() {
        let prefix = empty_prefix("poll-idle-evidence");
        let proved = drive_poll(prefix.path(), 30.0, None);
        assert!(proved.elapsed < 60.0, "elapsed {}", proved.elapsed);

        let prefix = empty_prefix("poll-no-evidence");
        let unproved = drive_poll(prefix.path(), 0.0, None);
        assert!(
            unproved.elapsed > 60.0,
            "an instant wineserver answer was read as proof: elapsed {}",
            unproved.elapsed
        );
        assert!(
            unproved.elapsed >= INSTALL_WIZARD_SECONDS,
            "elapsed {}",
            unproved.elapsed
        );
    }

    /// `test_an_already_installed_executable_is_returned_immediately`
    /// (`:494-498`).
    #[test]
    fn an_already_installed_executable_is_returned_after_the_handoff_only() {
        let prefix = empty_prefix("poll-already");
        touch(
            &prefix
                .path()
                .join("drive_c/Program Files (x86)/Steam/steam.exe"),
        );
        let run = drive_poll(prefix.path(), 30.0, None);
        assert_eq!(
            run.found,
            Some(
                prefix
                    .path()
                    .join("drive_c/Program Files (x86)/Steam/steam.exe")
            )
        );
        assert!(
            run.idle_timeouts.is_empty(),
            "a wineserver wait happened anyway"
        );
        assert_eq!(run.elapsed, INSTALL_HANDOFF_SECONDS);
    }

    /// `test_finds_the_launcher_while_wineserver_is_still_busy` (`:500-511`).
    ///
    /// Steam, the EA App and Battle.net keep `wineserver` alive after writing
    /// the executable, so an implementation that waited for idle *before*
    /// looking would stall for hours — which is the bug this pins. The slice
    /// timeouts are asserted to be at most [`INSTALL_POLL_SECONDS`], which is
    /// what "sliced" means.
    #[test]
    fn the_launcher_is_found_while_wineserver_is_still_busy() {
        let prefix = empty_prefix("poll-busy");
        let run = drive_poll(prefix.path(), INSTALL_SETTLE_TIMEOUT_SECONDS, Some(10.0));
        let found = run.found.expect("the launcher was not found while busy");
        assert!(found.is_file());
        assert!(run.elapsed < 60.0, "elapsed {}", run.elapsed);
        assert!(!run.idle_timeouts.is_empty());
        assert!(
            run.idle_timeouts.iter().all(|timeout| *timeout <= 2),
            "un-sliced waits: {:?}",
            run.idle_timeouts
        );
    }

    // -----------------------------------------------------------------------
    // `wineserver` lookup — WineserverLookupTests
    // (tests/test_installers.py:531-564)
    // -----------------------------------------------------------------------

    /// A fake `wineserver` that records the `WINEPREFIX` it was given and then
    /// does whatever the script says.
    fn fake_wineserver(directory: &Path, record: &Path, sleep_seconds: u64) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("wineserver");
        std::fs::create_dir_all(directory).unwrap();
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$WINEPREFIX\" >> '{}'\nsleep {sleep_seconds}\nexit 0\n",
            record.display()
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// `test_prefers_the_wineserver_beside_the_runners_own_wine` and
    /// `test_falls_back_to_the_one_on_PATH` (`tests/test_installers.py:539-552`).
    ///
    /// Both arms, because a lookup that only ever returned the sibling would
    /// pass the first and a lookup that only ever consulted `PATH` would pass
    /// the second.
    #[test]
    fn the_wineserver_beside_the_runners_wine_wins_over_the_one_on_path() {
        let scratch = Scratch::new("wineserver-sibling");
        let binaries = scratch.path().join("files/bin");
        let sibling = fake_wineserver(&binaries, &scratch.path().join("sibling-record"), 0);
        let on_path = fake_wineserver(scratch.path(), &scratch.path().join("path-record"), 0);
        let env = FakeLaunchEnv::new().with_which("wineserver", &on_path.to_string_lossy());
        let runner = crate::runners::WineRunner::with_binary(Some(binaries.join("wine")));

        assert_eq!(wineserver_binary(&runner, &env), Some(sibling));

        // A sibling that is not executable is not a wineserver.
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(binaries.join("wineserver"))
            .unwrap()
            .permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(binaries.join("wineserver"), permissions).unwrap();
        assert_eq!(wineserver_binary(&runner, &env), Some(on_path));

        // And with no runner binary at all, only `PATH` is left.
        let bare = crate::runners::WineRunner::with_binary(None);
        assert_eq!(
            wineserver_binary(&bare, &env),
            Some(env.which("wineserver").unwrap())
        );
    }

    /// `test_no_wineserver_means_no_wait_rather_than_a_crash` and
    /// `test_a_prefixless_environment_is_never_waited_on`
    /// (`tests/test_installers.py:554-564`).
    ///
    /// The control arm is the third case: with a server and a prefix the wait
    /// really runs, so the two `false`s above are not a function that always
    /// says `false`.
    #[test]
    fn an_idle_wait_without_a_server_or_a_prefix_says_so_instead_of_crashing() {
        let scratch = Scratch::new("idle-guards");
        let record = scratch.path().join("record");
        let server = fake_wineserver(scratch.path(), &record, 0);
        let env = FakeLaunchEnv::new().with_which("wineserver", &server.to_string_lossy());
        let prefix = scratch.path().join("prefix");
        std::fs::create_dir_all(&prefix).unwrap();
        let variables = [(
            "WINEPREFIX".to_string(),
            prefix.to_string_lossy().into_owned(),
        )]
        .into_iter()
        .collect();

        let bare = crate::runners::WineRunner::with_binary(None);
        assert!(!wait_for_prefix_idle(
            &bare,
            &variables,
            1,
            &FakeLaunchEnv::new()
        ));
        assert!(!wait_for_prefix_idle(
            &bare,
            &std::collections::BTreeMap::new(),
            1,
            &env
        ));
        assert!(!record.exists(), "a wait ran despite the guard");

        // The control: the server exists and there is a prefix, so it runs.
        //
        // The bound is generous — a minute for a script that exits at once —
        // because the bound is *real time* and this suite runs its cases
        // concurrently. A tighter one made this test flake under load, which is
        // a statement about the machine rather than about the code: what is
        // being asserted is that the wait ran at all, and the script's exit is
        // the evidence, not how long it took.
        assert!(wait_for_prefix_idle(&bare, &variables, 60, &env));
        assert_eq!(
            std::fs::read_to_string(&record).unwrap().trim(),
            prefix.to_string_lossy()
        );
    }

    /// The wait is given the prefix the *wineserver* owns, which for a Proton
    /// install is `pfx` rather than the directory the caller named.
    ///
    /// The fake server records the `WINEPREFIX` it was handed, so this observes
    /// the substitution from outside the process rather than reading it back
    /// out of the code that made it.
    #[test]
    fn the_idle_wait_uses_the_prefix_the_wineserver_owns() {
        let scratch = Scratch::new("idle-pfx");
        let record = scratch.path().join("record");
        let server = fake_wineserver(scratch.path(), &record, 0);
        let env = FakeLaunchEnv::new().with_which("wineserver", &server.to_string_lossy());
        let prefix = scratch.path().join("prefix");
        // `wine_prefix_root` only descends when `pfx` holds a `drive_c`, which
        // is what makes it a Proton prefix rather than a directory that happens
        // to be named `pfx`.
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        let variables = [(
            "WINEPREFIX".to_string(),
            prefix.to_string_lossy().into_owned(),
        )]
        .into_iter()
        .collect();

        let bare = crate::runners::WineRunner::with_binary(None);
        assert!(wait_for_prefix_idle(&bare, &variables, 60, &env));
        assert_eq!(
            std::fs::read_to_string(&record).unwrap().trim(),
            prefix.join("pfx").to_string_lossy()
        );
    }

    /// A server that outlives its slice reports "still busy" rather than
    /// blocking the poll — which is what keeps a leftover client from hiding a
    /// finished install.
    ///
    /// The bound is real time here, so the assertion is on the order of the
    /// slice rather than on an exact figure; a `wineserver` script that sleeps
    /// for a minute against a one-second bound is unambiguous.
    #[test]
    fn a_wineserver_that_outlives_its_slice_is_not_waited_on() {
        let scratch = Scratch::new("idle-timeout");
        let record = scratch.path().join("record");
        let server = fake_wineserver(scratch.path(), &record, 60);
        let env = FakeLaunchEnv::new().with_which("wineserver", &server.to_string_lossy());
        let prefix = scratch.path().join("prefix");
        std::fs::create_dir_all(&prefix).unwrap();
        let variables = [(
            "WINEPREFIX".to_string(),
            prefix.to_string_lossy().into_owned(),
        )]
        .into_iter()
        .collect();

        let started = std::time::Instant::now();
        let bare = crate::runners::WineRunner::with_binary(None);
        assert!(!wait_for_prefix_idle(&bare, &variables, 1, &env));
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the bound was not applied"
        );
    }

    // -----------------------------------------------------------------------
    // The spawn retry — the divergence recorded at `spawn_retrying`
    // -----------------------------------------------------------------------

    /// The retries this thread has spent on `ETXTBSY`.
    fn retries_taken() -> usize {
        SPAWN_RETRIES_TAKEN.with(|count| count.get())
    }

    /// Why `spawn_retrying` exists, observed from the kernel rather than
    /// asserted about the kernel.
    ///
    /// The flake this closes was: `installers::tests::progress_is_reported_as_a_
    /// fraction_and_ends_at_one` failing with
    /// `Io(Os { code: 26, kind: ExecutableFileBusy, message: "Text file busy" })`
    /// in about one run in six. The cause is that a `fork` in a sibling test
    /// thread copies the descriptor table, so a script written a moment earlier
    /// is still open for writing *somewhere* when it is executed. This test
    /// reproduces that condition deliberately — this process holds the write
    /// descriptor itself — so the diagnosis is checked rather than believed, and
    /// it needs no timing, no thread and no load to do it.
    ///
    /// The third assertion is the other half of the contract: with the write
    /// descriptor still held the condition never clears, so what comes back is
    /// the bounded error and not a hang, and the instrument shows the budget
    /// was spent in full.
    #[test]
    fn a_held_open_script_is_executable_file_busy() {
        let scratch = Scratch::new("etxtbsy");
        let record = scratch.path().join("record");
        let script = fake_wineserver(scratch.path(), &record, 0);
        let argv = vec![script.to_string_lossy().into_owned(), "-w".to_string()];

        // The control: with nothing writing to it the script is executed and
        // the fake records the prefix it was handed.
        let Ok(ran) = run_capturing(&argv, Duration::from_secs(60)) else {
            panic!("the control run failed");
        };
        assert_eq!(ran.status, Some(0));
        assert!(record.exists(), "the control did not run the script");

        let before = retries_taken();
        let held = std::fs::OpenOptions::new()
            .write(true)
            .open(&script)
            .unwrap();
        let failure = match run_capturing(&argv, Duration::from_secs(60)) {
            Ok(_) => panic!("a file this process is writing to was executed"),
            Err(failure) => failure,
        };
        let RunFailure::Failed(error) = failure else {
            panic!("a busy file was reported as a timeout")
        };
        assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
        assert_eq!(
            retries_taken() - before,
            SPAWN_RETRIES as usize,
            "the retry budget was not spent on the error it exists for"
        );
        drop(held);
    }

    /// The recovery path: a transient `ETXTBSY` is waited out and the spawn
    /// then succeeds.
    ///
    /// The failure is injected rather than induced, because inducing it means
    /// releasing the write descriptor from another thread at the right moment
    /// and that is a test which flakes on a loaded machine — the very thing
    /// being fixed here. The injected error is the one
    /// [`a_held_open_script_is_executable_file_busy`] has just shown the kernel
    /// really produces for this condition, so what is fabricated is the
    /// *timing*, not the error.
    ///
    /// The second half is the control the first half needs: a failure that is
    /// not that transient must not be retried at all, or the loop would be a
    /// two-attempt delay on every genuine error.
    #[test]
    fn a_momentary_executable_file_busy_is_retried_until_the_spawn_succeeds() {
        let scratch = Scratch::new("etxtbsy-retry");
        let record = scratch.path().join("record");
        let script = fake_wineserver(scratch.path(), &record, 0);
        let program = script.to_string_lossy().into_owned();

        let before = retries_taken();
        let mut attempts = 0;
        let child = spawn_retrying(|| {
            attempts += 1;
            if attempts <= 2 {
                return Err(std::io::Error::from(std::io::ErrorKind::ExecutableFileBusy));
            }
            std::process::Command::new(&program).arg("-w").spawn()
        })
        .expect("the retry never reached the attempt that works");
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(attempts, 3);
        assert_eq!(retries_taken() - before, 2);
        assert!(
            record.exists(),
            "the script that finally ran was not the fake"
        );

        let before = retries_taken();
        let mut attempts = 0;
        let error = spawn_retrying(|| {
            attempts += 1;
            Err::<std::process::Child, _>(std::io::Error::from(std::io::ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(attempts, 1, "a permanent failure was retried");
        assert_eq!(retries_taken(), before);
    }
}
