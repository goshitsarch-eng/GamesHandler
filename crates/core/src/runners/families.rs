//! The runner catalogue: which builds exist, which release asset belongs to
//! which, and how an install directory is named.
//!
//! A port of `runners.py:68-342`. The catalogue is data, the matching is pure
//! string work, and nothing here touches the filesystem except
//! [`find_wine_binary`] — so it is the cheapest part of the module to test and
//! the part that pins the most user-visible behaviour. A wrong `exclude` token
//! does not crash: it silently offers someone a `wow64` build that cannot run
//! their game.
//!
//! # Why the asset matching is worth this much care
//!
//! GitHub releases carry many archives per release, and the differences are in
//! the *name* — `wow64`, `x86_64`, `v3`, `znver4`, `slr`. Getting the tokens
//! wrong picks a build that installs cleanly and then fails on launch, which is
//! the most expensive kind of bug to diagnose. `asset_matches` is therefore
//! ported by transcription from `runners.py` rather than by reimplementation,
//! and `every_family_matches_its_own_real_asset_name` pins one real asset name
//! per family.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Layouts used by Proton tarballs and Kron4ek Wine-Builds. `runners.py:60`.
const WINE_CANDIDATES: [&[&str]; 3] = [
    &["files", "bin", "wine"],
    &["dist", "bin", "wine"],
    &["bin", "wine"],
];

/// The system Wine pseudo-runner's id, as `config.py` writes it.
pub const SYSTEM_WINE: &str = "wine-system";

/// Locate the Wine binary inside an extracted Proton or Wine build.
/// `runners.py:303`.
///
/// Lives here rather than in `archive` because the search order *is* the
/// family's layout knowledge; `archive::validate_staged_runner` imports it.
pub fn find_wine_binary(root: &std::path::Path) -> Option<std::path::PathBuf> {
    for parts in WINE_CANDIDATES {
        let candidate = root.join(parts.iter().collect::<std::path::PathBuf>());
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// A downloadable Proton or Wine family, matching ProtonPlus coverage.
/// `runners.py:68`.
///
/// Deliberately not `Serialize`/`Deserialize`: the catalogue is a compile-time
/// constant, never read from or written to a file — Python's is a frozen
/// dataclass with the same property. `ReleaseInfo` is the serialised one, and
/// it carries the `family_id` rather than the family for exactly this reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerFamily {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub github: &'static str,
    /// `proton` or `wine`.
    pub kind: &'static str,
    pub maintainer: &'static str,
    /// Every token must appear in the asset name for it to match.
    pub require: &'static [&'static str],
    /// Any token appearing disqualifies the asset.
    pub exclude: &'static [&'static str],
    /// Narrowing tokens, applied only if any asset satisfies them.
    pub prefer: &'static [&'static str],
    pub when_to_use: &'static str,
}

impl RunnerFamily {
    /// `https://api.github.com/repos/{github}/releases`.
    pub fn releases_url(&self) -> String {
        format!("https://api.github.com/repos/{}/releases", self.github)
    }

    /// `https://github.com/{github}`.
    pub fn homepage(&self) -> String {
        format!("https://github.com/{}", self.github)
    }
}

/// Families ProtonPlus exposes that ship public tarball releases we can install
/// into a standalone launcher (wrappers like Luxtorpeda/Boxtron are Steam-only).
/// `runners.py:90`.
///
/// Order is load-bearing: it is the order the Runners page lists them in, and
/// `pick_asset`'s first-match rule is only meaningful because of it.
pub const RUNNER_FAMILIES: &[RunnerFamily] = &[
    RunnerFamily {
        id: "proton-ge",
        name: "Proton-GE",
        description: "GloriousEggroll's community Proton with codecs and game fixes.",
        github: "GloriousEggroll/proton-ge-custom",
        kind: "proton",
        maintainer: "GloriousEggroll",
        require: &[],
        exclude: &[],
        prefer: &[],
        when_to_use: "Start here for most Windows games. GE includes codecs, protonfixes, \
                      and the broadest out-of-the-box compatibility.",
    },
    RunnerFamily {
        id: "proton-ge-rtsp",
        name: "Proton-GE RTSP",
        description: "Proton build with RTSP and media playback patches (VRChat).",
        github: "SpookySkeletons/proton-rtsp",
        kind: "proton",
        maintainer: "SpookySkeletons",
        require: &[],
        exclude: &[],
        prefer: &[],
        when_to_use: "Use for VRChat or titles that play in-game video over RTSP. \
                      Not a general replacement for Proton-GE.",
    },
    RunnerFamily {
        id: "proton-cachyos",
        name: "Proton-CachyOS",
        description: "CachyOS Proton with extra performance and Wayland work.",
        github: "CachyOS/proton-cachyos",
        kind: "proton",
        maintainer: "The CachyOS project",
        require: &[],
        exclude: &["v3", "znver4", "native"],
        prefer: &["slr", "x86_64"],
        when_to_use: "A performance-oriented Proton. Try it when a game already runs \
                      but you want a bit more speed on recent hardware.",
    },
    RunnerFamily {
        id: "proton-em",
        name: "Proton-EM",
        description: "Etaash Proton with Wine Wayland, HDR, and FSR additions.",
        github: "Etaash-mathamsetty/Proton",
        kind: "proton",
        maintainer: "Etaash Mathamsetty",
        require: &[],
        exclude: &[],
        prefer: &[],
        when_to_use: "Pick this for native Wine Wayland, HDR, or FSR extras. Needs a \
                      recent GPU stack; keep Proton-GE as the fallback.",
    },
    RunnerFamily {
        id: "wine-vanilla",
        name: "Wine-Vanilla",
        description: "Kron4ek upstream Wine, without staging patches.",
        github: "Kron4ek/Wine-Builds",
        kind: "wine",
        maintainer: "Kron4ek",
        require: &["amd64"],
        exclude: &["staging", "tkg", "proton", "wow64"],
        prefer: &[],
        when_to_use: "Use for older games or Windows apps that behave better on plain \
                      Wine than on Proton. No Steam runtime bundled.",
    },
    RunnerFamily {
        id: "wine-staging",
        name: "Wine-Staging",
        description: "Kron4ek Wine with the Staging patchset.",
        github: "Kron4ek/Wine-Builds",
        kind: "wine",
        maintainer: "Kron4ek",
        require: &["staging", "amd64"],
        exclude: &["tkg", "wow64"],
        prefer: &[],
        when_to_use: "Newer Wine features that have not landed upstream yet. Try this \
                      when Vanilla is too old for a specific title.",
    },
    RunnerFamily {
        id: "wine-staging-tkg",
        name: "Wine-Staging-Tkg",
        description: "Kron4ek Wine Staging plus TkG gaming patches.",
        github: "Kron4ek/Wine-Builds",
        kind: "wine",
        maintainer: "Kron4ek",
        require: &["staging-tkg", "amd64"],
        exclude: &["wow64"],
        prefer: &[],
        when_to_use: "Wine with extra gaming patches, without a full Proton tree. \
                      Good when you want Wine, not Steam's Proton layout.",
    },
    RunnerFamily {
        id: "wine-proton",
        name: "Wine-Proton",
        description: "Kron4ek Wine built from Proton's Wine tree.",
        github: "Kron4ek/Wine-Builds",
        kind: "wine",
        maintainer: "Kron4ek",
        require: &["proton", "amd64"],
        exclude: &["staging", "wow64"],
        prefer: &[],
        when_to_use: "Proton's Wine packaged as standalone Wine. A middle ground if \
                      full Proton is heavier than you need.",
    },
];

/// Advice for the system Wine entry, kept as a constant because two callers
/// need it and they must not drift. `runners.py:202`.
pub const SYSTEM_WINE_GUIDE: &str = "The Wine already installed on this system. Fine for simple apps and \
     older games. For modern titles, download Proton-GE from the list below.";

/// Look a family up by id. `runners.py:210`.
///
/// `Err` carries the id, because the caller is a UI path that has to say which
/// family was unknown — and because Python's `KeyError` message is
/// `Unknown runner family: {id}`, which a port that returned `None` would lose.
pub fn family_by_id(family_id: &str) -> Result<&'static RunnerFamily, String> {
    RUNNER_FAMILIES
        .iter()
        .find(|family| family.id == family_id)
        .ok_or_else(|| format!("Unknown runner family: {family_id}"))
}

/// The catalogue, in listing order. `runners.py:217`.
pub fn families() -> &'static [RunnerFamily] {
    RUNNER_FAMILIES
}

/// A "which runner should I use?" row, with credit to its maintainer.
/// `runners.py:222`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerGuide {
    pub title: String,
    pub kind: String,
    pub advice: String,
    pub maintainer: String,
    pub homepage: String,
}

/// Guide rows including who maintains each build and where it lives.
/// `runners.py:231`.
///
/// The system Wine row is first and is not a family — it has no GitHub repo and
/// no downloadable release, so it cannot be derived from the catalogue.
pub fn runner_guide_details() -> Vec<RunnerGuide> {
    let mut rows = vec![RunnerGuide {
        title: "System Wine".to_string(),
        kind: "wine".to_string(),
        advice: SYSTEM_WINE_GUIDE.to_string(),
        maintainer: "WineHQ".to_string(),
        homepage: "https://www.winehq.org".to_string(),
    }];
    rows.extend(RUNNER_FAMILIES.iter().map(|family| RunnerGuide {
        title: family.name.to_string(),
        kind: family.kind.to_string(),
        advice: family.when_to_use.to_string(),
        maintainer: family.maintainer.to_string(),
        homepage: family.homepage(),
    }));
    rows
}

/// `(title, kind, advice)` rows for the Runners guide. `runners.py:252`.
pub fn runner_guides() -> Vec<(String, String, String)> {
    runner_guide_details()
        .into_iter()
        .map(|row| (row.title, row.kind, row.advice))
        .collect()
}

/// Whether a name looks like one of the archive formats we can extract.
/// `runners.py:260`.
pub fn looks_like_archive(name: &str) -> bool {
    let lowered = name.to_lowercase();
    [".tar.gz", ".tar.xz", ".tgz", ".tar.bz2"]
        .iter()
        .any(|suffix| lowered.ends_with(suffix))
}

/// Return whether a GitHub asset belongs to `family`. `runners.py:266`.
///
/// Exclude is checked before require, matching Python: an asset that is both
/// `wow64` and `amd64` is rejected, not accepted. Reversing the two would offer
/// 32-bit-only builds on a 64-bit host.
pub fn asset_matches(asset_name: &str, family: &RunnerFamily) -> bool {
    asset_matches_tokens(asset_name, family.require, family.exclude)
}

/// [`asset_matches`] over the token lists themselves rather than a family.
///
/// The narrower signature exists for two reasons. It is honest about what the
/// rule actually reads — a family contributes only its `require` and `exclude`
/// lists, and nothing else about it is consulted — and it makes the rule
/// reachable from a test or an oracle vector without inventing a
/// [`RunnerFamily`], whose fields are `&'static` because the catalogue is a
/// compile-time constant. The Python vector script can build a synthetic family
/// because Python dataclasses have no such constraint; the port cannot, and
/// fabricating a `'static` leak to make a test read nicely would be worse than
/// widening this one signature.
///
/// Exclude is checked before require, matching Python: an asset that is both
/// `wow64` and `amd64` is rejected, not accepted. Reversing the two would offer
/// 32-bit-only builds on a 64-bit host.
pub fn asset_matches_tokens(asset_name: &str, require: &[&str], exclude: &[&str]) -> bool {
    if !looks_like_archive(asset_name) {
        return false;
    }
    let lowered = asset_name.to_lowercase();
    for token in exclude {
        if lowered.contains(&token.to_lowercase()) {
            return false;
        }
    }
    for token in require {
        if !lowered.contains(&token.to_lowercase()) {
            return false;
        }
    }
    true
}

/// Choose the best archive asset for `family` from a GitHub release.
/// `runners.py:280`.
///
/// Takes `serde_json::Value`s rather than a typed asset struct because the
/// caller has the raw release JSON and nothing else needs the extra fields —
/// the same shape as the Python, which takes `dict`s and reads only `name`.
pub fn pick_asset(assets: &[Value], family: &RunnerFamily) -> Option<Value> {
    pick_asset_tokens(assets, family.require, family.exclude, family.prefer)
}

/// [`pick_asset`] over the token lists themselves. See [`asset_matches_tokens`]
/// for why the narrower form exists.
pub fn pick_asset_tokens(
    assets: &[Value],
    require: &[&str],
    exclude: &[&str],
    prefer: &[&str],
) -> Option<Value> {
    let mut matches: Vec<&Value> = assets
        .iter()
        .filter(|asset| asset_matches_tokens(asset_name(asset).as_str(), require, exclude))
        .collect();
    if matches.is_empty() {
        return None;
    }
    if !prefer.is_empty() {
        let preferred: Vec<&Value> = matches
            .iter()
            .copied()
            .filter(|asset| {
                let name = asset_name(asset).to_lowercase();
                prefer
                    .iter()
                    .all(|token| name.contains(&token.to_lowercase()))
            })
            .collect();
        // Only narrow when at least one asset satisfies every `prefer` token;
        // otherwise the preference is advisory and the first match still wins.
        if !preferred.is_empty() {
            matches = preferred;
        }
    }
    matches.first().map(|asset| (*asset).clone())
}

/// `str(asset.get("name") or "")`. `runners.py:283`.
///
/// Two Python rules are folded into this one expression, and neither is
/// optional:
///
/// * `or ""` is a *truthiness* test, not a null test. `0`, `0.0`, `False`, `""`,
///   `[]` and `{}` are all falsy, so all six read as the empty string. A first
///   draft of this port checked only for `null` and returned `"0"` for
///   `{"name": 0}` and `"False"` for `{"name": false}` — caught by the vectors
///   in `run_runners_vectors.py`, which is the point of having them.
/// * `str(value)` on a truthy non-string is the scalar's rendering — `True`,
///   `42`, `0.5` — and for a container it is `repr`, so `[1, "a"]` becomes
///   `[1, 'a']` with single quotes.
///
/// The container branch is not defensive padding: it is the branch where a port
/// reasons "GitHub would never send that" and is then wrong in the same way F-I
/// and F-J were wrong. It is implemented, and
/// `asset_names_are_read_exactly_as_python_reads_them` pins it for arrays.
pub fn asset_name(asset: &Value) -> String {
    match asset.get("name") {
        None => String::new(),
        Some(value) if !is_truthy(value) => String::new(),
        Some(value) => python_str(value),
    }
}

/// Python truthiness for a JSON value.
///
/// `Number` is the only interesting case, and it needs care in both of its
/// storage forms: serde_json keeps an integer as `i64`/`u64` and anything else
/// as an `f64`, so `0` and `0.0` are different variants that are both falsy.
/// `-0.0` is falsy too, which `as_f64() == 0.0` gets right and a sign check
/// would not.
pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => {
            number.as_f64().is_some_and(|float| float != 0.0)
                || number.as_i64().is_some_and(|int| int != 0)
                || number.as_u64().is_some_and(|int| int != 0)
        }
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// Python's `str()` for a JSON value.
///
/// Divergences, both already recorded rather than discovered again here:
///
/// * A float is rendered by `serde_json`'s shortest-roundtrip formatter, so the
///   mantissa is exact but an exponent is spelled without Python's leading zero
///   — `1e-7` where Python writes `1e-07`. That is F-F, and the same accepted
///   divergence the JSON writer has.
/// * An object's keys come out in `serde_json::Map`'s order, which is insertion
///   order only when the `preserve_order` feature is on. It is on workspace-wide
///   (via `cosmic-theme`) and off under `cargo test -p gamehandler-core`, so the
///   object branch is deliberately *not* pinned by a test: a test whose result
///   depends on who else is in the build is worse than no test. The array branch
///   has no such dependency and is pinned.
///
/// Everything else — null, booleans, integers that fit `i64`/`u64`, strings,
/// and arrays — is exact.
pub(crate) fn python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(python_repr).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(map) => {
            let rendered: Vec<String> = map
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}: {}",
                        python_repr(&Value::String(key.clone())),
                        python_repr(item)
                    )
                })
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

/// Python's `repr()`, which differs from `str()` for exactly one type: a string
/// is quoted, with single quotes unless that would need escaping.
/// Python's `repr()` for a `str`, as far as the errors in this crate need it.
///
/// Python's `f"Unsafe runner id: {value!r}"` puts the value through `repr`, and
/// `repr` of a string is **not** Rust's `{:?}`: it prefers single quotes, and
/// switches to double quotes only when the string contains a `'` and no `"`.
/// So `repr("")` is `''` where `{:?}` gives `""`, and a message carrying the
/// wrong quote character is a parity defect the moment anyone compares the two
/// implementations' output — which the vectors in
/// `docs/migration/oracle/` do.
///
/// # The approximation, stated rather than hidden
///
/// Python decides "printable" with `unicodedata.category`, which needs the
/// Unicode character database: it escapes unassigned code points, format
/// characters, surrogates and private-use characters, which this does not. What
/// this does cover is every control character — the reachable class for a
/// release tag or an install id — plus the quote and escape rules, which are
/// the two things a naive port gets wrong.
///
/// Nothing here is reachable from a well-formed tag; the vectors pin it because
/// "nothing here is reachable" is the claim that has failed before.
pub(crate) fn python_str_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            // C0 and C1 controls, and DEL: Python never prints these raw.
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                let code = c as u32;
                if code <= 0xff {
                    out.push_str(&format!("\\x{code:02x}"));
                } else {
                    out.push_str(&format!("\\u{code:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

pub(crate) fn python_repr(value: &Value) -> String {
    match value {
        Value::String(text) => {
            if text.contains('\'') && !text.contains('"') {
                format!("\"{text}\"")
            } else {
                format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
            }
        }
        other => python_str(other),
    }
}

/// A downloadable Proton or Wine build published on GitHub. `runners.py:313`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub download_url: String,
    pub size: i64,
    pub family_id: String,
}

impl ReleaseInfo {
    /// The default family is `proton-ge`, matching the Python dataclass field
    /// default — a release with no family is a Proton-GE release, never an
    /// unknown one.
    pub fn new(
        tag: impl Into<String>,
        name: impl Into<String>,
        download_url: impl Into<String>,
        size: i64,
    ) -> Self {
        Self {
            tag: tag.into(),
            name: name.into(),
            download_url: download_url.into(),
            size,
            family_id: "proton-ge".to_string(),
        }
    }

    pub fn size_mb(&self) -> f64 {
        self.size as f64 / (1024.0 * 1024.0)
    }

    /// Stable directory name used under the runners folder. `runners.py:326`.
    ///
    /// Three cases, and the last two are the reason this is not a one-liner:
    ///
    /// * `proton-ge` keeps the bare tag, so existing installs keep their
    ///   familiar directory names and a Proton-GE install is never orphaned by
    ///   a change here.
    /// * Any other family is *bound* to its tag with `~f`, because two families
    ///   publish builds with the same tag (`v1.0`) and unbound they would
    ///   collide in one directory — installing Wine-Staging would overwrite
    ///   Wine-Proton. `~` is outside the accepted raw-tag alphabet, so a bound
    ///   id can never collide with an unbound one.
    /// * If the bound name is over 180 characters it is truncated and bound
    ///   again with `~i{hash}` over the *full* family id and tag, so two long
    ///   tags that share a 166-character prefix still get distinct directories.
    pub fn install_id(&self) -> Result<String, super::archive::ArchiveError> {
        install_id_for(&self.tag, &self.family_id)
    }

    /// The family this release belongs to. `runners.py:343`.
    pub fn family(&self) -> Result<&'static RunnerFamily, String> {
        family_by_id(&self.family_id)
    }
}

/// [`ReleaseInfo::install_id`] over its two inputs alone.
///
/// Free-standing because `install_id` reads nothing else off the release — not
/// the name, not the URL, not the size — and the oracle corpus exercises it
/// against family ids the catalogue does not contain, which no `ReleaseInfo`
/// constructor will build. See `asset_matches_tokens` for the same reasoning.
pub fn install_id_for(tag: &str, family_id: &str) -> Result<String, super::archive::ArchiveError> {
    use super::archive::{safe_install_id, sanitise_release_tag};

    let safe_tag = sanitise_release_tag(tag)?;
    if family_id == "proton-ge" {
        return Ok(safe_tag);
    }
    let safe_family = sanitise_release_tag(family_id)?;
    let combined = format!("{safe_family}~f{safe_tag}");
    if combined.len() <= 180 {
        return safe_install_id(&combined);
    }
    let identity = format!("{}:{}{}", family_id.len(), family_id, tag);
    let digest = &crate::hash::sha256_hex(identity.as_bytes())[..12];
    let head: String = combined.chars().take(166).collect();
    safe_install_id(&format!("{head}~i{digest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_catalogue_is_well_formed() {
        for family in RUNNER_FAMILIES {
            assert!(!family.id.is_empty(), "a family has no id");
            assert!(
                family.kind == "proton" || family.kind == "wine",
                "{} has an unknown kind {:?}",
                family.id,
                family.kind
            );
            assert!(
                !family.when_to_use.is_empty(),
                "{} has no advice",
                family.id
            );
            assert!(
                !family.github.is_empty(),
                "{} has no GitHub repo, so no releases URL",
                family.id
            );
            assert_eq!(
                family.releases_url(),
                format!("https://api.github.com/repos/{}/releases", family.github)
            );
            assert_eq!(
                family.homepage(),
                format!("https://github.com/{}", family.github)
            );
        }
    }

    #[test]
    fn family_ids_are_unique_and_lookups_round_trip() {
        // A duplicate id would make `family_by_id` return whichever came first
        // and silently orphan the other family's installs.
        for (index, family) in RUNNER_FAMILIES.iter().enumerate() {
            assert_eq!(
                RUNNER_FAMILIES
                    .iter()
                    .filter(|other| other.id == family.id)
                    .count(),
                1,
                "duplicate family id {:?} at index {index}",
                family.id
            );
            assert_eq!(family_by_id(family.id).unwrap().id, family.id);
        }
    }

    #[test]
    fn an_unknown_family_is_an_error_naming_the_id() {
        let error = family_by_id("proton-nope").unwrap_err();
        assert_eq!(error, "Unknown runner family: proton-nope");
    }

    #[test]
    fn every_family_matches_its_own_real_asset_name() {
        // One real asset name per family, so a wrong token is caught here
        // rather than by someone installing a build that will not run. The
        // negative cases are the point: `wow64`, `v3` and `znver4` are the
        // tokens that silently select a wrong build.
        let cases: &[(&str, &str)] = &[
            ("proton-ge", "GE-Proton9-5.tar.gz"),
            ("proton-ge-rtsp", "GE-Proton-RTSP-9-5.tar.gz"),
            (
                "proton-cachyos",
                "proton-cachyos-9.0-20250101-slr-x86_64.tar.xz",
            ),
            ("proton-em", "Proton-EM-9.0-1.tar.xz"),
            ("wine-vanilla", "wine-9.0-amd64.tar.xz"),
            ("wine-staging", "wine-9.0-staging-amd64.tar.xz"),
            ("wine-staging-tkg", "wine-9.0-staging-tkg-amd64.tar.xz"),
            ("wine-proton", "wine-9.0-proton-amd64.tar.xz"),
        ];
        for (id, asset) in cases {
            let family = family_by_id(id).unwrap();
            assert!(
                asset_matches(asset, family),
                "{asset:?} should match {id} (require={:?} exclude={:?})",
                family.require,
                family.exclude
            );
        }

        let cachyos = family_by_id("proton-cachyos").unwrap();
        for rejected in [
            "proton-cachyos-9.0-v3-slr-x86_64.tar.xz",
            "proton-cachyos-9.0-znver4-slr-x86_64.tar.xz",
            "proton-cachyos-9.0-native-slr-x86_64.tar.xz",
        ] {
            assert!(
                !asset_matches(rejected, cachyos),
                "{rejected:?} is a CPU-specific build and must not match"
            );
        }

        // The kron4ek families are distinguished only by tokens that are
        // substrings of each other, which is where transcription errors live.
        let vanilla = family_by_id("wine-vanilla").unwrap();
        let staging = family_by_id("wine-staging").unwrap();
        let tkg = family_by_id("wine-staging-tkg").unwrap();
        assert!(!asset_matches("wine-9.0-staging-amd64.tar.xz", vanilla));
        assert!(!asset_matches("wine-9.0-staging-tkg-amd64.tar.xz", vanilla));
        assert!(!asset_matches("wine-9.0-proton-amd64.tar.xz", vanilla));
        assert!(!asset_matches("wine-9.0-staging-tkg-amd64.tar.xz", staging));
        assert!(!asset_matches("wine-9.0-staging-amd64.tar.xz", tkg));
        for family in [vanilla, staging, tkg] {
            assert!(
                !asset_matches("wine-9.0-staging-amd64-wow64.tar.xz", family),
                "a wow64 build must never match {}",
                family.id
            );
        }
    }

    #[test]
    fn non_archives_never_match() {
        let proton = family_by_id("proton-ge").unwrap();
        for name in [
            "GE-Proton9-5.tar.gz.sha512sum",
            "GE-Proton9-5.tar.zst",
            "GE-Proton9-5.zip",
            "",
        ] {
            assert!(!asset_matches(name, proton), "{name:?} is not extractable");
        }
    }

    #[test]
    fn asset_matching_is_case_insensitive_on_both_sides() {
        // The asset name is lowercased and so is each token, so an upstream
        // release that changes its capitalisation does not drop out of the list.
        let vanilla = family_by_id("wine-vanilla").unwrap();
        assert!(asset_matches("Wine-9.0-AMD64.TAR.XZ", vanilla));
    }

    #[test]
    fn the_first_matching_asset_wins() {
        let proton = family_by_id("proton-ge").unwrap();
        let assets = vec![
            json!({ "name": "GE-Proton9-4.tar.gz" }),
            json!({ "name": "GE-Proton9-5.tar.gz" }),
        ];
        let picked = pick_asset(&assets, proton).unwrap();
        assert_eq!(asset_name(&picked), "GE-Proton9-4.tar.gz");
    }

    #[test]
    fn preference_narrows_only_when_a_preferred_asset_exists() {
        let cachyos = family_by_id("proton-cachyos").unwrap();
        let matching = vec![
            json!({ "name": "proton-cachyos-9.0-generic-x86_64.tar.xz" }),
            json!({ "name": "proton-cachyos-9.0-slr-x86_64.tar.xz" }),
        ];
        assert_eq!(
            asset_name(&pick_asset(&matching, cachyos).unwrap()),
            "proton-cachyos-9.0-slr-x86_64.tar.xz",
            "slr+x86_64 satisfies every prefer token, so the generic one loses"
        );

        // No asset satisfies both tokens, so the preference is advisory and the
        // first match stands rather than nothing being offered. This is the
        // branch that a naive "filter by prefer" port would turn into `None`,
        // leaving the family with no installable release at all.
        let none_preferred = vec![
            json!({ "name": "proton-cachyos-9.0-generic-x86_64.tar.xz" }),
            json!({ "name": "proton-cachyos-9.0-slr-aarch64.tar.xz" }),
        ];
        assert_eq!(
            asset_name(&pick_asset(&none_preferred, cachyos).unwrap()),
            "proton-cachyos-9.0-generic-x86_64.tar.xz"
        );
    }

    #[test]
    fn no_matching_asset_is_none_rather_than_a_panic() {
        let vanilla = family_by_id("wine-vanilla").unwrap();
        let assets = vec![json!({ "name": "wine-9.0-wow64.tar.xz" })];
        assert!(pick_asset(&assets, vanilla).is_none());
        assert!(pick_asset(&[], vanilla).is_none());
    }

    #[test]
    fn string_repr_matches_pythons_quoting_rules() {
        // Every expectation below was produced by CPython, not inferred: the
        // quote-switching rule and `\xNN` for control characters are the two
        // that a `{:?}`-shaped implementation gets wrong.
        let cases: &[(&str, &str)] = &[
            ("", "''"),
            (".", "'.'"),
            ("..", "'..'"),
            ("a/b", "'a/b'"),
            ("a\\b", "'a\\\\b'"),
            ("with space", "'with space'"),
            ("\u{fc}n\u{ef}c\u{f8}de", "'\u{fc}n\u{ef}c\u{f8}de'"),
            ("a\"b", "'a\"b'"),
            // A `'` with no `"` switches the delimiter, which is the whole
            // reason a `{:?}` port cannot be right here.
            ("a'b", "\"a'b\""),
            ("a\nb", "'a\\nb'"),
            ("a\tb", "'a\\tb'"),
            ("a\u{b}b", "'a\\x0bb'"),
            ("a\u{0}b", "'a\\x00b'"),
            ("a\u{7f}b", "'a\\x7fb'"),
        ];
        for (value, expected) in cases {
            assert_eq!(&python_str_repr(value), expected, "for {value:?}");
        }
    }

    #[test]
    fn asset_names_are_read_exactly_as_python_reads_them() {
        // Expected values generated by
        // `docs/migration/oracle/run_runners_vectors.py` (op `asset_name`), not
        // written from a reading of `str(x or "")`.
        //
        // The falsy half is the half that matters: `or ""` is a truthiness
        // test, so six non-null shapes read as empty. A draft of this function
        // that checked only for `null` returned `"0"` and `"False"` here, and
        // this table is what caught it.
        let cases: &[(&str, &str)] = &[
            // Falsy: every shape of "no name" reads as the empty string.
            (r#"{}"#, ""),
            (r#"{"name": null}"#, ""),
            (r#"{"name": false}"#, ""),
            (r#"{"name": 0}"#, ""),
            (r#"{"name": 0.0}"#, ""),
            (r#"{"name": -0.0}"#, ""),
            (r#"{"name": ""}"#, ""),
            (r#"{"name": []}"#, ""),
            (r#"{"name": {}}"#, ""),
            // Truthy scalars render as Python's `str()`.
            (r#"{"name": true}"#, "True"),
            (r#"{"name": 42}"#, "42"),
            (r#"{"name": -1}"#, "-1"),
            (r#"{"name": 0.5}"#, "0.5"),
            (r#"{"name": "0"}"#, "0"),
            (r#"{"name": "a.tar.gz"}"#, "a.tar.gz"),
            // Truthy containers render as Python's `repr`, so their strings
            // are single-quoted and their keywords capitalised. Only the array
            // branch is pinned — see `python_str` on why an object's key order
            // is not something a test may assert on.
            (r#"{"name": [1, 2]}"#, "[1, 2]"),
            (r#"{"name": ["a", "b"]}"#, "['a', 'b']"),
            (r#"{"name": [true, null, 1.5]}"#, "[True, None, 1.5]"),
            (r#"{"name": [{"k": "v"}]}"#, "[{'k': 'v'}]"),
            (r#"{"name": [[1], []]}"#, "[[1], []]"),
        ];
        for (asset, expected) in cases {
            let value: Value = serde_json::from_str(asset).unwrap();
            assert_eq!(
                asset_name(&value),
                *expected,
                "asset_name({asset}) should be {expected:?}"
            );
        }

        // And a name that reads as empty can never be picked, even for a family
        // that requires nothing — which is the consequence that matters.
        let proton = family_by_id("proton-ge").unwrap();
        for asset in [
            json!({}),
            json!({ "name": null }),
            json!({ "name": 0 }),
            json!({ "name": false }),
        ] {
            assert!(
                pick_asset(std::slice::from_ref(&asset), proton).is_none(),
                "{asset} has no usable name and must not be picked"
            );
        }
    }

    #[test]
    fn guide_rows_lead_with_system_wine_and_credit_every_family() {
        let rows = runner_guide_details();
        assert_eq!(rows.len(), RUNNER_FAMILIES.len() + 1);
        assert_eq!(rows[0].title, "System Wine");
        assert_eq!(rows[0].maintainer, "WineHQ");
        assert_eq!(rows[0].homepage, "https://www.winehq.org");
        assert_eq!(rows[0].advice, SYSTEM_WINE_GUIDE);

        for (row, family) in rows[1..].iter().zip(RUNNER_FAMILIES) {
            assert_eq!(row.title, family.name);
            assert_eq!(row.maintainer, family.maintainer);
            assert_eq!(row.homepage, family.homepage());
            assert_eq!(row.advice, family.when_to_use);
        }

        // Every row must name a maintainer: the page credits the build author,
        // and an empty credit is the one thing the guide exists to prevent.
        for row in &rows {
            assert!(
                !row.maintainer.is_empty(),
                "{} has no maintainer",
                row.title
            );
            assert!(!row.homepage.is_empty(), "{} has no homepage", row.title);
        }
    }

    #[test]
    fn the_three_column_guides_are_the_details_projected() {
        let details = runner_guide_details();
        let guides = runner_guides();
        assert_eq!(guides.len(), details.len());
        for (guide, detail) in guides.iter().zip(&details) {
            assert_eq!(guide.0, detail.title);
            assert_eq!(guide.1, detail.kind);
            assert_eq!(guide.2, detail.advice);
        }
    }

    #[test]
    fn the_wine_binary_is_found_in_each_supported_layout() {
        let root = std::env::temp_dir().join(format!("gh-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for layout in [["files", "bin"], ["dist", "bin"], ["bin", ""]] {
            let _ = std::fs::remove_dir_all(&root);
            let mut wine = root.clone();
            for part in layout.iter().filter(|part| !part.is_empty()) {
                wine.push(part);
            }
            std::fs::create_dir_all(&wine).unwrap();
            std::fs::write(wine.join("wine"), b"wine").unwrap();
            assert_eq!(find_wine_binary(&root), Some(wine.join("wine")));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_build_with_no_wine_binary_is_reported_as_such() {
        let root = std::env::temp_dir().join(format!("gh-nowine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("files/bin")).unwrap();
        assert_eq!(find_wine_binary(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // install_id — the directory name two families must never share
    // -----------------------------------------------------------------------

    #[test]
    fn proton_ge_keeps_the_bare_tag() {
        // So existing Proton-GE installs are never orphaned by a change here.
        let release = ReleaseInfo::new("GE-Proton9-5", "GE-Proton9-5", "https://x/y", 10);
        assert_eq!(release.install_id().unwrap(), "GE-Proton9-5");
    }

    #[test]
    fn another_family_binds_its_tag_to_the_family() {
        let mut release = ReleaseInfo::new("GE-Proton9-5", "n", "https://x/y", 10);
        release.family_id = "wine-staging".to_string();
        assert_eq!(release.install_id().unwrap(), "wine-staging~fGE-Proton9-5");
    }

    #[test]
    fn the_same_tag_in_two_families_never_shares_an_install_id() {
        // The whole reason `~f` exists: two families publish builds with the
        // same tag, and unbound they would collide, so installing one would
        // overwrite the other.
        let mut staging = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        staging.family_id = "wine-staging".to_string();
        let mut proton = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        proton.family_id = "wine-proton".to_string();

        assert_ne!(staging.install_id().unwrap(), proton.install_id().unwrap());
        // And neither collides with the bare-tag Proton-GE namespace.
        let ge = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        assert_ne!(ge.install_id().unwrap(), staging.install_id().unwrap());
        assert_ne!(ge.install_id().unwrap(), proton.install_id().unwrap());
    }

    #[test]
    fn a_tag_that_needs_sanitising_still_produces_a_safe_directory() {
        let mut release = ReleaseInfo::new("../../relocated", "n", "https://x/y", 10);
        release.family_id = "wine-staging".to_string();
        let id = release.install_id().unwrap();
        assert!(!id.contains('/') && !id.contains('\\'), "got {id:?}");
        assert!(!id.starts_with('.'), "got {id:?}");
    }

    #[test]
    fn a_long_family_and_tag_pair_is_truncated_and_bound_by_hash() {
        // Two distinct long tags that share a 166-character prefix must still
        // land in different directories, which is why the fallback hashes the
        // *full* identity rather than the truncated name.
        let prefix = "t".repeat(200);
        let mut first = ReleaseInfo::new(format!("{prefix}A"), "n", "https://x/y", 10);
        first.family_id = "wine-staging".to_string();
        let mut second = ReleaseInfo::new(format!("{prefix}B"), "n", "https://x/y", 10);
        second.family_id = "wine-staging".to_string();

        let first_id = first.install_id().unwrap();
        let second_id = second.install_id().unwrap();
        assert!(
            first_id.contains("~i"),
            "expected the hash fallback, got {first_id:?}"
        );
        assert!(first_id.len() <= 180, "got {} chars", first_id.len());
        assert_ne!(
            first_id, second_id,
            "truncation must not alias distinct tags"
        );
    }

    #[test]
    fn an_unsafe_family_id_is_refused_rather_than_escaped() {
        let mut release = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        release.family_id = "../..".to_string();
        assert!(release.install_id().is_err());
    }

    #[test]
    fn an_unsafe_tag_is_refused_rather_than_escaped() {
        let release = ReleaseInfo::new("...", "n", "https://x/y", 10);
        assert!(release.install_id().is_err());
    }

    #[test]
    fn a_release_reports_its_family_or_fails_loudly() {
        let ge = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        assert_eq!(ge.family().unwrap().id, "proton-ge");

        let mut unknown = ReleaseInfo::new("v1.0", "n", "https://x/y", 10);
        unknown.family_id = "not-a-family".to_string();
        assert!(unknown.family().is_err());
        // And it still gets a usable install directory: a release from a family
        // we do not know about is still installable, it just is not bound to a
        // catalogue entry. `install_id` does not consult the catalogue.
        assert_eq!(unknown.install_id().unwrap(), "not-a-family~fv1.0");
    }

    #[test]
    fn size_is_reported_in_mebibytes() {
        let release = ReleaseInfo::new("v1.0", "n", "https://x/y", 1024 * 1024);
        assert!((release.size_mb() - 1.0).abs() < f64::EPSILON);
        let half = ReleaseInfo::new("v1.0", "n", "https://x/y", 512 * 1024);
        assert!((half.size_mb() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn serialising_a_release_uses_the_python_field_names() {
        // `ReleaseInfo` is written into the runner metadata file, which the
        // Python app also reads, so the key names are a compatibility surface.
        let release = ReleaseInfo::new("v1.0", "Proton 1.0", "https://x/y", 42);
        let Value::Object(map) = serde_json::to_value(&release).unwrap() else {
            panic!("a release must serialise to an object");
        };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["download_url", "family_id", "name", "size", "tag"],
            "field names must match the Python dataclass"
        );
    }
}
