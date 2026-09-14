//! The spawn-and-bound helpers the installer halves share.
//!
//! `spawn_retrying`, `run_capturing`, `CommandOutput`, `RunFailure` and
//! `SPAWN_RETRIES` are not download code — they are the subprocess machinery
//! the signature verifier (`signature.rs`), the prefix-idle poller
//! (`wizard.rs`) and the download's own tool calls all sit on. They were the
//! middle third of `download.rs` until `ARCH-11` cut the file along its real
//! seams; a `signature.rs` that imported its process spawner from
//! `download.rs` would have declared the dependency backwards.
//!
//! Nothing here is `pub` beyond `pub(crate)`: these are the crate's
//! internals, and the signature API is [`verify_installer_authenticity`].

use std::process::{Child, Command as ProcessCommand};
use std::time::Duration;

/// What running a child produced.
pub(crate) struct CommandOutput {
    /// The exit status, or `None` when the child was killed by a signal.
    pub status: Option<i32>,
    /// `stdout` and `stderr` merged, as `stderr=subprocess.STDOUT` gives.
    pub text: String,
}

/// Why a child could not be run to completion.
pub(crate) enum RunFailure {
    /// It outlived its bound and was killed.
    TimedOut,
    /// It could not be spawned, or its streams could not be read.
    Failed(std::io::Error),
}

/// How many times a spawn is retried after `ETXTBSY`, and how long between
/// attempts. Five retries over ten milliseconds is fifty milliseconds — three
/// orders of magnitude more than the window they are covering, and small enough
/// that a caller's own timeout still means what it says.
pub(crate) const SPAWN_RETRIES: u32 = 5;
/// See [`SPAWN_RETRIES`].
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// `Command::spawn`, retrying the one failure that is not a property of the
/// program being run.
///
/// `execve` reports `ETXTBSY` while the target file is open for writing by
/// *any* process. In a threaded program that is a race rather than a fact about
/// the file: `fork` copies the whole descriptor table, so a child another
/// thread is midway through spawning inherits the write descriptor some third
/// thread had open on a file it has already closed and is about to execute. The
/// window is the microseconds between that `fork` and its `exec`, so the
/// condition is always momentary — which is exactly why it is safe to wait it
/// out and why it is never worth reporting.
///
/// # This is a divergence from the reference, and it is here for a named reason
///
/// `subprocess.run` spawns once and lets the `OSError` out. The port cannot do
/// that and keep a hermetic suite: the fakes below are shell scripts written a
/// moment before they are executed, in a test binary where four hundred cases
/// fork concurrently, so the reference's single-spawn behaviour turned into a
/// flake at roughly one run in six — measured, not feared (see
/// `a_held_open_executable_is_retried_instead_of_reported`). The alternative
/// was to sprinkle blind retries through the tests, which would re-run whole
/// downloads and would be unable to tell this transient apart from a real
/// refusal; retrying at the one place that can see the errno is both narrower
/// and cheaper.
///
/// It is bounded, and it is keyed on that single error: every other reason a
/// spawn can fail is returned on the first attempt, and once the retries are
/// exhausted the original error is returned unchanged. So the only observable
/// difference from the reference is on a call that was going to fail for a
/// reason that had already stopped being true.
/// The spawn is the caller's closure rather than an argv this builds itself, so
/// the retry can be exercised without a file that is genuinely busy — the
/// recovery path is then covered by a case that cannot flake, and the diagnosis
/// above is covered separately by one that observes the kernel's own error
/// (`a_held_open_script_is_executable_file_busy`).
///
/// The environment is **inherited** here and cleared by the two callers instead,
/// because clearing it for the tests below would clear it for the fake verifier
/// scripts those tests execute — and a script needs `PATH` to find its
/// interpreter. That is a real constraint rather than a preference: it is why
/// `SEC-09`'s fix is an `env_clear` at the spawn site rather than a default
/// inside this helper.
pub(crate) fn spawn_retrying(
    mut attempt_spawn: impl FnMut() -> std::io::Result<Child>,
) -> std::io::Result<Child> {
    let mut retries = 0;
    loop {
        match attempt_spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && retries < SPAWN_RETRIES =>
            {
                retries += 1;
                #[cfg(test)]
                SPAWN_RETRIES_TAKEN.with(|count| count.set(count.get() + 1));
                std::thread::sleep(SPAWN_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

// How many spawns this thread has retried — the instrument for the divergence
// above, so the test that covers it observes the retry *firing* rather than
// inferring it from a call that happened to succeed.
//
// A thread-local rather than an atomic because the spawn it counts is on the
// caller's own thread, and a counter shared with the other four hundred tests
// would be unreadable. (Written as `//` rather than `///` because rustdoc does
// not document macro invocations — the note the directory-read counter above
// also carries.)
#[cfg(test)]
thread_local! {
    pub(crate) static SPAWN_RETRIES_TAKEN: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Run `argv` with merged output, bounded by `timeout`
/// (`subprocess.run(..., stderr=subprocess.STDOUT, timeout=…)`).
///
/// The bound is implemented by polling [`std::process::Child::try_wait`],
/// because `std` has no `wait_timeout` and `core` may not take an async runtime
/// (D-03) — the same shape as [`crate::runners::run_version`], and it carries
/// the same caveat: output is read only after the child exits, so a child that
/// writes more than a pipe buffer's worth blocks instead of finishing. Ninety
/// seconds is the reference's own bound and the verifier prints a page, so the
/// branch is out of reach in practice; it is written down rather than assumed.
///
/// # The environment is cleared, and that is a divergence
///
/// `SECURITY.md` `SEC-09`. `subprocess.run` with no `env=` inherits the
/// launcher's whole environment, and this port copied that here while every
/// other spawn in the tree replaces it ([`wait_for_prefix_idle`] below,
/// `start_prefix_tool`, `run_installer`). The child is `osslsigncode`, and it
/// needs **nothing** from this process: the binary is resolved by the caller's
/// `which` into an absolute path (`:1516`), the trust root and the file under
/// test are absolute paths in the argv, and the program reads no configuration.
/// So an empty environment is a complete one, and it means a hostile
/// `LD_PRELOAD`, `OPENSSL_CONF` or `SSL_CERT_FILE` in the launcher's environment
/// cannot redirect the verifier's answer — which matters more here than
/// elsewhere, because this process's *output* is a security decision the
/// launcher then trusts.
///
/// The cost is that `$PATH` is gone, which is why this is only safe for a
/// program already resolved to a path. `view::plugins::run_to_completion` — the
/// other inheriting spawn `SEC-09` named — clears too, and clears for the same
/// reason; what it does *not* inherit is `osslsigncode`'s happy position of
/// being convenient to test. It needed `install_command` and
/// `privileged_command` rather than a temporary file, so it proves the same
/// claim a level up: the verifier's environment is asserted here, the package
/// manager's argv is asserted in `view::plugins`'s own tests.
pub(crate) fn run_capturing(
    argv: &[String],
    timeout: Duration,
) -> Result<CommandOutput, RunFailure> {
    use std::process::Stdio;

    const POLL: Duration = Duration::from_millis(20);

    let Some((program, arguments)) = argv.split_first() else {
        return Err(RunFailure::Failed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        )));
    };
    let mut child = spawn_retrying(|| {
        ProcessCommand::new(program)
            .args(arguments)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })
    .map_err(RunFailure::Failed)?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunFailure::Failed(error));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunFailure::TimedOut);
                }
                std::thread::sleep(POLL);
            }
        }
    }

    let output = child.wait_with_output().map_err(RunFailure::Failed)?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CommandOutput {
        status: output.status.code(),
        text,
    })
}
