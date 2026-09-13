//! Acknowledgements for the upstream projects GameHandler is built on — a port
//! of `gamehandler/credits.py`.
//!
//! GameHandler runs no Windows game by itself. Every title it launches is
//! running on someone else's compatibility layer, someone else's Proton build,
//! and someone else's Vulkan translation layer. This module is the single
//! source of truth for crediting them: the in-app About & Credits page (P-65)
//! and the README acknowledgements section are both generated from the data
//! here, so they cannot drift apart.
//!
//! # What the oracle for this module is
//!
//! Unlike most of the port, this module had **no schema fixtures to inherit** —
//! but it did have a test file and, more usefully, a generated artefact.
//! `tests/test_credits.py` asserts things *about* the data (every required
//! project is present, every row has a role and a link), and the strongest of
//! them is `ReadmeSyncTests`: the README's acknowledgements section is written
//! by `credits.markdown()`, so the README is a byte-level record of what
//! Python actually produces.
//!
//! That is the oracle used here, and it is a stronger one than a table of
//! expected strings would be. `markdown()` in this module is compared against
//! `README.md` **read off disk at test time**: if a role, a license, an
//! em-dash or a trailing double-space is transcribed wrongly, or if Python's
//! data is later edited without the port following, the test fails naming the
//! first byte that differs. Verify with
//! `python3 -m unittest tests.test_credits` — the reference suite is green.
//!
//! # `KeyError` has no Rust equivalent here
//!
//! `credits.py` signals a bad key by raising: `section_by_id("nope")` and
//! `credit_by_name("nope")` raise `KeyError`. This crate has no exception
//! channel, and absence is the honest answer to "is there a section with this
//! id", so [`section_by_id`] and [`credit_by_name`] return `Option`. The
//! reference test asserts `assertRaises(KeyError)`; the ported assertion is
//! `assert_eq!(..., None)`, which is the same statement about the same input.

/// One upstream project, and what GameHandler actually uses it for.
///
/// Mirrors the `Credit` dataclass. Python's `license` and `authors` default to
/// `""`; Rust has no field defaults, so a credit without them names the empty
/// string explicitly at its declaration — which is also what makes `""`
/// greppable as "the reference had no value here".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credit {
    /// The project's name, as it is credited.
    pub name: &'static str,
    /// The project's homepage, always `https://` (asserted).
    pub url: &'static str,
    /// What GameHandler uses this project for.
    pub role: &'static str,
    /// An SPDX id, or `""` where the reference declines to guess.
    ///
    /// `credits.py`'s comment explains the empty cases: Proton/Wine derivative
    /// builds inherit their upstreams' terms rather than carrying one SPDX id,
    /// so they are described by role instead of being labelled with a guess.
    /// `""` is that decision, not a missing value.
    pub license: &'static str,
    /// Who maintains it, or `""` when the reference records no one.
    pub authors: &'static str,
}

impl Credit {
    /// `Name — Authors` for list rows that show attribution inline.
    ///
    /// The separator is an em-dash (U+2014) with a space either side, exactly
    /// as `Credit.label` builds it; the README oracle pins the character.
    pub fn label(&self) -> String {
        if self.authors.is_empty() {
            self.name.to_string()
        } else {
            format!("{} — {}", self.name, self.authors)
        }
    }
}

/// A titled group of credits, mirrors the `CreditSection` dataclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditSection {
    /// A stable identifier, used for lookups and asserted unique.
    pub id: &'static str,
    /// The heading the page and the README both print.
    pub title: &'static str,
    /// A sentence introducing the group.
    pub summary: &'static str,
    /// The projects in this group, in the reference's order.
    pub entries: &'static [Credit],
}

/// Every credited project, grouped. The port of `CREDIT_SECTIONS`.
///
/// Transcribed in the reference's order and wording. `credits.py`'s own comment
/// about licenses is preserved on [`Credit::license`].
pub const CREDIT_SECTIONS: &[CreditSection] = &[
    CreditSection {
        id: "layers",
        title: "Compatibility layers",
        summary: "The projects that actually run Windows games. GameHandler does not \
                  reimplement any of this — it configures it and gets out of the way.",
        entries: &[
            Credit {
                name: "Wine",
                url: "https://www.winehq.org",
                role: "The Windows compatibility layer underneath every Windows title \
                       GameHandler launches, including every Proton build. Without Wine \
                       there is no GameHandler.",
                license: "LGPL-2.1-or-later",
                authors: "WineHQ and hundreds of contributors since 1993",
            },
            Credit {
                name: "Proton",
                url: "https://github.com/ValveSoftware/Proton",
                role: "Valve's Wine distribution with gaming patches and the Steam Linux \
                       Runtime. Every Proton family GameHandler downloads is a fork of it.",
                license: "BSD-3-Clause, plus each bundled component's own license",
                authors: "Valve Software and CodeWeavers",
            },
            Credit {
                name: "DXVK",
                url: "https://github.com/doitsujin/dxvk",
                role: "Direct3D 8/9/10/11 over Vulkan — the DXVK toggle, and the runtime \
                       GameHandler bundles for raw Wine runners that ship without it.",
                license: "zlib",
                authors: "Philip Rebohle and contributors",
            },
            Credit {
                name: "VKD3D-Proton",
                url: "https://github.com/HansKristian-Work/vkd3d-proton",
                role: "Direct3D 12 over Vulkan, behind the VKD3D toggle.",
                license: "LGPL-2.1-or-later",
                authors: "Hans-Kristian Arntzen, Philip Rebohle, and contributors",
            },
            Credit {
                name: "DXVK-NVAPI",
                url: "https://github.com/jp7677/dxvk-nvapi",
                role: "NVIDIA NVAPI and DLSS support, behind the NVAPI/DLSS toggle.",
                license: "MIT",
                authors: "Jens Peters and contributors",
            },
        ],
    },
    CreditSection {
        id: "builds",
        title: "Proton and Wine builds",
        summary: "GameHandler ships none of these. It downloads the maintainers' own \
                  official release archives, from their own release pages, on request.",
        entries: &[
            Credit {
                name: "Proton-GE",
                url: "https://github.com/GloriousEggroll/proton-ge-custom",
                role: "Community Proton with codecs, protonfixes, and broad game fixes.",
                license: "",
                authors: "GloriousEggroll",
            },
            Credit {
                name: "proton-rtsp",
                url: "https://github.com/SpookySkeletons/proton-rtsp",
                role: "Proton with RTSP and in-game media playback patches.",
                license: "",
                authors: "SpookySkeletons",
            },
            Credit {
                name: "Proton-CachyOS",
                url: "https://github.com/CachyOS/proton-cachyos",
                role: "Performance-oriented Proton with additional Wayland work.",
                license: "",
                authors: "The CachyOS project",
            },
            Credit {
                name: "Proton-EM",
                url: "https://github.com/Etaash-mathamsetty/Proton",
                role: "Proton with Wine Wayland, HDR, and FSR additions.",
                license: "",
                authors: "Etaash Mathamsetty",
            },
            Credit {
                name: "Wine-Builds",
                url: "https://github.com/Kron4ek/Wine-Builds",
                role: "The standalone Wine, Wine-Staging, Staging-TkG, and Wine-Proton \
                       builds behind GameHandler's four Wine families.",
                license: "",
                authors: "Kron4ek",
            },
        ],
    },
    CreditSection {
        id: "launchers",
        title: "Launchers that shaped GameHandler",
        summary: "Prior art we studied and learned from. No code was copied from any of \
                  them; what GameHandler borrowed is their design thinking.",
        entries: &[
            Credit {
                name: "Lutris",
                url: "https://lutris.net",
                role: "The per-game runner and environment model, and the shape of the \
                       Esync/Fsync/DXVK/VKD3D/FSR compatibility toggles.",
                license: "GPL-3.0-or-later",
                authors: "Mathieu Comandon and contributors",
            },
            Credit {
                name: "Faugus Launcher",
                url: "https://github.com/Faugus/faugus-launcher",
                role: "The case for a small, focused launcher, and the idea of \
                       one-click store-launcher installs.",
                license: "MIT",
                authors: "Faugus",
            },
            Credit {
                name: "Bottles",
                url: "https://usebottles.com",
                role: "Isolated per-title prefixes and the installable-programs catalog idea.",
                license: "GPL-3.0-or-later",
                authors: "The Bottles project",
            },
            Credit {
                name: "ProtonPlus",
                url: "https://github.com/Vysp3r/ProtonPlus",
                role: "The compatibility-tool family list and the upstream release \
                       sources GameHandler fetches Proton and Wine builds from.",
                license: "GPL-3.0-or-later",
                authors: "Vysp3r and contributors",
            },
            Credit {
                name: "Heroic Games Launcher",
                url: "https://heroicgameslauncher.com",
                role: "Prior art for treating store launchers as first-class library entries.",
                license: "GPL-3.0-or-later",
                authors: "The Heroic Games Launcher team",
            },
        ],
    },
    CreditSection {
        id: "helpers",
        title: "Runtime helpers",
        summary: "Optional tools GameHandler detects and wraps launches with. They are \
                  installed from your distribution, never vendored here.",
        entries: &[
            Credit {
                name: "umu-launcher",
                url: "https://github.com/Open-Wine-Components/umu-launcher",
                role: "Runs Proton builds with the Steam Linux Runtime outside Steam — \
                       how GameHandler launches Proton runners at all.",
                license: "GPL-3.0-or-later",
                authors: "Open Wine Components",
            },
            Credit {
                name: "MangoHud",
                url: "https://github.com/flightlessmango/MangoHud",
                role: "The performance overlay behind the per-game MangoHud switch.",
                license: "MIT",
                authors: "FlightlessMango and contributors",
            },
            Credit {
                name: "Feral GameMode",
                url: "https://github.com/FeralInteractive/gamemode",
                role: "Temporary system tuning while a game runs.",
                license: "BSD-3-Clause",
                authors: "Feral Interactive",
            },
            Credit {
                name: "Gamescope",
                url: "https://github.com/ValveSoftware/gamescope",
                role: "The nested compositor behind scaling, a stable session, and HDR.",
                license: "BSD-2-Clause",
                authors: "Valve Software",
            },
            Credit {
                name: "Winetricks",
                url: "https://github.com/Winetricks/winetricks",
                role: "Installs Windows runtimes and fonts from a game's Prefix tools menu.",
                license: "LGPL-2.1-or-later",
                authors: "The Winetricks maintainers",
            },
        ],
    },
    CreditSection {
        id: "platform",
        title: "Platform",
        summary: "What GameHandler itself is built and shipped with.",
        entries: &[
            Credit {
                name: "Qt",
                url: "https://www.qt.io",
                role: "The Qt 6 application framework the whole interface runs on.",
                license: "LGPL-3.0-only",
                authors: "The Qt Company and the Qt Project",
            },
            Credit {
                name: "Kirigami",
                url: "https://develop.kde.org/frameworks/kirigami/",
                role: "KDE's QML framework behind the adaptive pages, drawer \
                       navigation, and the light and dark themes.",
                license: "LGPL-2.0-or-later",
                authors: "The KDE community",
            },
            Credit {
                name: "PySide6",
                url: "https://doc.qt.io/qtforpython-6/",
                role: "The official Python bindings that let GameHandler drive Qt.",
                license: "LGPL-3.0-only",
                authors: "The Qt Company",
            },
            Credit {
                name: "Meson and Flatpak",
                url: "https://flatpak.org",
                role: "How GameHandler is built and packaged.",
                license: "",
                authors: "The Meson and Flatpak projects",
            },
            Credit {
                name: "Steam store web API",
                url: "https://store.steampowered.com",
                role: "Public artwork and genre lookup for automatic cover art.",
                license: "",
                authors: "Valve Software",
            },
        ],
    },
];

/// The paragraph the Credits page and the README both lead with.
pub const ACKNOWLEDGEMENT: &str = "GameHandler is a front-end. It does not implement \
    Windows compatibility, Direct3D translation, or a Proton build of its own — it \
    configures the work of the projects below and stays out of their way. All of them \
    are independent; none endorse GameHandler.";

/// Why one app instead of asking people to assemble the stack themselves.
pub const WHY_ALL_IN_ONE: &[(&str, &str)] = &[
    (
        "The stack was always the hard part, not the games",
        "Running a Windows game on Linux takes a compatibility layer, a Proton or \
         Wine build, Vulkan translation for Direct3D, a clean prefix, and the right \
         environment variables. Each of those is a mature, excellent project. \
         Assembling them is the part that stops people.",
    ),
    (
        "One app instead of five",
        "Without GameHandler the usual route is ProtonPlus (or a hand-extracted \
         tarball) for runners, Lutris or Bottles for prefixes, winetricks by hand \
         for runtimes, a separate wiki tab to learn which Proton fork a game needs, \
         and a vendor installer run manually for each store launcher. GameHandler \
         does those five jobs in one window.",
    ),
    (
        "Bundling the workflow, not the projects",
        "GameHandler vendors none of the runners it offers. It downloads the \
         maintainers' own official release archives from their own release pages, \
         on request, and keeps them in your data directory. Optional helpers like \
         MangoHud and GameMode come from your distribution. Upgrading is still the \
         upstream project's business, not ours.",
    ),
    (
        "Defaults that match what the projects recommend",
        "Esync, Fsync, DXVK, VKD3D, and the anti-cheat runtimes are on by default \
         because that is what Proton does. Every one of them is a switch you can \
         turn off per game, because the projects that wrote them documented when \
         you should.",
    ),
    (
        "The credit stays visible",
        "Every runner family names its maintainer in the app, every compatibility \
         toggle names the project that implements it, and this list ships inside \
         the application rather than only in a file on a repository page.",
    ),
];

/// Every section, in the reference's order. The port of `sections()`.
pub fn sections() -> &'static [CreditSection] {
    CREDIT_SECTIONS
}

/// The section with this id, or `None`.
///
/// Python raises `KeyError`; see the module note on why this is an `Option`.
pub fn section_by_id(section_id: &str) -> Option<&'static CreditSection> {
    CREDIT_SECTIONS.iter().find(|s| s.id == section_id)
}

/// Every credit, flattened across sections, in the reference's order.
pub fn all_credits() -> Vec<&'static Credit> {
    CREDIT_SECTIONS
        .iter()
        .flat_map(|section| section.entries.iter())
        .collect()
}

/// The credit with this name, or `None`.
///
/// The comparison is case-insensitive (`credit_by_name("wine")` answers with
/// Wine), matching Python's `.lower() == .lower()` test. `to_lowercase` is used
/// rather than `eq_ignore_ascii_case` so the mapping stays Unicode-aware if a
/// future name is not ASCII, which is what Python does.
pub fn credit_by_name(name: &str) -> Option<&'static Credit> {
    let wanted = name.to_lowercase();
    all_credits()
        .into_iter()
        .find(|credit| credit.name.to_lowercase() == wanted)
}

/// One `"Name https://url"` line for an about-style listing.
///
/// A named function rather than a closure inside [`about_credit_sections`]
/// because a closure inside a builder is unreachable by an assertion — the same
/// wall that hid the dropdown's index/name bug in `view::library` (finding #57)
/// and the `Toggler` label (finding #46). Every credit in the reference data
/// has a url today, so the empty branch below is dead in Python as well; the
/// only way to keep it honest is to be able to call it with a url-less credit.
pub fn about_line(credit: &Credit) -> String {
    if credit.url.is_empty() {
        credit.name.to_string()
    } else {
        format!("{} {}", credit.name, credit.url)
    }
}

/// `(title, ["Name https://url", ...])` pairs for about-style listings.
///
/// The name and its homepage are joined into one string per project so a
/// plain-text renderer can show the link inline.
pub fn about_credit_sections() -> Vec<(String, Vec<String>)> {
    CREDIT_SECTIONS
        .iter()
        .map(|section| {
            let people = section.entries.iter().map(about_line).collect();
            (section.title.to_string(), people)
        })
        .collect()
}

/// Render the acknowledgements as the README section, so the two agree.
///
/// Transcribed line for line from `credits.markdown()`, including the two
/// trailing spaces that make each credit line a Markdown hard break and the
/// final `rstrip()` before the newline. It is checked byte-for-byte against the
/// generated `README.md` in this module's tests.
pub fn markdown() -> String {
    let mut lines: Vec<String> = vec![
        "## Thanks to the projects GameHandler stands on".to_string(),
        String::new(),
        ACKNOWLEDGEMENT.to_string(),
        String::new(),
    ];
    for section in CREDIT_SECTIONS {
        lines.push(format!("### {}", section.title));
        lines.push(String::new());
        lines.push(section.summary.to_string());
        lines.push(String::new());
        for credit in section.entries {
            let suffix = if credit.license.is_empty() {
                String::new()
            } else {
                format!(" _({})_", credit.license)
            };
            let author = if credit.authors.is_empty() {
                String::new()
            } else {
                format!(" — {}", credit.authors)
            };
            lines.push(format!(
                "- **[{name}]({url})**{author}{suffix}  ",
                name = credit.name,
                url = credit.url,
            ));
            lines.push(format!("  {}", credit.role));
        }
        lines.push(String::new());
    }
    lines.push("## Why one app instead of assembling the stack yourself".to_string());
    lines.push(String::new());
    for (heading, body) in WHY_ALL_IN_ONE {
        lines.push(format!("### {heading}"));
        lines.push(String::new());
        lines.push((*body).to_string());
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n").trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::families::RUNNER_FAMILIES;

    /// Every project whose code actually runs when a game launches must be
    /// credited. Transcribed from `tests/test_credits.py`'s `REQUIRED`.
    const REQUIRED: &[&str] = &[
        "Wine",
        "Proton",
        "DXVK",
        "VKD3D-Proton",
        "DXVK-NVAPI",
        "umu-launcher",
        "MangoHud",
        "Feral GameMode",
        "Gamescope",
        "Winetricks",
        "Lutris",
        "Faugus Launcher",
        "Bottles",
        "ProtonPlus",
        "Qt",
        "Kirigami",
        "PySide6",
    ];

    #[test]
    fn the_generated_readme_contains_this_modules_markdown_byte_for_byte() {
        // The oracle. `README.md`'s acknowledgements section is written by
        // `credits.markdown()`, so the README is a byte-level recording of what
        // the reference produces. Markdown syntax errors do not show up
        // visually until rendered, so the comparison is against the raw bytes.
        //
        // A stronger assertion than a block-wise containment check: an edit to
        // any role, license, name or punctuation in this module shifts the text
        // and the test names the first differing byte.
        let readme = crate::oracle_support::repo_file("README.md");
        let ours = markdown();
        assert!(
            readme.contains(&ours),
            "README.md does not contain this module's markdown verbatim.\n\
             Regenerate with `python3 -m gamehandler.credits`-equivalent \
             (`credits.markdown()`) or fix the transcription.\n\
             first divergence at byte {:?}",
            first_divergence(&readme, &ours)
        );
    }

    /// Where `needle` first stops matching `haystack`, for a readable failure.
    fn first_divergence(haystack: &str, needle: &str) -> String {
        if haystack.contains(needle) {
            return "none".to_string();
        }
        let (h, n) = (haystack.as_bytes(), needle.as_bytes());
        // The section heading is the anchor; from there the two must agree
        // until one runs out.
        let start = match haystack.find("## Thanks to the projects GameHandler stands on") {
            Some(index) => index,
            None => return "the acknowledgements heading is absent from README.md".to_string(),
        };
        for offset in 0..n.len() {
            match (h.get(start + offset), n.get(offset)) {
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b)) => {
                    return format!(
                        "offset {offset} in the section: README has {a:?}, this module has {b:?}"
                    );
                }
                (None, Some(b)) => {
                    return format!(
                        "README ends {offset} bytes into the section; this module has {b:?} there"
                    );
                }
                (_, None) => return format!("agrees for {offset} bytes, then extra output"),
            }
        }
        "unknown".to_string()
    }

    #[test]
    fn the_table_has_the_reference_counts() {
        // Completeness, which the README comparison alone cannot prove: a
        // credit omitted from *both* this table and the README would still
        // match. The counts are read out of the Python source rather than
        // restated as literals, so "5 sections / 25 entries" is derived from
        // the reference instead of from this test's author.
        let source = crate::oracle_support::repo_file("gamehandler/credits.py");
        let section_count = source.matches("CreditSection(").count();
        let credit_count = source.matches("Credit(").count();
        assert_eq!(
            CREDIT_SECTIONS.len(),
            section_count,
            "credits.py constructs {section_count} sections, this module has {}",
            CREDIT_SECTIONS.len()
        );
        assert_eq!(
            all_credits().len(),
            credit_count,
            "credits.py constructs {credit_count} credits, this module has {}",
            all_credits().len()
        );
    }

    #[test]
    fn every_upstream_dependency_is_credited() {
        let names: Vec<&str> = all_credits().iter().map(|credit| credit.name).collect();
        for required in REQUIRED {
            assert!(
                names.contains(required),
                "{required} runs when a game launches and is not credited; known: {names:?}"
            );
        }
    }

    #[test]
    fn every_credit_carries_a_role_and_a_link() {
        for credit in all_credits() {
            assert!(
                !credit.role.trim().is_empty(),
                "{} has no role; every credit explains what it is used for",
                credit.name
            );
            assert!(
                credit.url.starts_with("https://"),
                "{} has url {:?}, which is not https",
                credit.name,
                credit.url
            );
        }
    }

    #[test]
    fn section_ids_are_unique_and_populated() {
        let mut ids: Vec<&str> = CREDIT_SECTIONS.iter().map(|s| s.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "two sections share an id");
        for section in CREDIT_SECTIONS {
            assert!(!section.entries.is_empty(), "{} has no entries", section.id);
            assert!(
                !section.title.trim().is_empty(),
                "{} has no title",
                section.id
            );
            assert!(
                !section.summary.trim().is_empty(),
                "{} has no summary",
                section.id
            );
        }
    }

    #[test]
    fn the_lookup_helpers_answer_and_refuse() {
        assert_eq!(
            section_by_id("layers").unwrap().title,
            "Compatibility layers"
        );
        assert_eq!(credit_by_name("wine").unwrap().name, "Wine");
        // `KeyError` in Python; `None` here. The reference test is
        // `assertRaises(KeyError)` for both.
        assert!(section_by_id("nope").is_none());
        assert!(credit_by_name("nope").is_none());
    }

    #[test]
    fn the_lookup_is_case_insensitive() {
        // `credit_by_name("wine")` answering is the reference's own assertion,
        // so a case-sensitive port would pass `the_lookup_helpers_answer_and_refuse`
        // only by accident of the spelling used there. This pins the comparison
        // itself, in both directions.
        assert_eq!(credit_by_name("WINE").unwrap().name, "Wine");
        assert_eq!(credit_by_name("dxvk-nvapi").unwrap().name, "DXVK-NVAPI");
        assert_eq!(credit_by_name("proton-ge").unwrap().name, "Proton-GE");
    }

    #[test]
    fn a_label_names_the_authors_only_when_they_are_known() {
        assert_eq!(
            credit_by_name("Proton-GE").unwrap().label(),
            "Proton-GE — GloriousEggroll"
        );
        assert!(credit_by_name("DXVK").unwrap().label().contains('—'));

        // `Credit.label`'s empty-authors branch. It is **unreachable with the
        // reference's data** — every one of the 25 credits names its authors —
        // so it is pinned with a synthetic credit and the unreachability is
        // asserted rather than assumed. A port that always formatted would
        // leave a dangling em-dash the moment a credit lost its authors.
        let nameless_authors = Credit {
            name: "Untitled",
            url: "https://example.invalid",
            role: "r",
            license: "",
            authors: "",
        };
        assert_eq!(nameless_authors.label(), "Untitled");
        assert!(!nameless_authors.label().contains('—'));

        for credit in all_credits() {
            assert!(
                !credit.authors.is_empty(),
                "{} has no authors, so the empty-authors arm of label() is now live",
                credit.name
            );
        }
    }

    #[test]
    fn a_licence_is_shown_only_where_the_reference_records_one() {
        // The suffix is ` _(license)_`, and `""` means the reference declined to
        // guess rather than "unknown". Both branches are exercised so the
        // empty-string case cannot silently render `_()_`.
        let with = credit_by_name("Wine").unwrap();
        assert!(markdown().contains(&format!(" _({})_", with.license)));
        assert!(markdown().contains("_(LGPL-2.1-or-later)_"));

        let without = credit_by_name("Proton-GE").unwrap();
        assert_eq!(without.license, "");
        assert!(
            markdown().contains("**[Proton-GE](https://github.com/GloriousEggroll/proton-ge-custom)** — GloriousEggroll  \n"),
            "a credit with no licence must carry no empty `_()_` suffix"
        );
    }

    #[test]
    fn every_runner_family_maintainer_is_acknowledged() {
        // The reference ties the two modules together this way: if a runner
        // family names a maintainer, the credits must name them too. It is a
        // genuine cross-module invariant, and it can fail — adding a family
        // whose maintainer is not credited turns it red.
        let text = markdown();
        for family in RUNNER_FAMILIES {
            assert!(
                !family.maintainer.is_empty(),
                "{} names no maintainer",
                family.id
            );
            assert!(
                text.contains(family.maintainer),
                "{} is maintained by {:?}, who is not acknowledged in the credits",
                family.id,
                family.maintainer
            );
        }
    }

    #[test]
    fn about_sections_pair_names_with_urls() {
        let rendered = about_credit_sections();
        assert_eq!(rendered.len(), CREDIT_SECTIONS.len());
        let (first_title, people) = &rendered[0];
        assert_eq!(first_title, CREDIT_SECTIONS[0].title);
        assert!(people.contains(&"Wine https://www.winehq.org".to_string()));
        // Every section is represented, and no pair is empty.
        for (title, people) in &rendered {
            assert!(!title.is_empty());
            assert!(!people.is_empty(), "{title} rendered no projects");
        }
    }

    #[test]
    fn a_credit_with_no_url_renders_as_its_name_alone() {
        // `about_line`'s empty-url arm, which the reference data never reaches:
        // every credit today has a url, so `test_credits.py`'s
        // `assertTrue(credit.url.startswith("https://"))` holds and Python's
        // matching branch is dead there too. It is kept because the reference
        // keeps it, and it is pinned here so that a future url-less credit
        // renders `Name` rather than `Name ` with a trailing space.
        let url_less = Credit {
            name: "Untitled",
            url: "",
            role: "r",
            license: "",
            authors: "",
        };
        assert_eq!(about_line(&url_less), "Untitled");

        // The live arm, so the assertion above is known not to be the only
        // behaviour this function has.
        let linked = Credit {
            name: "Wine",
            url: "https://www.winehq.org",
            role: "r",
            license: "",
            authors: "",
        };
        assert_eq!(about_line(&linked), "Wine https://www.winehq.org");

        // Records *why* the arm above is unreachable today, so that a credit
        // gaining or losing a url is a deliberate event rather than a surprise.
        for credit in all_credits() {
            assert!(
                !credit.url.is_empty(),
                "{} has no url, so the empty-url arm of about_line is now live",
                credit.name
            );
        }
    }

    #[test]
    fn the_rationale_entries_are_headed_paragraphs() {
        assert!(WHY_ALL_IN_ONE.len() >= 3);
        for (heading, body) in WHY_ALL_IN_ONE {
            assert!(!heading.trim().is_empty());
            assert!(
                body.split_whitespace().count() > 15,
                "{heading:?} has a body too short to be an argument: {body:?}"
            );
        }
    }

    #[test]
    fn the_rationale_is_in_the_readme_too() {
        // `test_credits.py::test_readme_explains_why_it_is_one_app`.
        let readme = crate::oracle_support::repo_file("README.md");
        assert!(readme.contains("Why one app instead of assembling the stack yourself"));
        for (heading, _) in WHY_ALL_IN_ONE {
            assert!(
                readme.contains(heading),
                "{heading:?} is in the port but not in the generated README"
            );
        }
    }
}
