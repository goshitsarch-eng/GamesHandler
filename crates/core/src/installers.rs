//! The first-party easy installer — the whole of `installers.py` (702 lines).
//!
//! Three groups, in the reference's order:
//!
//! * **The catalog** — [`LAUNCHERS`], [`APPS`], [`INSTALLER_CATEGORIES`],
//!   [`Installer`], [`INSTALLERS`], [`installers`], [`installer_by_id`],
//!   [`search_installers`]. This is what the page reads.
//! * **The command half** — [`installer_argv`], [`build_installer_command`],
//!   [`safe_download_name`], [`resolve_case_insensitive`], [`find_prefix_exe`],
//!   the bounded fallback scan, [`prepare_prefix`], [`game_from_install`].
//! * **The download and wizard halves** — [`download_installer`],
//!   [`verify_installer_authenticity`], [`wineserver_binary`],
//!   [`wait_for_prefix_idle`], [`wait_for_installer`].
//!
//! # Callers
//!
//! **These functions are wired.** This header used to say the opposite — that
//! the file "has no *caller*" and that five named functions "have no call site
//! outside their own tests" — and every clause of that was false by the time it
//! was read. The easy-install worker
//! ([`easy_install_worker`](crate::installers)) lives in the app crate at
//! `crates/app/src/main.rs:3160`, wired there by T-38, and it drives this file's
//! download and wizard halves directly:
//!
//! | Function | Production call site |
//! |---|---|
//! | [`download_installer`] | `crates/app/src/main.rs`, in `easy_install_worker` |
//! | [`wait_for_installer`] | `crates/app/src/main.rs`, in `easy_install_worker` |
//! | [`wait_for_prefix_idle`] | `crates/app/src/main.rs`, in `easy_install_worker` |
//! | [`verify_installer_authenticity`] | `crates/core/src/installers.rs`, in `download_into` |
//! | [`wineserver_binary`] | `crates/core/src/installers.rs`, in `wait_for_prefix_idle` |
//!
//! `wait_for_prefix_idle` reaches the worker as the closure `wait_for_installer`
//! is handed, not as a statement of its own, which is why its row is inside that
//! function rather than beside it.
//!
//! **Each row names a function, not a line number, and that is a correction.**
//! The first version of this table cited `path:line`, and the check below
//! asserted the exact line. It went stale within the hour: concurrent edits to
//! `main.rs` moved the worker from `:3160` to `:3269` and the whole install
//! path's three citations stopped landing, which is `ARCH-16`'s defect — *a
//! pointer that no longer lands is worse than no pointer, because it is
//! trusted* — reappearing in the fix for `ARCH-05`. A symbol survives an edit;
//! a line number does not. The enclosing function is now the locator, and the
//! check asserts the **call is inside it**, which is both drift-free and a
//! stronger statement than "some line somewhere holds a call".
//!
//! Every row above is checked by `crates/core/tests/wiring_claims.rs`, which
//! reads this table and fails when a cited function stops containing a live call
//! to the function its row names.
//!
//! The header was corrected because a false "not landed yet" is not a neutral
//! error: a maintainer reading the crate that is supposed to be self-describing
//! is told the whole install path is unwired, and may re-wire it, delete it, or
//! decline to touch it. `crates/app/src/view/installers.rs:46-50` records that
//! this exact failure mode has already happened once in this project, which is
//! why the rule here is that a stale claim about wiring is a defect and not a
//! stale comment.
//!
//! Two smaller things that are genuinely incomplete rather than unwired:
//!
//! * [`InstallerError::AuthenticodeRootUnavailable`] has no control arm, because
//!   `data/` in this repository carries the `.pem` and every candidate list
//!   therefore ends at a file that exists. See the test that says so.
//! * `_authenticode_root_path`'s second candidate in the reference — the `.pem`
//!   beside the Python module — has no counterpart here, because there is no
//!   Python module. The Flatpak path (the fourth candidate) is the one that
//!   matters for an installed build and is a packaging question.
//!
//! # Why the catalog is a compile-time table
//!
//! Python builds `INSTALLERS` as a tuple of frozen dataclasses at import time,
//! and the catalog is *curated data*, not something the program ever computes:
//! nine entries, each with a vendor URL, a list of paths where the vendor's
//! installer lands, the hosts it may redirect to and the publishers its
//! signature may name. Nothing about it is a function of the environment, so it
//! is a `const` array of `&'static str` here rather than something built at
//! startup — there is no way for the installed app to disagree with the source
//! about what the catalog contains.
//!
//! The entries were **generated from the reference module itself**, not
//! transcribed, and read back field by field by the tests below. All three of
//! those facts matter for the same reason: the catalog is the input to a
//! download-and-execute path, so a mistyped host (a real one, `gog.com` for
//! `content-system.gog.com`) would be an allowlist that quietly admits the
//! wrong origin, and a mistyped URL would be a 404 at the moment a user clicks
//! Install. Nothing about a wrong byte here fails loudly at build time.
//!
//! # `kind` is an enum, and that is the only representation change
//!
//! Python's `kind` is the string `"exe"` or `"msi"` and the module branches on
//! it in two places: `installer_argv` (`installers.py:258-267`) and the magic
//! check that decides whether to expect `MZ` or an OLE header
//! (`:561-566`). Here it is [`Kind`], so those two branches are exhaustive
//! matches and a third kind cannot be added without the compiler naming every
//! site that has to learn about it. The *strings* are still what the reference
//! compares — [`Kind::as_str`] is `"exe"`/`"msi"` and [`Kind::label`] is
//! `"EXE"`/`"MSI"`, which is what `f"{installer.kind.upper()}"` renders in the
//! reference's error message.
//!
//! # `category` and `library_category` are different string spaces
//!
//! `category` ([`LAUNCHERS`] / [`APPS`]) is the *page's* filter — the nine
//! cards' badges, and what `search_installers` narrows on. `library_category`
//! is what the created `Game` gets as its `Game::category` and is drawn from
//! the same set the Library page groups by (`models.py`'s `UNCATEGORIZED`,
//! `"Utility"`). Eight of the nine have `"Launchers"` for both, which is a
//! coincidence of the two vocabularies sharing one word, not a rule — Discord
//! is `Apps` on the page and `Utility` in the library. A port that collapsed
//! them would put Discord in a library category the Library page's dropdown
//! does not offer.
//!
//! # The seams, and why there are three of them
//!
//! `installers.py` reaches the outside world in three ways, and each one is a
//! parameter here rather than an import:
//!
//! * [`HttpClient`] (D-26) for the download. `core` has no networking
//!   dependency and must not acquire one, so the binary injects `ureq`.
//! * [`LaunchEnv`] for `shutil.which` and for `os.environ` — the same seam the
//!   runner code uses. The `osslsigncode` and `wineserver` lookups go through
//!   it, which is what lets this file's tests point them at scripts they wrote
//!   without touching `PATH`.
//! * [`InstallClock`] and [`IdleWait`] for the wizard poll. The reference
//!   injects `sleep` and `clock` and *monkey-patches* `wait_for_prefix_idle` in
//!   its own suite; there is no monkey-patching in Rust, so the idle wait is a
//!   parameter. Every arm of the poll is measured in tens of minutes, so
//!   without this seam the behaviour that matters — *when a wizard is given up
//!   on* — could not be tested at all.
//!
//! # The one place this port does something the reference does not
//!
//! Everywhere else, a difference between this file and `installers.py` is a
//! difference of *representation* ([`Kind`] instead of `"exe"`/`"msi"`) or of
//! *seam* (the three above). There is one behavioural divergence, and it is
//! deliberately not hidden inside a helper: `spawn_retrying` waits out a
//! transient `ETXTBSY` rather than reporting it, because the reference's
//! single-spawn `subprocess.run` turns into a flake in a threaded test suite
//! whose fakes are shell scripts written a moment before they are executed. The
//! reason, the bound, the measurement and the argument for retrying *there*
//! rather than in the tests are all on that function, and
//! `a_held_open_script_is_executable_file_busy` reproduces the condition from
//! the kernel rather than asserting a belief about it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand};

use crate::models::Game;
use crate::paths::{self, Env};
use crate::runners::launch_opts::apply_launch_options;
use crate::runners::shell::{ShellError, split_posix};
use crate::runners::{
    Command, LaunchEnv, Runner, RunnerError, prefix_drive_cs, pure_posix_name, uses_proton_runtime,
};

/// The page's category for store launchers (`installers.py:35`).
pub const LAUNCHERS: &str = "Launchers";

/// The page's category for standalone applications (`installers.py:36`).
pub const APPS: &str = "Apps";

/// The two categories the page offers, in the reference's order
/// (`installers.py:37`).
pub const INSTALLER_CATEGORIES: [&str; 2] = [LAUNCHERS, APPS];

/// The value that means "do not filter" (`installers.py:245`).
///
/// The reference writes this literal twice — here and in the bridge's
/// `installerCategories` (`bridge.py:787`), which is what puts it at the head
/// of the page's dropdown. Kept as a constant so the filter and the dropdown
/// agree by construction; `view/installers.rs` has its own copy today because
/// it landed before this module existed.
pub const ALL_CATEGORIES: &str = "All";

/// What kind of payload a recipe downloads (`installers.py:50`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A PE executable, run as `[wine, path]`.
    Exe,
    /// An MSI package, run as `[wine, "msiexec", "/i", path]`.
    Msi,
}

impl Kind {
    /// The reference's own spelling: `"exe"` / `"msi"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Exe => "exe",
            Kind::Msi => "msi",
        }
    }

    /// `installer.kind.upper()` — what the reference's invalid-file message
    /// interpolates (`installers.py:566`), so `"not a valid MSI file"` is
    /// spelled the way the reference spells it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Exe => "EXE",
            Kind::Msi => "MSI",
        }
    }
}

/// A curated one-click setup recipe for a Windows launcher or app.
///
/// Field-for-field `Installer` (`installers.py:40-60`), minus the dataclass
/// defaults: an entry that omits a field here takes the same default Python's
/// does (see [`Installer::DEFAULT`] for the two that are not obvious).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Installer {
    /// The id the install action names, and the last field of a card's row.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// [`LAUNCHERS`] or [`APPS`] — the *page's* category, not the library's.
    pub category: &'static str,
    /// The vendor's official HTTPS URL. Never a third-party mirror.
    pub download_url: &'static str,
    /// The name the download is saved as, passed through `safe_download_name`
    /// before it reaches the filesystem.
    pub filename: &'static str,
    pub kind: Kind,
    /// `drive_c`-relative paths the installer is expected to produce, tried in
    /// order. The first one that exists wins; the basenames are the fallback
    /// scan's search set.
    pub expected_exe: &'static [&'static str],
    /// Hosts the download may redirect to, checked against the *final* URL.
    pub allowed_hosts: &'static [&'static str],
    /// Substrings the Authenticode signer must match, case-folded.
    pub publishers: &'static [&'static str],
    /// Extra argv for the vendor's installer, `shlex`-split. `""` for almost
    /// every recipe.
    pub arguments: &'static str,
    /// `Game::arguments` for the created library entry. Only Discord needs it:
    /// `Update.exe --processStart Discord.exe` is how the Squirrel updater
    /// launches the app, and dropping it produces an entry that starts the
    /// updater and exits.
    pub launch_arguments: &'static str,
    /// A second line on the card, where the vendor's flow needs explaining.
    /// Shown verbatim, after the description, by `card_subtitle`.
    pub notes: &'static str,
    /// Whether the created entry starts with `WINEESYNC=1`.
    pub esync: bool,
    /// Whether the created entry starts with `WINEFSYNC=1`.
    pub fsync: bool,
    /// `Game::category` for the created entry — a *library* category.
    pub library_category: &'static str,
    /// Whether the signature must chain to the bundled Microsoft root rather
    /// than the host's trust store.
    pub microsoft_trust_root: bool,
}

impl Installer {
    /// The dataclass defaults (`installers.py:54-60`).
    pub const DEFAULT: Installer = Installer {
        id: "",
        name: "",
        description: "",
        category: LAUNCHERS,
        download_url: "",
        filename: "",
        kind: Kind::Exe,
        expected_exe: &[],
        allowed_hosts: &[],
        publishers: &[],
        arguments: "",
        launch_arguments: "",
        notes: "",
        esync: true,
        fsync: true,
        library_category: LAUNCHERS,
        microsoft_trust_root: false,
    };
}

/// The catalog, in the reference's order (`installers.py:63-225`).
///
/// The order is load-bearing in one visible place: `search_installers` filters
/// this array rather than sorting it, so the cards appear in exactly this
/// order, and `INSTALLERS[0]` is the first card the page draws.
pub const INSTALLERS: [Installer; 9] = [
    Installer {
        id: "battlenet",
        name: "Battle.net",
        description: "Blizzard and Activision store client (Warcraft, Diablo, Overwatch, Call of Duty).",
        category: LAUNCHERS,
        download_url: "https://downloader.battle.net/download/getInstaller\
            ?os=win&installer=Battle.net-Setup.exe",
        filename: "Battle.net-Setup.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files (x86)/Battle.net/Battle.net Launcher.exe",
            "Program Files (x86)/Battle.net/Battle.net.exe",
            "Program Files/Battle.net/Battle.net Launcher.exe",
            "Program Files/Battle.net/Battle.net.exe",
        ],
        allowed_hosts: &["downloader.battle.net"],
        publishers: &["Blizzard Entertainment"],
        notes: "Complete the Battle.net wizard, then sign in once before launching games.",
        ..Installer::DEFAULT
    },
    Installer {
        id: "epic",
        name: "Epic Games Launcher",
        description: "Epic Games Store client for Fortnite and Epic exclusives.",
        category: LAUNCHERS,
        download_url: "https://launcher-public-service-prod06.ol.epicgames.com/\
            launcher/api/installer/download/EpicGamesLauncherInstaller.msi",
        filename: "EpicGamesLauncherInstaller.msi",
        kind: Kind::Msi,
        expected_exe: &[
            "Program Files (x86)/Epic Games/Launcher/Portal/Binaries/Win32/EpicGamesLauncher.exe",
            "Program Files (x86)/Epic Games/Launcher/Portal/Binaries/Win64/EpicGamesLauncher.exe",
            "Program Files/Epic Games/Launcher/Portal/Binaries/Win64/EpicGamesLauncher.exe",
        ],
        allowed_hosts: &[
            "launcher-public-service-prod06.ol.epicgames.com",
            "epicgames-download1.akamaized.net",
        ],
        publishers: &["Epic Games Inc."],
        ..Installer::DEFAULT
    },
    Installer {
        id: "ea-app",
        name: "EA App",
        description: "EA Desktop client (Apex, Battlefield, The Sims, Steam-unlisted EA titles).",
        category: LAUNCHERS,
        download_url: "https://origin-a.akamaihd.net/EA-Desktop-Client-Download/\
            installer-releases/EAappInstaller.exe",
        filename: "EAappInstaller.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe",
            "Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe",
            "Program Files (x86)/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe",
        ],
        allowed_hosts: &["origin-a.akamaihd.net"],
        publishers: &["Electronic Arts"],
        ..Installer::DEFAULT
    },
    Installer {
        id: "ubisoft",
        name: "Ubisoft Connect",
        description: "Ubisoft store and overlay (Assassin's Creed, Far Cry, Rainbow Six).",
        category: LAUNCHERS,
        download_url: "https://static3.cdn.ubi.com/orbit/launcher_installer/UbisoftConnectInstaller.exe",
        filename: "UbisoftConnectInstaller.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe",
            "Program Files/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe",
            "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/upc.exe",
        ],
        allowed_hosts: &["static3.cdn.ubi.com"],
        publishers: &["UBISOFT ENTERTAINMENT"],
        // The only recipe that asks for the bundled root: Ubisoft's signer
        // chains to a 2020 Microsoft identity-verification root that a host
        // trust store does not necessarily carry.
        microsoft_trust_root: true,
        ..Installer::DEFAULT
    },
    Installer {
        id: "gog",
        name: "GOG Galaxy",
        description: "GOG's DRM-free store client and optional game overlay.",
        category: LAUNCHERS,
        download_url: "https://content-system.gog.com/open_link/download\
            ?path=/open/galaxy/client/setup_galaxy_2.1.8.30.exe",
        filename: "setup_galaxy_2.1.8.30.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files (x86)/GOG Galaxy/GalaxyClient.exe",
            "Program Files/GOG Galaxy/GalaxyClient.exe",
        ],
        allowed_hosts: &["content-system.gog.com", "gog-cdn-fastly.gog.com"],
        // The double space is the reference's, character for character
        // (`installers.py:159`) and is pinned by a test. It is a substring of
        // the signer `osslsigncode` prints, so "tidying" it into one space
        // would stop matching a signature that is genuinely GOG's.
        publishers: &["CN=GOG  sp. z o.o,O=GOG  sp. z o.o"],
        ..Installer::DEFAULT
    },
    Installer {
        id: "amazon",
        name: "Amazon Games",
        description: "Amazon Games app for Prime Gaming claims and Amazon-published titles.",
        category: LAUNCHERS,
        download_url: "https://download.amazongames.com/AmazonGamesSetup.exe",
        filename: "AmazonGamesSetup.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "users/steamuser/AppData/Local/Amazon Games/App/Amazon Games.exe",
            "Program Files/Amazon Games/App/Amazon Games.exe",
            "Program Files (x86)/Amazon Games/App/Amazon Games.exe",
        ],
        allowed_hosts: &["download.amazongames.com"],
        publishers: &["Amazon.com Services LLC"],
        ..Installer::DEFAULT
    },
    Installer {
        id: "rockstar",
        name: "Rockstar Games Launcher",
        description: "Rockstar store client (GTA, Red Dead, older Rockstar titles).",
        category: LAUNCHERS,
        download_url: "https://gamedownloads.rockstargames.com/public/installer/Rockstar-Games-Launcher.exe",
        filename: "Rockstar-Games-Launcher.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files/Rockstar Games/Launcher/Launcher.exe",
            "Program Files (x86)/Rockstar Games/Launcher/Launcher.exe",
        ],
        allowed_hosts: &["gamedownloads.rockstargames.com"],
        publishers: &["Rockstar Games"],
        ..Installer::DEFAULT
    },
    Installer {
        id: "steam",
        name: "Steam",
        description: "Windows Steam client, useful for titles that need the official Steam overlay.",
        category: LAUNCHERS,
        download_url: "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe",
        filename: "SteamSetup.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "Program Files (x86)/Steam/steam.exe",
            "Program Files/Steam/steam.exe",
        ],
        allowed_hosts: &["cdn.akamai.steamstatic.com"],
        publishers: &["Valve Corp."],
        ..Installer::DEFAULT
    },
    Installer {
        id: "discord",
        name: "Discord",
        description: "Discord desktop client for voice, chat, and overlays.",
        category: APPS,
        download_url: "https://discord.com/api/download?platform=win",
        filename: "DiscordSetup.exe",
        kind: Kind::Exe,
        expected_exe: &[
            "users/steamuser/AppData/Local/Discord/Update.exe",
            "users/steamuser/AppData/Local/Discord/Discord.exe",
        ],
        allowed_hosts: &["discord.com", "stable.dl2.discordapp.net"],
        publishers: &["Discord Inc."],
        launch_arguments: "--processStart Discord.exe",
        library_category: "Utility",
        notes: "Discord lives under Local AppData. Launch Update.exe if the app folder version changes.",
        ..Installer::DEFAULT
    },
];

/// The catalog (`installers.py:230-231`).
///
/// Returns the array itself, as the reference returns its module-level tuple —
/// so a caller cannot obtain a different catalog from the one [`INSTALLERS`]
/// describes, and no caller can mutate it.
pub fn installers() -> &'static [Installer] {
    &INSTALLERS
}

/// One recipe by id (`installers.py:234-238`).
///
/// Python raises `KeyError(f"Unknown installer: {installer_id}")`; the one
/// caller catches it and returns silently (`bridge.py:836-839`), so `Err` is
/// the shape that lets the app do the same rather than a panic.
///
/// A linear scan over nine entries rather than a `HashMap`: the reference's
/// `_INSTALLERS` dict exists for lookup speed, but nine string comparisons cost
/// nothing, and the array is the single source of truth for the catalog's
/// order, which a second map could drift from.
pub fn installer_by_id(installer_id: &str) -> Result<&'static Installer, InstallerError> {
    INSTALLERS
        .iter()
        .find(|item| item.id == installer_id)
        .ok_or_else(|| InstallerError::UnknownInstaller {
            id: installer_id.to_string(),
        })
}

/// Filter the catalog by name/description/id and by category
/// (`installers.py:241-255`).
///
/// # The four details that are easy to get wrong
///
/// * The query is `strip()`ed and lowercased once, and matched **anywhere** in
///   the name, the description or the id — so `"blizzard"` finds Battle.net by
///   its description, and `"ea"` finds three entries.
/// * The category is matched **exactly**, with no case folding: `"Launchers"`
///   filters and `"launchers"` matches nothing. That reads like an oversight
///   and is the reference's behaviour, so `test_search_and_category_filter` and
///   `bridge.py:787`'s dropdown agree that the only values that ever arrive are
///   `"All"` and the two constants.
/// * `""` and `"All"` both mean no filter. [`ALL_CATEGORIES`] is the second.
/// * An empty query returns everything the category left, and the catalog's
///   order is preserved — this filters, it never sorts.
pub fn search_installers(query: &str, category: &str) -> Vec<&'static Installer> {
    let needle = query.trim().to_lowercase();
    let filtered: Vec<&'static Installer> = INSTALLERS
        .iter()
        .filter(|item| {
            if !category.is_empty() && category != ALL_CATEGORIES {
                item.category == category
            } else {
                true
            }
        })
        .collect();
    if needle.is_empty() {
        return filtered;
    }
    filtered
        .into_iter()
        .filter(|item| {
            item.name.to_lowercase().contains(&needle)
                || item.description.to_lowercase().contains(&needle)
                || item.id.to_lowercase().contains(&needle)
        })
        .collect()
}

/// The catalog's errors — `KeyError` in `installer_by_id`, `ValueError` from
/// `shlex.split`, `RuntimeError` for the rest (`installers.py`).
///
/// Every `RuntimeError` the reference raises is a variant here, and each
/// [`fmt::Display`] is the reference's `f`-string byte for byte: these messages
/// reach the user through the same status line as the rest of the port, and
/// `tests/test_installers.py` asserts on their *substrings*
/// (`"untrusted download origin"`, `"not a valid EXE"`, `"approved
/// publisher"`), so the wording is behaviour rather than prose.
#[derive(Debug)]
pub enum InstallerError {
    /// `KeyError(f"Unknown installer: {installer_id}")` (`installers.py:238`).
    UnknownInstaller { id: String },
    /// `shlex.split` on a recipe's `arguments`. Python's `ValueError`, whose
    /// message this carries verbatim (`shell.rs` matches CPython's strings).
    Arguments(ShellError),
    /// `RuntimeError(f"Runner '{runner.name}' produced an empty command")`
    /// (`installers.py:285`) — a runner that could not build a command at all,
    /// which is a broken runner rather than a missing one.
    EmptyCommand { runner: String },
    /// Anything the runner or the launch-option matrix refused, propagated
    /// unchanged so the app can report the runner's own message.
    Runner(RunnerError),
    /// The response came from a host or scheme the recipe's `allowed_hosts`
    /// does not cover (`installers.py:551-558`).
    ///
    /// `url` is the **final** URL — the one after redirects — because that is
    /// what the reference checks: a redirect to an unapproved host is the
    /// attack this exists for, and the request URL would look fine.
    UntrustedOrigin { name: String, url: String },
    /// The first eight bytes are not the payload the recipe declared
    /// (`installers.py:561-566`). `kind` renders as `EXE`/`MSI`, which is what
    /// `installer.kind.upper()` puts in the reference's message.
    NotValidPayload { name: String, kind: Kind },
    /// The declared or streamed length crossed [`MAX_INSTALLER_BYTES`]
    /// (`installers.py:615`/`:622`).
    TooLarge { name: String },
    /// `shutil.which("osslsigncode")` found nothing
    /// (`installers.py:571-573`) — a missing dependency, not a bad installer,
    /// and the wording is the reference's.
    SignatureToolMissing,
    /// The verifier outlived its 90-second bound (`installers.py:588-589`).
    SignatureTimedOut { name: String },
    /// The verifier failed, or its output omitted the success line
    /// (`installers.py:594`). `tail` is the last eight lines, joined with
    /// newlines, whichever the tool chose to print.
    SignatureInvalid { name: String, tail: String },
    /// The signature verified but names a publisher the recipe does not
    /// approve (`installers.py:595`). The comparison is a case-folded
    /// substring test against the whole output.
    PublisherUnapproved { name: String },
    /// `_authenticode_root_path` found the bundled Microsoft root in none of
    /// its four candidate locations (`installers.py:548`).
    ///
    /// Only [`Installer`]s with `microsoft_trust_root` reach this, and in this
    /// port no candidate is a path inside a Python package — so unless the
    /// packaging installs the `.pem` somewhere this build can see, that one
    /// recipe cannot be verified. Named rather than defaulted, because the
    /// alternative (verifying without the pinned root) would accept a chain the
    /// recipe explicitly asked to pin.
    AuthenticodeRootUnavailable,
    /// A filesystem operation in the download failed — the destination
    /// directory could not be created, the temporary file could not be written,
    /// or the rename into place failed.
    ///
    /// Its own variant rather than [`InstallerError::Runner`], which also
    /// carries an `io::Error` but means "the *runner* could not do this": a
    /// disk that filled up during a download has nothing to do with the runner
    /// that would have launched the installer, and folding the two together
    /// would put the wrong subsystem in the error.
    Io(std::io::Error),
}

impl From<RunnerError> for InstallerError {
    fn from(error: RunnerError) -> Self {
        InstallerError::Runner(error)
    }
}

impl From<ShellError> for InstallerError {
    fn from(error: ShellError) -> Self {
        InstallerError::Arguments(error)
    }
}

impl From<std::io::Error> for InstallerError {
    fn from(error: std::io::Error) -> Self {
        InstallerError::Io(error)
    }
}

impl fmt::Display for InstallerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallerError::UnknownInstaller { id } => {
                write!(formatter, "Unknown installer: {id}")
            }
            InstallerError::Arguments(error) => error.fmt(formatter),
            InstallerError::EmptyCommand { runner } => {
                write!(formatter, "Runner '{runner}' produced an empty command")
            }
            InstallerError::Runner(error) => error.fmt(formatter),
            InstallerError::UntrustedOrigin { name, url } => write!(
                formatter,
                "{name} redirected to an untrusted download origin: {url}"
            ),
            InstallerError::NotValidPayload { name, kind } => write!(
                formatter,
                "{name} download is not a valid {} file",
                kind.label()
            ),
            InstallerError::TooLarge { name } => {
                write!(
                    formatter,
                    "{name} installer exceeds the download size limit"
                )
            }
            InstallerError::SignatureToolMissing => write!(
                formatter,
                "osslsigncode is required to verify downloaded installers"
            ),
            InstallerError::SignatureTimedOut { name } => {
                write!(formatter, "Timed out verifying {name}'s signature")
            }
            InstallerError::SignatureInvalid { name, tail } => {
                write!(
                    formatter,
                    "{name} has an invalid Authenticode signature\n{tail}"
                )
            }
            InstallerError::PublisherUnapproved { name } => {
                write!(formatter, "{name} is not signed by an approved publisher")
            }
            InstallerError::AuthenticodeRootUnavailable => {
                write!(
                    formatter,
                    "The Microsoft Authenticode trust root is unavailable"
                )
            }
            InstallerError::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InstallerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstallerError::Arguments(error) => Some(error),
            InstallerError::Runner(error) => Some(error),
            InstallerError::Io(error) => Some(error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The command half (installers.py:258-530)
// ---------------------------------------------------------------------------

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
    static DIRECTORY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

// ---------------------------------------------------------------------------
// The download half (installers.py:533-646)
// ---------------------------------------------------------------------------

use std::time::Duration;

use crate::runners::proton::{HttpClient, ResponseHead};
use crate::runners::{USER_AGENT, wine_prefix_root};

/// `urlparse(url).scheme.lower()` and `urlparse(url).hostname`
/// (`installers.py:552-554`).
///
/// # Why this is a port and not `split("://")`
///
/// The one caller is an allowlist check whose whole value is that it cannot be
/// talked out of: `_validate_download_origin` compares the *final* host against
/// `installer.allowed_hosts`, so a host this function reports as an allowed one
/// is a host the download is then accepted from. A naive parse that reported a
/// host the reference would not is an allowlist bypass, not a cosmetic
/// difference — `https://allowed.example@evil.example/x` is exactly that case,
/// and it lands the other way depending on whether userinfo is split at the
/// first `@` or the last.
///
/// So the rules below are CPython's `urlsplit`/`_hostinfo`, taken from the
/// implementation and then checked against it: the vector battery in the tests
/// was produced by running `urllib.parse` on this machine, and every line of it
/// has to agree. The rules that matter:
///
/// * Leading C0 controls and spaces are stripped, and `\t`, `\r` and `\n` are
///   removed **everywhere** — CPython's `_UNSAFE_URL_BYTES_TO_REMOVE`. Google's
///   URL spec strips both ends; CPython deliberately does not strip the right
///   end, and neither does this.
/// * The scheme is only recognised when the text before the first `:` is a
///   valid scheme, so `cdn.example/x` has no scheme and no host. A URL with no
///   scheme can never pass the `https` test, which is the fail-closed direction.
/// * The netloc runs from `//` to the first `/`, `?` or `#`.
/// * Userinfo is split at the **last** `@` (`netloc.rpartition('@')`).
/// * A bracketed host is everything up to the closing `]`; otherwise the host
///   is everything before the first `:`. The port is neither validated nor
///   reported, which is why `https://allowed.example:notaport/x` resolves to
///   `allowed.example`.
/// * The host is lowercased but **not** IDNA-encoded and **not** stripped of a
///   trailing dot, both of which are what the measured CPython does — and both
///   of which mean `allowed.example.` is a *different* host from
///   `allowed.example`, so a host that merely looks like an allowed one fails
///   closed.
pub(crate) struct UrlParts {
    /// `urlsplit(url).scheme`, lowercased.
    pub scheme: String,
    /// The raw netloc, exactly as CPython keeps it — case preserved, and `""`
    /// when there is no `//`. Kept because `bool(parsed.netloc)` is what
    /// `netpaths.is_remote_url` tests, and that is **not** the same question as
    /// "is there a hostname": `smb:///x` has a scheme and no netloc, and
    /// `//host/x` has a netloc and no scheme.
    pub netloc: String,
    /// `urlsplit(url).hostname`, lowercased, or `None` when empty.
    pub host: Option<String>,
    /// `urlsplit(url).username` — **not** percent-decoded, because CPython's is
    /// not; `netpaths` decodes it itself with `unquote`.
    pub user: Option<String>,
    /// `urlsplit(url).port`, or `None`. See this function's note on the one
    /// case where CPython raises instead.
    pub port: Option<u32>,
    /// `urlsplit(url).path` — the path component only, with any `?query` and
    /// `#fragment` already removed.
    pub path: String,
}

/// `urlsplit()`'s five components (`urlparse` in `installers.py:552`,
/// `urlsplit` in `netpaths.py`), in one place because there is only one
/// CPython URL parser worth having a fidelity opinion about.
///
/// # Why this is one function and not two
///
/// It used to return `(scheme, Option<host>)`, which is all the download
/// origin allowlist needs. `netpaths` needs the username, port and path as
/// well, and the tempting move is to parse again inside `netpaths`. That would
/// be two independent ports of `urlsplit` for one behaviour: two fidelities,
/// only one of which the vector battery below exercises, and the unexercised
/// one is the copy that rots. So the components were added here instead, and
/// the battery was extended over all five rather than a second one started.
///
/// **The widening is return-type-only.** Every caller that read
/// `(scheme, host)` before still reads exactly that and nothing else —
/// [`validate_download_origin`] is the one that matters, and the security
/// property is untouched by construction because widening what a parser
/// *returns* cannot change what a caller *reads*. The 27 allowlist vectors are
/// the evidence that it did not.
///
/// # The rules, all of them measured against CPython rather than recalled
///
/// See the notes on the scheme, netloc, userinfo and host rules that were
/// already here; the additions are that the username is the text before the
/// **first** `:` of the userinfo (which was itself split at the **last** `@`,
/// so `a@b@host` has the username `a@b`), and that `port` is `None` both when
/// the port is absent or empty and when it is not a number.
///
/// That last one is a **divergence**, and it is exact rather than a rounding:
/// CPython's `.port` property raises `ValueError` on a port that is not ASCII
/// digits within `0..=65535`, so `netpaths.as_local_path("sftp://host:abc/x")`
/// raises out of the reference and takes the caller with it. Returning `None`
/// here cannot crash, and the difference is observable only on a URL the
/// reference cannot handle at all. The accepted *text*, though, is reproduced
/// exactly — see the ASCII-digit guard in the body, which is why `+8` and
/// `8_0` are `None` here as well as a raise there, and not `8`.
pub(crate) fn url_parts(url: &str) -> UrlParts {
    /// CPython's `_WHATWG_C0_CONTROL_OR_SPACE`.
    fn is_c0_control_or_space(character: char) -> bool {
        character == ' ' || (character as u32) <= 0x1f
    }

    let url = url.trim_start_matches(is_c0_control_or_space);
    let url: String = url
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();

    // The scheme is recognised only when everything before the first `:` is a
    // scheme character *and* the first character is an ASCII letter — the
    // `url[0].isascii() and url[0].isalpha()` guard in CPython, which is why
    // `1http://x` has no scheme.
    let mut scheme = String::new();
    let mut rest = url.as_str();
    if let Some(colon) = url.find(':') {
        let candidate = &url[..colon];
        let first_is_letter = candidate
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic());
        let all_scheme_chars = candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
        if colon > 0 && first_is_letter && all_scheme_chars {
            scheme = candidate.to_lowercase();
            rest = &url[colon + 1..];
        }
    }

    // `_splitnetloc`: the netloc runs from just after the `//` to the first
    // `/`, `?` or `#`. When there is no `//` there is no netloc but there is
    // still a path — `https:/host/x` keeps `/host/x` — so this cannot return
    // early the way the two-component version did.
    let (netloc, after_netloc) = match rest.strip_prefix("//") {
        Some(remainder) => {
            let end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
            (&remainder[..end], &remainder[end..])
        }
        None => ("", rest),
    };
    // Fragment first, then query — CPython's order, and the two are equivalent
    // for the path only because neither can contain the other's delimiter.
    let path = after_netloc.split('#').next().unwrap_or("");
    let path = path.split('?').next().unwrap_or("");

    // `_userinfo`, then `_hostinfo`: both split at the **last** `@`.
    let at = netloc.rfind('@');
    let user = at.map(|at| {
        let userinfo = &netloc[..at];
        // `userinfo.partition(':')` — the **first** colon, and a username with
        // no colon at all is the whole thing.
        let end = userinfo.find(':').unwrap_or(userinfo.len());
        userinfo[..end].to_string()
    });
    let hostinfo = match at {
        Some(at) => &netloc[at + 1..],
        None => netloc,
    };

    let (hostname, port_text) = match hostinfo.find('[') {
        Some(open) => {
            let bracketed = &hostinfo[open + 1..];
            match bracketed.find(']') {
                // The port, when there is one, is after the `]` — so
                // `[::1]:8080` reports `::1` and `8080`, and `[::1]` reports
                // no port rather than a port of `:1`.
                Some(close) => {
                    let tail = &bracketed[close + 1..];
                    (&bracketed[..close], tail.strip_prefix(':').unwrap_or(""))
                }
                None => (bracketed, ""),
            }
        }
        None => match hostinfo.find(':') {
            Some(colon) => (&hostinfo[..colon], &hostinfo[colon + 1..]),
            None => (hostinfo, ""),
        },
    };

    // `SplitResult.hostname` lowercases **only up to the first `%`**, because a
    // scoped IPv6 literal carries its zone there and the zone is
    // case-significant: `smb://[fe80::1%tESt]/x` has the host
    // `fe80::1%tESt`, not `fe80::1%test`. Lowercasing the whole thing would
    // quietly rewrite the zone and pick a different interface.
    let host = if hostname.is_empty() {
        None
    } else {
        match hostname.split_once('%') {
            Some((before, zone)) => Some(format!("{}%{zone}", before.to_lowercase())),
            None => Some(hostname.to_lowercase()),
        }
    };
    // CPython guards with `port.isdigit() and port.isascii()` **before**
    // converting, so the accepted text is ASCII digits and nothing else — no
    // sign, no underscores, no whitespace, no Unicode digits. `+8` and `8_0`
    // both raise there. The `is_ascii_digit` check below reproduces that
    // guard exactly, which matters because `"+8".parse::<u32>()` is `Ok(8)`
    // in Rust and would otherwise turn a URL the reference rejects into a
    // port of 8 here.
    let digits_only = !port_text.is_empty() && port_text.chars().all(|c| c.is_ascii_digit());
    // `0..=65535` is CPython's own range check; anything else it would raise
    // on, and this returns `None` instead. See this function's note.
    let port = port_text
        .parse::<u32>()
        .ok()
        .filter(|port| digits_only && *port <= 65_535);

    UrlParts {
        scheme,
        netloc: netloc.to_string(),
        host,
        user,
        port,
        path: path.to_string(),
    }
}

/// Refuse a response that did not come from where the recipe says it may come
/// from (`installers.py:551-558`).
///
/// Two conditions, both required: the scheme is `https` and the host is one of
/// `installer.allowed_hosts`. The reference raises a `RuntimeError` whose
/// message is [`InstallerError::UntrustedOrigin`]'s Display.
///
/// `final_url` is the URL after redirects, which is the whole point — see
/// [`ResponseHead::final_url`]. An empty string (a client that cannot report
/// one) has no host and is rejected.
pub fn validate_download_origin(
    installer: &Installer,
    final_url: &str,
) -> Result<(), InstallerError> {
    // Reads the scheme and the host and **nothing else**. The parser was
    // widened to five components for `netpaths`; this caller's inputs are
    // unchanged, which is what makes the widening unable to affect it.
    let parts = url_parts(final_url);
    let (scheme, host) = (parts.scheme, parts.host);
    let allowed = installer
        .allowed_hosts
        .iter()
        .map(|item| item.to_lowercase())
        .collect::<Vec<_>>();
    let host_is_allowed = host.as_ref().is_some_and(|host| allowed.contains(host));
    if scheme != "https" || !host_is_allowed {
        return Err(InstallerError::UntrustedOrigin {
            name: installer.name.to_string(),
            url: final_url.to_string(),
        });
    }
    Ok(())
}

/// Refuse a payload whose first bytes are not the container the recipe declared
/// (`installers.py:561-566`).
///
/// `MZ` for an exe, the OLE compound-file header for an MSI. This is a cheap
/// shape check, not a security control — a real attacker's payload passes it —
/// and it is here to catch a *wrong download* (an HTML error page, a captive
/// portal's redirect body) with a message that says what happened, before
/// spending ninety seconds on the signature verifier. The tests assert that
/// ordering: a non-PE payload must fail here and the verifier must not be
/// called.
pub fn validate_installer_magic(installer: &Installer, path: &Path) -> Result<(), InstallerError> {
    use std::io::Read;
    const OLE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    let mut magic = [0u8; 8];
    let mut file = std::fs::File::open(path)?;
    let read = file.read(&mut magic)?;
    let magic = &magic[..read];
    let expected: &[u8] = match installer.kind {
        Kind::Exe => b"MZ",
        Kind::Msi => &OLE,
    };
    // `magic.startswith(expected)`, including for a file shorter than the
    // header: a one-byte `M` does not start with `MZ`.
    if magic.len() < expected.len() || &magic[..expected.len()] != expected {
        return Err(InstallerError::NotValidPayload {
            name: installer.name.to_string(),
            kind: installer.kind,
        });
    }
    Ok(())
}

/// The bundled Microsoft root, if this build can find it
/// (`installers.py:537-548`).
///
/// The reference's four candidates, in order. The second is
/// `Path(__file__).with_name(...)` — the `.pem` sitting beside the Python
/// module — and has no counterpart here because there is no Python module; the
/// third is the repository's `data/` directory relative to the module, which
/// translates to a path relative to this crate's source; the fourth is Flatpak's
/// `/app/share/gamehandler`.
///
/// The first candidate is the environment override, and it is the one that
/// makes this testable and packagable: `GAMEHANDLER_AUTHENTICODE_ROOT` pointing
/// at a real `.pem` satisfies the check without anything having to be installed
/// beside the binary.
pub fn authenticode_root_path(env: &dyn Env) -> Result<PathBuf, InstallerError> {
    let override_path = env
        .var("GAMEHANDLER_AUTHENTICODE_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = override_path {
        candidates.push(path);
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(AUTHENTICODE_ROOT_NAME),
    );
    candidates.push(Path::new("/app/share/gamehandler").join(AUTHENTICODE_ROOT_NAME));
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(InstallerError::AuthenticodeRootUnavailable)
}

/// The filename the bundled root is expected under (`installers.py:534`).
pub const AUTHENTICODE_ROOT_NAME: &str = "microsoft-identity-verification-root-ca-2020.pem";

/// Seconds the signature verifier is given before it is killed
/// (`installers.py:586`).
const SIGNATURE_TIMEOUT: Duration = Duration::from_secs(90);

/// Verify the Authenticode chain and the expected publisher before execution
/// (`installers.py:569-595`).
///
/// # The argv, and why it is two inserts rather than a conditional tail
///
/// The reference builds `[verifier, "verify", "-in", path]` and then, when the
/// recipe pins the Microsoft root, splices two option *pairs* in at index 2 —
/// so they end up between the subcommand and `-in`, and the result is
/// `[verifier, "verify", "-CAfile", root, "-TSA-CAfile", root, "-in", path]`.
/// Building it in that order matters: appending the options at the end would
/// put them after the file argument, which `osslsigncode` parses differently.
///
/// # The two independent conditions
///
/// The output must contain `Signature verification: ok` **and** the
/// case-folded output must contain one of the recipe's publisher strings. The
/// publisher test is a substring test against the whole output, not against a
/// parsed `Subject:` field — which is what makes `CN=GOG  sp. z o.o,O=GOG  sp.
/// z o.o` (two spaces, a comma, in that order) the string the catalog has to
/// carry. A publisher check that split the subject line would accept the two
/// halves in any order and would not need that exact string.
pub fn verify_installer_authenticity(
    installer: &Installer,
    path: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<(), InstallerError> {
    let Some(verifier) = launch_env.which("osslsigncode") else {
        return Err(InstallerError::SignatureToolMissing);
    };
    let mut command: Vec<String> = vec![
        verifier.to_string_lossy().into_owned(),
        "verify".to_string(),
        "-in".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    if installer.microsoft_trust_root {
        let root = authenticode_root_path(launch_env)?
            .to_string_lossy()
            .into_owned();
        command.splice(
            2..2,
            [
                "-CAfile".to_string(),
                root.clone(),
                "-TSA-CAfile".to_string(),
                root,
            ],
        );
    }

    let output = match run_capturing(&command, SIGNATURE_TIMEOUT) {
        Ok(output) => output,
        Err(RunFailure::TimedOut) => {
            return Err(InstallerError::SignatureTimedOut {
                name: installer.name.to_string(),
            });
        }
        Err(RunFailure::Failed(error)) => return Err(error.into()),
    };

    let text = output.text;
    if output.status != Some(0) || !text.contains("Signature verification: ok") {
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(8)..].join("\n");
        return Err(InstallerError::SignatureInvalid {
            name: installer.name.to_string(),
            tail,
        });
    }
    let folded = text.to_lowercase();
    if !installer
        .publishers
        .iter()
        .any(|publisher| folded.contains(&publisher.to_lowercase()))
    {
        return Err(InstallerError::PublisherUnapproved {
            name: installer.name.to_string(),
        });
    }
    Ok(())
}

/// What running a child produced.
struct CommandOutput {
    /// The exit status, or `None` when the child was killed by a signal.
    status: Option<i32>,
    /// `stdout` and `stderr` merged, as `stderr=subprocess.STDOUT` gives.
    text: String,
}

/// Why a child could not be run to completion.
enum RunFailure {
    /// It outlived its bound and was killed.
    TimedOut,
    /// It could not be spawned, or its streams could not be read.
    Failed(std::io::Error),
}

/// How many times a spawn is retried after `ETXTBSY`, and how long between
/// attempts. Five retries over ten milliseconds is fifty milliseconds — three
/// orders of magnitude more than the window they are covering, and small enough
/// that a caller's own timeout still means what it says.
const SPAWN_RETRIES: u32 = 5;
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
fn spawn_retrying(
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
    static SPAWN_RETRIES_TAKEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
fn run_capturing(argv: &[String], timeout: Duration) -> Result<CommandOutput, RunFailure> {
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

/// Create the `.part` file beside the target
/// (`tempfile.mkstemp(dir=dest, prefix=f".{filename}.", suffix=".part")`).
///
/// `mkstemp` guarantees two things that matter and one that does not. The
/// exclusive create is the load-bearing one: `create_new` is `O_CREAT|O_EXCL`,
/// which fails on an existing path *including* a symlink, so a `.part` name an
/// attacker guessed cannot be used to redirect the write. The other is 0600
/// permissions, which `mkstemp` gives and `create_new` does not: `create_new`
/// is bounded by the process umask, so under the usual 022 the download sits
/// at 0644 for the length of the transfer, readable by every local user. That
/// is the file the mode is set on below — an explicit `0o600` on the handle
/// `create_new` just returned, which is the mode the file has from the instant
/// it exists rather than from the instant the download finishes, and which no
/// umask can widen. The part that does not matter is the *unpredictability* of
/// the name, which `mkstemp` gets from random bytes; exclusivity is what makes
/// the name safe, so this counts up from the process id instead, which is also
/// what makes a leftover file nameable in a bug report.
fn create_partial(directory: &Path, filename: &str) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    /// `TMP_MAX`-ish: `mkstemp` gives up after a bounded number of attempts.
    const ATTEMPTS: u32 = 1_000;
    let process = std::process::id();
    for attempt in 0..ATTEMPTS {
        let path = directory.join(format!(".{filename}.{process}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            // `mkstemp`'s own mode argument: the file is never wider than
            // 0600 even for the instant between the create and the chmod below.
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => {
                // A umask with owner bits in it would narrow the create's mode,
                // so the 0600 is then made exact. A failure here removes the
                // file, so a caller that got an `Err` has nothing to reason
                // about but the error.
                if let Err(error) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                return Ok((file, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("could not create a temporary file for {filename}"),
    ))
}

/// Atomically download and authenticate a vendor installer
/// (`installers.py:598-646`).
///
/// # The order, which is the whole function
///
/// The bytes are written to a `.part` file beside the target and the target is
/// only ever created by a rename, so an interrupted or refused download cannot
/// leave something at the target path that the caller would then treat as a
/// verified installer. Every failure — an untrusted origin, a payload that is
/// not the declared container, a signature that does not verify, a missing
/// verifier — removes the `.part` file and returns the target to whatever it
/// was before, which for a first install is nothing at all.
///
/// The checks happen in a deliberate order, and the reference's tests pin all
/// three steps of it:
///
/// 1. **Origin, in the head callback, before a byte is kept.**
///    [`validate_download_origin`] runs on the final URL the client reports.
///    Returning `Err` there abandons the transfer, so a redirect to an
///    unapproved host costs no bytes at all rather than up to
///    [`MAX_INSTALLER_BYTES`] of them.
/// 2. **Size, twice.** The declared `Content-Length` first — the cheap refusal
///    — and then the running total, because a server that understates its
///    length still cannot fill the disk.
/// 3. **Shape then signature**, both against the `.part` file. Shape first so a
///    captive portal's HTML costs no verifier run, and the verifier only after
///    the file is complete.
///
/// `progress` is called with `downloaded / total` while the body arrives, only
/// when a length was declared, and exactly once with `1.0` at the end — which
/// is what the reference does, and is why the final call is outside the
/// `if declared` guard.
pub fn download_installer(
    installer: &Installer,
    dest_dir: &Path,
    progress: Option<&dyn Fn(f64)>,
    timeout: Duration,
    client: &dyn HttpClient,
    launch_env: &dyn LaunchEnv,
) -> Result<PathBuf, InstallerError> {
    std::fs::create_dir_all(dest_dir)?;
    let filename = safe_download_name(installer.filename, installer.id);
    let target = dest_dir.join(&filename);
    let (file, temporary) = create_partial(dest_dir, &filename)?;

    let result = download_into(
        installer, &temporary, file, progress, timeout, client, launch_env,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, &target) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Some(progress) = progress {
        progress(1.0);
    }
    Ok(target)
}

/// The `Err` a transfer callback returns to abandon a download it has already
/// refused.
///
/// The trait's callbacks speak [`RunnerError`] while this module speaks
/// [`InstallerError`], so a refusal recorded in the callback and returned as
/// `RunnerError` would lose the variant that says *which* refusal it was —
/// `UntrustedOrigin` and `TooLarge` are different things to report, and the
/// reference's tests distinguish them by message. So the callback records the
/// real error in [`download_into`]'s `refusal` cell and returns this to stop the
/// transfer; the cell is what gets returned to the caller.
///
/// **The message is never rendered** — the cell always wins — but it carries
/// the refusal's own text anyway, so that a future caller which propagated this
/// instead would still print the right sentence rather than a placeholder.
fn abandon(reason: &InstallerError) -> RunnerError {
    RunnerError::Http {
        message: reason.to_string(),
    }
}

/// The body of [`download_installer`], split out so every exit from it — errors
/// included — is a single `Result` the caller can clean up after.
///
/// # Why the shared state is in cells rather than `mut` locals
///
/// `on_head` and `sink` are two simultaneous `&mut dyn FnMut`, so anything both
/// of them touch cannot be a plain `&mut` local — the second closure's borrow
/// would overlap the first's. A `Cell`/`RefCell` behind a shared reference is
/// borrowed by each closure rather than moved into one, which is what lets the
/// head record `total` and the sink read it, and lets either of them record a
/// refusal. This is the same problem the trait's two callbacks create on the
/// client side.
#[allow(clippy::too_many_arguments)]
fn download_into(
    installer: &Installer,
    temporary: &Path,
    mut file: std::fs::File,
    progress: Option<&dyn Fn(f64)>,
    timeout: Duration,
    client: &dyn HttpClient,
    launch_env: &dyn LaunchEnv,
) -> Result<(), InstallerError> {
    use std::cell::{Cell, RefCell};
    use std::io::Write;

    let total = Cell::new(0u64);
    let downloaded = Cell::new(0u64);
    let refusal: RefCell<Option<InstallerError>> = RefCell::new(None);

    let headers = [("User-Agent", USER_AGENT)];
    let outcome = client.get(
        installer.download_url,
        &headers,
        timeout,
        &mut |head: &ResponseHead| {
            // Before a byte is kept: a redirect to a host the recipe never
            // approved costs nothing at all rather than the whole body.
            if let Err(reason) = validate_download_origin(installer, &head.final_url) {
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            total.set(parse_content_length(head.content_length.as_deref()));
            if total.get() > MAX_INSTALLER_BYTES {
                let reason = InstallerError::TooLarge {
                    name: installer.name.to_string(),
                };
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            Ok(())
        },
        &mut |chunk: &[u8]| {
            downloaded.set(downloaded.get() + chunk.len() as u64);
            if downloaded.get() > MAX_INSTALLER_BYTES {
                let reason = InstallerError::TooLarge {
                    name: installer.name.to_string(),
                };
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            if let Err(error) = file.write_all(chunk) {
                let reason = InstallerError::Io(error);
                let stopping = abandon(&reason);
                refusal.replace(Some(reason));
                return Err(stopping);
            }
            if let Some(progress) = progress.filter(|_| total.get() > 0) {
                progress((downloaded.get() as f64 / total.get() as f64).min(1.0));
            }
            Ok(())
        },
    );

    // The recorded refusal is the specific error; `outcome`'s is the generic
    // "abandoned" that stopped the transfer.
    if let Some(reason) = refusal.into_inner() {
        return Err(reason);
    }
    outcome?;

    validate_installer_magic(installer, temporary)?;
    verify_installer_authenticity(installer, temporary, launch_env)?;
    Ok(())
}

/// `Content-Length`, parsed the way the reference parses it
/// (`int(resp.headers.get("Content-Length", 0) or 0)`).
///
/// A header that is absent, empty, or not a number all give `0`, which the
/// progress callback reads as "no denominator, report nothing" rather than as
/// an error. It is deliberately **not** [`crate::runners::proton::parse_content_length`]:
/// that one raises `"Runner download has an invalid Content-Length"` because
/// the runner download's size drives a staging decision, while this one only
/// ever feeds a progress bar — and the reference is equally relaxed here.
fn parse_content_length(declared: Option<&str>) -> u64 {
    declared
        .map(|value| value.trim())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The wizard half (installers.py:299-408)
// ---------------------------------------------------------------------------

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
pub fn wait_for_installer(
    runner: &dyn Runner,
    env: &std::collections::BTreeMap<String, String>,
    prefix: &Path,
    expected: &[&str],
    clock: &dyn InstallClock,
    idle: IdleWait<'_>,
) -> Option<PathBuf> {
    clock.sleep(INSTALL_HANDOFF_SECONDS);
    let started = clock.now();
    let mut busy_wait = 0.0f64;
    let mut deadline = started + INSTALL_SETTLE_TIMEOUT_SECONDS;
    loop {
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
    use crate::runners::env::tests::FakeLaunchEnv;

    /// `EXPECTED_IDS` (`tests/test_installers.py:31-41`), as a list rather than
    /// as the catalog's own order, so this test cannot agree with a reordered
    /// catalog by reading it back.
    const EXPECTED_IDS: [&str; 9] = [
        "battlenet",
        "epic",
        "ea-app",
        "ubisoft",
        "gog",
        "amazon",
        "rockstar",
        "steam",
        "discord",
    ];

    /// `test_catalog_contains_eight_launchers_and_discord`
    /// (`tests/test_installers.py:45-50`).
    #[test]
    fn the_catalog_is_eight_launchers_and_one_app() {
        let ids: Vec<&str> = installers().iter().map(|item| item.id).collect();
        assert_eq!(ids, EXPECTED_IDS);
        assert_eq!(
            installers()
                .iter()
                .filter(|item| item.category == LAUNCHERS)
                .count(),
            8
        );
        assert_eq!(
            installers()
                .iter()
                .filter(|item| item.category == APPS)
                .count(),
            1
        );
        assert_eq!(installer_by_id("discord").unwrap().category, APPS);
    }

    /// `test_every_recipe_has_official_https_url_and_expected_exe`
    /// (`tests/test_installers.py:52-61`).
    #[test]
    fn every_recipe_is_complete_enough_to_be_used() {
        for item in installers() {
            assert!(item.download_url.starts_with("https://"), "{}", item.id);
            assert!(!item.filename.is_empty(), "{}", item.id);
            assert!(!item.expected_exe.is_empty(), "{}", item.id);
            assert!(!item.name.is_empty(), "{}", item.id);
            assert!(!item.description.is_empty(), "{}", item.id);
            assert!(!item.allowed_hosts.is_empty(), "{}", item.id);
            assert!(!item.publishers.is_empty(), "{}", item.id);
        }
    }

    /// The invariant the reference's `kind in {"exe", "msi"}` test only
    /// gestures at: for every recipe the kind and the filename agree, which is
    /// what `installer_argv` and the magic check both rely on. A tenth recipe
    /// added as an `Msi` with an `.exe` filename would pass the reference's
    /// test and fail this one.
    #[test]
    fn the_kind_and_the_filename_extension_agree() {
        for item in installers() {
            assert_eq!(
                item.kind == Kind::Msi,
                item.filename.to_lowercase().ends_with(".msi"),
                "{}: {:?} vs {}",
                item.id,
                item.kind,
                item.filename
            );
        }
        assert_eq!(Kind::Exe.as_str(), "exe");
        assert_eq!(Kind::Msi.as_str(), "msi");
        assert_eq!(Kind::Msi.label(), "MSI");
    }

    /// `test_epic_is_msi` (`tests/test_installers.py:63-65`).
    #[test]
    fn epic_is_the_one_msi() {
        assert_eq!(installer_by_id("epic").unwrap().kind, Kind::Msi);
        assert_eq!(
            installers()
                .iter()
                .filter(|item| item.kind == Kind::Msi)
                .count(),
            1
        );
    }

    /// `test_gog_requires_the_full_publisher_identity`
    /// (`tests/test_installers.py:67-71`). The double space is the point.
    #[test]
    fn gog_requires_the_full_publisher_identity() {
        assert_eq!(
            installer_by_id("gog").unwrap().publishers,
            ["CN=GOG  sp. z o.o,O=GOG  sp. z o.o"]
        );
    }

    /// `test_search_and_category_filter` (`tests/test_installers.py:73-79`).
    #[test]
    fn search_matches_by_id_and_the_category_filter_narrows() {
        let hits = search_installers("epic", "");
        assert_eq!(
            hits.iter().map(|item| item.id).collect::<Vec<_>>(),
            ["epic"]
        );
        let apps = search_installers("", APPS);
        assert_eq!(
            apps.iter().map(|item| item.id).collect::<Vec<_>>(),
            ["discord"]
        );
        assert!(search_installers("no-such-launcher", "").is_empty());
    }

    /// The three fields the reference searches, one test each, plus the folding
    /// of the query.
    #[test]
    fn search_reaches_the_name_the_description_and_the_id() {
        // By name.
        assert_eq!(
            search_installers("rockstar games launcher", "")
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["rockstar"]
        );
        // By description only: "Blizzard" is in no name or id.
        assert_eq!(
            search_installers("blizzard", "")
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["battlenet"]
        );
        // By id only: "ea-app" is in no name, and the description says "EA App".
        assert_eq!(
            search_installers("ea-app", "")
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["ea-app"]
        );
        // `query.strip().lower()` — surrounding whitespace and case are folded.
        assert_eq!(
            search_installers("  EPIC  ", "").len(),
            1,
            "the query is stripped and lowercased"
        );
    }

    /// The category comparison is exact, which is the detail a case-folding
    /// "improvement" would silently change.
    #[test]
    fn the_category_filter_is_exact_and_never_folded() {
        assert_eq!(search_installers("", "").len(), 9);
        assert_eq!(search_installers("", ALL_CATEGORIES).len(), 9);
        assert_eq!(search_installers("", LAUNCHERS).len(), 8);
        assert_eq!(search_installers("", "launchers").len(), 0);
        assert_eq!(search_installers("", "Games").len(), 0);
    }

    /// The filter preserves the catalog's order, which is what puts the cards
    /// on the page in the reference's order.
    #[test]
    fn filtering_preserves_the_catalog_order() {
        let all: Vec<&str> = installers().iter().map(|item| item.id).collect();
        let filtered: Vec<&str> = search_installers("", LAUNCHERS)
            .iter()
            .map(|item| item.id)
            .collect();
        let expected: Vec<&str> = all.into_iter().filter(|id| *id != "discord").collect();
        assert_eq!(filtered, expected);
    }

    /// `test_exported_categories_match_the_catalog`
    /// (`tests/test_installers.py:369-372`).
    #[test]
    fn the_exported_categories_match_the_catalog() {
        assert_eq!(INSTALLER_CATEGORIES, [LAUNCHERS, APPS]);
        for item in installers() {
            assert!(
                INSTALLER_CATEGORIES.contains(&item.category),
                "{} names a category the page does not offer",
                item.id
            );
        }
    }

    /// `test_notes_are_present_where_the_vendor_needs_an_explanation`
    /// (`tests/test_installers.py:374-377`), plus the two entries whose
    /// `library_category` is not the page's category — the pair that would be
    /// lost if the two strings were collapsed into one.
    #[test]
    fn the_notes_and_the_library_categories_are_where_the_reference_puts_them() {
        assert!(!installer_by_id("battlenet").unwrap().notes.is_empty());
        assert!(!installer_by_id("discord").unwrap().notes.is_empty());
        assert_eq!(
            installer_by_id("discord").unwrap().library_category,
            "Utility"
        );
        assert_eq!(
            installer_by_id("discord").unwrap().launch_arguments,
            "--processStart Discord.exe"
        );
        for item in installers() {
            if item.id != "discord" {
                assert_eq!(
                    item.library_category, LAUNCHERS,
                    "{}: only Discord leaves the page category for the library",
                    item.id
                );
            }
        }
    }

    /// `installers.py:238` — the message names the id, which is what the app's
    /// silent `except KeyError` is silently swallowing.
    #[test]
    fn an_unknown_id_is_an_error_that_names_it() {
        let error = installer_by_id("no-such-launcher").unwrap_err();
        assert_eq!(error.to_string(), "Unknown installer: no-such-launcher");
        assert!(matches!(
            error,
            InstallerError::UnknownInstaller { ref id } if id == "no-such-launcher"
        ));
    }

    // -----------------------------------------------------------------------
    // The command half — MsiexecArgvTests (tests/test_installers.py:159-218)
    // -----------------------------------------------------------------------

    /// A scratch directory that cleans itself up.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gh-installers-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Write a file, creating parents as needed.
    fn touch(path: &Path) -> PathBuf {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "stub").unwrap();
        path.to_path_buf()
    }

    fn wine() -> crate::runners::WineRunner {
        crate::runners::WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")))
    }

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

    /// Install `steam.exe` under `drive_c` and return it.
    fn install_steam(drive_c: &Path) -> PathBuf {
        touch(&drive_c.join("Program Files (x86)/Steam/steam.exe"))
    }

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

    // -----------------------------------------------------------------------
    // `urllib.parse` — the allowlist's host parser
    // -----------------------------------------------------------------------

    /// One row of the battery: `(url, scheme, host, user, port, path, netloc)`.
    ///
    /// A named alias rather than the tuple spelled out at the array, because
    /// clippy's `type_complexity` fires on the literal form and the gate is
    /// `-D warnings` — and because the row is easier to read named.
    type UrlVector<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Option<u32>,
        &'a str,
        &'a str,
    );

    /// The vector battery was produced by running CPython's `urllib.parse` on
    /// this machine and copying its output, then checked line by line.
    ///
    /// # The four lines that are the reason this is a table rather than an
    /// assertion about `split("://")`
    ///
    /// * `https://cdn.akamai.steamstatic.com@evil.example/x` → `evil.example`.
    ///   Userinfo is split at the last `@`, so the host is what comes *after*
    ///   it. A parser that split at the first `@` would report
    ///   `evil.example@...` — harmless here because it is not in an allowlist
    ///   either — but a parser that *ignored* userinfo would report the allowed
    ///   host and accept the download. That is the bypass this test exists for.
    /// * `https://evil.example@cdn.akamai.steamstatic.com/x` → the allowed
    ///   host, correctly: that URL really is served by the allowed host.
    ///   The pair is the control arm — one line alone cannot distinguish "splits
    ///   at the last `@`" from "splits at the first" or "ignores `@`".
    /// * `https://cdn.akamai.steamstatic.com./x` → a **different** host
    ///   (trailing dot), which is not in the allowlist and so fails closed.
    /// * `https://cdn.akamai.steamstatic.com:notaport/x` → the allowed host:
    ///   `.hostname` does not validate the port. Pinned so a later change that
    ///   "tidied" the parse by rejecting a bad port cannot silently make this
    ///   stricter than the reference.
    #[test]
    fn the_url_parser_matches_cpython() {
        // **58 vectors, and the count went up twice for a reason worth
        // naming.** The first 27 are the allowlist's own and every one of them
        // was generated by running CPython's `urlsplit` on this machine, not
        // written from memory. `netpaths` then needed the username, port and
        // path as well, so the *same* battery was extended over all five
        // components rather than a second one started — one parser, one
        // fidelity, one table. It went 27 → 47 → 58 for exactly that reason,
        // and the last eleven are the IPv6-zone and port-zero rows that caught
        // two real defects rather than illustrating a rule already known.
        //
        // Each tuple is `(url, scheme, host, user, port, path, netloc)` and
        // every value in it came out of `urlsplit(...)`, so a disagreement here
        // is a disagreement with CPython and not with a prior reading of it.
        let vectors: [UrlVector; 58] = [
            (
                "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/client/installer/SteamSetup.exe",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "HTTPS://CDN.AKAMAI.STEAMSTATIC.COM/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "CDN.AKAMAI.STEAMSTATIC.COM",
            ),
            (
                "https://cdn.akamai.steamstatic.com:443/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                Some(443),
                "/x",
                "cdn.akamai.steamstatic.com:443",
            ),
            (
                "https://user:pw@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("user"),
                None,
                "/x",
                "user:pw@cdn.akamai.steamstatic.com",
            ),
            (
                "http://cdn.akamai.steamstatic.com/x",
                "http",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://evil.example/SteamSetup.exe",
                "https",
                Some("evil.example"),
                None,
                None,
                "/SteamSetup.exe",
                "evil.example",
            ),
            (
                "//cdn.akamai.steamstatic.com/x",
                "",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "cdn.akamai.steamstatic.com/x",
                "",
                None,
                None,
                None,
                "cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https://cdn.akamai.steamstatic.com./x",
                "https",
                Some("cdn.akamai.steamstatic.com."),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com.",
            ),
            (
                "https://cdn.akamai.steamstatic.com@evil.example/x",
                "https",
                Some("evil.example"),
                Some("cdn.akamai.steamstatic.com"),
                None,
                "/x",
                "cdn.akamai.steamstatic.com@evil.example",
            ),
            (
                "https://evil.example@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("evil.example"),
                None,
                "/x",
                "evil.example@cdn.akamai.steamstatic.com",
            ),
            (
                "https://a@b@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("a@b"),
                None,
                "/x",
                "a@b@cdn.akamai.steamstatic.com",
            ),
            (
                "https://[2001:db8::1]/x",
                "https",
                Some("2001:db8::1"),
                None,
                None,
                "/x",
                "[2001:db8::1]",
            ),
            (
                "https://cdn.akamai.steamstatic.com:notaport/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com:notaport",
            ),
            ("https:///x", "https", None, None, None, "/x", ""),
            ("https://", "https", None, None, None, "", ""),
            ("", "", None, None, None, "", ""),
            (
                "https://cdn.akamai.steamstatic.com\t/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                " https://cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://\tcdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "ftp://cdn.akamai.steamstatic.com/x",
                "ftp",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://cdn.akamai.steamstatic.com.evil.example/x",
                "https",
                Some("cdn.akamai.steamstatic.com.evil.example"),
                None,
                None,
                "/x",
                "cdn.akamai.steamstatic.com.evil.example",
            ),
            (
                "https://cdn.akamai.steamstatic.com?x=1",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "https://cdn.akamai.steamstatic.com#frag",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                None,
                "",
                "cdn.akamai.steamstatic.com",
            ),
            (
                "1https://cdn.akamai.steamstatic.com/x",
                "",
                None,
                None,
                None,
                "1https://cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https:/cdn.akamai.steamstatic.com/x",
                "https",
                None,
                None,
                None,
                "/cdn.akamai.steamstatic.com/x",
                "",
            ),
            (
                "https://cdn.akamai.steamstatic.com:80/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                None,
                Some(80),
                "/x",
                "cdn.akamai.steamstatic.com:80",
            ),
            (
                "smb://server/share/game.exe",
                "smb",
                Some("server"),
                None,
                None,
                "/share/game.exe",
                "server",
            ),
            (
                "smb://user@server/share/game.exe",
                "smb",
                Some("server"),
                Some("user"),
                None,
                "/share/game.exe",
                "user@server",
            ),
            (
                "smb://user:pw@SERVER/Share/dir/game.exe",
                "smb",
                Some("server"),
                Some("user"),
                None,
                "/Share/dir/game.exe",
                "user:pw@SERVER",
            ),
            (
                "smb://server:445/share/game.exe",
                "smb",
                Some("server"),
                None,
                Some(445),
                "/share/game.exe",
                "server:445",
            ),
            (
                "sftp://host/pub/game.exe",
                "sftp",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ssh://host/pub/game.exe",
                "ssh",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ftp://host/pub/game.exe",
                "ftp",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "ftps://host/pub/game.exe",
                "ftps",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "dav://host/pub/game.exe",
                "dav",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "davs://host/pub/game.exe",
                "davs",
                Some("host"),
                None,
                None,
                "/pub/game.exe",
                "host",
            ),
            (
                "nfs://host/export/game.exe",
                "nfs",
                Some("host"),
                None,
                None,
                "/export/game.exe",
                "host",
            ),
            (
                "sftp://user@host:2222/pub/x.exe",
                "sftp",
                Some("host"),
                Some("user"),
                Some(2222),
                "/pub/x.exe",
                "user@host:2222",
            ),
            (
                "smb://server/share/a%20b.exe",
                "smb",
                Some("server"),
                None,
                None,
                "/share/a%20b.exe",
                "server",
            ),
            (
                "smb://us%40er@server/share/x.exe",
                "smb",
                Some("server"),
                Some("us%40er"),
                None,
                "/share/x.exe",
                "us%40er@server",
            ),
            (
                "smb:///nohost/x.exe",
                "smb",
                None,
                None,
                None,
                "/nohost/x.exe",
                "",
            ),
            (
                "file:///home/u/game.exe",
                "file",
                None,
                None,
                None,
                "/home/u/game.exe",
                "",
            ),
            (
                "file://host/home/u/game.exe",
                "file",
                Some("host"),
                None,
                None,
                "/home/u/game.exe",
                "host",
            ),
            (
                "https://host/x?q=1#f",
                "https",
                Some("host"),
                None,
                None,
                "/x",
                "host",
            ),
            (
                "smb://[fe80::1]/share/x.exe",
                "smb",
                Some("fe80::1"),
                None,
                None,
                "/share/x.exe",
                "[fe80::1]",
            ),
            (
                "http://host/pub/x.exe",
                "http",
                Some("host"),
                None,
                None,
                "/pub/x.exe",
                "host",
            ),
            // The rows below were added with `netpaths`, and two of them are
            // here because writing them caught a real defect in the widening
            // rather than because they were obvious:
            //
            // * `[fe80::1%tESt]` — the IPv6 zone. `.hostname` lowercases only
            //   the part before `%`, so the host is `fe80::1%tESt` with the
            //   zone's case intact. Lowercasing the whole literal, which is
            //   what the first cut of the widened parser did, silently picks a
            //   different interface.
            // * `h:0` — port **zero**, which is not `None`. `_hostinfo` only
            //   blanks a port that is the empty string, and `"0"` is a
            //   perfectly good `isdigit()`; so CPython reports 0 here while
            //   `_generic_mount_names`'s `if port:` still treats it as unset.
            //   Both halves of that are load-bearing and neither is guessable.
            (
                "smb://[fe80::1%tESt]/share/x.exe",
                "smb",
                Some("fe80::1%tESt"),
                None,
                None,
                "/share/x.exe",
                "[fe80::1%tESt]",
            ),
            ("smb://h:0/x", "smb", Some("h"), None, Some(0), "/x", "h:0"),
            (
                "smb://[::1]:8080/share/x.exe",
                "smb",
                Some("::1"),
                None,
                Some(8080),
                "/share/x.exe",
                "[::1]:8080",
            ),
            (
                "sftp://user@host:2222/pub/x",
                "sftp",
                Some("host"),
                Some("user"),
                Some(2222),
                "/pub/x",
                "user@host:2222",
            ),
            (
                "davs://host/path/x",
                "davs",
                Some("host"),
                None,
                None,
                "/path/x",
                "host",
            ),
            (
                "file:///home/u/game.exe",
                "file",
                None,
                None,
                None,
                "/home/u/game.exe",
                "",
            ),
            // The username is the text before the **first** colon of the
            // userinfo, so the password never leaks into it — while the
            // userinfo itself split at the **last** `@`, which is why the
            // `a@b` row below has a username containing an `@`.
            (
                "smb://us:er:pw@h/s/x",
                "smb",
                Some("h"),
                Some("us"),
                None,
                "/s/x",
                "us:er:pw@h",
            ),
            (
                "https://a@b@cdn.akamai.steamstatic.com/x",
                "https",
                Some("cdn.akamai.steamstatic.com"),
                Some("a@b"),
                None,
                "/x",
                "a@b@cdn.akamai.steamstatic.com",
            ),
            // A scheme with no netloc, and a netloc with no host.
            (
                "smb:///nohost/x.exe",
                "smb",
                None,
                None,
                None,
                "/nohost/x.exe",
                "",
            ),
            (
                "smb://H/S/dir/game.exe",
                "smb",
                Some("h"),
                None,
                None,
                "/S/dir/game.exe",
                "H",
            ),
            (
                "ftp://host:21/pub/x",
                "ftp",
                Some("host"),
                None,
                Some(21),
                "/pub/x",
                "host:21",
            ),
        ];
        for (url, scheme, host, user, port, path, netloc) in vectors {
            let parts = url_parts(url);
            assert_eq!(parts.scheme, scheme, "scheme of {url:?}");
            assert_eq!(parts.host.as_deref(), host, "host of {url:?}");
            assert_eq!(parts.user.as_deref(), user, "user of {url:?}");
            assert_eq!(parts.port, port, "port of {url:?}");
            assert_eq!(parts.path, path, "path of {url:?}");
            assert_eq!(parts.netloc, netloc, "netloc of {url:?}");
        }
    }

    /// The ports the reference **raises** on, where this returns `None`.
    ///
    /// These cannot live in the table above, because there is no value for
    /// CPython to have produced: `SplitResult.port` raises `ValueError` on
    /// anything that is not ASCII digits in range. This test is the control
    /// arm for the port's `isdigit()`-and-`isascii()` guard — without it, a
    /// parser that used Rust's `str::parse` and nothing else would report
    /// `+8` as port 8, a URL the reference refuses outright, and no row in the
    /// table would notice.
    ///
    /// Recorded rather than replicated: a `ValueError` out of a pure function
    /// has no honest port here, and returning `None` cannot take the caller
    /// down the way the raise does.
    #[test]
    fn a_port_the_reference_raises_on_is_none_here_and_never_a_number() {
        // Measured: every one of these raises `ValueError` from `.port` in
        // CPython, the first with "Port could not be cast to integer value".
        for url in [
            "sftp://h:any/x",
            "sftp://h:8_0/x",
            "sftp://h:+8/x",
            "sftp://h:-8/x",
            "sftp://h: 80/x",
            "sftp://h:1:2/x",
            "sftp://h:99999/x",
            "sftp://h:65536/x",
        ] {
            assert_eq!(url_parts(url).port, None, "port of {url:?}");
        }
        // The control arm: the boundary values the reference *accepts*, so the
        // assertion above cannot be satisfied by a parser that returns `None`
        // for every port.
        assert_eq!(url_parts("sftp://h:0/x").port, Some(0));
        assert_eq!(url_parts("sftp://h:65535/x").port, Some(65_535));
        assert_eq!(url_parts("sftp://h:0080/x").port, Some(80));
    }

    /// The allowlist check itself, on the two arms that matter.
    #[test]
    fn the_origin_check_accepts_the_recipes_host_and_refuses_another() {
        let steam = installer_by_id("steam").unwrap();
        assert!(validate_download_origin(steam, steam.download_url).is_ok());
        // The redirect the reference's test uses.
        let error =
            validate_download_origin(steam, "https://evil.example/SteamSetup.exe").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam redirected to an untrusted download origin: https://evil.example/SteamSetup.exe"
        );
        // The same host over http is still refused: the scheme is checked
        // independently of the host.
        assert!(validate_download_origin(steam, "http://cdn.akamai.steamstatic.com/x").is_err());
        // And a client that cannot report a final URL fails closed rather than
        // falling back to the request URL.
        assert!(validate_download_origin(steam, "").is_err());
        // A look-alike suffix is a different host.
        assert!(
            validate_download_origin(steam, "https://cdn.akamai.steamstatic.com.evil.example/x")
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // The download half — DownloadSecurityTests
    // (tests/test_installers.py:83-155)
    // -----------------------------------------------------------------------

    /// The bytes an EXE payload must start with.
    const PE_BODY: &[u8] = b"MZsafe-installer";

    /// A client that answers with canned bytes and one declared length.
    ///
    /// `delivered` counts what reached the sink, which is how a test shows the
    /// origin check ran in the *head*: a refusal there must leave `delivered`
    /// at zero. The reference's version of this test cannot see that — it mocks
    /// `urlopen` and reads the whole body either way — so this is stronger than
    /// a port of it.
    struct FakeResponse {
        final_url: String,
        content_length: Option<String>,
        body: Vec<u8>,
        delivered: std::cell::Cell<usize>,
    }

    impl FakeResponse {
        /// A response from `url`'s own host, declaring its real length.
        fn at(url: &str, body: &[u8]) -> Self {
            Self {
                final_url: url.to_string(),
                content_length: Some(body.len().to_string()),
                body: body.to_vec(),
                delivered: std::cell::Cell::new(0),
            }
        }

        fn redirected_to(mut self, url: &str) -> Self {
            self.final_url = url.to_string();
            self
        }

        fn without_declared_length(mut self) -> Self {
            self.content_length = None;
            self
        }

        fn declaring(mut self, length: &str) -> Self {
            self.content_length = Some(length.to_string());
            self
        }

        fn delivered(&self) -> usize {
            self.delivered.get()
        }
    }

    impl crate::runners::proton::HttpClient for FakeResponse {
        fn get(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            on_head(&ResponseHead {
                content_length: self.content_length.clone(),
                final_url: self.final_url.clone(),
            })?;
            // Only reached when the head was accepted — which is what makes
            // `delivered` an observation about the head callback's answer.
            sink(&self.body)?;
            self.delivered.set(self.body.len());
            Ok(())
        }
    }

    /// A fake `osslsigncode` that records its argv and prints canned output.
    ///
    /// A real script rather than a mock, and the values are baked into it
    /// rather than read from the environment: `cargo test` runs cases in
    /// threads of one process, so `std::env::set_var` in a test would be a race
    /// against every other case. The record file is the evidence that the
    /// verifier ran at all, which is what the reference asserts with
    /// `verify.assert_not_called()`.
    fn fake_osslsigncode(directory: &Path, record: &Path, output: &str, exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("osslsigncode");
        std::fs::create_dir_all(directory).unwrap();
        // `#!/bin/sh` plus the absolute path of the real shell, because
        // `run_capturing` now spawns with a cleared environment (`SEC-09`) and a
        // bare shebang would leave the kernel to resolve `sh` through a `PATH`
        // the child no longer has.
        let body = format!(
            "#!{}\nprintf '%s\\n' \"$@\" >> '{}'\nprintf '%s\\n' '{}'\nexit {exit}\n",
            fake_shell(),
            record.display(),
            output
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// `/bin/sh` as an absolute path, for the fakes' shebangs.
    ///
    /// Hardcoded rather than looked up through `PATH`, because the whole point
    /// is that the child has no `PATH` — and because a lookup here would make
    /// the fakes depend on the developer's shell, which is the sort of hidden
    /// input that turns a hermetic suite into one that passes on one machine.
    fn fake_shell() -> &'static str {
        "/bin/sh"
    }

    /// A launch environment whose `osslsigncode` is `script`.
    fn verifier_env(script: &Path) -> FakeLaunchEnv {
        FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy())
    }

    /// A fake verifier that prints an approving line and then dumps its own
    /// environment to `env_record` (`SEC-09`).
    ///
    /// A separate helper rather than a flag on [`fake_osslsigncode`], because
    /// the dump has to land in its own file: the other tests assert on the
    /// merged stdout/stderr text and on the argv record, and adding lines to
    /// either would change what they are reading.
    fn fake_osslsigncode_dumping_env(
        directory: &Path,
        env_record: &Path,
        output: &str,
        exit: i32,
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("osslsigncode");
        std::fs::create_dir_all(directory).unwrap();
        // `export -p` rather than `env`, because the shell's `export` is a
        // builtin: with `PATH` cleared there is nothing to resolve `env`, `cat`
        // or any other external command with, and the dump would be empty for
        // the wrong reason. `printf` is a builtin too.
        //
        // Note what this *cannot* show: `sh` sets `PWD`, `SHLVL` and `OLDPWD`
        // itself, so those three appear even in a completely empty environment.
        // The test excludes them by name rather than assuming an empty dump.
        let body = format!(
            "#!{}\nprintf '%s\\n' '{}'\nexport -p > '{}'\nexit {exit}\n",
            fake_shell(),
            output,
            env_record.display()
        );
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// Every `.part` file left in `directory` — asserted empty after both a
    /// success and a refusal, which is the reference's
    /// `[item for item in dest.iterdir() if item.name.endswith(".part")]`.
    fn part_files(directory: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".part"))
            })
            .collect()
    }

    /// The `.part` file is `0600` from the moment it exists, as `mkstemp`'s is
    /// (`installers.py:612-614`), and it is `create_new` that makes the name
    /// unguessable-in-effect: `O_CREAT|O_EXCL` refuses a path that already
    /// exists *including* one that is a symlink, so a `.part` name an attacker
    /// planted cannot redirect the write.
    ///
    /// Both halves are asserted against the file the real download uses, not
    /// against a re-created temp file: the mode is read from inside the
    /// transfer's own progress callback, which is the only moment the `.part`
    /// exists, and the symlinks are planted at the names this process's own id
    /// generates, so they are the names `create_partial` actually reaches for.
    #[test]
    fn a_partial_download_is_private_to_its_owner_and_cannot_be_redirected() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("part-mode");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &record,
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        let steam = installer_by_id("steam").unwrap();

        let observed: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
        let progress = |_value: f64| {
            for entry in std::fs::read_dir(&dest).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().ends_with(".part") {
                    let mode = entry
                        .metadata()
                        .expect("the temporary file is statable")
                        .permissions()
                        .mode()
                        & 0o777;
                    observed.borrow_mut().push(mode);
                }
            }
        };

        // A symlink at the *target* path is the store-installer case the
        // recipe catalogue cannot rule out, and the reason the install is a
        // rename: `rename` replaces it rather than writing through it.
        let target = dest.join("SteamSetup.exe");
        std::fs::create_dir_all(&dest).expect("the destination directory");
        std::os::unix::fs::symlink(scratch.path().join("sentinel"), &target)
            .expect("plant the target symlink");

        download_installer(
            steam,
            &dest,
            Some(&progress),
            Duration::from_secs(60),
            &FakeResponse::at(steam.download_url, PE_BODY),
            &verifier_env(&script),
        )
        .expect("an authenticated download");

        let observed = observed.into_inner();
        assert!(
            !observed.is_empty(),
            "no .part file was observed during the transfer"
        );
        assert!(
            observed.iter().all(|mode| *mode == 0o600),
            "the .part file was readable by more than its owner: {observed:?}"
        );
        assert!(
            !target.is_symlink(),
            "the rename wrote through the target symlink"
        );
        assert_eq!(std::fs::read(&target).unwrap(), PE_BODY);
    }

    /// A hostile `.part` name cannot make `create_partial` panic, escape the
    /// destination, or open something it did not create.
    ///
    /// The name reaches it from the recipe catalogue, which is data this app
    /// did not write; a name the filesystem refuses is a failed download, not
    /// a crash. Written as a battery because the failures are all different
    /// kinds — `InvalidInput` for a NUL, `ENAMETOOLONG`, `ENOENT` for a
    /// component that is not there — and the contract is only that none of
    /// them is a panic.
    #[test]
    fn hostile_partial_names_fail_cleanly() {
        let scratch = Scratch::new("part-hostile");
        let dir = scratch.path();
        let cases = [
            String::new(),
            ".".to_string(),
            "..".to_string(),
            "a/b".to_string(),
            "..\\..\\evil.exe".to_string(),
            "nul\u{0}byte".to_string(),
            "x".repeat(4096),
        ];
        for name in cases {
            match create_partial(dir, &name) {
                Ok((file, path)) => {
                    // Accepted names still obey the two invariants.
                    assert!(path.starts_with(dir), "{name:?} left {dir:?}");
                    assert!(!path.is_symlink(), "{name:?} opened a symlink");
                    drop(file);
                }
                Err(error) => {
                    // A refused name is a value the caller reports, not a panic.
                    assert!(!error.to_string().is_empty());
                }
            }
        }
    }

    /// The same protection on the path a hostile name cannot be sanitised away
    /// on: an existing `.part` name this process would itself generate.
    #[test]
    fn a_guessed_partial_name_is_neither_followed_nor_reused() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("part-symlink");
        let dir = scratch.path();
        let victim = dir.join("victim");
        std::fs::write(&victim, b"private").expect("the victim file");
        let process = std::process::id();
        // The first three names `create_partial` tries, all pointed at the
        // victim, and the fourth already taken by a directory.
        for attempt in 0..3 {
            std::os::unix::fs::symlink(
                &victim,
                dir.join(format!(".SteamSetup.exe.{process}.{attempt}.part")),
            )
            .expect("plant the symlink");
        }
        std::fs::create_dir(dir.join(format!(".SteamSetup.exe.{process}.3.part")))
            .expect("take the fourth name");

        let (mut file, path) =
            create_partial(dir, "SteamSetup.exe").expect("the fifth name is free");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!(".SteamSetup.exe.{process}.4.part")
        );
        assert!(!path.is_symlink());
        std::io::Write::write_all(&mut file, b"payload").expect("write the partial");
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"private",
            "the write followed a planted symlink onto the victim"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// `test_download_is_authenticated_before_atomic_install`
    /// (`tests/test_installers.py:101-111`).
    ///
    /// The control arm for the three refusal tests below: with an approved
    /// origin, an `MZ` payload and a publisher the recipe approves, the
    /// download lands, keeps its bytes, and leaves no temporary file. Without
    /// it a `download_installer` that refused everything would pass all of them.
    #[test]
    fn an_authenticated_download_is_installed_atomically() {
        let scratch = Scratch::new("download-ok");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &record,
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);

        let target = download_installer(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap();

        assert_eq!(target, dest.join("SteamSetup.exe"));
        assert_eq!(std::fs::read(&target).unwrap(), PE_BODY);
        assert!(part_files(&dest).is_empty(), "a .part file was left behind");
        // The verifier ran, and it ran on the temporary file — so the
        // signature was checked before anything existed at the target path.
        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], "verify");
        assert_eq!(lines[lines.len() - 2], "-in");
        assert!(lines[lines.len() - 1].ends_with(".part"), "{lines:?}");
    }

    /// `test_untrusted_redirect_is_rejected_and_removed`
    /// (`tests/test_installers.py:113-123`).
    ///
    /// # The four assertions, and why each is separate
    ///
    /// The error message is the reference's. The verifier must **not** have run
    /// — checked by the absence of its record file, which is the strongest form
    /// of `verify.assert_not_called()`: a mock can only say the function was not
    /// called, while this says the program was never executed. The target must
    /// not exist. No `.part` file may remain.
    ///
    /// And the fourth, which the reference cannot make: **no body byte was
    /// delivered**. That is the property that makes the check's placement in
    /// the head callback load-bearing rather than cosmetic — moving it after
    /// the transfer would still delete the file and still pass the other three.
    #[test]
    fn an_untrusted_redirect_is_rejected_before_a_single_byte() {
        let scratch = Scratch::new("redirect");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &record,
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"MZpayload")
            .redirected_to("https://evil.example/SteamSetup.exe");

        let error = download_installer(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Steam redirected to an untrusted download origin: https://evil.example/SteamSetup.exe"
        );
        // The *variant*, not only the sentence. Both matter, and they are
        // separately losable: a refusal that returned a generic transport error
        // carrying this text would satisfy any message assertion while telling
        // a caller nothing about why the download stopped.
        assert!(
            matches!(error, InstallerError::UntrustedOrigin { ref name, ref url }
                if name == "Steam" && url == "https://evil.example/SteamSetup.exe"),
            "{error:?}"
        );
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
        assert_eq!(
            client.delivered(),
            0,
            "the body was transferred from a host the allowlist never approved"
        );
    }

    /// `test_non_pe_payload_is_rejected_before_signature_check`
    /// (`tests/test_installers.py:125-133`).
    #[test]
    fn a_payload_that_is_not_an_exe_is_refused_before_the_signature() {
        let scratch = Scratch::new("not-pe");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &record,
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"not-an-executable");

        let error = download_installer(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "Steam download is not a valid EXE file");
        assert!(!record.exists(), "the signature verifier was run anyway");
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(part_files(&dest).is_empty());
    }

    /// The MSI arm of the same check, which the reference does not have a test
    /// for: `_validate_installer_magic` branches on `kind`, and the branch that
    /// expects the OLE header is only reachable through Epic. Without this,
    /// a port that expected `MZ` for both kinds would pass every other test.
    #[test]
    fn a_payload_that_is_not_an_ole_file_is_refused_for_an_msi() {
        let scratch = Scratch::new("not-ole");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        let epic = installer_by_id("epic").unwrap();
        assert_eq!(epic.kind, Kind::Msi);
        // An `MZ` payload for an MSI recipe is the cross-wired case.
        let client = FakeResponse::at(epic.download_url, PE_BODY);

        let error = download_installer(
            epic,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Epic Games Launcher download is not a valid MSI file"
        );

        // The control for this arm: the real OLE header is accepted, so the
        // test above cannot pass by refusing every MSI.
        let mut ole = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend_from_slice(b"payload");
        let good = FakeResponse::at(epic.download_url, &ole);
        let script = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-2"),
            // The recipe's own publisher string, which is `Epic Games Inc.` —
            // no comma, and the difference matters because the check is a
            // substring test against the whole output.
            "Signature verification: ok\nSubject: /O=Epic Games Inc./CN=Epic Games Inc.",
            0,
        );
        let target = download_installer(
            epic,
            &dest,
            None,
            Duration::from_secs(60),
            &good,
            &verifier_env(&script),
        )
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), ole);
    }

    /// `test_signature_requires_approved_publisher`
    /// (`tests/test_installers.py:135-145`) and
    /// `test_signature_accepts_verified_approved_publisher` (`:147-155`).
    ///
    /// Both arms in one test because they are one mechanism: the *only*
    /// difference between them is the publisher string the verifier prints, so
    /// a test with one arm cannot tell "checks the publisher" from "rejects
    /// everything" or from "accepts everything".
    #[test]
    fn the_signature_must_name_a_publisher_the_recipe_approves() {
        let scratch = Scratch::new("publisher");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        let approved = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-approved"),
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        assert!(verify_installer_authenticity(steam, &path, &verifier_env(&approved)).is_ok());

        let impostor = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-impostor"),
            "Signature verification: ok\nSubject: /O=Impostor Corp./CN=Impostor Corp.",
            0,
        );
        let error =
            verify_installer_authenticity(steam, &path, &verifier_env(&impostor)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );

        // The substring nature of the publisher test, spelled out: a name that
        // merely *contains* an approved one is approved, and that is the
        // reference's behaviour rather than an accident of this port.
        let padded = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-padded"),
            "Signature verification: ok\nSubject: /O=NotValve Corp./CN=NotValve Corp.",
            0,
        );
        assert!(verify_installer_authenticity(steam, &path, &verifier_env(&padded)).is_ok());
    }

    /// The success line is required as well as the exit status, and the failure
    /// carries the last eight lines the tool printed.
    #[test]
    fn a_verifier_that_did_not_say_ok_is_a_failure_with_its_own_output() {
        let scratch = Scratch::new("bad-signature");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        // Exit 0 but no success line — the case a status check alone misses.
        let quiet = fake_osslsigncode(scratch.path(), &scratch.path().join("argv-quiet"), "", 0);
        let error = verify_installer_authenticity(steam, &path, &verifier_env(&quiet)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\n"
        );

        // A non-zero exit with output: the tail is what the user sees.
        let failing = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-failing"),
            "Failed to open file",
            1,
        );
        let error =
            verify_installer_authenticity(steam, &path, &verifier_env(&failing)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\nFailed to open file"
        );
    }

    /// `SEC-09`: the verifier runs with an empty environment, and the two
    /// conditions that make that safe hold.
    ///
    /// The assertion is an *observation of the child*, not of the source: a
    /// script prints its own exported environment, and the test asks whether any
    /// variable this test process carries reached it. Under the pre-fix body the
    /// answer is "most of them"; with `env_clear` it is none of them.
    ///
    /// Three names are excluded by construction rather than by luck: `sh` sets
    /// `PWD`, `SHLVL` and `OLDPWD` for itself, so they appear in the dump even
    /// when the environment it was given is empty. Asserting an empty dump
    /// instead would fail for a reason that has nothing to do with this fix.
    #[test]
    fn the_verifier_is_spawned_without_the_launcher_s_environment() {
        let scratch = Scratch::new("env-clear");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let env_record = scratch.path().join("verifier-env");
        let script = fake_osslsigncode_dumping_env(
            scratch.path(),
            &env_record,
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );

        verify_installer_authenticity(steam, &path, &verifier_env(&script)).unwrap();

        let dumped = std::fs::read_to_string(&env_record).unwrap();
        let names: Vec<&str> = dumped
            .lines()
            .filter_map(|line| line.strip_prefix("export "))
            .filter_map(|rest| rest.split('=').next())
            .collect();
        assert!(
            !names.is_empty(),
            "the fake verifier wrote no environment at all — the dump, not the \
             environment, is what failed here"
        );
        const SET_BY_THE_SHELL_ITSELF: [&str; 4] = ["PWD", "SHLVL", "OLDPWD", "_"];
        let leaked: Vec<String> = std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| !SET_BY_THE_SHELL_ITSELF.contains(&name.as_str()))
            .filter(|name| names.contains(&name.as_str()))
            .collect();
        let sample: Vec<&String> = leaked.iter().take(5).collect();
        assert!(
            leaked.is_empty(),
            "the verifier inherited {} variable(s) from the launcher, including {sample:?}",
            leaked.len()
        );

        // The two conditions that make clearing safe, each asserted where it can
        // be: the program is resolved to an absolute path before the spawn, so
        // an empty `PATH` is not a problem for reaching it — asserted here by
        // the fact that the script above ran at all, since it is only reachable
        // by its own path.
        assert!(script.is_absolute(), "{script:?}");
        // ...and everything the verifier needs is in the argv. The trust root is
        // the one input this could have got wrong, so it is checked rather than
        // argued: the pinned-root recipe passes an absolute `-CAfile`.
        let ubisoft = installer_by_id("ubisoft").unwrap();
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let argv_record = scratch.path().join("argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &argv_record,
            "Signature verification: ok\nSubject: /CN=UBISOFT ENTERTAINMENT.",
            0,
        );
        let env = FakeLaunchEnv::new()
            .with_which("osslsigncode", &script.to_string_lossy())
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        verify_installer_authenticity(ubisoft, &path, &env).unwrap();
        let recorded = std::fs::read_to_string(&argv_record).unwrap();
        assert!(recorded.contains(&root.to_string_lossy().to_string()));
    }

    /// `shutil.which("osslsigncode")` finding nothing is its own error, and it
    /// is reached before any process is spawned.

    #[test]
    fn a_missing_verifier_is_an_error_that_names_the_dependency() {
        let scratch = Scratch::new("no-verifier");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let error = verify_installer_authenticity(steam, &path, &FakeLaunchEnv::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "osslsigncode is required to verify downloaded installers"
        );
    }

    /// The pinned root is spliced in **before** `-in`, which is where the
    /// reference puts it (`command[2:2] = [...]`).
    ///
    /// Ubisoft is the one recipe with `microsoft_trust_root`, so this is also
    /// the only place the two-kinds-of-argv difference is observable. The
    /// control arm is the Steam run above: its argv has three elements and no
    /// `-CAfile`, which a test of Ubisoft alone could not distinguish from "the
    /// option is always added and Steam's override failed".
    #[test]
    fn a_recipe_that_pins_the_microsoft_root_passes_it_to_the_verifier() {
        let scratch = Scratch::new("ca-file");
        let ubisoft = installer_by_id("ubisoft").unwrap();
        assert!(ubisoft.microsoft_trust_root);
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let record = scratch.path().join("argv");
        let script = fake_osslsigncode(
            scratch.path(),
            &record,
            "Signature verification: ok\nSubject: /O=UBISOFT ENTERTAINMENT./CN=UBISOFT ENTERTAINMENT.",
            0,
        );
        let env = FakeLaunchEnv::new()
            .with_which("osslsigncode", &script.to_string_lossy())
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        let path = touch(&scratch.path().join("UbisoftConnectInstaller.exe"));

        verify_installer_authenticity(ubisoft, &path, &env).unwrap();

        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], "verify");
        assert_eq!(lines[1], "-CAfile");
        assert_eq!(lines[2], root.to_string_lossy());
        assert_eq!(lines[3], "-TSA-CAfile");
        assert_eq!(lines[4], root.to_string_lossy());
        assert_eq!(lines[5], "-in");
        assert_eq!(lines[6], path.to_string_lossy());

        // The bundled candidate resolves in a source tree: `data/` in this
        // repository carries the root, so a build of this crate verifies
        // Ubisoft without any override. That is what the third candidate
        // (`<manifest>/../../data/<name>`) is for, and this is the assertion
        // that it points where the file actually is.
        //
        // The fourth candidate (`/app/share/gamehandler/<name>`) is the
        // installed-Flatpak path and cannot be exercised here; whether the
        // packaging puts the `.pem` there is a packaging question, and it is
        // reported rather than assumed.
        let bare = FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy());
        let bundled = authenticode_root_path(&bare).unwrap();
        assert_eq!(
            bundled,
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data")
                .join(AUTHENTICODE_ROOT_NAME)
        );
        assert!(bundled.is_file(), "{bundled:?}");

        // `AuthenticodeRootUnavailable` is therefore **unreachable in this
        // tree** — every candidate list ends at a file that exists — so it has
        // no control arm here. It is kept because it is the reference's
        // behaviour for an installed tree with no `data/` beside it, and
        // because the alternative (returning `Ok` with no root) would silently
        // drop the pin for the one recipe that asked for it. Named as an
        // uncovered arm rather than left to look covered.
    }

    /// The size cap's cheap arm: a declared length over the cap is refused from
    /// the head, before the body — the same `delivered == 0` observation as the
    /// origin test, and here it is the *only* way to show the check is cheap.
    #[test]
    fn a_declared_length_over_the_cap_is_refused_before_the_body() {
        let scratch = Scratch::new("declared-cap");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY)
            .declaring(&(MAX_INSTALLER_BYTES + 1).to_string());

        let error = download_installer(
            steam,
            &dest,
            None,
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam installer exceeds the download size limit"
        );
        // The variant as well as the sentence: the head callback records
        // `TooLarge` and returns a generic error to abandon the transfer, and a
        // test that only read the message could not see the difference.
        assert!(matches!(error, InstallerError::TooLarge { ref name } if name == "Steam"));
        assert_eq!(client.delivered(), 0);
        assert!(!dest.join("SteamSetup.exe").exists());
        assert!(part_files(&dest).is_empty());
    }

    /// A server that understates its length still cannot fill the disk.
    ///
    /// **This test has no counterpart in the reference**, which covers the
    /// origin, the magic and the publisher and never the streaming cap. It is
    /// kept because `total` is checked here and `Content-Length` is checked in
    /// the head callback, and only this one is observable from a server that
    /// lies — but its cost is this port's own and is named rather than hidden:
    /// the cap is a gibibyte, so reaching the second check really does write a
    /// gibibyte to the destination before the refusal.
    ///
    /// An earlier version of this comment justified that by saying the
    /// destination is under [`std::env::temp_dir`], which is a tmpfs "in the
    /// common case". On the machine this was written on it is not — `/tmp` is
    /// ext4 there, measured — so the sentence was doing the opposite of its job:
    /// it made a gigabyte of real I/O look free to every later reader. The
    /// assertions below are what the test is for, and none of them depend on
    /// where the file lives, so the honest statement is simply that the write is
    /// real and the coverage is worth it.
    ///
    /// The client is bounded at one chunk past the cap on purpose: a cap that
    /// stopped working would otherwise make this test hang instead of fail.
    #[test]
    fn a_body_that_crosses_the_cap_is_refused_while_it_streams() {
        const CHUNK: usize = 64 * 1024 * 1024;
        struct Endless {
            chunk: Vec<u8>,
            chunk_index: std::cell::Cell<u64>,
        }
        impl crate::runners::proton::HttpClient for Endless {
            fn get(
                &self,
                _url: &str,
                _headers: &[(&str, &str)],
                _timeout: Duration,
                on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
                sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
            ) -> Result<(), RunnerError> {
                on_head(&ResponseHead {
                    // A declared length the server lies about: under the cap.
                    content_length: Some("1024".to_string()),
                    final_url: installer_by_id("steam").unwrap().download_url.to_string(),
                })?;
                loop {
                    self.chunk_index.set(self.chunk_index.get() + 1);
                    sink(&self.chunk)?;
                    // 1 GiB plus one chunk, then stop: an unbounded loop would
                    // hang rather than fail if the cap stopped working.
                    if self.chunk_index.get() > MAX_INSTALLER_BYTES / CHUNK as u64 + 1 {
                        return Ok(());
                    }
                }
            }
        }

        let scratch = Scratch::new("streamed-cap");
        let dest = scratch.path().join("downloads");
        let record = scratch.path().join("verifier-argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        let client = Endless {
            chunk: vec![0u8; CHUNK],
            chunk_index: std::cell::Cell::new(0),
        };
        let error = download_installer(
            installer_by_id("steam").unwrap(),
            &dest,
            None,
            Duration::from_secs(600),
            &client,
            &verifier_env(&script),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam installer exceeds the download size limit"
        );
        assert!(!record.exists());
        assert!(
            part_files(&dest).is_empty(),
            "the .part file was left behind"
        );
        // The loop stops within a chunk or two of the cap rather than running
        // to its own bound, which is what "refused while it streams" means.
        assert!(
            client.chunk_index.get() <= MAX_INSTALLER_BYTES / CHUNK as u64 + 2,
            "read {} chunks",
            client.chunk_index.get()
        );
    }

    /// The progress callback reports the fraction of the declared length, and
    /// finishes at exactly `1.0` even though the last chunk usually overshoots
    /// the declaration.
    #[test]
    fn progress_is_reported_as_a_fraction_and_ends_at_one() {
        let scratch = Scratch::new("progress");
        let dest = scratch.path().join("downloads");
        let script = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv"),
            "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
            0,
        );
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, PE_BODY);
        let seen = std::cell::RefCell::new(Vec::new());
        let progress = |fraction: f64| seen.borrow_mut().push(fraction);

        download_installer(
            steam,
            &dest,
            Some(&progress),
            Duration::from_secs(60),
            &client,
            &verifier_env(&script),
        )
        .unwrap();

        let seen = seen.into_inner();
        assert_eq!(seen.last(), Some(&1.0));
        assert!(
            seen.iter().all(|value| (0.0..=1.0).contains(value)),
            "{seen:?}"
        );
        assert!(seen.len() >= 2, "the body arrives in chunks: {seen:?}");

        // With no declared length there is no denominator, so the reference
        // reports nothing during the transfer and only the final `1.0` — which
        // is why that last call sits outside the `if total` guard.
        let seen = std::cell::RefCell::new(Vec::new());
        let progress = |fraction: f64| seen.borrow_mut().push(fraction);
        let client = FakeResponse::at(steam.download_url, PE_BODY).without_declared_length();
        download_installer(
            steam,
            &scratch.path().join("downloads-undeclared"),
            Some(&progress),
            Duration::from_secs(60),
            &client,
            &verifier_env(&fake_osslsigncode(
                scratch.path(),
                &scratch.path().join("argv-undeclared"),
                "Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
                0,
            )),
        )
        .unwrap();
        assert_eq!(seen.into_inner(), vec![1.0]);
    }

    /// A download that fails does not destroy an installer that is already
    /// there: the temporary file is written beside the target and the target is
    /// only ever created by a rename.
    #[test]
    fn a_refused_download_leaves_an_existing_installer_alone() {
        let scratch = Scratch::new("keep-existing");
        let dest = scratch.path().join("downloads");
        std::fs::create_dir_all(&dest).unwrap();
        let existing = dest.join("SteamSetup.exe");
        std::fs::write(&existing, b"the good copy").unwrap();
        let script = fake_osslsigncode(scratch.path(), &scratch.path().join("argv"), "", 0);
        let steam = installer_by_id("steam").unwrap();
        let client = FakeResponse::at(steam.download_url, b"not-an-executable");

        assert!(
            download_installer(
                steam,
                &dest,
                None,
                Duration::from_secs(60),
                &client,
                &verifier_env(&script),
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&existing).unwrap(), b"the good copy");
        assert!(part_files(&dest).is_empty());
    }

    // -----------------------------------------------------------------------
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
