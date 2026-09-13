//! The command half of the easy installer (`installers.py:258-530`).
//!
//! Command construction, prefix lookup and the created library entry. See
//! [`crate::installers`] for the catalog, the error type and the seams.

use std::path::{Path, PathBuf};

use crate::installers::{Installer, InstallerError, Kind};
use crate::models::Game;
use crate::paths::{self, Env};
use crate::runners::launch_opts::apply_launch_options;
use crate::runners::shell::split_posix;
use crate::runners::{
    Command, LaunchEnv, Runner, prefix_drive_cs, pure_posix_name, uses_proton_runtime,
};

/// Cap on the *downloaded* installer (`installers.py:533`).
///
/// Enforced twice, as Python enforces it: against the declared
/// `Content-Length` before any byte is kept, and against the running total
/// during the transfer. The first is the cheap refusal that makes the second
/// unnecessary when the server is honest about being hostile.
pub const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;

/// How long a vendor wizard is given to produce its executable
/// (`installers.py:299`).
pub const INSTALL_SETTLE_TIMEOUT_SECONDS: f64 = 6.0 * 60.0 * 60.0;

/// Long enough for a bootstrapper to unpack and start the real wizard
/// (`installers.py:346`).
pub const INSTALL_HANDOFF_SECONDS: f64 = 3.0;

/// How long a single wineserver wait is allowed to take before the poll loop
/// looks at the prefix again (`installers.py:347`).
pub const INSTALL_POLL_SECONDS: f64 = 2.0;

/// How long a `wineserver -w` has to have actually waited before its answer
/// counts as evidence (`installers.py:352`).
pub const INSTALL_WAIT_EVIDENCE_SECONDS: f64 = 5.0;

/// What is left to wait after the prefix is *known* idle — the last of the
/// wizard's writes (`installers.py:356`).
pub const INSTALL_FLUSH_SECONDS: f64 = 20.0;

/// What is left to wait when the wizard itself is what we are waiting on, so
/// the window covers a person reading and clicking through it
/// (`installers.py:357`).
pub const INSTALL_WIZARD_SECONDS: f64 = 10.0 * 60.0;

/// Bash on the "users" component of an expected path (`installers.py:502`).
const USERS_COMPONENT: &str = "users";

/// The basename search's depth bound, past the root it starts from
/// (`installers.py:454`).
pub const MAX_SCAN_DEPTH: usize = 6;

/// The basename search's entry budget (`installers.py:455`).
///
/// A finished prefix holds tens of thousands of files, so this is what keeps a
/// missing executable costing a moment rather than a frozen window.
pub const MAX_SCAN_ENTRIES: usize = 40_000;

/// Directories the basename search does not descend into (`installers.py:456`).
///
/// `windows`, `syswow64` and `system32` are the ones that matter: a Wine prefix
/// has an entire Windows installation inside it, and every one of those files
/// is irrelevant to "where did the vendor put its launcher".
const SKIP_DIRS: [&str; 7] = [
    "windows", "syswow64", "system32", "winsxs", "temp", "tmp", "cache",
];

/// Build the argv used to run a downloaded `exe` or `msi`
/// (`installers.py:258-267`).
///
/// An MSI is not executable: it is a *database* that `msiexec` reads, so it is
/// passed as that program's argument rather than as the program. An exe is
/// handed straight to Wine.
///
/// `arguments` is `shlex`-split rather than split on whitespace, and an
/// unbalanced quote is an error rather than a silent truncation — Python lets
/// `shlex.split`'s `ValueError` escape here, and it is reached from the
/// catalog's own `arguments` field, so a recipe with a typo fails at install
/// time rather than running half a command line.
pub fn installer_argv(
    wine_or_umu: &str,
    installer_path: &Path,
    kind: Kind,
    arguments: &str,
) -> Result<Vec<String>, InstallerError> {
    let mut argv = match kind {
        Kind::Msi => vec![
            wine_or_umu.to_string(),
            "msiexec".to_string(),
            "/i".to_string(),
            installer_path.to_string_lossy().into_owned(),
        ],
        Kind::Exe => vec![
            wine_or_umu.to_string(),
            installer_path.to_string_lossy().into_owned(),
        ],
    };
    if !arguments.is_empty() {
        argv.extend(split_posix(arguments)?);
    }
    Ok(argv)
}

/// `(argv, env)` that launches `installer` inside `prefix`
/// (`installers.py:270-291`).
///
/// The runner builds its own command for a *placeholder* game carrying only the
/// four fields the install cares about — the name (for error messages), the
/// prefix, and the two sync knobs the recipe chose — and then the installer is
/// spliced in as that command's arguments. That indirection is the point: a
/// runner knows how to put a program into *its* prefix, and an installer is
/// just another program.
///
/// # The Proton gate is applied to the runner's own argv, not the wrapped one
///
/// `uses_proton_runtime(runner, argv)` is asked about the command the **runner**
/// produced, before the installer is spliced in — and the answer decides whether
/// `apply_launch_options` may export `PROTONPATH`, the anti-cheat runtime
/// variables and the rest. Asking about the wrapped argv instead would compare
/// `argv[0]` against `"umu-run"` when `argv[0]` is now `umu-run` followed by the
/// installer, which happens to give the same answer today and would stop doing
/// so the moment `installer_argv` prepended anything. It is also literally what
/// the reference does.
pub fn build_installer_command(
    runner: &dyn Runner,
    prefix: &Path,
    installer: &Installer,
    installer_path: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<Command, InstallerError> {
    let mut placeholder = Game::new_named(installer.name);
    placeholder.prefix_path = prefix.to_string_lossy().into_owned();
    placeholder.esync = installer.esync;
    placeholder.fsync = installer.fsync;
    let command = runner.build_command(&placeholder, launch_env)?;
    let Some(first) = command.argv.first() else {
        return Err(InstallerError::EmptyCommand {
            runner: runner.name().to_string(),
        });
    };
    let wrapped = installer_argv(first, installer_path, installer.kind, installer.arguments)?;
    let proton_features = uses_proton_runtime(runner, &command.argv);
    Ok(apply_launch_options(
        &placeholder,
        &wrapped,
        &command.env,
        proton_features,
        launch_env,
    )?)
}

/// Reduce a catalog filename to a bare name before joining it onto a path
/// (`installers.py:411-416`).
///
/// The name is joined onto a download directory and used as a temporary-file
/// prefix, neither of which tolerates a separator — and one that *starts* with
/// `.` would hide the download, which the wizard then cannot find. So the
/// basename is taken after folding `\` to `/` (a Windows-style name in the
/// catalog would otherwise survive as one component on Linux), surrounding
/// whitespace is stripped, and empty, `.` and `..` all fall back.
///
/// `fallback` is the installer's own id, so the fallback name is stable per
/// recipe rather than a constant two downloads would collide on.
pub fn safe_download_name(filename: &str, fallback: &str) -> String {
    let candidate = pure_posix_name(&filename.replace('\\', "/"))
        .trim()
        .to_string();
    if candidate.is_empty() || candidate == "." || candidate == ".." {
        return format!("{fallback}.exe");
    }
    candidate
}

/// The children of `directory`, or nothing when it cannot be read
/// (`installers.py:419-423`).
///
/// Python's `except OSError: return`. A prefix the user has deleted, or a
/// directory this process may not list, is a directory with nothing in it as
/// far as every caller here is concerned — and returning an empty iterator
/// rather than an error is what keeps each of them from growing an error path
/// that would have to decide what "cannot list a directory" means for a search
/// whose answer is "not found" either way.
fn iter_children(directory: &Path) -> Vec<PathBuf> {
    #[cfg(test)]
    DIRECTORY_READS.with(|count| count.set(count.get() + 1));
    match std::fs::read_dir(directory) {
        Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    }
}

// Directory reads performed by `iter_children`, counted for one test.
//
// The property that test pins — one walk per root rather than one walk per
// candidate name — is invisible from outside the function: a per-name scan
// returns the *same answer* and only costs more I/O, so no assertion on a
// return value can see the difference. The reference measures it by
// monkey-patching `Path.iterdir` with a counting wrapper
// (`tests/test_installers.py:340-350`); Rust's `std::fs::read_dir` cannot be
// patched that way, so the count lives in the one function that reads a
// directory. It is `thread_local` rather than a global because `cargo test`
// runs cases concurrently — a shared counter would be incremented by whichever
// other test happened to be scanning at the same moment.
//
// `#[cfg(test)]` on the counter *and* on the increment keeps this out of a
// production build entirely, rather than behind a branch that always runs.
#[cfg(test)]
thread_local! {
    pub(crate) static DIRECTORY_READS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Walk `relative` under `root`, matching each path component
/// case-insensitively (`installers.py:426-442`).
///
/// Wine's `drive_c` is a case-insensitive filesystem's idea of a directory
/// tree rendered onto a case-sensitive one, so every recipe's expected path is
/// a guess at capitalisation: the catalog says `users/steamuser/...` and the
/// prefix holds `users/steamuser/...` or `Users/SteamUser/...` depending on
/// which Wine built it.
///
/// Two details are the reference's and are easy to lose. A leading `/` is
/// **skipped**, not honoured — Python iterates `Path(relative).parts` and
/// skips `os.sep`, `"/"` and `"."`, so an absolute expected path is resolved
/// *under* `root` rather than against the filesystem root. And the first
/// case-insensitive match wins, so a prefix holding both `Steam` and `steam`
/// resolves to whichever the directory happens to list first. That tie is
/// unspecified in the reference and unspecified here — the same shape of rule
/// as [`crate::runners::families::asset_matches`], which also lowercases both
/// sides and keeps the first match rather than imposing an order. (This
/// paragraph used to cite a `families::search_anticheat` that does not exist,
/// which is how a false citation reads as a real one: the sentence was
/// plausible, and nothing checked it until rustdoc's link linter did.)
///
/// `root` is returned unchanged for an empty or all-skipped `relative`, which
/// is what Python's `Path()` gives.
pub fn resolve_case_insensitive(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        // `os.sep` is `/` on this platform, so the skip set is the two slashes
        // and `.`. A `..` component is **not** skipped and not resolved: it is
        // looked for as a literal child name, exactly as Python does.
        if part.is_empty() || part == "." {
            continue;
        }
        if !current.is_dir() {
            return None;
        }
        let wanted = part.to_lowercase();
        let found = iter_children(&current)
            .into_iter()
            .find(|child| child_name_matches(child, &wanted))?;
        current = found;
    }
    Some(current)
}

/// Whether `child`'s file name case-folds to `wanted` (already lowercased).
///
/// A child path with no file name — the filesystem root, or a `..`-terminated
/// path — cannot match, and Python's `child.name.lower()` on such a path is
/// `""`, which matches nothing either.
fn child_name_matches(child: &Path, wanted: &str) -> bool {
    child
        .file_name()
        .is_some_and(|name| name.to_string_lossy().to_lowercase() == wanted)
}

/// Every directory directly under `<drive_c>/users` (`installers.py:445-449`).
///
/// The fallback for `users/<name>/...` expectations: the catalog guesses
/// `steamuser`, and a prefix whose first user was created by an installer that
/// named them something else has `users/alice/...` instead. Searching the
/// profiles by name is what makes that expectation work anyway.
fn user_profile_candidates(drive_c: &Path) -> Vec<PathBuf> {
    let users = drive_c.join("users");
    if !users.is_dir() {
        return Vec::new();
    }
    iter_children(&users)
        .into_iter()
        .filter(|child| child.is_dir())
        .collect()
}

/// Find the first of `filenames` under the typical install locations
/// (`installers.py:459-494`).
///
/// # Why one walk for the whole set, and why it is bounded
///
/// The first version scanned once per candidate name, re-reading `Program
/// Files` from scratch for each — `test_extra_candidate_names_do_not_multiply_
/// the_fallback_scan` exists because a recipe with four expected paths cost four
/// full scans. Here the names go into one set and each directory is read once,
/// which is what the budget counts.
///
/// The walk is a depth-first stack at most [`MAX_SCAN_DEPTH`] deep and costs at
/// most [`MAX_SCAN_ENTRIES`] entries examined, so an unlucky prefix costs a
/// moment rather than a frozen window. Both bounds are asserted by tests that
/// build the directory they trip.
///
/// The order is the roots' order and, within a root, a LIFO stack — so it is
/// **not** a breadth-first or alphabetical search. That is Python's order and
/// therefore what its tests pin; a "tidier" traversal would change which of two
/// identically-named files is found.
fn search_known_roots(drive_c: &Path, filenames: &[String]) -> Option<PathBuf> {
    let targets: Vec<String> = filenames
        .iter()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_lowercase())
        .collect();
    if targets.is_empty() {
        return None;
    }
    let mut roots = vec![
        drive_c.join("Program Files"),
        drive_c.join("Program Files (x86)"),
    ];
    roots.extend(user_profile_candidates(drive_c));

    let mut budget = MAX_SCAN_ENTRIES;
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut stack: Vec<(PathBuf, usize)> = vec![(root, 0)];
        while let Some((current, depth)) = stack.pop() {
            if budget == 0 {
                break;
            }
            for child in iter_children(&current) {
                budget -= 1;
                if budget == 0 {
                    break;
                }
                if child.is_dir() {
                    let name = child
                        .file_name()
                        .map(|name| name.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    if depth < MAX_SCAN_DEPTH && !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push((child, depth + 1));
                    }
                } else if child
                    .file_name()
                    .is_some_and(|name| targets.contains(&name.to_string_lossy().to_lowercase()))
                {
                    return Some(child);
                }
            }
        }
    }
    None
}

/// Locate an installed executable under one `drive_c`
/// (`installers.py:497-511`).
///
/// Three passes, in the reference's order, each one a fallback for the last:
///
/// 1. Each expected path, case-insensitively.
/// 2. For an expected path under `users/<name>/...`, the same tail under every
///    profile the prefix actually has.
/// 3. The expected **basenames**, anywhere under `Program Files`, `Program
///    Files (x86)` or a user profile, bounded by [`search_known_roots`].
fn find_in_drive_c(drive_c: &Path, expected: &[&str]) -> Option<PathBuf> {
    for relative in expected {
        if let Some(found) =
            resolve_case_insensitive(drive_c, relative).filter(|found| found.is_file())
        {
            return Some(found);
        }
        let parts: Vec<&str> = relative.split('/').collect();
        if parts.len() >= 2 && parts[0].to_lowercase() == USERS_COMPONENT {
            let rest = parts[2..].join("/");
            for profile in user_profile_candidates(drive_c) {
                if let Some(found) =
                    resolve_case_insensitive(&profile, &rest).filter(|found| found.is_file())
                {
                    return Some(found);
                }
            }
        }
    }

    let names: Vec<String> = expected
        .iter()
        .filter_map(|relative| {
            let name = pure_posix_name(relative);
            if name.is_empty() { None } else { Some(name) }
        })
        .collect();
    search_known_roots(drive_c, &names)
}

/// Locate an installed executable under `prefix` (`installers.py:514-530`).
///
/// Both prefix layouts are searched, Proton's first: umu passes `WINEPREFIX` to
/// Proton as `STEAM_COMPAT_DATA_PATH`, so a title installed with any Proton
/// runner lands one directory deeper than a raw Wine one, and a search that
/// only knew the Wine layout reported every Proton install as a missing
/// executable.
///
/// An empty `expected` finds nothing rather than everything — the alternative
/// would be returning `drive_c` itself, which `is_file()` then rejects, but the
/// early return makes that explicit.
pub fn find_prefix_exe(prefix: &Path, expected: &[&str]) -> Option<PathBuf> {
    let expected: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|item| !item.is_empty())
        .collect();
    if expected.is_empty() {
        return None;
    }
    for drive_c in prefix_drive_cs(prefix) {
        if let Some(found) = find_in_drive_c(&drive_c, &expected) {
            return Some(found);
        }
    }
    None
}

/// The prefix directory a new install is placed in
/// (`installers.py:649-652`).
///
/// Named after the game's id, which the caller generates *before* the install
/// starts — so a wizard that is cancelled leaves a prefix directory behind
/// rather than a library entry, and the id in it is the one the entry will use
/// if the user retries.
pub fn prepare_prefix(game_id: &str) -> std::io::Result<PathBuf> {
    let prefix = paths::prefixes_dir().join(game_id);
    std::fs::create_dir_all(&prefix)?;
    Ok(prefix)
}

/// [`prepare_prefix`] against an injected environment.
pub fn prepare_prefix_in(game_id: &str, env: &dyn Env) -> std::io::Result<PathBuf> {
    let prefix = paths::prefixes_dir_in(env).join(game_id);
    std::fs::create_dir_all(&prefix)?;
    Ok(prefix)
}

/// The library entry created after a successful easy install
/// (`installers.py:655-676`).
///
/// # The four fields that are not the recipe's name
///
/// `working_directory` is the executable's own directory, which is what
/// `bridge.py` passes so a launcher that resolves its data files relative to
/// its working directory finds them. `arguments` is `launch_arguments`, not
/// `arguments` — the first is what the *user* runs afterwards, the second is
/// what the *installer* was run with, and Discord is the one recipe where they
/// differ (`--processStart Discord.exe`). `category` is the recipe's
/// `library_category`, which is a different string space from the page's
/// category. And `kind` is `"windows"` for all nine, including the native
/// launchers — every one of these is a Windows binary run through a runner.
pub fn game_from_install(
    installer: &Installer,
    exe_path: &Path,
    prefix: &Path,
    runner_id: &str,
    game_id: Option<&str>,
) -> Game {
    let exe = exe_path.to_path_buf();
    let mut game = Game::new_named(installer.name);
    game.id = game_id
        .map(str::to_string)
        .unwrap_or_else(crate::models::new_id);
    game.exe_path = exe.to_string_lossy().into_owned();
    game.arguments = installer.launch_arguments.to_string();
    game.runner = runner_id.to_string();
    game.prefix_path = prefix.to_string_lossy().into_owned();
    game.kind = "windows".to_string();
    game.category = installer.library_category.to_string();
    game.esync = installer.esync;
    game.fsync = installer.fsync;
    game.working_directory = exe
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();
    game
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installers::tests_support::*;
    use crate::installers::{LAUNCHERS, installer_by_id, installers};
    use crate::runners::RunnerError;
    use crate::runners::env::tests::FakeLaunchEnv;
    use crate::runners::shell::ShellError;

    // The command half — MsiexecArgvTests (tests/test_installers.py:159-218)
    // -----------------------------------------------------------------------

    /// `test_msi_uses_msiexec` (`tests/test_installers.py:160-162`).
    ///
    /// An MSI is a database, not a program: handed to Wine directly it is
    /// opened by whatever the file association says, which on a bare prefix is
    /// nothing at all.
    #[test]
    fn an_msi_is_run_through_msiexec() {
        let argv =
            installer_argv("/usr/bin/wine", Path::new("/tmp/Epic.msi"), Kind::Msi, "").unwrap();
        assert_eq!(argv, ["/usr/bin/wine", "msiexec", "/i", "/tmp/Epic.msi"]);
    }

    /// `test_exe_passes_installer_path` (`tests/test_installers.py:164-166`).
    #[test]
    fn an_exe_is_handed_straight_to_wine() {
        let argv = installer_argv(
            "/usr/bin/wine",
            Path::new("/tmp/SteamSetup.exe"),
            Kind::Exe,
            "",
        )
        .unwrap();
        assert_eq!(argv, ["/usr/bin/wine", "/tmp/SteamSetup.exe"]);
    }

    /// `test_extra_arguments_are_appended` (`tests/test_installers.py:168-170`).
    #[test]
    fn extra_arguments_are_appended() {
        let argv = installer_argv(
            "/usr/bin/wine",
            Path::new("/tmp/setup.exe"),
            Kind::Exe,
            "/S",
        )
        .unwrap();
        assert_eq!(argv.last().unwrap(), "/S");
        assert_eq!(argv.len(), 3);
    }

    /// The split is `shlex`'s, not whitespace's — a quoted argument with a
    /// space in it stays one argument, which is the whole reason the reference
    /// calls `shlex.split` here rather than `str.split`.
    #[test]
    fn extra_arguments_are_shlex_split_not_whitespace_split() {
        let argv = installer_argv(
            "/usr/bin/wine",
            Path::new("/tmp/setup.exe"),
            Kind::Exe,
            "/S /D=\"C:\\Program Files\\App\"",
        )
        .unwrap();
        assert_eq!(argv.len(), 4);
        assert_eq!(argv[2], "/S");
        assert_eq!(argv[3], "/D=C:\\Program Files\\App");
    }

    /// An unbalanced quote is an error rather than a silent truncation.
    /// Python lets `shlex.split`'s `ValueError` escape, and the recipe's
    /// `arguments` field is the only thing that can reach here.
    #[test]
    fn an_unbalanced_quote_in_the_arguments_is_an_error() {
        let error = installer_argv(
            "/usr/bin/wine",
            Path::new("/tmp/setup.exe"),
            Kind::Exe,
            "/S 'unclosed",
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "No closing quotation");
        assert!(matches!(
            error,
            InstallerError::Arguments(ShellError::NoClosingQuotation)
        ));
    }

    /// `test_build_installer_command_sets_prefix_and_sync`
    /// (`tests/test_installers.py:208-217`).
    #[test]
    fn the_installer_command_carries_the_prefix_and_the_sync_knobs() {
        let launch_env = FakeLaunchEnv::new();
        let installer = installer_by_id("epic").unwrap();
        let command = build_installer_command(
            &wine(),
            Path::new("/tmp/prefix"),
            installer,
            Path::new("/tmp/Epic.msi"),
            &launch_env,
        )
        .unwrap();

        assert_eq!(
            command.argv[..4],
            ["/usr/bin/wine", "msiexec", "/i", "/tmp/Epic.msi"]
        );
        assert_eq!(command.env.get("WINEPREFIX").unwrap(), "/tmp/prefix");
        assert_eq!(command.env.get("WINEESYNC").unwrap(), "1");
        assert_eq!(command.env.get("WINEFSYNC").unwrap(), "1");
    }

    /// `test_raw_wine_installer_run_gets_no_proton_only_variables`
    /// (`tests/test_installers.py:172-189`).
    ///
    /// The lane matters: a vendor installer run under raw Wine must not be
    /// handed Proton-only variables, for the same reason `launch` gates them —
    /// `PROTONPATH` would make Wine look for a Proton it is not.
    #[test]
    fn a_raw_wine_install_run_gets_no_proton_only_variables() {
        let launch_env = FakeLaunchEnv::new();
        let command = build_installer_command(
            &wine(),
            Path::new("/tmp/prefix"),
            installer_by_id("steam").unwrap(),
            Path::new("/tmp/SteamSetup.exe"),
            &launch_env,
        )
        .unwrap();

        for key in [
            "PROTON_BATTLEYE_RUNTIME",
            "PROTON_EAC_RUNTIME",
            "PROTONPATH",
        ] {
            assert!(
                !command.env.contains_key(key),
                "{key} leaked into a raw Wine installer run"
            );
        }
        // Wine's own knobs still apply, so this is a gate and not an early
        // return that dropped the environment.
        assert_eq!(command.env.get("WINEESYNC").unwrap(), "1");
        assert_eq!(command.env.get("WINEFSYNC").unwrap(), "1");
    }

    /// `test_proton_installer_run_keeps_proton_features`
    /// (`tests/test_installers.py:191-206`) — the control arm for the test
    /// above. Without it, a `build_installer_command` that dropped the whole
    /// environment would pass that one.
    #[test]
    fn a_proton_install_run_keeps_proton_features() {
        let scratch = Scratch::new("proton-runner");
        let root = scratch.path().join("GE-Proton");
        touch(&root.join("files/bin/wine"));
        touch(&root.join("proton"));
        let runner = crate::runners::ProtonRunner::new(&root, "", "GE-Proton");
        // `umu-run` on PATH is what makes this a Proton *runtime* run rather
        // than a Proton build being driven by hand.
        let launch_env = FakeLaunchEnv::new()
            .with_which("umu-run", "/usr/bin/umu-run")
            .with_anticheat("battleye", "/opt/runtime");
        let command = build_installer_command(
            &runner,
            Path::new("/tmp/prefix"),
            installer_by_id("steam").unwrap(),
            Path::new("/tmp/SteamSetup.exe"),
            &launch_env,
        )
        .unwrap();

        assert_eq!(
            Path::new(&command.argv[0]).file_name().unwrap(),
            "umu-run",
            "{:?}",
            command.argv
        );
        assert_eq!(
            command.env.get("PROTONPATH").unwrap(),
            &root.to_string_lossy()
        );
    }

    /// A runner that cannot produce a command at all is an error naming it,
    /// not an empty argv handed to the process layer.
    #[test]
    fn a_runner_without_a_binary_is_an_error_that_names_it() {
        let runner = crate::runners::WineRunner::with_binary(None);
        let error = build_installer_command(
            &runner,
            Path::new("/tmp/prefix"),
            installer_by_id("steam").unwrap(),
            Path::new("/tmp/SteamSetup.exe"),
            &FakeLaunchEnv::new(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, InstallerError::Runner(RunnerError::NotAvailable { name }) if name == "System Wine"),
            "{error:?}"
        );
    }

    // -----------------------------------------------------------------------
    // The download name — installers.py:411-416, no reference test
    // -----------------------------------------------------------------------

    /// The catalog's own filenames come out unchanged, and every shape that
    /// could steer the write out of the download directory falls back.
    #[test]
    fn a_download_name_is_reduced_to_a_bare_name() {
        assert_eq!(
            safe_download_name("SteamSetup.exe", "steam"),
            "SteamSetup.exe"
        );
        // A separator, either spelling, is folded to a basename.
        assert_eq!(safe_download_name("../../etc/passwd", "steam"), "passwd");
        assert_eq!(safe_download_name("..\\..\\evil.exe", "steam"), "evil.exe");
        // Whitespace around the name is stripped...
        assert_eq!(
            safe_download_name("  SteamSetup.exe  ", "steam"),
            "SteamSetup.exe"
        );
        // ...and the fallback is the installer's id, not a constant.
        assert_eq!(safe_download_name("", "steam"), "steam.exe");
        assert_eq!(safe_download_name("   ", "steam"), "steam.exe");
        assert_eq!(safe_download_name(".", "steam"), "steam.exe");
        assert_eq!(safe_download_name("..", "steam"), "steam.exe");
        assert_eq!(safe_download_name("a/b/", "steam"), "b");
        // Every recipe's own filename survives, which is what makes this
        // reachable-but-inert for the catalog as shipped.
        for item in installers() {
            assert_eq!(safe_download_name(item.filename, item.id), item.filename);
        }
    }

    // -----------------------------------------------------------------------
    // Prefix lookup — PrefixLookupTests (tests/test_installers.py:220-271)
    // -----------------------------------------------------------------------

    /// `test_case_insensitive_path_resolution` (`tests/test_installers.py:230-239`).
    #[test]
    fn a_path_resolves_case_insensitively() {
        let scratch = Scratch::new("ci-path");
        let drive_c = scratch.path().join("drive_c");
        let target = touch(&drive_c.join("Program Files (x86)/Steam/steam.exe"));

        assert_eq!(
            resolve_case_insensitive(&drive_c, "program files (x86)/STEAM/Steam.EXE"),
            Some(target.clone())
        );
        assert_eq!(
            find_prefix_exe(scratch.path(), &["Program Files (x86)/Steam/steam.exe"]),
            Some(target)
        );
    }

    /// `test_user_profile_fallback` (`tests/test_installers.py:241-257`).
    ///
    /// The catalog guesses `steamuser`; this prefix's user is `alice`. The
    /// substitution is what makes Discord and Amazon resolve on a prefix
    /// created by a normal Wine run.
    #[test]
    fn a_users_path_is_retried_under_every_profile() {
        let scratch = Scratch::new("profile");
        let drive_c = scratch.path().join("drive_c");
        let target = touch(&drive_c.join("users/alice/AppData/Local/Discord/Update.exe"));

        assert_eq!(
            find_prefix_exe(
                scratch.path(),
                &["users/steamuser/AppData/Local/Discord/Update.exe"]
            ),
            Some(target)
        );
    }

    /// `test_basename_search_under_program_files`
    /// (`tests/test_installers.py:259-267`) — the third pass, reached because
    /// neither the exact path nor a profile holds it.
    #[test]
    fn a_relocated_executable_is_found_by_its_basename() {
        let scratch = Scratch::new("basename");
        let drive_c = scratch.path().join("drive_c");
        let target = touch(&drive_c.join("Program Files/GOG Galaxy/GalaxyClient.exe"));

        assert_eq!(
            find_prefix_exe(
                scratch.path(),
                &["Program Files (x86)/GOG Galaxy/GalaxyClient.exe"]
            ),
            Some(target)
        );
    }

    /// `test_missing_prefix_returns_none` (`tests/test_installers.py:269-270`).
    #[test]
    fn a_missing_prefix_finds_nothing() {
        let scratch = Scratch::new("missing-prefix");
        assert_eq!(
            find_prefix_exe(&scratch.path().join("missing"), &["Steam/steam.exe"]),
            None
        );
    }

    // -----------------------------------------------------------------------
    // Prefix scan bounds — PrefixScanBoundsTests
    // (tests/test_installers.py:303-365)
    // -----------------------------------------------------------------------

    /// `test_fallback_finds_a_relocated_executable`
    /// (`tests/test_installers.py:318-321`).
    #[test]
    fn the_fallback_finds_a_relocated_executable() {
        let scratch = Scratch::new("relocated");
        let drive_c = scratch.path().join("drive_c");
        let moved = touch(&drive_c.join("Program Files/Somewhere Else/steam.exe"));

        assert_eq!(
            find_prefix_exe(scratch.path(), &["Program Files (x86)/Steam/steam.exe"]),
            Some(moved)
        );
    }

    /// `test_scan_does_not_descend_past_the_depth_limit`
    /// (`tests/test_installers.py:323-326`).
    #[test]
    fn the_fallback_scan_stops_at_the_depth_limit() {
        let scratch = Scratch::new("depth");
        let drive_c = scratch.path().join("drive_c");
        let deep: String = (0..MAX_SCAN_DEPTH + 4)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        touch(&drive_c.join(format!("Program Files/{deep}/steam.exe")));

        assert_eq!(
            find_prefix_exe(scratch.path(), &["Program Files/Steam/steam.exe"]),
            None
        );
    }

    /// `test_windows_system_directories_are_skipped`
    /// (`tests/test_installers.py:328-330`).
    #[test]
    fn the_fallback_scan_does_not_enter_the_windows_directories() {
        let scratch = Scratch::new("skip-dirs");
        let drive_c = scratch.path().join("drive_c");
        touch(&drive_c.join("Program Files/windows/system32/steam.exe"));

        assert_eq!(
            find_prefix_exe(scratch.path(), &["Program Files/Steam/steam.exe"]),
            None
        );
    }

    /// `test_extra_candidate_names_do_not_multiply_the_fallback_scan`
    /// (`tests/test_installers.py:332-365`) — the repair for a scan that
    /// restarted from scratch for every expected basename.
    ///
    /// # Why this counts directory reads
    ///
    /// The bug this pins is invisible in the answer: a scan that restarts per
    /// candidate name returns exactly what a shared scan returns, and only
    /// costs more I/O. So the reference counts `Path.iterdir` calls, and this
    /// counts [`iter_children`] calls through the `DIRECTORY_READS` cell. Both
    /// runs below find nothing — the assertion with teeth is the *ratio*, and
    /// the control arm is `one_reads > 0`: without it, `many < one * 2` is a
    /// comparison between two zeroes and would pass on a scan that never ran.
    #[test]
    fn extra_candidate_names_do_not_multiply_the_fallback_scan() {
        let scratch = Scratch::new("scan-budget");
        let drive_c = scratch.path().join("drive_c");
        let target = touch(&drive_c.join("Program Files/Epic Games/EpicGamesLauncher.exe"));
        for index in 0..30 {
            touch(&drive_c.join(format!("Program Files/Filler{index}/thing.dat")));
        }

        fn scan(prefix: &Path, expected: &[&str]) -> (Option<PathBuf>, usize) {
            DIRECTORY_READS.with(|count| count.set(0));
            let found = find_prefix_exe(prefix, expected);
            let reads = DIRECTORY_READS.with(std::cell::Cell::get);
            (found, reads)
        }

        let (one_found, one_reads) = scan(scratch.path(), &["Program Files (x86)/Epic/A.exe"]);
        let (many_found, many_reads) = scan(
            scratch.path(),
            &[
                "Program Files (x86)/Epic/A.exe",
                "Program Files (x86)/Epic/B.exe",
                "Program Files (x86)/Epic/C.exe",
                "Program Files (x86)/Epic/D.exe",
            ],
        );

        assert_eq!(one_found, None);
        assert_eq!(many_found, None);
        // The control arm. If the counter stopped being wired — the increment
        // deleted, the function short-circuited, the cell read from the wrong
        // thread — this is the assertion that fails, and it fails by name
        // rather than leaving the ratio below to pass vacuously.
        assert!(
            one_reads > 0,
            "the counter saw no directory reads, so the ratio below proves nothing"
        );
        // Four basenames instead of one must not cost four times the reads; the
        // exact-path probes add a little, the scan itself is shared.
        assert!(
            many_reads < one_reads * 2,
            "one name cost {one_reads} reads, four cost {many_reads}"
        );
        assert!(target.exists());
    }

    // -----------------------------------------------------------------------
    // Proton prefix layout — ProtonPrefixLayoutTests
    // (tests/test_installers.py:380-418)
    // -----------------------------------------------------------------------

    /// `test_an_executable_under_pfx_is_found` (`tests/test_installers.py:400-403`).
    #[test]
    fn an_executable_under_pfx_is_found() {
        let scratch = Scratch::new("pfx");
        let target = install_steam(&scratch.path().join("pfx/drive_c"));
        assert_eq!(
            find_prefix_exe(
                scratch.path(),
                installer_by_id("steam").unwrap().expected_exe
            ),
            Some(target)
        );
    }

    /// `test_the_wine_layout_still_works` (`tests/test_installers.py:405-408`).
    #[test]
    fn the_wine_layout_still_works() {
        let scratch = Scratch::new("wine-layout");
        let target = install_steam(&scratch.path().join("drive_c"));
        assert_eq!(
            find_prefix_exe(
                scratch.path(),
                installer_by_id("steam").unwrap().expected_exe
            ),
            Some(target)
        );
    }

    /// `test_the_proton_layout_wins_when_both_directories_exist`
    /// (`tests/test_installers.py:410-414`) — the reason `prefix_drive_cs`
    /// returns Proton's first.
    #[test]
    fn the_proton_layout_wins_when_both_directories_exist() {
        let scratch = Scratch::new("both-layouts");
        std::fs::create_dir_all(scratch.path().join("drive_c")).unwrap();
        let target = install_steam(&scratch.path().join("pfx/drive_c"));
        assert_eq!(
            find_prefix_exe(
                scratch.path(),
                installer_by_id("steam").unwrap().expected_exe
            ),
            Some(target)
        );
    }

    /// `test_an_empty_expected_list_finds_nothing`
    /// (`tests/test_installers.py:416-418`).
    #[test]
    fn an_empty_expected_list_finds_nothing() {
        let scratch = Scratch::new("empty-expected");
        install_steam(&scratch.path().join("drive_c"));
        assert_eq!(find_prefix_exe(scratch.path(), &[]), None);
    }

    // -----------------------------------------------------------------------
    // The created library entry — GameFromInstallTests
    // (tests/test_installers.py:273-300)
    // -----------------------------------------------------------------------

    /// `test_library_entry_uses_recipe_defaults`
    /// (`tests/test_installers.py:274-290`).
    #[test]
    fn the_library_entry_uses_the_recipe_defaults() {
        let game = game_from_install(
            installer_by_id("steam").unwrap(),
            Path::new("/pfx/drive_c/Program Files (x86)/Steam/steam.exe"),
            Path::new("/pfx"),
            "GE-Proton9-5",
            Some("abc123"),
        );

        assert_eq!(game.id, "abc123");
        assert_eq!(game.name, "Steam");
        assert_eq!(game.runner, "GE-Proton9-5");
        assert_eq!(game.prefix_path, "/pfx");
        assert_eq!(game.category, LAUNCHERS);
        assert!(game.esync);
        assert!(game.fsync);
        assert!(!game.is_linux());
        // The executable's own directory, which a launcher that resolves its
        // data files relatively depends on.
        assert_eq!(
            game.working_directory,
            "/pfx/drive_c/Program Files (x86)/Steam"
        );
        assert_eq!(game.kind, "windows");
    }

    /// `test_discord_update_entry_keeps_required_process_start_arguments`
    /// (`tests/test_installers.py:292-300`) — `launch_arguments`, not
    /// `arguments`. Discord is the only recipe where they could be confused,
    /// because it is the only one with either.
    #[test]
    fn the_discord_entry_keeps_its_process_start_arguments() {
        let game = game_from_install(
            installer_by_id("discord").unwrap(),
            Path::new("/pfx/drive_c/users/steamuser/AppData/Local/Discord/Update.exe"),
            Path::new("/pfx"),
            "wine-system",
            None,
        );

        assert_eq!(game.arguments, "--processStart Discord.exe");
        // Discord is also the entry whose library category differs from the
        // page's, so the two strings have to be read from different fields.
        assert_eq!(game.category, "Utility");
        assert_ne!(game.category, installer_by_id("discord").unwrap().category);
        // A generated id when the caller has none: 32 lowercase hex characters,
        // the same shape `uuid.uuid4().hex` produces.
        assert_eq!(game.id.len(), 32);
        assert!(
            game.id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    /// A game id the caller supplies is used verbatim, which is what keeps the
    /// prefix directory and the library entry naming the same id.
    #[test]
    fn prepare_prefix_creates_and_returns_the_games_directory() {
        let scratch = Scratch::new("prepare-prefix");
        let home = scratch.path().to_string_lossy().into_owned();
        let env = crate::paths::tests::FakeEnv::new(&[("HOME", &home)]);
        let prefix = prepare_prefix_in("abc123", &env).unwrap();
        assert_eq!(
            prefix,
            scratch
                .path()
                .join(".local/share/gamehandler/prefixes/abc123")
        );
        assert!(prefix.is_dir());
        // Idempotent, as `mkdir(parents=True, exist_ok=True)` is: a retry after
        // a cancelled wizard is not an error.
        assert_eq!(prepare_prefix_in("abc123", &env).unwrap(), prefix);
    }
}
