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
//! ([`easy_install_worker`](crate::installers)) lives in the app crate, wired
//! there by T-38, and it drives this file's download and wizard halves directly:
//!
//! | Function | Production call site |
//! |---|---|
//! | [`download_installer`] | `crates/app/src/easy_install.rs`, in `easy_install_worker` |
//! | [`wait_for_installer`] | `crates/app/src/easy_install.rs`, in `easy_install_worker` |
//! | [`wait_for_prefix_idle`] | `crates/app/src/easy_install.rs`, in `easy_install_worker` |
//! | [`verify_installer_authenticity`] | `crates/core/src/installers/download.rs`, in `download_into` |
//! | [`wineserver_binary`] | `crates/core/src/installers/wizard.rs`, in `wait_for_prefix_idle` |
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

use crate::runners::RunnerError;
use crate::runners::shell::ShellError;

pub mod command;
pub mod download;
#[cfg(test)]
pub(crate) mod tests_support;
pub mod wizard;

pub use command::*;
pub use download::*;
pub use wizard::*;

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
    /// The chain verified, but the signature's own PKCS#7 blob could not be
    /// pulled out of the file (SEC-11), so there is no certificate to read a
    /// publisher out of.
    ///
    /// Its own variant rather than [`InstallerError::SignatureInvalid`],
    /// because the two say different things: `SignatureInvalid` means the
    /// verifier **rejected** the signature, and this means the signature was
    /// accepted and then could not be *read*. It is also not a missing tool —
    /// `osslsigncode` ran and failed, and `tail` says why (`No signature
    /// found`, `Failed to open file`, …).
    ///
    /// Fail-closed by construction: the publisher check needs the certificates,
    /// so a blob that cannot be extracted is a refusal and never a pass. There
    /// is deliberately no fallback to the verifier's printed output, which is
    /// the text the signer chooses (that was SEC-03, then SEC-11).
    SignatureUnreadable { name: String, tail: String },
    /// `openssl` is not on `PATH`, or is not executable.
    ///
    /// `openssl` is the **reader** the publisher check uses to get the
    /// certificate's subject, and it is a different dependency from the
    /// verifier: a host can have one without the other. It is not a new
    /// packaging requirement — the Flatpak runtime this app ships on provides
    /// it (`/usr/bin/openssl` in `org.freedesktop.Platform`, mounted from the
    /// runtime rather than the host) — but a host without it gets this named
    /// error rather than a panic or a silent pass.
    ///
    /// Reached **after** the chain gate, so a host with no `openssl` still
    /// reports a bad signature as bad: this variant is only about the
    /// certificates of a signature that already verified.
    CertificateReaderMissing,
    /// The reader ran and could not produce subjects for the signature's
    /// certificates: it exited non-zero on the blob, printed no certificates,
    /// or failed on a certificate it did print. `tail` carries that output.
    ///
    /// Fail-closed for the same reason as [`Self::SignatureUnreadable`]: an
    /// empty certificate list is refused rather than read as "no constraint".
    CertificateReadFailed { name: String, tail: String },
    /// The signature verified but names a publisher the recipe does not
    /// approve (`installers.py:595`). The comparison is a case-folded
    /// substring test, against the RFC2253 subject of each certificate the
    /// signature carries — not against the verifier's output, which the signer
    /// chooses (`SEC-03`, `SEC-11`).
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
            InstallerError::SignatureUnreadable { name, tail } => {
                write!(
                    formatter,
                    "{name}'s Authenticode signature could not be read\n{tail}"
                )
            }
            InstallerError::CertificateReaderMissing => write!(
                formatter,
                "openssl is required to read a downloaded installer's signing certificates"
            ),
            InstallerError::CertificateReadFailed { name, tail } => {
                write!(
                    formatter,
                    "{name}'s signing certificates could not be read\n{tail}"
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
