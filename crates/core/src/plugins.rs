//! Optional launch helpers (MangoHud, GameMode, Winetricks, UMU, Gamescope) —
//! a port of `gamehandler/plugins.py`.
//!
//! Source installations can offer a package-manager command when a helper is
//! missing. Flatpak builds must never run sandbox package managers as if they
//! could modify the host, so installation is explicitly disabled there.
//!
//! # One function is deliberately not here
//!
//! `plugins.py:172-175`, `install_plugin`, calls
//! `subprocess.run(argv, check=False, timeout=timeout)`. It is the **only**
//! execution in `plugins.py` or `credits.py`: every other name in both modules
//! either describes a command as data or answers a question about the host.
//! Process execution is not this module's, so `install_plugin` was routed back
//! to the lead rather than ported, and nothing here spawns anything. The
//! consequence to know about: this module can tell you what the install command
//! *is* and refuse to invent one, but it cannot install.
//!
//! # The seam
//!
//! `plugins.py` reads the world through `os.environ`, `shutil.which`,
//! `Path.exists` and `os.geteuid`, and `tests/test_plugins.py` steers all of
//! them by `mock.patch`. A Rust module has no global to patch, so those four
//! reads are behind [`PluginEnv`], the same shape [`crate::runners::env`]
//! documents for the runner code (DECISIONS D-27). `cargo test
//! -p gamehandler-core` stays instant, headless and offline because of it.
//!
//! [`PluginEnv::which`] delegates to
//! [`crate::runners::env::which_in`] rather than reimplementing `shutil.which`
//! — CPython's empty-`PATH`-entry and default-path rules are already reasoned
//! out and tested there, and a second copy would be a second thing to get
//! wrong.
//!
//! # The oracle
//!
//! `tests/test_plugins.py` is the behavioural oracle and is green (`python3 -m
//! unittest tests.test_plugins`). Its assertions are ported one for one below,
//! including the ones that look trivial: `test_package_manager_detection_is_a_string`
//! becomes a `&str`-returning signature, which makes the type check a compiler
//! check instead of a runtime one.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::paths::{Env, SystemEnv};

/// One optional helper, mirrors the `Plugin` dataclass.
///
/// `packages` is a slice of pairs rather than a map: it preserves the
/// reference's declaration order, and with five entries a linear lookup is
/// both faster and simpler than a hashed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plugin {
    /// The stable id the QML hands back to `installPlugin`, e.g. `"mangohud"`.
    pub id: &'static str,
    /// The display name, e.g. `"MangoHud"`.
    pub name: &'static str,
    /// The executable `is_installed` looks for on `PATH`.
    pub binary: &'static str,
    /// One line saying what the helper is.
    pub description: &'static str,
    /// One line saying how GameHandler uses it.
    pub used_for: &'static str,
    /// `(package manager, package name)` pairs. Not every helper is packaged
    /// for every manager: UMU has no `zypper` or `flatpak` entry in the
    /// reference, and only MangoHud and Gamescope are published as Flatpak
    /// extensions.
    pub packages: &'static [(&'static str, &'static str)],
}

impl Plugin {
    /// The package this helper is known as under `manager`, or `None`.
    ///
    /// Python: `plugin.packages.get(manager)`, which answers `None` for a
    /// missing key and for a manager it has never heard of alike.
    pub fn package_for(&self, manager: &str) -> Option<&'static str> {
        self.packages
            .iter()
            .find(|(key, _)| *key == manager)
            .map(|(_, package)| *package)
    }

    /// `shutil.which(self.binary) is not None` — is the helper on `PATH`.
    pub fn is_installed(&self, env: &dyn PluginEnv) -> bool {
        env.which(self.binary).is_some()
    }
}

/// The helper catalogue, transcribed from `PLUGINS` in declaration order.
pub const PLUGINS: &[Plugin; 5] = &[
    Plugin {
        id: "mangohud",
        name: "MangoHud",
        binary: "mangohud",
        description: "On-screen overlay for FPS, frame times, GPU, and CPU.",
        used_for: "Enable it per game under Launch options, or as the default in Settings.",
        packages: &[
            ("apt", "mangohud"),
            ("pacman", "mangohud"),
            ("dnf", "mangohud"),
            ("zypper", "mangohud"),
            ("flatpak", "org.freedesktop.Platform.VulkanLayer.MangoHud"),
        ],
    },
    Plugin {
        id: "gamemode",
        name: "Feral GameMode",
        binary: "gamemoderun",
        description: "Temporarily tunes the system for better game performance.",
        used_for: "Wraps the launch command when GameMode is enabled on a game.",
        packages: &[
            ("apt", "gamemode"),
            ("pacman", "gamemode"),
            ("dnf", "gamemode"),
            ("zypper", "gamemode"),
        ],
    },
    Plugin {
        id: "winetricks",
        name: "Winetricks",
        binary: "winetricks",
        description: "Installs common Windows runtimes, fonts, and DLL overrides.",
        used_for: "Open it from a game’s Prefix tools menu.",
        packages: &[
            ("apt", "winetricks"),
            ("pacman", "winetricks"),
            ("dnf", "winetricks"),
            ("zypper", "winetricks"),
        ],
    },
    Plugin {
        id: "umu",
        name: "UMU Launcher",
        binary: "umu-run",
        description: "Runs Proton builds with the Steam runtime outside Steam.",
        used_for: "Used automatically when a downloaded Proton build includes a proton script.",
        packages: &[
            ("apt", "umu-launcher"),
            ("pacman", "umu-launcher"),
            ("dnf", "umu-launcher"),
        ],
    },
    Plugin {
        id: "gamescope",
        name: "Gamescope",
        binary: "gamescope",
        description: "Nested compositor for scaling, HDR, and a stable game session.",
        used_for: "Wraps the launch command when Gamescope is enabled on a game.",
        packages: &[
            ("apt", "gamescope"),
            ("pacman", "gamescope"),
            ("dnf", "gamescope"),
            ("zypper", "gamescope"),
            ("flatpak", "org.freedesktop.Platform.VulkanLayer.gamescope"),
        ],
    },
];

/// Every helper, in the reference's order.
pub fn plugins() -> &'static [Plugin] {
    PLUGINS
}

/// The helper with this id, or `None`.
///
/// Python raises `KeyError`; see the module note in [`crate::credits`] for why
/// the port answers with an `Option`.
pub fn plugin_by_id(plugin_id: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|plugin| plugin.id == plugin_id)
}

/// The impure lookups `plugins.py` performs, injectable for tests.
pub trait PluginEnv: Env {
    /// `shutil.which(name)` — the resolved path, or `None`.
    fn which(&self, name: &str) -> Option<PathBuf>;

    /// `Path(path).exists()`.
    ///
    /// Only ever asked about absolute system paths (`/.flatpak-info`,
    /// `/etc/debian_version`, `/etc/arch-release`), so the two places the
    /// reference tests it are the two it uses.
    fn exists(&self, path: &str) -> bool;

    /// `os.geteuid()`, or `None` where the host cannot be asked.
    ///
    /// `None` is a divergence, and it is in the safe direction: every caller
    /// compares against `Some(0)` to decide "already root, do not prefix a
    /// privilege helper", so an unknown uid is treated as *not* root and the
    /// command is prefixed. That either works or is refused by `pkexec`/`sudo`
    /// with a visible error, where the opposite guess would silently attempt an
    /// unprivileged install that cannot succeed.
    fn euid(&self) -> Option<u32>;
}

/// The real host: the process environment, the real filesystem and `PATH`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPluginEnv;

impl Env for SystemPluginEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl PluginEnv for SystemPluginEnv {
    fn which(&self, name: &str) -> Option<PathBuf> {
        crate::runners::env::which_in(name, &SystemEnv)
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn euid(&self) -> Option<u32> {
        effective_uid()
    }
}

/// `os.geteuid()`, read from `/proc`.
///
/// There is no `std` way to ask for the effective uid and this crate denies
/// `unsafe_code`, so the syscall path is closed. `rustix` is already a
/// dependency of this crate but only with its `fs` feature, and
/// `rustix::process::geteuid` is behind `process`; enabling it is a manifest
/// change that belongs to whoever owns the dependency set, so this reads
/// `/proc/self/status` instead. Its `Uid:` line is
/// `real effective saved filesystem`, which makes this exactly `geteuid()` on
/// Linux and not an approximation of it.
///
/// `/proc` is mounted on every kernel GameHandler can run on, including inside
/// the Flatpak sandbox — but `None` is returned rather than a guess if it is
/// not, and the caller's fallback is documented on [`PluginEnv::euid`].
fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            return rest.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

/// Whether GameHandler is running in a Flatpak sandbox (`plugins.py:107-109`).
pub fn in_flatpak(env: &dyn PluginEnv) -> bool {
    match env.var("FLATPAK_ID") {
        Some(value) => !value.is_empty(),
        None => env.exists("/.flatpak-info"),
    }
}

/// A short id for the host package manager, or `""`.
///
/// The order matters and is the reference's: `apt` needs both the binary and
/// `/etc/debian_version` (because `apt-get` is installed on plenty of non-Debian
/// systems), `pacman` needs both, and `dnf`/`zypper`/`flatpak` are decided by
/// the binary alone. Flatpak is checked first and short-circuits everything:
/// inside the sandbox the host's manager is not ours to use.
pub fn detect_package_manager(env: &dyn PluginEnv) -> &'static str {
    if in_flatpak(env) {
        return "";
    }
    if env.which("apt-get").is_some() && env.exists("/etc/debian_version") {
        return "apt";
    }
    if env.which("pacman").is_some() && env.exists("/etc/arch-release") {
        return "pacman";
    }
    if env.which("dnf").is_some() {
        return "dnf";
    }
    if env.which("zypper").is_some() {
        return "zypper";
    }
    if env.which("flatpak").is_some() {
        return "flatpak";
    }
    ""
}

/// Why no install command can be produced.
///
/// The variants carry the reference's `RuntimeError` messages verbatim;
/// `test_plugins.py::test_flatpak_never_offers_host_package_commands` asserts
/// on the text (`assertRaisesRegex(RuntimeError, "unavailable inside Flatpak")`),
/// so the message is part of the contract rather than a convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// Raised inside the sandbox, before any manager is considered.
    Flatpak,
    /// No package is known for this helper under the detected or given manager.
    NoPackage {
        /// The helper's display name, as the message names it.
        plugin: &'static str,
    },
    /// The manager is known but has no argument shape. Unreachable with the
    /// catalogue as transcribed; see the test that exercises it.
    UnsupportedManager(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::Flatpak => f.write_str(
                "Plugin installation is unavailable inside Flatpak; sandbox package \
                 managers cannot install or expose host packages",
            ),
            InstallError::NoPackage { plugin } => {
                write!(f, "No install package is known for {plugin} on this system")
            }
            InstallError::UnsupportedManager(manager) => {
                write!(f, "Unsupported package manager: {manager}")
            }
        }
    }
}

impl std::error::Error for InstallError {}

/// The argv used to install `plugin`, without a privilege helper.
///
/// `manager` is `None` to detect one, matching Python's
/// `manager: str | None = None`. An **empty** manager string also detects one,
/// because the reference writes `manager or detect_package_manager()` and `""`
/// is falsy in Python — a port that only tested for `None` would take a
/// different branch for `Some("")`.
pub fn install_command(
    plugin: &Plugin,
    manager: Option<&str>,
    env: &dyn PluginEnv,
) -> Result<Vec<String>, InstallError> {
    if in_flatpak(env) {
        return Err(InstallError::Flatpak);
    }
    let manager = match manager {
        Some(value) if !value.is_empty() => value,
        _ => detect_package_manager(env),
    };
    let package = plugin.package_for(manager);
    let package = match (manager.is_empty(), package) {
        (false, Some(package)) => package,
        _ => {
            return Err(InstallError::NoPackage {
                plugin: plugin.name,
            });
        }
    };
    let argv: &[&str] = match manager {
        "apt" => &["apt-get", "install", "-y", package],
        "pacman" => &["pacman", "-S", "--noconfirm", package],
        "dnf" => &["dnf", "install", "-y", package],
        "zypper" => &["zypper", "--non-interactive", "install", package],
        "flatpak" => &["flatpak", "install", "-y", "flathub", package],
        _ => return Err(InstallError::UnsupportedManager(manager.to_string())),
    };
    Ok(argv.iter().map(|part| (*part).to_string()).collect())
}

/// Prefix `argv` with `pkexec` or `sudo` when installing system packages.
///
/// A Flatpak install needs no privilege escalation — it is the sandbox's own
/// package manager writing to the user's flatpak installation — so the
/// `flatpak` case returns the argv untouched. Python tests `if argv and
/// argv[0] == "flatpak"`, i.e. an *empty* argv falls through to the uid check
/// rather than being special-cased.
pub fn privileged_command(argv: &[String], env: &dyn PluginEnv) -> Vec<String> {
    if argv
        .first()
        .is_some_and(|first| first.as_str() == "flatpak")
    {
        return argv.to_vec();
    }
    if env.euid() == Some(0) {
        return argv.to_vec();
    }
    if let Some(pkexec) = env.which("pkexec") {
        return prefixed(&pkexec, argv);
    }
    if let Some(sudo) = env.which("sudo") {
        return prefixed(&sudo, argv);
    }
    argv.to_vec()
}

/// `[helper, *argv]`, with the helper rendered as `shutil.which` returned it.
fn prefixed(helper: &Path, argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 1);
    out.push(helper.to_string_lossy().into_owned());
    out.extend(argv.iter().cloned());
    out
}

/// `" ".join(argv)`, as shown to the user on the Plugins page.
pub fn format_command(argv: &[String]) -> String {
    argv.join(" ")
}

/// The three states the Plugins page renders (P-64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// The helper is on `PATH`.
    Installed,
    /// Not installed, and installable on this host.
    Missing,
    /// Not installed, and not installable from here.
    Unavailable,
}

impl PluginState {
    /// The string the QML compares against (`PluginsPage.qml:48-51`).
    pub fn as_str(self) -> &'static str {
        match self {
            PluginState::Installed => "installed",
            PluginState::Missing => "missing",
            PluginState::Unavailable => "unavailable",
        }
    }
}

/// One row of the Plugins page, mirrors `bridge.py::_plugin_row`'s dict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    /// `pluginId` — what the Install button sends back.
    pub plugin_id: &'static str,
    /// `name`.
    pub name: &'static str,
    /// `subtitle` — the row's one line of explanation.
    pub subtitle: String,
    /// `state` — drives which button the row offers.
    pub state: PluginState,
}

/// Build the Plugins page row for one helper (`bridge.py:951-979`).
///
/// The order of the branches is the reference's and is load-bearing: a helper
/// that *is* installed is reported as installed even inside the sandbox, so the
/// sandbox branch is only reached for one that is missing.
pub fn plugin_row(plugin: &Plugin, env: &dyn PluginEnv) -> PluginRow {
    let (subtitle, state) = if plugin.is_installed(env) {
        (
            format!("{} {}", plugin.description, plugin.used_for),
            PluginState::Installed,
        )
    } else if in_flatpak(env) {
        (
            format!(
                "{} Not bundled in this Flatpak. Installing it on the host would \
                 not expose it to the sandbox.",
                plugin.description
            ),
            PluginState::Unavailable,
        )
    } else {
        match install_command(plugin, None, env) {
            Ok(argv) => (
                format!(
                    "{} Not installed. Install with: {}",
                    plugin.description,
                    format_command(&privileged_command(&argv, env))
                ),
                PluginState::Missing,
            ),
            // The reference catches `RuntimeError` here. Every error this
            // module can raise maps to the same sentence, because the page has
            // one thing to say when it cannot name a command.
            Err(_) => (
                format!(
                    "{} Not installed. No package mapping is available for this system.",
                    plugin.description
                ),
                PluginState::Unavailable,
            ),
        }
    };
    PluginRow {
        plugin_id: plugin.id,
        name: plugin.name,
        subtitle,
        state,
    }
}

/// Every row, in the catalogue's order (`bridge.py::_get_plugins`).
pub fn plugin_rows(env: &dyn PluginEnv) -> Vec<PluginRow> {
    PLUGINS
        .iter()
        .map(|plugin| plugin_row(plugin, env))
        .collect()
}

/// The sentence above the list (`bridge.py::pluginsIntro`).
pub fn plugins_intro(env: &dyn PluginEnv) -> String {
    if in_flatpak(env) {
        return "These tools are optional. GameHandler offers them on each game. \
                The Flatpak can only use helpers bundled in its sandbox; host \
                package installation is intentionally disabled."
            .to_string();
    }
    let manager = detect_package_manager(env);
    let detected = if manager.is_empty() {
        ".".to_string()
    } else {
        format!(" ({manager}).")
    };
    format!(
        "These tools are optional. GameHandler offers them on each game. Missing \
         helpers can be installed with the detected host package manager{detected}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path as StdPath;

    /// A whole host, faked: `PATH` results, filesystem answers and the uid.
    ///
    /// The Rust equivalent of `mock.patch("gamehandler.plugins.shutil.which")`
    /// plus `mock.patch.dict(os.environ, …)` plus
    /// `mock.patch("gamehandler.plugins.os.geteuid")`, which together are how
    /// `tests/test_plugins.py` steers the reference.
    ///
    /// Deliberately not `Default`: a test that wants a bare host writes
    /// `FakePluginEnv::new()`, and the difference between that and the real
    /// development box is the whole point.
    #[derive(Debug, Default)]
    pub struct FakePluginEnv {
        vars: BTreeMap<String, String>,
        which: BTreeMap<String, String>,
        files: Vec<String>,
        euid: Option<u32>,
    }

    impl FakePluginEnv {
        /// No environment, nothing on `PATH`, no files, unknown uid.
        pub fn new() -> Self {
            Self::default()
        }

        /// `FLATPAK_ID` and anything else [`Env::var`] should see.
        pub fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), value.to_string());
            self
        }

        /// Make `name` resolve on `PATH`.
        pub fn with_which(mut self, name: &str, path: &str) -> Self {
            self.which.insert(name.to_string(), path.to_string());
            self
        }

        /// Make `Path(path).exists()` answer true.
        pub fn with_file(mut self, path: &str) -> Self {
            self.files.push(path.to_string());
            self
        }

        /// `os.geteuid()`.
        pub fn with_euid(mut self, uid: u32) -> Self {
            self.euid = Some(uid);
            self
        }

        /// A host with a package manager but nothing installed — the shape
        /// every "missing" case wants.
        pub fn with_apt_host() -> Self {
            Self::new()
                .with_which("apt-get", "/usr/bin/apt-get")
                .with_file("/etc/debian_version")
        }
    }

    impl Env for FakePluginEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    impl PluginEnv for FakePluginEnv {
        fn which(&self, name: &str) -> Option<PathBuf> {
            self.which.get(name).map(PathBuf::from)
        }

        fn exists(&self, path: &str) -> bool {
            self.files.iter().any(|known| known == path)
        }

        fn euid(&self) -> Option<u32> {
            self.euid
        }
    }

    /// A file from the repository root, read at test time.
    ///
    /// `CARGO_MANIFEST_DIR` is `crates/core`, so the root is two levels up.
    /// Reading the reference off disk is the point: a constant compared against
    /// a copy of itself cannot disagree with itself, which is how a mistyped
    /// string survives a suite.
    fn repo_file(relative: &str) -> String {
        let path = StdPath::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
    }

    /// Python's implicit string concatenation, undone.
    ///
    /// `bridge.py` writes a long sentence as several adjacent literals across
    /// lines, and the joined text is what the user actually reads. Collapsing
    /// `"` + whitespace + `"` back into one literal lets a sentence be looked
    /// up in the source exactly as the page shows it, instead of as the source
    /// happens to wrap it. The whitespace *inside* a literal is left alone —
    /// only the newline and indentation between two literals are dropped.
    ///
    /// A following literal may carry a prefix, and `pluginsIntro` is where that
    /// matters: it is written `"... host package " f"manager{detected}"`, so
    /// the boundary is `"` + whitespace + `f"` and a joiner that only knew
    /// about a bare quote found nothing. Up to two prefix letters are accepted
    /// (`f`, `r`, `b`, `u` and their combinations), and the whitespace
    /// requirement is what keeps this from merging anything that is not a
    /// concatenation — `"Wine",` on the next line is protected by its comma.
    fn join_adjacent_literals(source: &str) -> String {
        fn is_prefix(character: char) -> bool {
            matches!(character, 'f' | 'F' | 'r' | 'R' | 'b' | 'B' | 'u' | 'U')
        }
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '"' {
                let mut probe = index + 1;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                let mut quote = probe;
                while quote < chars.len() && quote - probe < 2 && is_prefix(chars[quote]) {
                    quote += 1;
                }
                if probe > index + 1 && quote < chars.len() && chars[quote] == '"' {
                    index = quote + 1;
                    continue;
                }
            }
            out.push(chars[index]);
            index += 1;
        }
        out
    }

    // ------------------------------------------------------------ catalogue

    #[test]
    fn the_catalogue_is_the_reference_catalogues_data() {
        // The transcription oracle. Every field of every entry is looked up in
        // `plugins.py` as the literal the reference writes, so a mistyped name,
        // binary, description or package — including the curly apostrophe in
        // Winetricks' `used_for` — fails here rather than on the page.
        let source = repo_file("gamehandler/plugins.py");
        for plugin in plugins() {
            for (field, value) in [
                ("id", plugin.id),
                ("name", plugin.name),
                ("binary", plugin.binary),
                ("description", plugin.description),
                ("used_for", plugin.used_for),
            ] {
                let literal = format!("{field}=\"{value}\"");
                assert!(
                    source.contains(&literal),
                    "{:?}'s {field} is not the reference's: {literal:?} does not appear in plugins.py",
                    plugin.id
                );
            }
            for (manager, package) in plugin.packages {
                let literal = format!("\"{manager}\": \"{package}\"");
                assert!(
                    source.contains(&literal),
                    "{:?} maps {manager} to {package:?}, which is not in plugins.py",
                    plugin.id
                );
            }
        }
    }

    #[test]
    fn the_catalogue_has_the_reference_counts_and_order() {
        // Completeness, which the per-field check above cannot catch on its
        // own: an entry omitted from both this table and the check would still
        // pass. The count comes from the Python source, so "five helpers" is
        // derived from the reference rather than asserted by fiat.
        let source = repo_file("gamehandler/plugins.py");
        assert_eq!(plugins().len(), source.matches("Plugin(").count());
        assert_eq!(
            plugins().iter().map(|plugin| plugin.id).collect::<Vec<_>>(),
            vec!["mangohud", "gamemode", "winetricks", "umu", "gamescope"],
            "the catalogue is displayed in the reference's declaration order"
        );
        for plugin in plugins() {
            assert!(
                !plugin.description.is_empty() && !plugin.used_for.is_empty(),
                "{} renders a description and a use; both must be present",
                plugin.id
            );
            assert!(
                !plugin.packages.is_empty(),
                "{} has no package mapping, so every state would be unavailable",
                plugin.id
            );
        }
    }

    #[test]
    fn the_reference_catalogues_own_assertions_hold() {
        // `test_plugins.py::test_mangohud_is_offered` and
        // `test_gamescope_plugin_is_offered`, which pin a binary and a manager
        // per helper.
        let mangohud = plugin_by_id("mangohud").unwrap();
        assert_eq!(mangohud.binary, "mangohud");
        assert!(mangohud.package_for("apt").is_some());

        let gamescope = plugin_by_id("gamescope").unwrap();
        assert_eq!(gamescope.binary, "gamescope");
        assert!(gamescope.package_for("pacman").is_some());

        // `plugin_by_id` raises `KeyError` in Python; `None` here.
        assert!(plugin_by_id("nope").is_none());
    }

    // ------------------------------------------------------- install commands

    #[test]
    fn the_install_command_shapes_are_the_references() {
        // `test_plugins.py::test_apt_install_command` plus the four other
        // managers the reference's if-chain names, every one of which is
        // otherwise unexercised by the Python suite.
        let cases: &[(&str, &[&str])] = &[
            ("apt", &["apt-get", "install", "-y", "mangohud"]),
            ("pacman", &["pacman", "-S", "--noconfirm", "mangohud"]),
            ("dnf", &["dnf", "install", "-y", "mangohud"]),
            (
                "zypper",
                &["zypper", "--non-interactive", "install", "mangohud"],
            ),
            (
                "flatpak",
                &[
                    "flatpak",
                    "install",
                    "-y",
                    "flathub",
                    "org.freedesktop.Platform.VulkanLayer.MangoHud",
                ],
            ),
        ];
        let env = FakePluginEnv::new();
        let mangohud = plugin_by_id("mangohud").unwrap();
        for (manager, expected) in cases {
            let argv = install_command(mangohud, Some(*manager), &env).unwrap();
            assert_eq!(argv, *expected, "the {manager} command shape changed");
        }
    }

    #[test]
    fn a_helper_with_no_entry_for_the_manager_is_refused() {
        // `test_plugins.py::test_unknown_manager_raises`. Note which error this
        // is: an unknown *manager name* never reaches the argv table, because
        // `packages.get(manager)` is already `None`. The message is the
        // package one, not the manager one.
        let env = FakePluginEnv::new();
        let mangohud = plugin_by_id("mangohud").unwrap();
        let error = install_command(mangohud, Some("nix"), &env).unwrap_err();
        assert_eq!(error, InstallError::NoPackage { plugin: "MangoHud" });
        assert_eq!(
            error.to_string(),
            "No install package is known for MangoHud on this system"
        );
    }

    #[test]
    fn an_unknown_manager_name_has_a_reachable_message_of_its_own() {
        // The `Unsupported package manager` arm. With the catalogue as
        // transcribed it is **unreachable**: every manager key in every
        // plugin's package map has an argv shape, so the `packages.get` guard
        // above rejects anything else first. It is kept because the reference
        // keeps it, and it is pinned with a synthetic helper so that the arm is
        // a checked line rather than an untested claim — if a future helper
        // gains, say, an `apk` key, this is the branch that would run.
        let synthetic = Plugin {
            id: "synthetic",
            name: "Synthetic",
            binary: "synthetic",
            description: "d",
            used_for: "u",
            packages: &[("apk", "synthetic")],
        };
        let env = FakePluginEnv::new();
        let error = install_command(&synthetic, Some("apk"), &env).unwrap_err();
        assert_eq!(error, InstallError::UnsupportedManager("apk".to_string()));
        assert_eq!(error.to_string(), "Unsupported package manager: apk");

        // And nothing in the real catalogue can reach it, so the arm's
        // unreachability is recorded rather than assumed.
        for plugin in plugins() {
            for (manager, _) in plugin.packages {
                assert!(
                    ["apt", "pacman", "dnf", "zypper", "flatpak"].contains(manager),
                    "{} introduces the manager {manager:?}, which has no argv shape",
                    plugin.id
                );
            }
        }
    }

    #[test]
    fn an_empty_manager_is_detected_rather_than_taken_literally() {
        // Python writes `manager or detect_package_manager()`, and `""` is
        // falsy. A port testing only for `None` would look up the empty string
        // and report "no package mapping" on a host that has one.
        let env = FakePluginEnv::with_apt_host();
        let mangohud = plugin_by_id("mangohud").unwrap();
        let from_empty = install_command(mangohud, Some(""), &env).unwrap();
        let from_none = install_command(mangohud, None, &env).unwrap();
        assert_eq!(from_empty, from_none);
        assert_eq!(from_empty[0], "apt-get");
    }

    #[test]
    fn format_command_is_a_space_join() {
        // `test_plugins.py::test_format_command`.
        assert_eq!(
            format_command(&[
                "apt-get".to_string(),
                "install".to_string(),
                "-y".to_string(),
                "mangohud".to_string()
            ]),
            "apt-get install -y mangohud"
        );
        // An empty argv joins to the empty string rather than panicking, which
        // `privileged_command` can hand it.
        assert_eq!(format_command(&[]), "");
    }

    // ------------------------------------------------------------ privileges

    #[test]
    fn the_privilege_prefix_prefers_pkexec() {
        // `test_plugins.py::test_privileged_prefix_uses_pkexec`.
        let env = FakePluginEnv::new()
            .with_euid(1000)
            .with_which("pkexec", "/usr/bin/pkexec");
        let argv = vec!["apt-get".to_string(), "install".to_string()];
        let privileged = privileged_command(&argv, &env);
        assert_eq!(privileged[0], "/usr/bin/pkexec");
        assert!(privileged.contains(&"apt-get".to_string()));
        assert_eq!(privileged.len(), argv.len() + 1);
    }

    #[test]
    fn the_privilege_prefix_falls_back_in_order() {
        let argv = vec!["apt-get".to_string()];
        // pkexec, then sudo.
        let sudo_only = FakePluginEnv::new()
            .with_euid(1000)
            .with_which("sudo", "/usr/bin/sudo");
        assert_eq!(privileged_command(&argv, &sudo_only)[0], "/usr/bin/sudo");
        // Neither helper found: the argv is returned unchanged rather than
        // dropped, which is what makes the page's "Install with: apt-get …"
        // subtitle still true.
        let neither = FakePluginEnv::new().with_euid(1000);
        assert_eq!(privileged_command(&argv, &neither), argv);
        // Already root: no prefix, even with pkexec present.
        let root = FakePluginEnv::new()
            .with_euid(0)
            .with_which("pkexec", "/usr/bin/pkexec");
        assert_eq!(privileged_command(&argv, &root), argv);
        // Unknown uid is treated as not-root (see `PluginEnv::euid`).
        let unknown = FakePluginEnv::new().with_which("pkexec", "/usr/bin/pkexec");
        assert_eq!(privileged_command(&argv, &unknown)[0], "/usr/bin/pkexec");
    }

    #[test]
    fn a_flatpak_install_is_never_prefixed() {
        // `flatpak install` writes to the user's own installation, so wrapping
        // it in pkexec would prompt for a password to do something that needs
        // no privilege. The reference returns the argv before the uid check.
        let env = FakePluginEnv::new()
            .with_euid(1000)
            .with_which("pkexec", "/usr/bin/pkexec");
        let argv = vec![
            "flatpak".to_string(),
            "install".to_string(),
            "-y".to_string(),
            "flathub".to_string(),
            "org.freedesktop.Platform.VulkanLayer.MangoHud".to_string(),
        ];
        assert_eq!(privileged_command(&argv, &env), argv);
        // An empty argv is **not** special-cased: `if argv and argv[0] == "flatpak"`
        // is false for it, so it falls through to the uid check and comes back
        // as just the helper. That is the reference's answer, not this port's
        // reading of it — checked by running
        // `plugins.privileged_command([])` under the same two mocks, which
        // returned `['/usr/bin/pkexec']`. Unreachable from `install_command`,
        // which never produces an empty argv, but the branch is the reference's
        // and pretending otherwise would be a silent divergence.
        assert_eq!(privileged_command(&[], &env), vec!["/usr/bin/pkexec"]);
    }

    // ------------------------------------------------------- package manager

    #[test]
    fn the_package_manager_detection_order_is_the_references() {
        // `test_plugins.py::test_package_manager_detection_is_a_string` asserts
        // only the type; the order below is the part that actually decides
        // which command a user is shown, so it is pinned case by case.
        assert_eq!(
            detect_package_manager(&FakePluginEnv::with_apt_host()),
            "apt"
        );
        assert_eq!(
            detect_package_manager(
                &FakePluginEnv::new()
                    .with_which("pacman", "/usr/bin/pacman")
                    .with_file("/etc/arch-release")
            ),
            "pacman"
        );
        assert_eq!(
            detect_package_manager(&FakePluginEnv::new().with_which("dnf", "/usr/bin/dnf")),
            "dnf"
        );
        assert_eq!(
            detect_package_manager(&FakePluginEnv::new().with_which("zypper", "/usr/bin/zypper")),
            "zypper"
        );
        assert_eq!(
            detect_package_manager(&FakePluginEnv::new().with_which("flatpak", "/usr/bin/flatpak")),
            "flatpak"
        );
        // Nothing found at all: the empty string, which is what makes the
        // intro say "the detected host package manager." with no name in it.
        assert_eq!(detect_package_manager(&FakePluginEnv::new()), "");
    }

    #[test]
    fn a_binary_without_its_release_file_is_not_a_debian_or_arch_host() {
        // `apt-get` exists on plenty of non-Debian systems and `pacman` can be
        // installed on one too, which is why the reference demands both halves.
        // Dropping either `exists` check makes this fail.
        let apt_without_release = FakePluginEnv::new().with_which("apt-get", "/usr/bin/apt-get");
        assert_eq!(detect_package_manager(&apt_without_release), "");

        let pacman_without_release = FakePluginEnv::new().with_which("pacman", "/usr/bin/pacman");
        assert_eq!(detect_package_manager(&pacman_without_release), "");
    }

    #[test]
    fn a_debian_host_with_dnf_prefers_apt() {
        // The order is what makes this true; reversing two branches in
        // `detect_package_manager` would show a user the wrong install command.
        let both = FakePluginEnv::with_apt_host().with_which("dnf", "/usr/bin/dnf");
        assert_eq!(detect_package_manager(&both), "apt");
    }

    // -------------------------------------------------------------- flatpak

    #[test]
    fn flatpak_never_offers_host_package_commands() {
        // `test_plugins.py::test_flatpak_never_offers_host_package_commands`,
        // assertion for assertion — including that an *explicit* manager is
        // refused, which is what makes this a policy rather than a detection
        // fallback.
        let env = FakePluginEnv::with_apt_host().with_var("FLATPAK_ID", "com.goshapps.GameHandler");
        assert!(in_flatpak(&env));
        assert_eq!(detect_package_manager(&env), "");
        let error =
            install_command(plugin_by_id("mangohud").unwrap(), Some("apt"), &env).unwrap_err();
        assert_eq!(error, InstallError::Flatpak);
        assert!(
            error.to_string().contains("unavailable inside Flatpak"),
            "the reference asserts on this wording: {error}"
        );
    }

    #[test]
    fn the_flatpak_marker_is_either_the_variable_or_the_file() {
        // `bool(os.environ.get("FLATPAK_ID")) or Path("/.flatpak-info").exists()`
        // — two independent signals, either sufficient.
        let by_var = FakePluginEnv::new().with_var("FLATPAK_ID", "com.goshapps.GameHandler");
        assert!(in_flatpak(&by_var));
        let by_file = FakePluginEnv::new().with_file("/.flatpak-info");
        assert!(in_flatpak(&by_file));
        assert!(!in_flatpak(&FakePluginEnv::new()));
        // An empty `FLATPAK_ID` is falsy in Python, so it must not count as a
        // sandbox on its own.
        let empty = FakePluginEnv::new().with_var("FLATPAK_ID", "");
        assert!(!in_flatpak(&empty));
    }

    // ---------------------------------------------------------- the 3 states

    #[test]
    fn the_three_states_are_reachable_and_distinct() {
        // P-64's acceptance criterion. Each state comes from a different host,
        // so the test would fail if any branch were collapsed into another.
        let installed = FakePluginEnv::with_apt_host().with_which("mangohud", "/usr/bin/mangohud");
        let row = plugin_row(plugin_by_id("mangohud").unwrap(), &installed);
        assert_eq!(row.state, PluginState::Installed);
        assert_eq!(row.state.as_str(), "installed");
        assert_eq!(
            row.subtitle,
            "On-screen overlay for FPS, frame times, GPU, and CPU. Enable it per game \
             under Launch options, or as the default in Settings."
        );

        let missing = FakePluginEnv::with_apt_host().with_euid(1000);
        let row = plugin_row(plugin_by_id("mangohud").unwrap(), &missing);
        assert_eq!(row.state, PluginState::Missing);
        assert_eq!(row.state.as_str(), "missing");
        assert_eq!(
            row.subtitle,
            "On-screen overlay for FPS, frame times, GPU, and CPU. Not installed. \
             Install with: apt-get install -y mangohud"
        );

        let sandboxed =
            FakePluginEnv::with_apt_host().with_var("FLATPAK_ID", "com.goshapps.GameHandler");
        let row = plugin_row(plugin_by_id("mangohud").unwrap(), &sandboxed);
        assert_eq!(row.state, PluginState::Unavailable);
        assert_eq!(row.state.as_str(), "unavailable");

        // All three are distinct values, so a `PluginState` that lost a variant
        // to a copy-paste cannot pass the assertions above.
        let states = [
            PluginState::Installed,
            PluginState::Missing,
            PluginState::Unavailable,
        ];
        let names: Vec<&str> = states.iter().map(|state| state.as_str()).collect();
        assert_eq!(names, vec!["installed", "missing", "unavailable"]);
    }

    #[test]
    fn an_installed_helper_is_installed_even_inside_the_sandbox() {
        // The branch order in `_plugin_row` is load-bearing: `installed` is
        // checked *before* the sandbox, so a helper bundled in the Flatpak is
        // reported as available rather than as unbundled. Swapping the two
        // branches leaves `the_three_states_are_reachable_and_distinct` green,
        // which is why this case exists.
        let env = FakePluginEnv::new()
            .with_var("FLATPAK_ID", "com.goshapps.GameHandler")
            .with_which("mangohud", "/usr/bin/mangohud");
        let row = plugin_row(plugin_by_id("mangohud").unwrap(), &env);
        assert_eq!(row.state, PluginState::Installed);
    }

    #[test]
    fn a_helper_with_no_package_mapping_is_unavailable_rather_than_missing() {
        // The `except RuntimeError` arm — a host whose manager has no entry for
        // this helper. It must not claim an install command it cannot name.
        let gentoo = FakePluginEnv::new().with_which("flatpak", "/usr/bin/flatpak");
        let row = plugin_row(plugin_by_id("umu").unwrap(), &gentoo);
        assert_eq!(row.state, PluginState::Unavailable);
        assert_eq!(
            row.subtitle,
            "Runs Proton builds with the Steam runtime outside Steam. Not installed. \
             No package mapping is available for this system."
        );

        // The sandbox arm is a *different* sentence for a different reason, and
        // both are `unavailable`; conflating them would show Flatpak users the
        // wrong explanation.
        let sandboxed = FakePluginEnv::new().with_var("FLATPAK_ID", "x");
        assert_ne!(
            plugin_row(plugin_by_id("umu").unwrap(), &sandboxed).subtitle,
            row.subtitle
        );
    }

    /// The reference literal that begins at `anchor` and runs to its end.
    ///
    /// `bridge.py` builds its row text as f-string literals, so the source
    /// distinguishes fixed wording from an interpolation: a literal ends at its
    /// closing quote and an interpolation begins at `{`. Slicing to whichever
    /// comes first returns exactly the words the reference hardcodes —
    /// *extracted* rather than restated, so a paraphrase anywhere in the
    /// sentence fails here and not only a paraphrase of a phrase this test
    /// happens to name.
    fn reference_literal<'a>(joined: &'a str, anchor: &str) -> &'a str {
        let start = joined
            .find(anchor)
            .unwrap_or_else(|| panic!("bridge.py no longer says {anchor:?}"));
        let rest = &joined[start..];
        let end = rest.find(['"', '{']).unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn the_row_subtitles_are_the_references_wording() {
        // Each state is compared against the text `bridge.py` actually
        // hardcodes, sliced out of the joined source rather than retyped.
        let bridge = join_adjacent_literals(&repo_file("gamehandler/bridge.py"));
        let mangohud = plugin_by_id("mangohud").unwrap();

        let sandboxed = FakePluginEnv::new().with_var("FLATPAK_ID", "x");
        let row = plugin_row(mangohud, &sandboxed);
        let expected = reference_literal(&bridge, "Not bundled in this Flatpak.");
        assert!(
            row.subtitle.contains(expected),
            "the unavailable row paraphrases bridge.py:\n  reference: {expected:?}\n  port:      {:?}",
            row.subtitle
        );

        let debian = FakePluginEnv::with_apt_host();
        let row = plugin_row(mangohud, &debian);
        let expected = reference_literal(&bridge, "Not installed. Install with:");
        assert!(
            row.subtitle.contains(expected),
            "the missing row paraphrases bridge.py:\n  reference: {expected:?}\n  port:      {:?}",
            row.subtitle
        );

        let gentoo = FakePluginEnv::new().with_which("flatpak", "/usr/bin/flatpak");
        let row = plugin_row(plugin_by_id("umu").unwrap(), &gentoo);
        let expected = reference_literal(&bridge, "No package mapping is available");
        assert!(
            row.subtitle.contains(expected),
            "the no-mapping row paraphrases bridge.py:\n  reference: {expected:?}\n  port:      {:?}",
            row.subtitle
        );
    }

    #[test]
    fn the_intro_names_the_detected_manager_or_says_nothing() {
        // `test_plugins.py` never reaches `pluginsIntro`; this is the only pin
        // on it. Both branches matter: the sentence ends in `(apt).` when a
        // manager was found and in a bare `.` when none was.
        let debian = FakePluginEnv::with_apt_host();
        assert_eq!(
            plugins_intro(&debian),
            "These tools are optional. GameHandler offers them on each game. Missing \
             helpers can be installed with the detected host package manager (apt)."
        );

        let unknown = FakePluginEnv::new();
        assert_eq!(
            plugins_intro(&unknown),
            "These tools are optional. GameHandler offers them on each game. Missing \
             helpers can be installed with the detected host package manager."
        );
        assert!(!plugins_intro(&unknown).contains("()"));

        // And in the sandbox it is a different sentence that makes no offer.
        let sandboxed = FakePluginEnv::with_apt_host().with_var("FLATPAK_ID", "x");
        let intro = plugins_intro(&sandboxed);
        assert!(intro.contains("intentionally disabled"));
        assert!(!intro.contains("can be installed"));
    }

    #[test]
    fn the_intro_is_the_references_wording() {
        let bridge = join_adjacent_literals(&repo_file("gamehandler/bridge.py"));

        // The sandbox sentence is entirely literal in the reference — no
        // interpolation at all — so this comparison is exact in both
        // directions: a word changed on either side fails.
        let sandboxed = FakePluginEnv::with_apt_host().with_var("FLATPAK_ID", "x");
        let mine = plugins_intro(&sandboxed);
        assert!(
            bridge.contains(&mine),
            "the sandbox intro is not bridge.py's sentence:\n  port: {mine:?}"
        );

        // The host sentence ends in an interpolation, so only the fixed part
        // can be compared — but it is the whole fixed part, sliced out of the
        // reference, not a phrase quoted from it.
        let debian = FakePluginEnv::with_apt_host();
        let mine = plugins_intro(&debian);
        let fixed = reference_literal(&bridge, "Missing helpers");
        // This sentence's literal ends at an f-string boundary — the reference
        // writes `"... host package " f"manager{detected}"` — so the extractor
        // is only trustworthy if the joiner reassembled that boundary. Without
        // this guard a joiner that stopped at `f"` would return the sentence
        // *minus* its last word, and `contains` below would still hold: the
        // test would silently check most of a sentence and report success.
        assert!(
            fixed.ends_with("manager"),
            "the joiner stopped short of the f-string boundary, so this comparison \
             would not cover the end of the sentence: {fixed:?}"
        );
        assert!(
            mine.contains(fixed),
            "the host intro paraphrases bridge.py:\n  reference: {fixed:?}\n  port:      {mine:?}"
        );
        assert!(mine.ends_with("(apt)."));
    }

    #[test]
    fn every_row_carries_the_id_the_page_sends_back() {
        // `PluginsPage.qml:52` calls `backend.installPlugin(modelData.pluginId)`,
        // so a row whose id does not resolve is a button that does nothing —
        // the dropdown bug of finding #57 in a different widget.
        let env = FakePluginEnv::with_apt_host();
        let rows = plugin_rows(&env);
        assert_eq!(rows.len(), plugins().len());
        for row in &rows {
            let plugin = plugin_by_id(row.plugin_id)
                .unwrap_or_else(|| panic!("row {:?} does not resolve", row.plugin_id));
            assert_eq!(row.name, plugin.name);
        }
    }
}
