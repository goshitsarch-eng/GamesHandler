//! The first-party easy-installer catalog — `installers.py:1-255`.
//!
//! A port of `gamehandler/installers.py` (702 lines, 20 public names). This
//! file currently carries the **catalog half**: [`LAUNCHERS`], [`APPS`],
//! [`INSTALLER_CATEGORIES`], [`Installer`], [`INSTALLERS`], [`installers`],
//! [`installer_by_id`] and [`search_installers`]. The rest of the module is
//! named in "What has not landed yet" below, so nothing here reads as more
//! complete than it is.
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
//! # What has not landed yet, named so no caller assumes it
//!
//! The remaining three quarters of `installers.py`, in the order the reference
//! defines them. T-04's plan row calls this task "wizard/poll state machine
//! with injectable clock" and that is [`wait_for_installer`], which is in the
//! second group — so **this file landing is not T-04 landing**:
//!
//! * **The command half** — `installer_argv` (`:258-267`),
//!   `build_installer_command` (`:270-291`), `wineserver_binary` (`:302-309`),
//!   `wait_for_prefix_idle` (`:312-342`), `wait_for_installer` (`:360-408`)
//!   with its injected `sleep`/`clock`, `safe_download_name` (`:411-416`),
//!   `resolve_case_insensitive` (`:426-442`), `find_prefix_exe` (`:514-530`)
//!   and the bounded fallback scan (`:452-511`).
//! * **The download half** — `download_installer` (`:598-646`) and
//!   `verify_installer_authenticity` (`:569-595`), which need the D-26
//!   `HttpClient` seam to report a *final* URL and a subprocess seam for
//!   `osslsigncode`.
//! * **The game half** — `prepare_prefix` (`:649-652`) and `game_from_install`
//!   (`:655-676`).
//!
//! `installers()` and `search_installers` are what the page needs
//! (`view/installers.rs` takes the filtered rows through `InstallersView` and
//! says so in its own module doc); everything above is what the easy-install
//! *worker* needs, which is T-12's second half.

use std::fmt;

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

/// The catalog's errors.
///
/// One variant today. The download and verification half of the module adds
/// the rest (`installers.py` raises a `RuntimeError` for each of them); they
/// are named in the module doc's "What has not landed yet".
#[derive(Debug)]
pub enum InstallerError {
    /// `KeyError(f"Unknown installer: {installer_id}")` (`installers.py:238`).
    UnknownInstaller { id: String },
}

impl fmt::Display for InstallerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallerError::UnknownInstaller { id } => {
                write!(formatter, "Unknown installer: {id}")
            }
        }
    }
}

impl std::error::Error for InstallerError {}

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
        let expected: Vec<&str> = all
            .into_iter()
            .filter(|id| *id != "discord")
            .collect();
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
        assert_eq!(installer_by_id("discord").unwrap().library_category, "Utility");
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
