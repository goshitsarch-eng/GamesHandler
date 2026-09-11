//! Runner management: Proton and Wine builds, archives, and launch options.
//!
//! A port of `gamehandler/runners.py` (1547 lines, 43 public names) plus the
//! half of `tests/test_security.py` that guards it. It is the largest module in
//! the project and the only one where part of the surface is a security
//! boundary rather than an application feature.
//!
//! # Layout
//!
//! `docs/migration/architecture.md` §1.1 splits the Python module into a
//! directory, and the split is by *trust boundary* rather than by file size —
//! `archive.rs` handles untrusted bytes and the rest does not:
//!
//! * [`archive`] — bounded extraction of an untrusted tarball, and validation
//!   of the staged tree. Landed first (PLAN.md R-3), because it is the only
//!   part where a defect is a vulnerability, and because it is the part a
//!   summary would get wrong.
//! * [`families`] — the runner catalogue and asset selection.
//! * [`env`] — [`env::LaunchEnv`], the injected seam for everything the runner
//!   code needs from outside the process (DECISIONS D-27).
//! * [`shell`] — POSIX word splitting (`shlex.split`), shared with
//!   `parse_env_block` in `launch_opts`.
//! * the module root — the [`Runner`] trait, [`WineRunner`], [`ProtonRunner`]
//!   and [`RunnerManager`], plus the prefix-layout helpers.
//! * `launch_opts` — the launch-option matrix.
//! * `proton` — `ProtonManager`: download, stage, rename into place.
//! * `launch` — process launch and early-exit reporting.
//! * `desktop` — `.desktop` shortcut generation.

pub mod archive;
pub mod env;
pub mod families;
pub mod launch;
pub mod launch_opts;
pub mod proton;
pub mod shell;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::models::Game;
use crate::paths;

pub use archive::METADATA_NAME;
pub use archive::{ArchiveError, Limits};
pub use env::{LaunchEnv, SystemLaunchEnv};
pub use launch_opts::{
    DXVK_DLL_OVERRIDES, VKD3D_DLL_OVERRIDES, WINED3D_DLL_OVERRIDES,
};
pub use shell::{split_posix, ShellError};

/// The built-in Wine runner's id.
///
/// Defined in [`crate::models`] because `settings.py` and `models.py` both
/// hardcode the literal, and re-exported here so this module's surface still
/// matches `runners.__all__`.
pub use crate::models::SYSTEM_WINE;

/// Subdirectory Proton builds its Wine prefix in.
///
/// `umu-run` hands `WINEPREFIX` to Proton as `STEAM_COMPAT_DATA_PATH`, and
/// Proton builds its prefix in a `pfx` subdirectory of that. So a title
/// installed through a Proton runner lives in `<prefix>/pfx/drive_c`, while the
/// same directory used by raw Wine holds `drive_c` directly.
pub const PROTON_PREFIX_SUBDIR: &str = "pfx";

/// `User-Agent` sent to GitHub and to runner downloads.
pub const USER_AGENT: &str = "GameHandler";

/// Version of the bundled DXVK runtime.
pub const DXVK_VERSION: &str = "3.0.2";

/// Where the bundled DXVK runtime is installed in the Flatpak.
pub const DXVK_ROOT: &str = "/app/share/gamehandler/dxvk";

/// Environment variable that overrides [`DXVK_ROOT`], for source installs.
pub const DXVK_ROOT_ENV: &str = "GAMEHANDLER_DXVK_ROOT";

/// The DLL overrides that keep a Wine prefix quiet on first run.
///
/// `winemenubuilder.exe=d` disables the menu-entry builder, and the empty
/// `mscoree,mshtml=` entries stop Wine from offering Mono and Gecko dialogs.
pub const DEFAULT_DLL_OVERRIDES: &str = "winemenubuilder.exe=d;mscoree,mshtml=";

/// `WINEDEBUG=-all` — Wine is unusably chatty otherwise.
pub const DEFAULT_WINEDEBUG: &str = "-all";

/// Seconds a launched title gets to stay alive before it counts as running.
pub const LAUNCH_GRACE_SECONDS: f64 = 6.0;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Anything the runner layer can fail with.
///
/// The `Display` text is the Python error message, because the launch path
/// surfaces these strings to the user — `launch()`'s early-exit report and the
/// toasts in `bridge.py` show them verbatim, and `tests/test_runners.py` asserts
/// on substrings of them (`assertRaisesRegex(RuntimeError, "NVAPI/DLSS requires
/// a Proton runner")`). Keeping them byte-identical is therefore part of the
/// port, not a nicety.
#[derive(Debug)]
pub enum RunnerError {
    /// `Runner '{name}' is not available`.
    NotAvailable { name: String },
    /// Gamescope was requested but is not installed.
    ///
    /// Carries the Flathub extension name because that is the user's fix, and
    /// `test_gamescope_missing_fails_instead_of_silently_ignoring_toggle`
    /// asserts on it.
    GamescopeMissing,
    /// `NVAPI/DLSS requires a Proton runner through UMU`.
    NvapiNeedsProton,
    /// `FSR requires a compatible Proton runner through UMU`.
    FsrNeedsProton,
    /// `Wayland mode requires a Proton runner through UMU`.
    WaylandNeedsProton,
    /// `No executable is configured`.
    NoExecutable,
    /// `DXVK requires a configured Wine prefix`.
    DxvkNeedsPrefix,
    /// `Bundled DXVK runtime is unavailable`.
    DxvkUnavailable,
    /// `winetricks is not installed`.
    WinetricksMissing,
    /// `Unknown tool: {tool}`.
    UnknownTool { tool: String },
    /// A runner directory already exists for this release.
    AlreadyInstalled { id: String },
    /// `System Wine cannot be uninstalled`.
    SystemWineCannotBeUninstalled,
    /// `Runner archive exceeds the download size limit`.
    ///
    /// Raised for a declared `Content-Length` over the cap *before* the body is
    /// read, and again if the streamed body crosses the cap — so a server that
    /// lies about its length still cannot fill the disk.
    ArchiveTooLarge,
    /// `Runner archive contains an unsafe top-level link`.
    StagedTopLevelLink,
    /// `Could not locate one usable runner in the staged archive`.
    ///
    /// The archive was valid and extracted; it simply does not contain one
    /// directory that is a runner, which is a property of the release rather
    /// than of the download.
    NoUsableRunner,
    /// A library entry lives on a network share that has no local mount.
    ///
    /// The message is `netpaths.unreachable_share_message`, which is built from
    /// the URL the user can recognise rather than from a fixed sentence — so
    /// this variant carries it rather than formatting one of its own.
    UnreachableShare { message: String },
    /// A GitHub request failed, or its response was not what the API promises.
    ///
    /// Carries the message rather than a status code because the messages here
    /// are Python's, verbatim — `"Unexpected GitHub releases response"`,
    /// `"Runner download has an invalid Content-Length"` — and they reach the
    /// user through the same toasts as the other variants.
    Http { message: String },
    /// The archive layer refused the download or the staged tree.
    Archive(ArchiveError),
    /// A launch argument string could not be split (Python's `shlex` error).
    Shell(ShellError),
    /// An operation on the filesystem or a child process failed.
    Io(std::io::Error),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnerError::NotAvailable { name } => {
                write!(f, "Runner '{name}' is not available")
            }
            // Ported verbatim from `runners.py:1253-1256`, including the
            // extension name and the `//25.08` branch pin.
            RunnerError::GamescopeMissing => f.write_str(
                "Gamescope is enabled but unavailable. Flatpak users must install \
                 org.freedesktop.Platform.VulkanLayer.gamescope//25.08 from Flathub.",
            ),
            RunnerError::NvapiNeedsProton => {
                f.write_str("NVAPI/DLSS requires a Proton runner through UMU")
            }
            RunnerError::FsrNeedsProton => {
                f.write_str("FSR requires a compatible Proton runner through UMU")
            }
            RunnerError::WaylandNeedsProton => {
                f.write_str("Wayland mode requires a Proton runner through UMU")
            }
            RunnerError::NoExecutable => f.write_str("No executable is configured"),
            RunnerError::DxvkNeedsPrefix => {
                f.write_str("DXVK requires a configured Wine prefix")
            }
            RunnerError::DxvkUnavailable => {
                f.write_str("Bundled DXVK runtime is unavailable")
            }
            RunnerError::WinetricksMissing => f.write_str("winetricks is not installed"),
            RunnerError::UnknownTool { tool } => write!(f, "Unknown tool: {tool}"),
            RunnerError::AlreadyInstalled { id } => {
                write!(f, "Runner '{id}' is already installed")
            }
            RunnerError::SystemWineCannotBeUninstalled => {
                f.write_str("System Wine cannot be uninstalled")
            }
            RunnerError::ArchiveTooLarge => {
                f.write_str("Runner archive exceeds the download size limit")
            }
            RunnerError::StagedTopLevelLink => {
                f.write_str("Runner archive contains an unsafe top-level link")
            }
            RunnerError::NoUsableRunner => {
                f.write_str("Could not locate one usable runner in the staged archive")
            }
            RunnerError::UnreachableShare { message } => f.write_str(message),
            RunnerError::Http { message } => f.write_str(message),
            // The wrapped errors already carry Python's message.
            RunnerError::Archive(error) => error.fmt(f),
            RunnerError::Shell(error) => error.fmt(f),
            RunnerError::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunnerError::Archive(error) => Some(error),
            RunnerError::Shell(error) => Some(error),
            RunnerError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ArchiveError> for RunnerError {
    fn from(error: ArchiveError) -> Self {
        RunnerError::Archive(error)
    }
}

impl From<ShellError> for RunnerError {
    fn from(error: ShellError) -> Self {
        RunnerError::Shell(error)
    }
}

impl From<std::io::Error> for RunnerError {
    fn from(error: std::io::Error) -> Self {
        RunnerError::Io(error)
    }
}

// ---------------------------------------------------------------------------
// Shared path primitives
// ---------------------------------------------------------------------------

/// `PurePosixPath(name).name` — the last component, or `""` when there is none.
///
/// Not `rsplit('/').next()`. Python drops empty and `.` components *before*
/// taking the last one, so `"trailing/"` is `"trailing"` rather than `""`, and
/// `"a/./b"` is `"b"`. A `..` component survives, so `"a/.."` is `".."`.
///
/// This is the primitive behind `safe_archive_name` (the untrusted GitHub asset
/// name) and `uses_proton_runtime` (comparing `argv[0]`'s basename to
/// `"umu-run"`), so it is defined once here and called from both.
pub fn pure_posix_name(name: &str) -> String {
    name.split('/')
        .rfind(|part| !part.is_empty() && *part != ".")
        .unwrap_or_default()
        .to_string()
}

/// Every `drive_c` that exists under `prefix`, Proton's layout first.
///
/// Port of `prefix_drive_cs` (`runners.py:345-349`). Two entries at most, in
/// that order, so `[0]` is the one a prefix "actually installed into".
pub fn prefix_drive_cs(prefix: &Path) -> Vec<PathBuf> {
    [
        prefix.join(PROTON_PREFIX_SUBDIR).join("drive_c"),
        prefix.join("drive_c"),
    ]
    .into_iter()
    .filter(|candidate| candidate.is_dir())
    .collect()
}

/// The `drive_c` a prefix actually installed into, if it has one.
pub fn prefix_drive_c(prefix: &Path) -> Option<PathBuf> {
    prefix_drive_cs(prefix).into_iter().next()
}

/// Where `WINEPREFIX` must point for raw Wine to see `prefix`'s files.
///
/// A prefix that Proton created keeps its registry and `drive_c` one level
/// down. Pointing plain Wine at the parent makes it build a second, empty
/// prefix beside the real one, which looks exactly like a game that installed
/// fine and then refuses to start — see `ProtonPrefixIndirectionTests`.
pub fn wine_prefix_root(prefix: &Path) -> PathBuf {
    let proton = prefix.join(PROTON_PREFIX_SUBDIR);
    if proton.join("drive_c").is_dir() {
        proton
    } else {
        prefix.to_path_buf()
    }
}

/// A game's Wine prefix: its own `prefix_path`, or `$prefixes_dir/<id>`.
///
/// `game.prefix_path or str(config.prefixes_dir() / game.id)` — Python's `or`
/// is a truthiness test, so the empty string falls through to the default.
fn game_prefix(game: &Game, env: &dyn paths::Env) -> PathBuf {
    if game.prefix_path.is_empty() {
        paths::prefixes_dir_in(env).join(&game.id)
    } else {
        PathBuf::from(&game.prefix_path)
    }
}

/// The `(argv, environment)` pair a launch needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// `shlex.split` a game's argument string, skipping it when empty.
///
/// Python writes `if game.arguments: argv.extend(shlex.split(game.arguments))`,
/// so an empty string is not an error and a malformed one is. Splitting
/// unconditionally would make `""` produce no words either, but it would also
/// run the splitter on strings Python never looks at — and the error is
/// user-visible, so the guard is load-bearing.
fn push_arguments(argv: &mut Vec<String>, arguments: &str) -> Result<(), RunnerError> {
    if arguments.is_empty() {
        return Ok(());
    }
    argv.extend(split_posix(arguments)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// The runner hierarchy
// ---------------------------------------------------------------------------

/// Base behaviour for anything that can launch a Windows executable.
///
/// Python's `Runner` declares `id`/`name`/`family_id` as class attributes and
/// four methods that raise `NotImplementedError`; the port has one trait, and
/// every method is required except [`Runner::proton_script`], whose default
/// `None` *is* the `isinstance(…, ProtonRunner)` test that
/// [`uses_proton_runtime`] needs.
pub trait Runner {
    /// Stable identifier: [`SYSTEM_WINE`], or the directory name under the
    /// runners directory.
    fn id(&self) -> &str;

    /// Display name.
    fn name(&self) -> &str;

    /// Catalogue family this build came from, or `"system"`.
    fn family_id(&self) -> &str;

    /// Whether this runner can launch anything right now.
    fn is_available(&self) -> bool;

    /// A human-readable version string, or `"unknown"`.
    fn version(&self) -> String;

    /// The Wine binary inside this runner, if it has one.
    fn wine_binary(&self) -> Option<PathBuf>;

    /// `<root>/proton` when this build ships Proton's launcher script.
    ///
    /// `None` for everything that is not a [`ProtonRunner`], which is how
    /// [`uses_proton_runtime`] asks Python's
    /// `isinstance(runner, ProtonRunner) and runner.proton_script() is not None`.
    fn proton_script(&self) -> Option<PathBuf> {
        None
    }

    /// The `(argv, environment)` used to launch `game`.
    fn build_command(&self, game: &Game, env: &dyn LaunchEnv) -> Result<Command, RunnerError>;
}

/// The Wine installation provided by the host system (`runners.py:411-445`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WineRunner {
    binary: Option<PathBuf>,
}

impl WineRunner {
    /// `WineRunner()` — autodetect `wine` on `PATH`.
    pub fn detect(env: &dyn LaunchEnv) -> Self {
        Self {
            binary: env.which("wine"),
        }
    }

    /// `WineRunner(binary=…)`, including the explicit `None` case.
    ///
    /// Python's sentinel is `_AUTODETECT = object()`, so `binary=None` means
    /// "there is no Wine" and must not be confused with "look it up". Two
    /// constructors rather than an `Option<Option<PathBuf>>`.
    pub fn with_binary(binary: Option<PathBuf>) -> Self {
        Self { binary }
    }

    /// The detected binary, if any.
    pub fn binary(&self) -> Option<&Path> {
        self.binary.as_deref()
    }
}

impl Runner for WineRunner {
    fn id(&self) -> &str {
        SYSTEM_WINE
    }

    fn name(&self) -> &str {
        "System Wine"
    }

    fn family_id(&self) -> &str {
        "system"
    }

    fn is_available(&self) -> bool {
        self.binary.is_some()
    }

    fn wine_binary(&self) -> Option<PathBuf> {
        self.binary.clone()
    }

    /// `wine --version`, bounded at 15 seconds.
    ///
    /// Python catches `(OSError, subprocess.SubprocessError)` and returns
    /// `"unknown"`, which covers a missing binary and the timeout; the port does
    /// the same. Its `stdout.strip() or stderr.strip() or "unknown"` chain is a
    /// truthiness test at each step, so whitespace-only output falls through.
    ///
    /// One divergence, noted rather than hidden: Python decodes with the locale
    /// encoding and *strict* errors, so invalid UTF-8 in the output raises
    /// `UnicodeDecodeError` — a `ValueError`, which the except clause above does
    /// not catch, so it escapes. The port decodes lossily and returns the
    /// string. `wine --version` printing non-UTF-8 is the only way to reach it.
    fn version(&self) -> String {
        let Some(binary) = self.binary.as_deref() else {
            // Python's `if not self._binary: return "not installed"` — a
            // different message from the "unknown" fallback below.
            return "not installed".to_string();
        };
        run_version(binary).unwrap_or_else(|| "unknown".to_string())
    }

    fn build_command(&self, game: &Game, env: &dyn LaunchEnv) -> Result<Command, RunnerError> {
        let Some(wine) = self.binary.as_deref() else {
            return Err(RunnerError::NotAvailable {
                name: self.name().to_string(),
            });
        };

        let prefix = game_prefix(game, env);
        let mut command_env = env.environ();
        // Raw Wine has no Proton indirection, so it needs the directory that
        // actually holds drive_c — including when Proton created it.
        command_env.insert(
            "WINEPREFIX".to_string(),
            wine_prefix_root(&prefix).to_string_lossy().into_owned(),
        );
        // `setdefault`: an inherited value wins, as in Python.
        command_env
            .entry("WINEDLLOVERRIDES".to_string())
            .or_insert_with(|| DEFAULT_DLL_OVERRIDES.to_string());
        command_env
            .entry("WINEDEBUG".to_string())
            .or_insert_with(|| DEFAULT_WINEDEBUG.to_string());

        let mut argv = vec![wine.to_string_lossy().into_owned()];
        if !game.exe_path.is_empty() {
            argv.push(game.exe_path.clone());
        }
        push_arguments(&mut argv, &game.arguments)?;
        Ok(Command {
            argv,
            env: command_env,
        })
    }
}

/// Run `<binary> --version` with a 15-second bound, returning its trimmed
/// output, or `None` on any failure or timeout.
///
/// The timeout is implemented by polling [`std::process::Child::try_wait`],
/// because `std` has no `wait_timeout` and `core` may not take an async runtime
/// (DECISIONS D-03).
///
/// **The 64 KiB caveat.** Output is read only after the child has exited, so a
/// child that writes more than a pipe buffer blocks and is killed at the
/// deadline, reporting `None` where Python would report the version. Python's
/// `subprocess.run` avoids that by reading concurrently. `wine --version`
/// prints one line, so the branch is unreachable in practice; it is written
/// down because "unreachable" is a claim the next reader should be able to
/// check rather than assume.
fn run_version(binary: &Path) -> Option<String> {
    use std::process::{Command as ProcessCommand, Stdio};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(15);
    const POLL: Duration = Duration::from_millis(20);

    let mut child = ProcessCommand::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            // A wait failure means we cannot tell whether it finished; kill it
            // rather than block, matching the "unknown" outcome of an OSError.
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(POLL);
            }
        }
    }

    let output = child.wait_with_output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Some(stderr);
    }
    None
}

/// A downloaded Proton or Wine build extracted under the runners directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtonRunner {
    path: PathBuf,
    id: String,
    family_id: String,
    name: String,
}

impl ProtonRunner {
    /// `ProtonRunner(path, family_id="", display_name="")`.
    ///
    /// The family is read from the build's own
    /// [`METADATA_NAME`] file when the caller does not supply one, which is how
    /// a discovered runner knows what it is.
    pub fn new(path: impl Into<PathBuf>, family_id: &str, display_name: &str) -> Self {
        let path = path.into();
        let fallback_name = pure_posix_name(&path.to_string_lossy());
        let id = fallback_name.clone();
        Self {
            family_id: if family_id.is_empty() {
                read_family_id(&path)
            } else {
                family_id.to_string()
            },
            name: if display_name.is_empty() {
                fallback_name
            } else {
                display_name.to_string()
            },
            id,
            path,
        }
    }

    /// [`Self::new`] with the family read from the build's metadata.
    pub fn discovered(path: impl Into<PathBuf>) -> Self {
        Self::new(path, "", "")
    }

    /// The extracted build's root directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `"Proton-GE"`, `"System Wine"`, or `"Downloaded runner"`.
    ///
    /// Python's first branch tests membership in the catalogue, so an id that
    /// is merely *non-empty* still falls through to `"Downloaded runner"`.
    pub fn family_label(&self) -> String {
        if !self.family_id.is_empty()
            && let Ok(family) = families::family_by_id(&self.family_id)
        {
            return family.name.to_string();
        }
        if self.family_id == "system" {
            return "System Wine".to_string();
        }
        "Downloaded runner".to_string()
    }
}

impl Runner for ProtonRunner {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn family_id(&self) -> &str {
        &self.family_id
    }

    fn is_available(&self) -> bool {
        self.wine_binary().is_some() || self.proton_script().is_some()
    }

    fn version(&self) -> String {
        // Python returns the directory name — the build's tag, which is more
        // informative than a version probe and needs no subprocess.
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn wine_binary(&self) -> Option<PathBuf> {
        families::find_wine_binary(&self.path)
    }

    /// `<path>/proton`, if it exists.
    ///
    /// `.exists()` rather than `.is_file()`, matching `runners.py:461-463`: a
    /// *directory* named `proton` satisfies this. Two other places in the same
    /// module use `.is_file()` for the same path
    /// (`_resolve_staged`, `_validate_staged_runner`), and the inconsistency is
    /// Python's, so it is preserved rather than tidied.
    fn proton_script(&self) -> Option<PathBuf> {
        let candidate = self.path.join("proton");
        candidate.exists().then_some(candidate)
    }

    fn build_command(&self, game: &Game, env: &dyn LaunchEnv) -> Result<Command, RunnerError> {
        let prefix = game_prefix(game, env);
        let mut command_env = env.environ();
        // Unlike raw Wine, this is the *parent* — Proton appends `pfx` itself
        // when it is reached through umu.
        command_env.insert("WINEPREFIX".to_string(), prefix.to_string_lossy().into_owned());
        command_env
            .entry("WINEDLLOVERRIDES".to_string())
            .or_insert_with(|| DEFAULT_DLL_OVERRIDES.to_string());
        command_env
            .entry("WINEDEBUG".to_string())
            .or_insert_with(|| DEFAULT_WINEDEBUG.to_string());

        let umu = env.which("umu-run");
        if let (Some(umu), Some(_proton)) = (umu, self.proton_script()) {
            command_env.insert(
                "PROTONPATH".to_string(),
                self.path.to_string_lossy().into_owned(),
            );
            // A short, stable id: Proton keys its per-game state off it.
            command_env.insert(
                "GAMEID".to_string(),
                format!("gh-{}", take_chars(&game.id, 8)),
            );
            command_env.insert("STORE".to_string(), "none".to_string());
            let mut argv = vec![umu.to_string_lossy().into_owned()];
            if !game.exe_path.is_empty() {
                argv.push(game.exe_path.clone());
            }
            push_arguments(&mut argv, &game.arguments)?;
            return Ok(Command {
                argv,
                env: command_env,
            });
        }

        let Some(wine) = self.wine_binary() else {
            return Err(RunnerError::NotAvailable {
                name: self.name().to_string(),
            });
        };
        // Without umu this build runs as plain Wine, which cannot see through
        // Proton's "pfx" indirection on its own.
        command_env.insert(
            "WINEPREFIX".to_string(),
            wine_prefix_root(&prefix).to_string_lossy().into_owned(),
        );
        let mut argv = vec![wine.to_string_lossy().into_owned()];
        if !game.exe_path.is_empty() {
            argv.push(game.exe_path.clone());
        }
        push_arguments(&mut argv, &game.arguments)?;
        Ok(Command {
            argv,
            env: command_env,
        })
    }
}

/// The first `count` *characters* of `text`.
///
/// Python slices strings by code point, so `game.id[:8]` is not "the first
/// eight bytes" — an id containing a multi-byte character would otherwise be
/// cut mid-character. Ids are hex today, which is exactly why the difference
/// would never be noticed until it was.
fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

// ---------------------------------------------------------------------------
// Launch failure reporting (B-07)
// ---------------------------------------------------------------------------

/// How many trailing lines of a runner's output are kept. `runners.py:1297`.
const ERROR_TAIL_LINES: usize = 4;

/// The message cap, in *characters*, matching Python's slice. `runners.py:1298`.
const ERROR_MESSAGE_CHARS: usize = 240;

/// Prefixes that mark a line as noise. `runners.py:1300`.
///
/// Matched case-insensitively against the *start* of the stripped line, which
/// is why `"WARN: caps"` survives — see [`readable_error`].
const NOISE_PREFIXES: [&str; 4] = ["fixme:", "warn:", "trace:", "info:"];

/// The line breaks [`python_splitlines`] recognises beyond `\n`.
///
/// `str.splitlines()` is **not** `str.split("\n")`. It also breaks on these,
/// and the port has to say so explicitly because Rust's [`str::lines`] and a
/// plain `split('\n')` both disagree with Python here. A runner that emits a
/// form feed between two errors — Wine's own output does contain `\x0c` — would
/// otherwise be read as one long line instead of two.
const PYTHON_LINE_BREAKS: [char; 8] = [
    '\u{0b}',
    '\u{0c}',
    '\u{1c}',
    '\u{1d}',
    '\u{1e}',
    '\u{85}',
    '\u{2028}',
    '\u{2029}',
];

/// Python's `str.splitlines()`, including the breaks Rust does not treat as
/// line boundaries.
///
/// The full set is `\n \r \v \f \x1c \x1d \x1e \x85 \u2028 \u2029`, plus
/// the `\r\n` pair which counts once. Callers here pass text that has already
/// had `\r` replaced by `\n` (as Python's own caller does), so `\r` and `\r\n`
/// cannot reach this function from [`readable_error`] — but the splitter is
/// written for the general case so it is not a trap if it is reused.
fn python_splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let is_break = if c == '\r' {
            // A CRLF pair is one break, not two, so the LF is consumed here.
            if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                chars.next();
            }
            true
        } else if c == '\n' {
            true
        } else {
            PYTHON_LINE_BREAKS.contains(&c)
        };
        if is_break {
            lines.push(&text[start..index]);
            start = index + c.len_utf8();
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Reduce a runner's output to the part worth putting in a toast.
/// `_readable_error` (`runners.py:1304`).
///
/// Three rules, and each one has a case in the vector corpus:
///
/// * Every line is stripped, so indentation does not defeat the noise filter.
/// * Noise lines are dropped — **unless every line is noise**, in which case
///   the unstripped-of-noise list is used instead. A runner that emits nothing
///   but `warn:` lines still gets *something* shown rather than an empty string
///   that would fall back to the generic status message.
/// * The last four useful lines are joined with spaces and truncated to 240
///   characters. Joining is what makes a multi-line Wine error readable in a
///   single-line toast.
///
/// The prefix test is `line.lower().startswith(...)`, so `"WARN: caps"` is
/// noise but `"xwarn:"` and `" err: "` (stripped to `"err:"`) are not — `err:`
/// is not in the noise set at all, which is deliberate: it is where Wine puts
/// real failures.
pub fn readable_error(text: &str) -> String {
    let normalized = text.replace('\r', "\n");
    let lines: Vec<&str> = python_splitlines(&normalized)
        .into_iter()
        .map(str::trim)
        .collect();
    let useful: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !line.is_empty() && !is_noise(line))
        .collect();
    // The fallback deliberately re-derives from *all* lines rather than
    // reusing `lines`: Python's `or` picks the non-noise list only when it is
    // non-empty, and the alternative is the empty-stripped list.
    let tail: Vec<&str> = if useful.is_empty() {
        lines.iter().copied().filter(|line| !line.is_empty()).collect()
    } else {
        useful
    };
    let start = tail.len().saturating_sub(ERROR_TAIL_LINES);
    take_chars(&tail[start..].join(" "), ERROR_MESSAGE_CHARS)
}

/// The message a failed launch reports. `LaunchedGame.failure` (`runners.py:1358`).
///
/// `None` means the title did not fail: either it is still running when the
/// grace period expires, or it exited **zero**. Anything else is reported — with
/// the runner's own error text if there is any, and the status line otherwise.
///
/// # B-07 — the port deliberately diverges here, in reliability
///
/// The Python original reads its captured stderr like this:
///
/// ```text
/// code = self.process.wait(timeout=timeout)      # reaped, NOT drained
/// detail = _readable_error(self.errors.text())   # races the drain thread
/// return detail or f"the runner exited with status {code}"
/// ```
///
/// `wait()` returning proves the child was *reaped*. It says nothing about the
/// stderr pipe having been drained, and `_ErrorTail._drain` runs on a separate
/// daemon thread that `failure()` never joins. So the buffer can still be empty
/// when it is read, and `or` then silently substitutes the generic status line —
/// **the actual Wine/Proton error is lost**, which is the one thing the caller
/// needed. Measured at 90/300 runs under load (FINDINGS B-07); `python-tests`
/// was intermittently red because of it.
///
/// The fix is the *ordering*, and it is not available in Python: a daemon
/// thread has no join and `failure()` runs on the UI path with a six-second
/// deadline. Rust has the tools, so the port uses them — the reader is awaited
/// to completion before the buffer is read, and this function only ever sees
/// text that was already fully collected.
///
/// This is a divergence in **reliability, not in output format**. The
/// observable contract is unchanged and is what the port honours: *report the
/// runner's error text if there is any, else the status line*. The Python app
/// meets it only most of the time; the port meets it always. So the port never
/// degrades to the fallback when the runner did write something, and a test for
/// this asserts the error text is **present** — a test that accepted either
/// outcome would pass while the race was live, which is precisely how the
/// Python suite hid this for so long.
///
/// The ordering itself is [`crate::runners::mod`]'s
/// `the_stderr_drain_is_joined_before_the_text_is_read`; this function is the
/// pure half, so the message shape is also pinned by the vector corpus
/// (`launch_failure_text`, answered by `run_runners_vectors.py` with the join
/// applied).
pub fn failure_message(code: i32, detail: &str) -> Option<String> {
    if code == 0 {
        return None;
    }
    if detail.is_empty() {
        return Some(format!("the runner exited with status {code}"));
    }
    Some(detail.to_string())
}

/// Whether a stripped line carries one of [`NOISE_PREFIXES`].
fn is_noise(line: &str) -> bool {
    let lowered = line.to_lowercase();
    NOISE_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
}

// ---------------------------------------------------------------------------
// Runner metadata
// ---------------------------------------------------------------------------

/// Read `<root>/.gamehandler.json`, or an empty map when it is unusable.
///
/// Port of `_read_metadata` (`runners.py:692-702`). Three guards, each
/// deliberate:
///
/// * A **symlinked root** is not followed at all — the metadata is read from a
///   real build directory or not at all.
/// * A missing or unparseable file is not an error, and a JSON document that is
///   not an object is discarded. `json.loads("[]")` succeeds and has no
///   `.get`, so Python's `isinstance(data, dict)` check is load-bearing.
/// * A file that is not valid UTF-8 is treated as unreadable. Python's
///   `read_text(encoding="utf-8")` raises `UnicodeDecodeError`, which is a
///   `ValueError` and so is **not** caught by the `except (JSONDecodeError,
///   OSError)` — it escapes and takes down whatever called `is_installed`.
///   This is the same divergence D-20 settles for `Settings.load`, for the same
///   reason: a metadata file we wrote is not a reason to crash.
pub fn read_metadata(root: &Path) -> serde_json::Map<String, serde_json::Value> {
    if root.is_symlink() {
        return serde_json::Map::new();
    }
    let meta = root.join(METADATA_NAME);
    if !meta.exists() {
        return serde_json::Map::new();
    }
    let Ok(bytes) = std::fs::read(&meta) else {
        return serde_json::Map::new();
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return serde_json::Map::new();
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// The `family` recorded in a build's metadata, or `""`.
fn read_family_id(root: &Path) -> String {
    match read_metadata(root).get("family") {
        // `str(... or "")` — a falsy `family` (0, false, null) becomes "".
        Some(value) if crate::runners::families::is_truthy(value) => {
            crate::runners::families::python_str(value)
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// RunnerManager
// ---------------------------------------------------------------------------

/// Discovers the available runners: system Wine plus installed builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerManager {
    runners_directory: PathBuf,
}

impl RunnerManager {
    /// `RunnerManager()` — the configured runners directory.
    pub fn new(env: &dyn LaunchEnv) -> Self {
        Self {
            runners_directory: paths::runners_dir_in(env),
        }
    }

    /// `RunnerManager(runners_directory)`.
    pub fn at(runners_directory: impl Into<PathBuf>) -> Self {
        Self {
            runners_directory: runners_directory.into(),
        }
    }

    /// The directory scanned for installed builds.
    pub fn runners_directory(&self) -> &Path {
        &self.runners_directory
    }

    /// The host's Wine, autodetected.
    pub fn system_wine(&self, env: &dyn LaunchEnv) -> WineRunner {
        WineRunner::detect(env)
    }

    /// Every installed build that is usable, sorted by directory name.
    ///
    /// Python sorts the *paths*, which is a string comparison, so a directory
    /// whose name is not valid UTF-8 is unrepresentable there; on Unix the
    /// byte ordering this uses agrees with code-point ordering for anything
    /// Python could have produced.
    pub fn installed_protons(&self) -> Vec<ProtonRunner> {
        if !self.runners_directory.exists() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(&self.runners_directory) else {
            return Vec::new();
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        children.sort();
        children
            .into_iter()
            .filter(|child| child.is_dir())
            .map(ProtonRunner::discovered)
            .filter(|runner| runner.is_available())
            .collect()
    }

    /// System Wine first, then every installed build.
    pub fn all_runners(&self, env: &dyn LaunchEnv) -> Vec<Box<dyn Runner>> {
        let mut runners: Vec<Box<dyn Runner>> = vec![Box::new(self.system_wine(env))];
        runners.extend(
            self.installed_protons()
                .into_iter()
                .map(|runner| Box::new(runner) as Box<dyn Runner>),
        );
        runners
    }

    /// Resolve a runner id, falling back to system Wine.
    ///
    /// The fallback is deliberate and load-bearing: a game whose Proton build
    /// was deleted must still *try* to launch rather than refuse, because the
    /// alternative is a game that cannot be started at all. Note that the test
    /// is `.exists()`, not `.is_dir()` — a *file* named like a runner id is
    /// still resolved, and its `ProtonRunner` reports itself unavailable.
    pub fn get(&self, runner_id: &str, env: &dyn LaunchEnv) -> Box<dyn Runner> {
        if runner_id == SYSTEM_WINE || runner_id.is_empty() {
            return Box::new(self.system_wine(env));
        }
        let candidate = self.runners_directory.join(runner_id);
        if candidate.exists() {
            return Box::new(ProtonRunner::discovered(candidate));
        }
        Box::new(self.system_wine(env))
    }

    /// The label shown for a runner id: `"Proton-GE · Proton-GE"` style.
    pub fn label(&self, runner_id: &str) -> String {
        if runner_id == SYSTEM_WINE || runner_id.is_empty() {
            return "System Wine".to_string();
        }
        let candidate = self.runners_directory.join(runner_id);
        // Only a Proton runner has a family label, and only a *different* one
        // is worth appending — hence the comparison against the name.
        if candidate.exists() {
            let runner = ProtonRunner::discovered(candidate);
            let family = runner.family_label();
            if family != runner.name() {
                return format!("{} · {}", runner.name(), family);
            }
            return runner.name().to_string();
        }
        // Python calls `self.get(...)`, which falls back to system Wine.
        "System Wine".to_string()
    }

    /// `(id, label)` pairs suitable for a dropdown, System Wine first.
    pub fn choices(&self) -> Vec<(String, String)> {
        let mut items = vec![(SYSTEM_WINE.to_string(), "System Wine".to_string())];
        for runner in self.installed_protons() {
            items.push((runner.id().to_string(), runner.name().to_string()));
        }
        items
    }
}

/// Whether `argv` reaches a real Proton build through UMU.
///
/// Proton-only environment variables are meaningless, and misleading in a bug
/// report, when the command is plain Wine. Both `launch()` and the easy
/// installers gate on this, so it lives in one place.
///
/// Python's `isinstance(runner, ProtonRunner) and runner.proton_script() is not
/// None` is [`Runner::proton_script`]'s default `None`; the `argv[0]` basename
/// comparison is [`pure_posix_name`], because Python writes `Path(argv[0]).name`
/// and that is not a split on the last separator.
pub fn uses_proton_runtime(runner: &dyn Runner, argv: &[String]) -> bool {
    runner.proton_script().is_some()
        && argv
            .first()
            .is_some_and(|first| pure_posix_name(first) == "umu-run")
}

/// The non-empty value of `key`, as Python's `dict.get(key, "").strip()`.
///
/// Used by the launch path for `WINEPREFIX`, where Python's `if prefix_value:`
/// treats whitespace-only as absent.
pub fn stripped_var(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::env::tests::FakeLaunchEnv;

    /// A scratch directory under the real temp dir, cleaned on entry so a
    /// crashed earlier run cannot make this one pass or fail.
    fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("gh-runners-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The environment every path helper needs: a data home inside `root`.
    ///
    /// `paths` resolves on demand rather than caching, so pointing
    /// `GAMEHANDLER_DATA_HOME` at a scratch directory is enough to make
    /// `prefixes_dir()` and `runners_dir()` land there.
    fn env_at(root: &Path) -> FakeLaunchEnv {
        FakeLaunchEnv::new().with_vars(&[(
            "GAMEHANDLER_DATA_HOME",
            root.to_str().unwrap(),
        )])
    }

    /// A game that will not be launched, only turned into a command line.
    fn game(name: &str) -> Game {
        Game::new_named(name)
    }

    fn write_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    /// A Proton-shaped build: `<root>/files/bin/wine`, and optionally `proton`.
    fn make_proton(root: &Path, with_proton_script: bool) {
        write_executable(&root.join("files/bin/wine"));
        if with_proton_script {
            std::fs::write(root.join("proton"), "#!/usr/bin/env python\n").unwrap();
        }
    }

    // -- WineRunner::build_command (tests/test_runners.py:35-57) --------------

    #[test]
    fn build_command_uses_the_wine_binary_and_the_executable() {
        let root = scratch("wine-command");
        let env = env_at(&root);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));
        let mut value = game("App");
        value.exe_path = "/games/app.exe".to_string();
        value.arguments = "-fullscreen -dx11".to_string();

        let command = runner.build_command(&value, &env).unwrap();
        assert_eq!(
            command.argv,
            ["/usr/bin/wine", "/games/app.exe", "-fullscreen", "-dx11"]
        );
        assert!(command.env.contains_key("WINEPREFIX"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_prefix_defaults_to_one_directory_per_game() {
        let root = scratch("prefix-default");
        let env = env_at(&root);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));
        let value = game("App");

        let command = runner.build_command(&value, &env).unwrap();
        // Python asserts `.endswith(game.id)`; the directory is
        // `$prefixes_dir/<id>`, and checking the parent as well pins that the
        // id is a *path component* rather than a suffix of the whole path.
        let prefix = PathBuf::from(command.env.get("WINEPREFIX").unwrap());
        assert_eq!(prefix.file_name().unwrap().to_string_lossy(), value.id);
        assert!(prefix.starts_with(root.join("prefixes")), "got {prefix:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_explicit_prefix_is_respected() {
        let root = scratch("prefix-explicit");
        let env = env_at(&root);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));
        let mut value = game("App");
        value.prefix_path = "/custom/prefix".to_string();

        let command = runner.build_command(&value, &env).unwrap();
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            Some("/custom/prefix")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unavailable_runner_refuses_to_build_a_command() {
        let root = scratch("wine-missing");
        let env = env_at(&root);
        let runner = WineRunner::with_binary(None);
        let error = runner.build_command(&game("App"), &env).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Runner 'System Wine' is not available"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wine_runner_reports_not_installed_rather_than_unknown() {
        // Two distinct messages in Python, and they mean different things:
        // "not installed" is a fact about the host, "unknown" is a failed
        // probe. Collapsing them loses the distinction in the UI.
        let root = scratch("wine-version");
        assert_eq!(WineRunner::with_binary(None).version(), "not installed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn build_command_keeps_an_inherited_dll_override() {
        // `env.setdefault` — an inherited WINEDLLOVERRIDES wins, and the
        // default is only a fallback. Writing it unconditionally would clobber
        // a user's own override on every launch.
        let root = scratch("wine-setdefault");
        let env = env_at(&root).with_environ(&[("WINEDLLOVERRIDES", "custom=d")]);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));

        let command = runner.build_command(&game("App"), &env).unwrap();
        assert_eq!(
            command.env.get("WINEDLLOVERRIDES").map(String::as_str),
            Some("custom=d")
        );
        assert_eq!(
            command.env.get("WINEDEBUG").map(String::as_str),
            Some(DEFAULT_WINEDEBUG)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_arguments_are_an_error_not_a_silent_truncation() {
        // `shlex.split` raises, and the port must not quietly drop the
        // arguments — a game launched with half its command line is worse than
        // one that refuses to launch.
        let root = scratch("wine-bad-args");
        let env = env_at(&root);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));
        let mut value = game("App");
        value.arguments = "-DNAME='unterminated".to_string();

        let error = runner.build_command(&value, &env).unwrap_err();
        assert_eq!(error.to_string(), "No closing quotation");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- RunnerManager (tests/test_runners.py:60-101, 244-266) ---------------

    #[test]
    fn discovers_installed_builds() {
        let root = scratch("manager-discovers");
        make_proton(&root.join("GE-Proton9-5"), false);
        make_proton(&root.join("GE-Proton8-32"), false);
        let manager = RunnerManager::at(&root);

        let ids: Vec<String> = manager
            .installed_protons()
            .iter()
            .map(|runner| runner.id().to_string())
            .collect();
        assert!(ids.contains(&"GE-Proton9-5".to_string()));
        assert!(ids.contains(&"GE-Proton8-32".to_string()));
        // Sorted, so the dropdown order is stable across machines.
        assert_eq!(ids, ["GE-Proton8-32", "GE-Proton9-5"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_without_a_usable_runner_is_not_offered() {
        let root = scratch("manager-unusable");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        write_executable(&root.join("a-file-proton"));
        let manager = RunnerManager::at(&root);
        assert!(manager.installed_protons().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn choices_put_system_wine_first() {
        let root = scratch("manager-choices");
        make_proton(&root.join("GE-Proton9-5"), false);
        let manager = RunnerManager::at(&root);

        let choices = manager.choices();
        assert_eq!(choices[0], (SYSTEM_WINE.to_string(), "System Wine".to_string()));
        assert!(choices.iter().any(|(id, _)| id == "GE-Proton9-5"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_proton_build_launches_with_its_bundled_wine() {
        let root = scratch("manager-bundled-wine");
        make_proton(&root.join("GE-Proton9-5"), false);
        let env = env_at(&root);
        let manager = RunnerManager::at(&root);
        let mut value = game("App");
        value.exe_path = "/g/app.exe".to_string();
        value.runner = "GE-Proton9-5".to_string();

        let runner = manager.get("GE-Proton9-5", &env);
        let command = runner.build_command(&value, &env).unwrap();
        assert!(
            command.argv[0].ends_with("GE-Proton9-5/files/bin/wine"),
            "got {:?}",
            command.argv[0]
        );
        assert_eq!(command.argv[1], "/g/app.exe");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_runner_falls_back_to_system_wine() {
        // The fallback is the point: a game whose Proton build was deleted must
        // still try to launch, because the alternative is a game that cannot be
        // started at all.
        let root = scratch("manager-fallback");
        let env = env_at(&root);
        let manager = RunnerManager::at(&root);
        let runner = manager.get("GE-Proton-does-not-exist", &env);
        assert_eq!(runner.id(), SYSTEM_WINE);
        assert_eq!(runner.name(), "System Wine");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_wine_layout_is_discovered_too() {
        // Kron4ek builds put wine at `bin/wine` with no `files/` level.
        let root = scratch("manager-kron4ek");
        write_executable(&root.join("wine-vanilla-11.15/bin/wine"));
        let env = env_at(&root);
        let manager = RunnerManager::at(&root);
        let mut value = game("App");
        value.exe_path = "/g/app.exe".to_string();
        value.runner = "wine-vanilla-11.15".to_string();

        let ids: Vec<String> = manager
            .installed_protons()
            .iter()
            .map(|runner| runner.id().to_string())
            .collect();
        assert!(ids.contains(&"wine-vanilla-11.15".to_string()));
        let runner = manager.get("wine-vanilla-11.15", &env);
        let command = runner.build_command(&value, &env).unwrap();
        assert!(command.argv[0].ends_with("wine-vanilla-11.15/bin/wine"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_runner_label_appends_its_family_only_when_they_differ() {
        let root = scratch("manager-label");
        make_proton(&root.join("GE-Proton9-5"), false);
        // The metadata is what names the family; without it the label is just
        // the directory name.
        std::fs::write(
            root.join("GE-Proton9-5").join(METADATA_NAME),
            r#"{"family": "proton-ge", "tag": "GE-Proton9-5"}"#,
        )
        .unwrap();
        let manager = RunnerManager::at(&root);

        assert_eq!(manager.label(SYSTEM_WINE), "System Wine");
        assert_eq!(manager.label(""), "System Wine");
        assert_eq!(manager.label("GE-Proton9-5"), "GE-Proton9-5 · Proton-GE");
        // A build with no metadata is labelled `name · Downloaded runner` —
        // verified against Python rather than assumed, because the intuitive
        // reading ("no family, so no suffix") is wrong: `family_label()` has a
        // generic fallback and `label()` compares it to the name.
        make_proton(&root.join("anonymous"), false);
        assert_eq!(manager.label("anonymous"), "anonymous · Downloaded runner");
        // An id that does not exist falls back to System Wine, as `get` does.
        assert_eq!(manager.label("nope"), "System Wine");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_runner_reports_system_wine_first() {
        let root = scratch("manager-all");
        make_proton(&root.join("GE-Proton9-5"), false);
        let env = env_at(&root);
        let manager = RunnerManager::at(&root);
        let all = manager.all_runners(&env);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id(), SYSTEM_WINE);
        assert_eq!(all[1].id(), "GE-Proton9-5");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- ProtonRunner's two launch paths (tests/test_runners.py:678-696) -----

    #[test]
    fn umu_keeps_the_parent_because_proton_appends_pfx_itself() {
        let root = scratch("umu-parent");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        let build = root.join("GE-Proton");
        make_proton(&build, true);

        let env = env_at(&root).with_which("umu-run", "/usr/bin/umu-run");
        let mut value = game("Steam");
        value.exe_path = "/x.exe".to_string();
        value.prefix_path = prefix.to_str().unwrap().to_string();

        let command = ProtonRunner::discovered(&build)
            .build_command(&value, &env)
            .unwrap();
        assert_eq!(pure_posix_name(&command.argv[0]), "umu-run");
        // The parent, not `pfx` — Proton appends pfx itself.
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            prefix.to_str()
        );
        assert_eq!(
            command.env.get("PROTONPATH").map(String::as_str),
            build.to_str()
        );
        assert_eq!(command.env.get("STORE").map(String::as_str), Some("none"));
        assert_eq!(
            command.env.get("GAMEID").map(String::as_str),
            Some(format!("gh-{}", &value.id[..8]).as_str())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_proton_build_without_umu_falls_back_to_the_inner_prefix() {
        // The bug this whole area exists for: plain Wine pointed at the parent
        // builds a second, empty prefix beside the real one.
        let root = scratch("no-umu");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        let build = root.join("Wine-Proton");
        make_proton(&build, false);

        let env = env_at(&root);
        let mut value = game("Steam");
        value.exe_path = "/x.exe".to_string();
        value.prefix_path = prefix.to_str().unwrap().to_string();

        let command = ProtonRunner::discovered(&build)
            .build_command(&value, &env)
            .unwrap();
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            prefix.join("pfx").to_str()
        );
        assert!(!command.env.contains_key("PROTONPATH"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_build_with_no_wine_at_all_is_unavailable() {
        let root = scratch("proton-empty");
        let build = root.join("empty-build");
        std::fs::create_dir_all(&build).unwrap();
        let runner = ProtonRunner::discovered(&build);
        assert!(!runner.is_available());
        let error = runner
            .build_command(&game("App"), &env_at(&root))
            .unwrap_err();
        assert_eq!(error.to_string(), "Runner 'empty-build' is not available");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- prefix layout (tests/test_runners.py:646-704) -----------------------

    #[test]
    fn wine_prefix_root_descends_into_a_proton_prefix() {
        let root = scratch("prefix-root");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        assert_eq!(wine_prefix_root(&prefix), prefix.join("pfx"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wine_prefix_root_leaves_a_plain_wine_prefix_alone() {
        let root = scratch("prefix-plain");
        let plain = root.join("plain");
        std::fs::create_dir_all(plain.join("drive_c")).unwrap();
        assert_eq!(wine_prefix_root(&plain), plain);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_untouched_directory_is_used_as_is() {
        let root = scratch("prefix-empty");
        let empty = root.join("empty");
        assert_eq!(wine_prefix_root(&empty), empty);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_proton_layout_wins_when_both_drive_cs_exist() {
        // `prefix_drive_cs` returns Proton's first, and `prefix_drive_c` takes
        // the head — so a prefix with both layouts resolves the way Proton
        // would see it.
        let root = scratch("prefix-both");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();

        let found = prefix_drive_cs(&prefix);
        assert_eq!(found, [prefix.join("pfx/drive_c"), prefix.join("drive_c")]);
        assert_eq!(prefix_drive_c(&prefix), Some(prefix.join("pfx/drive_c")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_prefix_with_no_drive_c_has_none() {
        let root = scratch("prefix-none");
        assert!(prefix_drive_cs(&root).is_empty());
        assert_eq!(prefix_drive_c(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn system_wine_launches_into_the_prefix_proton_built() {
        let root = scratch("system-into-pfx");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        let env = env_at(&root);
        let runner = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));
        let mut value = game("Steam");
        value.prefix_path = prefix.to_str().unwrap().to_string();

        let command = runner.build_command(&value, &env).unwrap();
        assert_eq!(
            command.env.get("WINEPREFIX").map(String::as_str),
            prefix.join("pfx").to_str()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- uses_proton_runtime -------------------------------------------------

    #[test]
    fn proton_runtime_needs_both_a_script_and_a_umu_command() {
        let root = scratch("uses-proton");
        let with_script = root.join("GE-Proton");
        make_proton(&with_script, true);
        let without_script = root.join("Wine-Proton");
        make_proton(&without_script, false);

        let umu = vec!["/usr/bin/umu-run".to_string(), "/g/app.exe".to_string()];
        let wine = vec!["/usr/bin/wine".to_string(), "/g/app.exe".to_string()];
        let proton = ProtonRunner::discovered(&with_script);
        let plain = ProtonRunner::discovered(&without_script);
        let system = WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")));

        assert!(uses_proton_runtime(&proton, &umu));
        // A Proton build reached as plain Wine is not running Proton.
        assert!(!uses_proton_runtime(&proton, &wine));
        // Neither is a raw build whose argv[0] happens to be umu-run.
        assert!(!uses_proton_runtime(&plain, &umu));
        // Nor system Wine.
        assert!(!uses_proton_runtime(&system, &umu));
        // An empty argv cannot be running anything.
        assert!(!uses_proton_runtime(&proton, &[]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_umu_check_uses_the_pure_posix_basename() {
        // Python writes `Path(argv[0]).name`, which is not a split on the last
        // separator: a trailing slash yields the last *real* component, and a
        // `..` survives. Both cases are reachable — argv[0] comes from
        // `shutil.which`, but the same helper guards a hand-edited command.
        assert_eq!(pure_posix_name("/usr/bin/umu-run"), "umu-run");
        assert_eq!(pure_posix_name("umu-run"), "umu-run");
        assert_eq!(pure_posix_name("/usr/bin/umu-run/"), "umu-run");
        assert_eq!(pure_posix_name("/usr/bin/"), "bin");
        assert_eq!(pure_posix_name("/usr/./bin/umu-run"), "umu-run");
        assert_eq!(pure_posix_name("/usr/bin/.."), "..");
        assert_eq!(pure_posix_name(""), "");
        assert_eq!(pure_posix_name("/"), "");
    }

    // -- metadata ------------------------------------------------------------

    #[test]
    fn metadata_round_trips_a_family_and_survives_a_broken_file() {
        let root = scratch("metadata");
        std::fs::write(
            root.join(METADATA_NAME),
            r#"{"family": "proton-ge", "tag": "GE-Proton9-5"}"#,
        )
        .unwrap();
        assert_eq!(read_family_id(&root), "proton-ge");
        assert_eq!(
            ProtonRunner::discovered(&root).family_label(),
            "Proton-GE"
        );

        // Every unusable shape returns an empty map rather than raising: a
        // metadata file must never be the reason the runners page fails.
        for (label, bytes) in [
            ("not_json", &b"{{{"[..]),
            ("not_an_object", &b"[]"[..]),
            ("empty", &b""[..]),
            ("null", &b"null"[..]),
            ("invalid_utf8", &b"{\"family\": \"\xff\"}"[..]),
        ] {
            std::fs::write(root.join(METADATA_NAME), bytes).unwrap();
            assert!(
                read_metadata(&root).is_empty(),
                "{label} should not parse to a map"
            );
            assert_eq!(read_family_id(&root), "", "{label}");
        }

        // A falsy `family` is `str(... or "")`, so 0 and false are "" too.
        std::fs::write(root.join(METADATA_NAME), r#"{"family": 0}"#).unwrap();
        assert_eq!(read_family_id(&root), "");
        std::fs::write(root.join(METADATA_NAME), r#"{"family": false}"#).unwrap();
        assert_eq!(read_family_id(&root), "");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_symlinked_build_root_has_no_metadata() {
        // `_read_metadata` refuses to follow a symlinked root, so a link that
        // points at a directory with metadata does not inherit its family.
        let root = scratch("metadata-symlink");
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join(METADATA_NAME), r#"{"family": "proton-ge"}"#).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(read_family_id(&real), "proton-ge");
        assert!(read_metadata(&link).is_empty());
        assert_eq!(read_family_id(&link), "");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_family_falls_back_to_the_generic_label() {
        let root = scratch("metadata-unknown-family");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(METADATA_NAME), r#"{"family": "not-a-family"}"#).unwrap();
        let runner = ProtonRunner::discovered(&root);
        assert_eq!(runner.family_id(), "not-a-family");
        // Unknown ids are not errors here — the label is generic and the build
        // is still usable, which is what matters.
        assert_eq!(runner.family_label(), "Downloaded runner");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_display_name_and_family_can_be_supplied() {
        let root = scratch("metadata-explicit");
        let build = root.join("install-dir");
        std::fs::create_dir_all(&build).unwrap();
        let runner = ProtonRunner::new(&build, "wine-vanilla", "Wine 11.15");
        assert_eq!(runner.id(), "install-dir");
        assert_eq!(runner.name(), "Wine 11.15");
        assert_eq!(runner.family_id(), "wine-vanilla");
        assert_eq!(runner.family_label(), "Wine-Vanilla");
        assert_eq!(runner.version(), "install-dir");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_system_family_id_labels_as_system_wine() {
        // Not in the catalogue, but the `"system"` case has its own branch.
        let root = scratch("metadata-system");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(METADATA_NAME), r#"{"family": "system"}"#).unwrap();
        assert_eq!(ProtonRunner::discovered(&root).family_label(), "System Wine");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- stripped_var --------------------------------------------------------

    #[test]
    fn a_whitespace_only_variable_is_absent() {
        // Python's `if prefix_value:` after `.strip()`, which the DXVK
        // installer and the launch path both depend on.
        let mut env = BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "   ".to_string());
        assert_eq!(stripped_var(&env, "WINEPREFIX"), None);
        env.insert("WINEPREFIX".to_string(), " /prefix ".to_string());
        assert_eq!(stripped_var(&env, "WINEPREFIX").as_deref(), Some("/prefix"));
         assert_eq!(stripped_var(&env, "ABSENT"), None);
    }

    // -----------------------------------------------------------------------
    // B-07 — the ordering, against a real child
    // -----------------------------------------------------------------------
    //
    // The corpus pins the *message contract*; this pins the *ordering* that
    // makes the message available at all. The two are split because the corpus
    // replay must stay pure, and the ordering can only be observed with a
    // process. Neither test covers B-07 alone.

    /// Drain a child's stderr on a thread and wait for the child, joining the
    /// reader **before** returning the captured text.
    ///
    /// This is the shape the port must use, and the shape Python cannot.
    /// `Child::wait()` (as in `std::process` and as in Python's `Popen.wait`)
    /// returns when the child is *reaped*; the pipe is a separate object with a
    /// separate reader, and nothing about the reap implies the reader has seen
    /// the last bytes. The join is the whole fix.
    ///
    /// `wait_with_output()` is the stdlib spelling of the same idea, and it is
    /// what the production caller already uses: `WineRunner::run_version`
    /// above. This hand-built version exists so the test can assert on the
    /// ordering directly rather than trusting that a combined call orders it
    /// correctly — the whole of B-07 is that the ordering is easy to get
    /// wrong, so the test for it does not delegate the thing under test.
    fn capture_stderr_joined(child: &mut std::process::Child) -> (std::process::ExitStatus, String) {
        use std::io::Read;

        let mut pipe = child.stderr.take().expect("stderr was piped");
        let reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            // A read error on a closed pipe is not a failure to report; the
            // text collected so far is still worth having.
            let _ = pipe.read_to_end(&mut buffer);
            buffer
        });
        let status = child.wait().expect("the child should be waitable");
        // The line B-07 is about. Without it, `buffer` below can be empty or
        // short even though the child wrote a complete message.
        let bytes = reader.join().expect("the reader thread should not panic");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The child this test launches, chosen so B-07 is visible.
    ///
    /// `sh` rather than the game handler binary: the point is a process that
    /// writes a diagnostic and exits non-zero, and every Unix has `/bin/sh`.
    /// The payload is far larger than a pipe buffer is comfortable with, so the
    /// child cannot finish writing until the reader has been draining for a
    /// while — which is exactly the window in which the un-joined ordering
    /// loses the tail. The *last* lines are what `readable_error` keeps, so a
    /// torn capture shows up as missing text rather than as a shorter message.
    fn failing_child() -> std::process::Command {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("i=0; while [ $i -lt 4000 ]; do echo \"err:module:import_dll Library $i not found\" >&2; i=$((i+1)); done; exit 3")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        command
    }

    #[test]
    fn the_stderr_drain_is_joined_before_the_text_is_read() {
        // B-07's assertion, and the coordinator's requirement stated exactly:
        // the error text must be **present**, never the fallback. Accepting
        // either outcome is how the Python suite stayed green while the race
        // was live, so there is deliberately no `assert!(text == real ||
        // text == fallback)` here — the fallback is a failure.
        let mut child = failing_child().spawn().expect("/bin/sh should be runnable");
        let (status, captured) = capture_stderr_joined(&mut child);

        // The child really did fail, and really did write a diagnostic.
        assert_eq!(status.code(), Some(3), "the child should exit 3");
        assert!(
            !captured.is_empty(),
            "the child wrote a diagnostic to stderr; an empty capture here is \
             the drain race, not a quiet child"
        );

        let message = failure_message(status.code().unwrap(), &readable_error(&captured))
            .expect("a non-zero exit is a failure");

        // The whole point: the runner's own text, not the status line.
        assert!(
            message.contains("import_dll"),
            "the runner's error text should be reported, but got {message:?}"
        );
        assert!(
            !message.starts_with("the runner exited with status"),
            "the fallback means the drain lost the text — this is B-07 itself, \
             and it must fail loudly rather than be tolerated: {message:?}"
        );
        // And the tail specifically, since `readable_error` keeps the last four
        // lines — the part a torn capture would be missing.
        assert!(
            message.contains("3999"),
            "the *last* line the child wrote must survive the capture: {message:?}"
        );
    }

    #[test]
    fn a_capture_that_really_is_empty_falls_back_to_the_status_line() {
        // The other direction, so the test above cannot be satisfied by a
        // function that simply never uses the fallback. A child that exits
        // non-zero without writing anything is reported by status, exactly as
        // Python's contract says — the fallback is correct *here* and wrong
        // above, and both arms have to be pinned or the pair proves nothing.
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 4")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("/bin/sh should be runnable");
        let (status, captured) = capture_stderr_joined(&mut child);

        assert_eq!(status.code(), Some(4));
        assert!(captured.is_empty(), "this child writes nothing");
        assert_eq!(
            failure_message(4, &readable_error(&captured)).as_deref(),
            Some("the runner exited with status 4")
        );
    }

    #[test]
    fn a_clean_exit_is_never_a_failure() {
        // `code == 0` short-circuits before the text is even consulted, so a
        // zero-exit child that chattered on stderr is still not a failure. The
        // corpus carries this as `launch_failure_text` with `exit_code: 0`.
        assert_eq!(failure_message(0, "wine: noise on a clean exit"), None);
        assert_eq!(failure_message(0, ""), None);
        // And a non-zero exit with no text is the one case that *does* use the
        // status line — the shape the race degrades to.
        assert_eq!(
            failure_message(127, "").as_deref(),
            Some("the runner exited with status 127")
        );
    }
}
