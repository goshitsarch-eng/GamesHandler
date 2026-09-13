//! Everything the runner code needs from outside the process, behind one trait.
//!
//! `docs/migration/DECISIONS.md` D-27 introduces `LaunchEnv` for
//! `launch_opts`, because the Python tests make `apply_launch_options`
//! hermetic by `mock.patch("gamehandler.runners.shutil.which")` and a Rust
//! module has no global to patch. This module is where that trait lives, and it
//! also holds the real implementation of each lookup.
//!
//! # Scope note — the trait is one method wider than D-27 records
//!
//! D-27 gives the shape as `which` plus `anticheat_runtime`. Writing
//! `WineRunner`/`ProtonRunner` showed two more impure reads inside the same
//! code path, both already mocked by the same tests:
//!
//! * `build_command` and `tool_command` start from `dict(os.environ)`
//!   (`runners.py:395, 481, 1483`), and `launch` reads `GAMEHANDLER_DXVK_ROOT`
//!   from it (`1408`). `test_source_install_without_flatpak_dxvk_bundle_still_launches`
//!   and `test_per_game_winearch_reaches_dxvk_installer` both
//!   `mock.patch.dict(os.environ, …)` to steer exactly that read.
//! * `apply_launch_options` re-applies the parsed environment block last
//!   (`1258`), so the user's values win over the toggles — also asserted.
//!
//! So the trait carries `var` and `environ` as well. It is the same seam and
//! the same justification, not a second one; D-27's principle is stated
//! generally ("anything `core` needs from outside the process is injected, so
//! `cargo test -p gamehandler-core` stays instant, headless and offline") and
//! `os.environ` is squarely inside it. `std::env::set_var` is `unsafe` under
//! Rust 2024 and this crate denies `unsafe_code`, so without the seam those two
//! launch tests are not merely awkward to write — they are unwritable, and the
//! behaviour they guard (a per-game `WINEARCH` reaching the DXVK installer)
//! would go back to being checked only by hand. DECISIONS.md is owned by the
//! lead, so this note is the record on the code side.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::paths::{self, Env};

/// The impure lookups the runner code performs, injectable for tests.
pub trait LaunchEnv: Env {
    /// `shutil.which(name)` — the resolved path, or `None`.
    fn which(&self, name: &str) -> Option<PathBuf>;

    /// `os.environ` as a whole, for `dict(os.environ)`.
    ///
    /// A [`BTreeMap`] rather than a `HashMap` because nothing in the Python
    /// code depends on insertion order, and a deterministic iteration order
    /// makes a dumped environment comparable in a test or an oracle vector.
    ///
    /// # Non-UTF-8 entries
    ///
    /// A [`String`]-valued map cannot represent an environment variable whose
    /// bytes are not valid UTF-8, and this map is *the whole* child environment
    /// rather than a layer over an inherited one (every spawn site is
    /// `env_clear()` then `.envs(…)`, matching Python's `Popen(env=dict(...))`).
    /// Such a variable is therefore dropped rather than passed through. That is
    /// a deliberate, documented narrowing: `std::env::vars()` **panics** on one
    /// of these, and a launch that refuses to start is a worse answer than a
    /// launch missing one variable. Python loses the variable at the same point
    /// — its launch path is pure `str` and `subprocess` encodes with
    /// `surrogateescape`, so a non-UTF-8 value never survives
    /// `dict(os.environ)`'s trip through `subprocess` as itself either.
    fn environ(&self) -> BTreeMap<String, String>;

    /// `os.environ.get(name)`.
    fn var(&self, name: &str) -> Option<String>;

    /// `find_anticheat_runtime(kind, extra_roots)` — `None` where Python
    /// returns `""`, since every caller tests it for truthiness.
    fn anticheat_runtime(&self, kind: &str, extra_roots: &[PathBuf]) -> Option<String>;
}

/// The real host: the process environment and the real filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLaunchEnv;

impl Env for SystemLaunchEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl LaunchEnv for SystemLaunchEnv {
    fn which(&self, name: &str) -> Option<PathBuf> {
        which_in(name, &SystemLaunchEnv)
    }

    fn environ(&self) -> BTreeMap<String, String> {
        decode_environ(std::env::vars_os())
    }

    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn anticheat_runtime(&self, kind: &str, extra_roots: &[PathBuf]) -> Option<String> {
        find_anticheat_runtime(kind, extra_roots, &SystemLaunchEnv)
    }
}

/// Python's `os.defpath`, used when `PATH` is **unset**.
///
/// Not when it is empty: that case returns `None` before any directory is
/// tried (`BUG-37`). A Flatpak-launched app with a deliberately empty `PATH`
/// therefore finds nothing here, which is what the reference does and is the
/// safer reading besides — an empty `PATH` is a statement that the caller wants
/// no `PATH` search.
///
/// `os.defpath` is `/bin:/usr/bin`; CPython prefers `os.confstr("CS_PATH")`
/// and falls back to `os.defpath` only if that fails. On this host `CS_PATH` is
/// `/usr/bin` — a subset of `DEFPATH`, and the two find the same binaries on
/// any usrmerged system, where `/bin` is a link to `/usr/bin`. The difference is
/// unobservable without a system that has a binary in a real `/bin` and not in
/// `/usr/bin`, and this port does not invent one.
const DEFPATH: &str = "/bin:/usr/bin";

/// Turn the process environment into the `String`-valued map the launch path
/// carries, dropping the entries that cannot be represented.
///
/// The plain way to write this is `std::env::vars().collect()`, and it is what
/// this function replaced. It is not merely lossy on a non-UTF-8 variable —
/// `std::env::vars()` **panics** when it meets one, because it unwraps the
/// `OsString`→`String` conversion internally. A single such variable anywhere in
/// the environment (a `LC_*`, a `PATH` component, a game-specific variable the
/// user exported from a byte-oriented shell) took down the whole launch with an
/// `OsString` panic, on every launch path, since `environ()` is the first thing
/// each of them calls. A dropped variable is a degraded launch; a panic is no
/// launch at all, and the settings the caller inserts afterwards (`WINEPREFIX`,
/// `WINE`, `WINESERVER`, per-game `environment=`) are unaffected either way.
///
/// Names are required to be valid UTF-8; a name that is not is dropped with its
/// value, since no key in the map could address it. This is a `String`-typed
/// boundary by construction ([`LaunchEnv::environ`]), so the alternative is a
/// wider type across every consumer, which this defect does not warrant.
fn decode_environ<I>(vars: I) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
{
    vars.into_iter()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let value = value.into_string().ok()?;
            Some((name, value))
        })
        .collect()
}

/// `shutil.which(name)`, with the environment supplied rather than read.
///
/// Follows CPython's implementation rather than an approximation of it:
///
/// * A `name` containing a separator is used as-is and not searched for.
/// * An **empty** `PATH` entry is not skipped — `os.path.join("", name)` is a
///   relative path, so `PATH=":"` looks in the current directory. CPython's
///   comment says so explicitly, and skipping empties (the intuitive reading)
///   would change which binary is found.
/// * Duplicate entries are visited once, matching CPython's `seen` set.
/// * A `PATH` that is **set but empty** is not the same as one that is unset.
///   `os.environ.get("PATH")` answers `""`, which is falsy, and CPython's
///   `if not path: return None` (`which`, bpo-35755) returns before any
///   directory is tried. Only an *unset* `PATH` reaches `os.confstr("CS_PATH")`
///   / `os.defpath`. This function used to fold the two together, so
///   `PATH=""` searched `/bin:/usr/bin` and found wrappers the reference
///   refuses to find (`BUG-37`).
///
/// The execute check is `os.access(name, X_OK)` itself — `rustix::fs::access`,
/// which is `access(2)` over the real ids. An earlier revision checked the mode
/// bits instead and accepted a file executable by some *other* class and not by
/// ours, where `exec` then failed with a clear `EACCES`; the gap this paragraph
/// used to describe is closed (BUG-23).
pub fn which_in(name: &str, env: &dyn Env) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        let path = PathBuf::from(name);
        return is_executable_file(&path).then_some(path);
    }

    // Three cases, and the reference distinguishes all three: unset falls back
    // to the default path, **set-but-empty refuses to search at all**, and
    // anything else is the path. The middle case is the one a `.filter()`
    // cannot express, because it makes `Some("")` and `None` the same value.
    let raw = match env.var("PATH") {
        Some(value) if value.is_empty() => return None,
        Some(value) => value,
        None => DEFPATH.to_string(),
    };

    let mut seen: Vec<std::ffi::OsString> = Vec::new();
    for directory in raw.split(':') {
        // Normalised for the dedup key only; the joined path uses the entry as
        // written, exactly as `os.path.join(dir, cmd)` does.
        let key = std::ffi::OsString::from(directory);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if directory.is_empty() {
            // `PATH=":"` means the current directory, not "skip".
            if is_executable_file(Path::new(name)) {
                return Some(PathBuf::from(name));
            }
            continue;
        }
        let candidate = Path::new(directory).join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Python's `_access_check`: exists, is not a directory, and is executable.
fn is_executable_file(path: &Path) -> bool {
    if path.is_dir() {
        // `is_dir` follows symlinks, as `os.path.isdir` does; a dangling or
        // permission-denied path answers `false` here and fails the `access`
        // below either way.
        return false;
    }
    // `os.access(name, os.X_OK)` verbatim — the real ids, not the mode bits.
    // The mode check this replaced accepted a file executable only by a class
    // we are not in, so `which_in` could report a hit `exec` would refuse.
    rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}

/// Directory names a BattlEye or Easy Anti-Cheat runtime is published under.
///
/// Ported verbatim from `runners.py:954-963`, including the mixture of cases
/// and spaces: these are real directory names on disk, not identifiers.
pub const ANTICHEAT_DIR_NAMES: &[(&str, &[&str])] = &[
    (
        "battleye",
        &[
            "battleye_runtime",
            "BattlEye_Runtime",
            "proton-battleye-runtime",
            "Proton BattlEye Runtime",
        ],
    ),
    (
        "eac",
        &[
            "easyanticheat_runtime",
            "eac_runtime",
            "EasyAntiCheatRuntime",
            "proton-eac-runtime",
            "Proton Easy Anti-Cheat Runtime",
        ],
    ),
];

/// A non-empty directory, as `_runtime_dir_ok` tests it.
fn runtime_dir_ok(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// The names `find_anticheat_runtime` accepts for `kind`, if any.
fn anticheat_names(kind: &str) -> Option<&'static [&'static str]> {
    ANTICHEAT_DIR_NAMES
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, names)| *names)
}

/// The built-in roots an anti-cheat runtime is searched for, in order.
///
/// Split out from the search so the list can be asserted directly, and so a
/// test never has to depend on whether the machine it runs on happens to have
/// Steam installed. `extra_roots` are prepended by
/// [`find_anticheat_runtime`], matching Python.
pub fn anticheat_roots_in(env: &dyn Env) -> Vec<PathBuf> {
    let home = paths::home_dir_in(env);
    vec![
        home.join(".local/share/umu"),
        home.join(".local/share/lutris/runtime"),
        home.join(".local/share/Steam/steamapps/common"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/steamapps/common"),
        PathBuf::from("/usr/share/umu"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        paths::runners_dir_in(env),
    ]
}

/// Search an explicit list of roots for a `kind` runtime, or `None`.
///
/// `roots` is taken as given, so this is the half that a test can drive with
/// directories it created.
///
/// # One deliberate departure: children are sorted
///
/// Python walks `root.iterdir()` in filesystem order, so when two candidate
/// directories both qualify it returns whichever the directory happens to list
/// first — an unspecified answer that differs between machines and cannot be
/// pinned by a test. This sorts them, so the choice is reproducible. It is
/// behaviour-preserving in the sense that matters: both candidates are valid
/// runtimes, so either answer is correct, and no user-visible decision depends
/// on the tie.
pub fn search_anticheat(kind: &str, roots: &[PathBuf]) -> Option<String> {
    let names = anticheat_names(kind)?;

    for root in roots {
        // `root.exists()` is checked before `iterdir()` because a root we
        // cannot stat is skipped rather than treated as an error.
        if !root.exists() {
            continue;
        }
        for name in names {
            let candidate = root.join(name);
            if runtime_dir_ok(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            if !child.is_dir() {
                continue;
            }
            for name in names {
                let candidate = child.join(name);
                if runtime_dir_ok(&candidate) {
                    return Some(candidate.to_string_lossy().into_owned());
                }
                // Steam and Lutris nest the runtime under the tool's own tree.
                let nested = child.join("files").join("share").join(name);
                if runtime_dir_ok(&nested) {
                    return Some(nested.to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

/// Locate a BattlEye or EAC Proton runtime directory (`runners.py:1113-1154`).
///
/// Returns `None` for an unknown `kind`, and when nothing matches. `extra_roots`
/// are searched first, then [`anticheat_roots_in`].
pub fn find_anticheat_runtime(
    kind: &str,
    extra_roots: &[PathBuf],
    env: &dyn Env,
) -> Option<String> {
    let mut roots = extra_roots.to_vec();
    roots.extend(anticheat_roots_in(env));
    search_anticheat(kind, &roots)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::paths::tests::FakeEnv;
    use std::fs;

    /// A whole host, faked: environment, `which` results and anti-cheat
    /// runtimes, each supplied by the test that needs it.
    ///
    /// This is the Rust equivalent of the Python suite's
    /// `mock.patch("gamehandler.runners.shutil.which")` plus
    /// `mock.patch.dict(os.environ, …)`. Table-driven rather than a closure
    /// because the vector cases in `docs/migration/oracle/` carry their `which`
    /// spec as data, and a table is what a data-driven case can describe.
    ///
    /// Deliberately not `Default`: a test that wants an empty host writes
    /// `FakeLaunchEnv::new()`, and every other construction names what it is
    /// providing. Most `apply_launch_options` cases want an environment of
    /// `{}` and nothing on `PATH`, and the difference between that and "the
    /// real host" is the whole point of the seam.
    #[derive(Debug, Default)]
    pub struct FakeLaunchEnv {
        vars: Vec<(String, String)>,
        environ: BTreeMap<String, String>,
        which: BTreeMap<String, PathBuf>,
        anticheat: BTreeMap<String, String>,
        root_dirs: Vec<PathBuf>,
    }

    impl FakeLaunchEnv {
        /// No environment, nothing on `PATH`, no anti-cheat runtimes.
        pub fn new() -> Self {
            Self::default()
        }

        /// Variables visible to [`Env::var`] — `HOME`, `PATH`, the XDG
        /// overrides, `GAMEHANDLER_DXVK_ROOT`.
        pub fn with_vars(mut self, pairs: &[(&str, &str)]) -> Self {
            self.vars = pairs
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            self
        }

        /// The environment a launch starts from, i.e. what `dict(os.environ)`
        /// would have produced.
        pub fn with_environ(mut self, pairs: &[(&str, &str)]) -> Self {
            self.environ = pairs
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            self
        }

        /// Make `name` resolve, as installing it on `PATH` would.
        pub fn with_which(mut self, name: &str, path: &str) -> Self {
            self.which.insert(name.to_string(), PathBuf::from(path));
            self
        }

        /// Make `find_anticheat_runtime(kind, …)` return `path`.
        pub fn with_anticheat(mut self, kind: &str, path: &str) -> Self {
            self.anticheat.insert(kind.to_string(), path.to_string());
            self
        }

        /// The anti-cheat roots this host is pretending to have.
        ///
        /// Stands in for [`anticheat_roots_in`]. Deliberately empty by default
        /// rather than falling back to the real list: a fallback would make any
        /// case that only declared `extra_roots` depend on whether the build
        /// machine happens to have Lutris installed, which is exactly the
        /// non-hermeticity this seam exists to remove. Found the hard way — the
        /// first draft of this fake returned a real
        /// `~/.local/share/lutris/runtime/eac_runtime` from the development box.
        pub fn with_root_dirs(mut self, roots: &[&str]) -> Self {
            self.root_dirs = roots.iter().map(PathBuf::from).collect();
            self
        }

        /// The declared variables, for asserting a `which` spec was consulted.
        pub fn environ_of(&self) -> &BTreeMap<String, String> {
            &self.environ
        }
    }

    impl Env for FakeLaunchEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    impl LaunchEnv for FakeLaunchEnv {
        fn which(&self, name: &str) -> Option<PathBuf> {
            self.which.get(name).cloned()
        }

        fn environ(&self) -> BTreeMap<String, String> {
            self.environ.clone()
        }

        fn var(&self, name: &str) -> Option<String> {
            Env::var(self, name)
        }

        fn anticheat_runtime(&self, kind: &str, extra_roots: &[PathBuf]) -> Option<String> {
            if let Some(found) = self.anticheat.get(kind) {
                return Some(found.clone());
            }
            // Only what the case declared, in production's order: the caller's
            // extra roots first, then the host's own list.
            let mut roots = extra_roots.to_vec();
            roots.extend(self.root_dirs.iter().cloned());
            search_anticheat(kind, &roots)
        }
    }

    pub(crate) fn scratch(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("gh-env-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn which_finds_an_executable_on_path() {
        let root = scratch("which-found");
        make_executable(&root.join("wine"));
        let env = FakeEnv::new(&[("PATH", root.to_str().unwrap())]);
        assert_eq!(which_in("wine", &env), Some(root.join("wine")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn which_ignores_a_directory_named_like_the_binary() {
        // `shutil.which` requires a *file*; a directory with the right name is
        // not a hit. Without the `is_dir` check this returns the directory and
        // the launch fails at `exec` with a confusing error.
        let root = scratch("which-dir");
        fs::create_dir_all(root.join("wine")).unwrap();
        let env = FakeEnv::new(&[("PATH", root.to_str().unwrap())]);
        assert_eq!(which_in("wine", &env), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn which_ignores_a_non_executable_file() {
        let root = scratch("which-noexec");
        fs::write(root.join("wine"), "not a program\n").unwrap();
        let env = FakeEnv::new(&[("PATH", root.to_str().unwrap())]);
        assert_eq!(which_in("wine", &env), None);
        let _ = fs::remove_dir_all(&root);
    }

    /// **`access(2)` asks whether *we* can execute the file, not whether
    /// anyone can.**
    ///
    /// The test creates the file, so the process owns it; mode `0o001` then
    /// grants execute to the "other" class only. `os.access(name, os.X_OK)`
    /// consults the class the mode grants *us* — the owner bits — and answers
    /// no; the `mode() & 0o111` check this used to be answered yes, so
    /// `which_in` reported a binary `exec` would refuse (BUG-23).
    ///
    /// Uid 0 is exempt from the asymmetry — `access` succeeds for root when
    /// any execute bit is set — so the divergence only exists for a non-root
    /// process. `/proc/self`'s owner is the real uid, the same one `access`
    /// consults, and reading it needs no `rustix::process` feature.
    #[test]
    fn which_ignores_a_file_executable_only_by_another_class() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if std::fs::metadata("/proc/self")
            .map(|meta| meta.uid() == 0)
            .unwrap_or(true)
        {
            return;
        }
        let root = scratch("which-other-class");
        let probe = root.join("wine");
        fs::write(&probe, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&probe).unwrap().permissions();
        permissions.set_mode(0o001);
        fs::set_permissions(&probe, permissions).unwrap();
        let env = FakeEnv::new(&[("PATH", root.to_str().unwrap())]);
        assert_eq!(which_in("wine", &env), None);
        let _ = fs::remove_dir_all(&root);
    }

    /// **`PATH` set to `""` searches nothing; unset falls back to `DEFPATH`.**
    ///
    /// The two cases are one character apart in the environment and opposite in
    /// CPython: `os.environ.get("PATH")` answers `""` for the first, which is
    /// falsy, and `if not path: return None` returns before any directory is
    /// tried. Only the *unset* case reaches `os.confstr("CS_PATH")` /
    /// `os.defpath`. This port folded them together until `BUG-37`.
    ///
    /// Measured against CPython 3.14.7 on this host, for the same four inputs:
    /// `path=""` → `None`, `PATH` unset → `/usr/bin/sh`, `path="/usr/bin"` →
    /// `/usr/bin/sh`, `path=":"` → `'sh'` when the cwd holds an executable `sh`.
    ///
    /// The `:` half needs a *controlled working directory* and is therefore in
    /// `tests/which_cwd.rs`, which is its own process and can `chdir`. The test
    /// that used to sit here was named
    /// `which_checks_an_empty_path_entry_against_the_current_directory` and
    /// created its probe in a scratch **directory** without ever entering it,
    /// so it asserted against the default path and could not have observed the
    /// behaviour its name claims.
    #[test]
    fn which_refuses_search_when_path_is_set_but_empty() {
        let root = scratch("which-empty");
        let probe = root.join("gh-empty-path-probe");
        make_executable(&probe);

        // Set-but-empty: no search, even though the binary would be found if
        // the entry were treated as "the current directory" or as "unset".
        let empty = FakeEnv::new(&[("PATH", "")]);
        assert_eq!(which_in("gh-empty-path-probe", &empty), None);

        // Unset: the default path, which does not contain our scratch dir
        // either — so this assertion passes for a reason independent of the
        // one above, and a port that returned `None` unconditionally would
        // fail the third case.
        let unset = FakeEnv::new(&[]);
        assert_eq!(which_in("gh-empty-path-probe", &unset), None);

        // The default path really is searched when `PATH` is unset, so the
        // second assertion is not passing because nothing is ever found.
        assert!(
            which_in("sh", &unset).is_some(),
            "an unset PATH must still reach DEFPATH — a bare `None` would make \
             every wrapper lookup fail on a host that starts with no PATH"
        );

        // And a real path still works, so the first assertion is about the
        // empty string rather than about `which_in` being broken.
        let real = FakeEnv::new(&[("PATH", root.to_str().unwrap())]);
        assert_eq!(which_in("gh-empty-path-probe", &real), Some(probe));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn which_treats_a_name_with_a_separator_as_a_path() {
        let root = scratch("which-slash");
        let binary = root.join("bin/wine");
        make_executable(&binary);
        let env = FakeEnv::new(&[("PATH", "")]);
        assert_eq!(
            which_in(binary.to_str().unwrap(), &env),
            Some(binary.clone())
        );
        assert_eq!(
            which_in(root.join("bin/nope").to_str().unwrap(), &env),
            None
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn which_returns_none_for_an_empty_name() {
        let env = FakeEnv::new(&[("PATH", "/bin")]);
        assert_eq!(which_in("", &env), None);
    }

    #[test]
    fn anticheat_search_prefers_an_extra_root_over_the_built_in_list() {
        let root = scratch("anticheat-extra");
        let runtime = root.join("EasyAntiCheatRuntime");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("marker"), "x").unwrap();

        let env = FakeEnv::new(&[("HOME", "/nonexistent-home-for-test")]);
        let found = find_anticheat_runtime("eac", std::slice::from_ref(&root), &env);
        assert_eq!(found, Some(runtime.to_string_lossy().into_owned()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn anticheat_search_finds_a_nested_runtime_under_a_steam_tool_tree() {
        // The `files/share/<name>` layout Steam and Lutris use.
        let root = scratch("anticheat-nested");
        let nested = root.join("Proton-9.0/files/share/proton-battleye-runtime");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("marker"), "x").unwrap();

        let env = FakeEnv::new(&[("HOME", "/nonexistent-home-for-test")]);
        let found = find_anticheat_runtime("battleye", std::slice::from_ref(&root), &env);
        assert_eq!(found, Some(nested.to_string_lossy().into_owned()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_runtime_directory_does_not_count() {
        // `any(path.iterdir())` is the test — an empty directory is not a
        // runtime, and treating it as one points Proton at a directory with
        // nothing in it.
        let root = scratch("anticheat-empty");
        fs::create_dir_all(root.join("battleye_runtime")).unwrap();
        let env = FakeEnv::new(&[("HOME", "/nonexistent-home-for-test")]);
        assert_eq!(
            find_anticheat_runtime("battleye", std::slice::from_ref(&root), &env),
            None
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_fake_falls_back_to_the_real_anticheat_search() {
        // A case that declares the runtime it expects gets it; a case that
        // declares only extra roots still exercises the real search, so a
        // vector can cover both the happy path and the probe itself.
        let root = scratch("fake-fallback");
        let runtime = root.join("battleye_runtime");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("marker"), "x").unwrap();

        let declared = FakeLaunchEnv::new()
            .with_anticheat("battleye", "/declared/battleye")
            .with_root_dirs(&[root.to_str().unwrap()]);
        assert_eq!(
            declared.anticheat_runtime("battleye", &[]).as_deref(),
            Some("/declared/battleye")
        );
        // The declared value wins for its own kind only; `eac` falls through
        // to the probe over the declared root, which has nothing for it.
        assert_eq!(declared.anticheat_runtime("eac", &[]), None);
        // And a host with no declared roots finds nothing at all, however many
        // real runtimes the build machine happens to have installed.
        let empty = FakeLaunchEnv::new();
        assert_eq!(empty.anticheat_runtime("battleye", &[]), None);
        assert_eq!(empty.anticheat_runtime("eac", &[]), None);

        let probed = FakeLaunchEnv::new().with_root_dirs(&[root.to_str().unwrap()]);
        assert_eq!(
            probed.anticheat_runtime("battleye", &[]).as_deref(),
            Some(runtime.to_string_lossy().as_ref())
        );

        let described = FakeLaunchEnv::new().with_environ(&[("HOME", "/home/tester")]);
        assert_eq!(
            described.environ_of().get("HOME").map(String::as_str),
            Some("/home/tester")
        );
        assert_eq!(
            described.environ(),
            BTreeMap::from([("HOME".to_string(), "/home/tester".to_string())])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_anticheat_kind_is_not_an_error() {
        let env = FakeEnv::new(&[("HOME", "/nonexistent-home-for-test")]);
        assert_eq!(find_anticheat_runtime("vac", &[], &env), None);
    }

    /// The bytes of a variable that is not valid UTF-8: `0xFF` alone.
    #[cfg(unix)]
    fn invalid_utf8() -> std::ffi::OsString {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![0xff])
    }

    #[test]
    #[cfg(unix)]
    fn a_non_utf8_variable_is_dropped_rather_than_panicking() {
        // The defect: `std::env::vars()` panics on this input, and it was the
        // first thing every launch path called.
        let decoded = decode_environ([
            (
                std::ffi::OsString::from("GOOD"),
                std::ffi::OsString::from("kept"),
            ),
            (std::ffi::OsString::from("BAD_VALUE"), invalid_utf8()),
            (invalid_utf8(), std::ffi::OsString::from("bad name")),
            (
                std::ffi::OsString::from("ALSO_GOOD"),
                std::ffi::OsString::from("kept"),
            ),
        ]);

        // The representable entries survive, in both directions.
        assert_eq!(decoded.get("GOOD").map(String::as_str), Some("kept"));
        assert_eq!(decoded.get("ALSO_GOOD").map(String::as_str), Some("kept"));
        // The two unrepresentable ones are gone, and one bad variable does not
        // take its neighbours with it.
        assert_eq!(decoded.len(), 2, "got {decoded:?}");
        assert!(!decoded.contains_key("BAD_VALUE"));
    }

    #[test]
    fn an_ordinary_environment_is_unchanged_by_the_decode() {
        // Anti-vacuity: the decoding must not alter the ordinary case, or the
        // test above would pass on a function that returns nothing at all.
        let decoded = decode_environ([
            (
                std::ffi::OsString::from("HOME"),
                std::ffi::OsString::from("/home/tester"),
            ),
            (
                std::ffi::OsString::from("PATH"),
                std::ffi::OsString::from("/usr/bin"),
            ),
        ]);
        assert_eq!(
            decoded,
            BTreeMap::from([
                ("HOME".to_string(), "/home/tester".to_string()),
                ("PATH".to_string(), "/usr/bin".to_string()),
            ])
        );
    }
}
