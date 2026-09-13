//! XDG-compliant filesystem locations, ported from `gamehandler/config.py`.
//!
//! Every path is resolved on demand rather than cached, exactly as in Python,
//! so tests can redirect the whole tree by setting environment variables before
//! calling anything (`GAMEHANDLER_DATA_HOME` / `GAMEHANDLER_CONFIG_HOME` win
//! over `XDG_*`, which win over the `~/.local/share` and `~/.config` defaults).
//! Keeping that indirection is not incidental — the Python test suite relies on
//! it, and `docs/migration/architecture.md` §7.2 lists it as behaviour that
//! must survive the port.
//!
//! The environment is behind the [`Env`] trait rather than read directly from
//! [`std::env`](mod@std::env). Under Rust 2024 `std::env::set_var` is `unsafe`, and
//! this crate denies `unsafe_code`, so a test cannot mutate the process
//! environment to redirect a path. Injecting a `FakeEnv` does the same job
//! without it.
//!
//! `FakeEnv` is `tests::FakeEnv` and is written as plain code rather than a
//! link on purpose: `tests` is private, so no public doc can address it. (The
//! `mod@` above disambiguates the module from the `std::env!` macro of the same
//! name — a bare `std::env` in either the label or the target is ambiguous, and
//! rustdoc refuses to guess.)

use std::path::{Path, PathBuf};

/// Read access to the process environment.
pub trait Env {
    /// The value of `key`, or `None` when it is unset.
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnv;

impl Env for SystemEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// The user's home directory, as `Path.home()` resolves it.
///
/// Python falls back to the passwd database when `HOME` is unset; `/` is used
/// instead here. `HOME` is always set inside the Flatpak sandbox, and the
/// `GAMEHANDLER_*` overrides exist for every other case, so the fallback is
/// only reachable in a bare container.
fn home(env: &dyn Env) -> PathBuf {
    match non_empty(env, "HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from("/"),
    }
}

/// An environment variable, treating the empty string as unset (as Python's
/// `if value:` tests do).
fn non_empty(env: &dyn Env, key: &str) -> Option<String> {
    env.var(key).filter(|value| !value.is_empty())
}

/// One of the XDG base directories, `GAMEHANDLER_*` override first.
fn base_dir(env: &dyn Env, override_key: &str, xdg_key: &str, default: PathBuf) -> PathBuf {
    if let Some(value) = non_empty(env, override_key) {
        return PathBuf::from(value);
    }
    match non_empty(env, xdg_key) {
        Some(value) => PathBuf::from(value),
        None => default,
    }
    .join("gamehandler")
}

/// The user's home directory, as `Path.home()` resolves it.
///
/// Exposed because `runners.py` calls `Path.home()` directly in two places that
/// are not XDG lookups — the anti-cheat runtime search (`runners.py:1119-1122`)
/// and the desktop-shortcut directory (`runners.py:1451`) — and both must honour
/// an injected `HOME` for the same reason every other path here does.
pub fn home_dir() -> PathBuf {
    home_dir_in(&SystemEnv)
}

/// [`home_dir`], reading `HOME` from `env`.
pub fn home_dir_in(env: &dyn Env) -> PathBuf {
    home(env)
}

/// Base directory for persistent application data (runners, prefixes, covers).
pub fn data_home() -> PathBuf {
    data_home_in(&SystemEnv)
}

/// Base directory for configuration files.
pub fn config_home() -> PathBuf {
    config_home_in(&SystemEnv)
}

/// The library file: `$config_home/games.json`.
pub fn games_file() -> PathBuf {
    games_file_in(&SystemEnv)
}

/// The preferences file: `$config_home/settings.json`.
pub fn settings_file() -> PathBuf {
    settings_file_in(&SystemEnv)
}

/// Where downloaded Proton/Wine builds are extracted.
pub fn runners_dir() -> PathBuf {
    runners_dir_in(&SystemEnv)
}

/// Default parent directory for per-game Wine prefixes.
pub fn prefixes_dir() -> PathBuf {
    prefixes_dir_in(&SystemEnv)
}

/// Where downloaded and imported cover images are stored.
pub fn covers_dir() -> PathBuf {
    covers_dir_in(&SystemEnv)
}

/// Where vendor installer files are cached.
pub fn downloads_dir() -> PathBuf {
    downloads_dir_in(&SystemEnv)
}

/// Create every directory the application writes into.
///
/// Returns the first failure. Python's version raises on the same condition.
pub fn ensure_dirs() -> std::io::Result<()> {
    ensure_dirs_in(&SystemEnv)
}

// ---------------------------------------------------------------------------
// Injected-environment variants. The public functions above are one-line
// wrappers; everything testable lives here.
// ---------------------------------------------------------------------------

/// [`data_home`] against an injected environment.
pub fn data_home_in(env: &dyn Env) -> PathBuf {
    base_dir(
        env,
        "GAMEHANDLER_DATA_HOME",
        "XDG_DATA_HOME",
        home(env).join(".local").join("share"),
    )
}

/// [`config_home`] against an injected environment.
pub fn config_home_in(env: &dyn Env) -> PathBuf {
    base_dir(
        env,
        "GAMEHANDLER_CONFIG_HOME",
        "XDG_CONFIG_HOME",
        home(env).join(".config"),
    )
}

/// [`games_file`] against an injected environment.
pub fn games_file_in(env: &dyn Env) -> PathBuf {
    config_home_in(env).join("games.json")
}

/// [`settings_file`] against an injected environment.
pub fn settings_file_in(env: &dyn Env) -> PathBuf {
    config_home_in(env).join("settings.json")
}

/// [`runners_dir`] against an injected environment.
pub fn runners_dir_in(env: &dyn Env) -> PathBuf {
    data_home_in(env).join("runners")
}

/// [`prefixes_dir`] against an injected environment.
pub fn prefixes_dir_in(env: &dyn Env) -> PathBuf {
    data_home_in(env).join("prefixes")
}

/// [`covers_dir`] against an injected environment.
pub fn covers_dir_in(env: &dyn Env) -> PathBuf {
    data_home_in(env).join("covers")
}

/// [`downloads_dir`] against an injected environment.
pub fn downloads_dir_in(env: &dyn Env) -> PathBuf {
    data_home_in(env).join("downloads")
}

/// [`ensure_dirs`] against an injected environment.
pub fn ensure_dirs_in(env: &dyn Env) -> std::io::Result<()> {
    for path in [
        config_home_in(env),
        data_home_in(env),
        runners_dir_in(env),
        prefixes_dir_in(env),
        covers_dir_in(env),
        downloads_dir_in(env),
    ] {
        create_dir_all(&path)?;
    }
    Ok(())
}

/// `mkdir -p`, skipping the call when the directory already exists so a
/// read-only parent is not probed for nothing.
fn create_dir_all(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An environment built from literal pairs, for tests.
    #[derive(Debug, Default)]
    pub struct FakeEnv {
        vars: Vec<(String, String)>,
    }

    impl FakeEnv {
        pub fn new(pairs: &[(&str, &str)]) -> Self {
            Self {
                vars: pairs
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect(),
            }
        }
    }

    impl Env for FakeEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    const HOME: &str = "/home/tester";

    #[test]
    fn defaults_follow_the_xdg_specification() {
        let env = FakeEnv::new(&[("HOME", HOME)]);
        assert_eq!(
            data_home_in(&env),
            path("/home/tester/.local/share/gamehandler")
        );
        assert_eq!(
            config_home_in(&env),
            path("/home/tester/.config/gamehandler")
        );
        assert_eq!(
            games_file_in(&env),
            path("/home/tester/.config/gamehandler/games.json")
        );
        assert_eq!(
            settings_file_in(&env),
            path("/home/tester/.config/gamehandler/settings.json")
        );
    }

    #[test]
    fn xdg_variables_take_precedence_over_the_defaults() {
        let env = FakeEnv::new(&[
            ("HOME", HOME),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
        ]);
        assert_eq!(data_home_in(&env), path("/xdg/data/gamehandler"));
        assert_eq!(config_home_in(&env), path("/xdg/config/gamehandler"));
    }

    #[test]
    fn gamehandler_overrides_beat_the_xdg_variables() {
        let env = FakeEnv::new(&[
            ("HOME", HOME),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
            ("GAMEHANDLER_DATA_HOME", "/redirected/data"),
            ("GAMEHANDLER_CONFIG_HOME", "/redirected/config"),
        ]);
        assert_eq!(data_home_in(&env), path("/redirected/data"));
        assert_eq!(config_home_in(&env), path("/redirected/config"));
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        // Python tests `if value:`, so "" falls through to the next source.
        let env = FakeEnv::new(&[
            ("HOME", HOME),
            ("GAMEHANDLER_DATA_HOME", ""),
            ("GAMEHANDLER_CONFIG_HOME", ""),
            ("XDG_DATA_HOME", ""),
        ]);
        assert_eq!(
            data_home_in(&env),
            path("/home/tester/.local/share/gamehandler")
        );
        assert_eq!(
            config_home_in(&env),
            path("/home/tester/.config/gamehandler")
        );
    }

    #[test]
    fn data_subdirectories_hang_off_data_home() {
        let env = FakeEnv::new(&[("GAMEHANDLER_DATA_HOME", "/d")]);
        assert_eq!(runners_dir_in(&env), path("/d/runners"));
        assert_eq!(prefixes_dir_in(&env), path("/d/prefixes"));
        assert_eq!(covers_dir_in(&env), path("/d/covers"));
        assert_eq!(downloads_dir_in(&env), path("/d/downloads"));
    }

    #[test]
    fn the_overrides_are_independent() {
        // Redirecting data must not move the config file, and vice versa.
        let env = FakeEnv::new(&[("HOME", HOME), ("GAMEHANDLER_CONFIG_HOME", "/c")]);
        assert_eq!(games_file_in(&env), path("/c/games.json"));
        assert_eq!(
            data_home_in(&env),
            path("/home/tester/.local/share/gamehandler")
        );
    }

    #[test]
    fn ensure_dirs_creates_the_whole_tree() {
        let root = std::env::temp_dir().join(format!("gh-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let env = FakeEnv::new(&[
            ("GAMEHANDLER_DATA_HOME", root.join("data").to_str().unwrap()),
            (
                "GAMEHANDLER_CONFIG_HOME",
                root.join("config").to_str().unwrap(),
            ),
        ]);

        ensure_dirs_in(&env).expect("ensure_dirs should create every directory");

        for path in [
            config_home_in(&env),
            data_home_in(&env),
            runners_dir_in(&env),
            prefixes_dir_in(&env),
            covers_dir_in(&env),
            downloads_dir_in(&env),
        ] {
            assert!(path.is_dir(), "{} should exist", path.display());
        }
        // Idempotent: a second call is not an error.
        ensure_dirs_in(&env).expect("ensure_dirs should be idempotent");

        let _ = std::fs::remove_dir_all(&root);
    }

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }
}
