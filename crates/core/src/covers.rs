//! Cover artwork: lookup, scoring, download, and the pure rules the interface
//! renders with.
//!
//! Port of `gamehandler/covers.py` (T-05's second half: `exe_icons` is the
//! first, and `netpaths` landed at `aa54275`). A title is looked up on the
//! public Steam store search API, the best match is scored, and a portrait
//! library cover is downloaded with header/capsule fallbacks — no SteamGridDB
//! key required. Store launchers and ordinary Windows apps are not Steam store
//! products, so that search finds nothing for them; those fall back to the
//! icon the executable already carries ([`save_exe_icon`]), which needs no
//! network and is always the right artwork for the thing it was taken from.
//!
//! The network half takes the injected HTTP client of D-26
//! ([`HttpClient`](crate::runners::proton::HttpClient)): this crate has no
//! networking dependency and must not acquire one, so every function that
//! touches the network takes `client: &dyn HttpClient` and the binary injects
//! `ureq`. Everything here is blocking ([`crate`] docs), reporting nothing
//! until it returns — the app wraps these calls in `spawn_blocking`.
//!
//! # Why [`accent_index`] is in `core` rather than in the view
//!
//! `crates/app/src/view/cover.rs` needs it to pick a game's placeholder shade,
//! and it is a hash — so it was tempting to put it where it is used. It is here
//! because it is a **port of a Python function whose value is observable**: a
//! game's tile has a colour that must not change between the Python app and
//! this one, or across sessions. That makes it a compatibility surface, and
//! compatibility surfaces belong where the oracle fixtures can reach them,
//! which is `core`.
//!
//! The alternative the view originally reached for — widening
//! [`crate::hash::sha256_hex`] to `pub` — was rejected for the reason the view
//! itself gave: it exports the *primitive* while the thing that must not drift
//! is the *rule*. A caller holding `sha256_hex` can bucket the digest its own
//! way, and the drift would be invisible until someone compared two screenshots.
//! Exposing the decision keeps the rule in one place, and keeps the hash an
//! implementation detail of it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::runners::proton::HttpClient;
use crate::runners::{RunnerError, USER_AGENT};

/// The public Steam store search endpoint (`covers.py:29`).
pub const STORE_SEARCH_URL: &str = "https://store.steampowered.com/api/storesearch/";
/// The Steam app-details endpoint (`covers.py:30`).
pub const APPDETAILS_URL: &str = "https://store.steampowered.com/api/appdetails";
/// CDN roots tried for every cover asset, in order (`covers.py:31-35`).
pub const CDN_ROOTS: [&str; 3] = [
    "https://cdn.akamai.steamstatic.com/steam/apps",
    "https://cdn.cloudflare.steamstatic.com/steam/apps",
    "https://steamcdn-a.akamaihd.net/steam/apps",
];
/// Cover assets tried per CDN root, in order (`covers.py:36-43`).
pub const COVER_ASSETS: [&str; 6] = [
    "library_600x900.jpg",
    "library_600x900_2x.jpg",
    "library_capsule.jpg",
    "portrait.png",
    "header.jpg",
    "capsule_616x353.jpg",
];

/// Covers are small; refuse to buffer a CDN that streams without end
/// (`covers.py:214`).
pub const MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
/// A download under this is an error page, not artwork (`covers.py:274`).
pub const MIN_COVER_BYTES: usize = 1024;

/// A single-word title is too generic to accept a partial match on: Steam's
/// search answers "Steam" with "DCS World Steam Edition" and "Discord" with
/// "Bot Maker For Discord". Hanging that art on the tile is worse than leaving
/// the tile blank, so one-word queries have to match a title outright
/// (`covers.py:180`).
pub const GENERIC_QUERY_MINIMUM: f64 = 0.9;
/// The default `minimum` of [`pick_best_match`] (`covers.py:183`).
pub const MINIMUM_MATCH_SCORE: f64 = 0.45;

/// Artwork found on Steam (`covers.py:253`).
pub const STEAM_SOURCE: &str = "steam";
/// Artwork taken from the executable's own icon (`covers.py:254`).
pub const ICON_SOURCE: &str = "icon";

/// Artwork found for a game.
///
/// Port of `covers.CoverHit` (`covers.py:257-269`). This is the type the fetch
/// functions return and the one the app's state re-exports: there was a
/// second `CoverHit` in `crates/app/src/state.rs`, but the shape is a
/// compatibility surface — the five fields `coverFetched` puts in a map
/// (`bridge.py:575-581`) — so it lives here and the app converts through the
/// re-export rather than maintaining its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverHit {
    /// The Steam application id, `0` when the artwork did not come from Steam.
    pub appid: i64,
    /// The name the artwork is for — Steam's name, which may differ from the
    /// user's.
    pub name: String,
    /// The shelf it was found on, for a later re-fetch.
    pub category: String,
    /// Where the image was written.
    pub cover_path: PathBuf,
    /// Where it came from, for attribution.
    pub source_url: String,
    /// [`STEAM_SOURCE`] or [`ICON_SOURCE`]; see [`CoverHit::origin_label`].
    pub source: String,
}

impl CoverHit {
    /// Artwork found on Steam, which is the default the Python dataclass
    /// carries (`source: str = STEAM_SOURCE`, `covers.py:264`).
    ///
    /// A constructor rather than a struct literal at each call site: the
    /// default is part of the ported shape, and spelling it out here is what
    /// keeps [`STEAM_SOURCE`] from being a constant nothing reads.
    pub fn from_steam(
        appid: i64,
        name: String,
        category: String,
        cover_path: PathBuf,
        source_url: String,
    ) -> Self {
        Self {
            appid,
            name,
            category,
            cover_path,
            source_url,
            source: STEAM_SOURCE.to_string(),
        }
    }

    /// Where the artwork came from, phrased for the toast that announces it.
    ///
    /// `covers.py:266-269`. Only the icon case is special-cased; everything
    /// else is called "Steam", which is what the original does — including for
    /// a source that is neither.
    pub fn origin_label(&self) -> &'static str {
        if self.source == ICON_SOURCE {
            "the app icon"
        } else {
            "Steam"
        }
    }
}

/// One normalised Steam store search hit (`covers.py:157-173`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreHit {
    /// `int(item["id"])` — an `i64` because the reference keeps whatever
    /// `int()` accepts, including negatives.
    pub appid: i64,
    /// `item["name"]`, never empty: nameless hits are dropped while parsing.
    pub name: String,
    /// `item["tiny_image"]`, or `""`.
    pub tiny_image: String,
    /// `item["type"]`, or `""`.
    pub item_type: String,
}

/// The slice of an app-details response the cover lookup reads
/// (`covers.py:237-250`).
///
/// `Default` is the `{}` the lookup carries when details are unavailable —
/// [`steam_cover`] treats an empty name, header and category exactly as the
/// reference treats the missing keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppDetails {
    /// `data["name"]`, or `""`.
    pub name: String,
    /// `data["header_image"]`, or `""`.
    pub header_image: String,
    /// Every genre `description`, in order.
    pub genres: Vec<String>,
    /// [`map_steam_genre`] over [`AppDetails::genres`].
    pub category: String,
}

/// How many placeholder shades there are. `covers.py:95`.
pub const COVER_ACCENTS: usize = 8;

/// The built-in category list, `covers.py:75-89`.
///
/// The game form offers these plus whatever the library already holds
/// (`bridge.py:344-350`), and [`models::UNCATEGORIZED`] is deliberately the
/// first entry — it is the one the reference sorts last in a library listing
/// and the one a blank category folds to, so a form that seeded its selector
/// with anything else would offer a category no game can be in.
///
/// It lives here rather than in `models` because `covers.py` is where the
/// reference declares it, and because the module that answers "what shelf is
/// this game on" is the one that owns the shelf list. The order is
/// user-visible: it is the order the form's category selector reads.
///
/// [`models::UNCATEGORIZED`]: crate::models::UNCATEGORIZED
pub const DEFAULT_CATEGORIES: [&str; 13] = [
    "Uncategorized",
    "Action",
    "Adventure",
    "RPG",
    "Strategy",
    "Shooter",
    "Racing",
    "Simulation",
    "Sports",
    "Puzzle",
    "Indie",
    "Utility",
    "Emulation",
];

/// Up to two uppercase initials for placeholder cover art.
///
/// Port of `initials` (`covers.py:97-104`), moved here from
/// `crates/app/src/view/cover.rs`: the strings are user-visible and must match
/// the reference, which makes them a compatibility surface, and those live in
/// `core` — the same reason [`accent_index`] is here rather than in the view.
/// `re.split(r"[^0-9A-Za-z]+", name)` is a split on runs of
/// non-alphanumerics, which is what [`char::is_ascii_alphanumeric`] gives here
/// — note *ASCII*, since Python's character class is explicit and would split
/// on `é`. A name with no word characters at all gets `"?"`, which is also
/// `CoverArt.qml`'s default.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();

    match words.as_slice() {
        [] => "?".to_string(),
        // `words[0][:2].upper()` — two characters, not two bytes.
        [only] => only.chars().take(2).collect::<String>().to_uppercase(),
        [first, second, ..] => {
            let mut out = String::with_capacity(2);
            out.extend(first.chars().take(1));
            out.extend(second.chars().take(1));
            out.to_uppercase()
        }
    }
}

/// Lowercase a title and strip punctuation / trademark noise.
///
/// Port of `normalize_title` (`covers.py:114-120`). The `&` expansion runs
/// before the split, so `"R&D"` is three words, and the explicit `[™®©]`
/// removal needs no code of its own: those are not ASCII alphanumerics, so
/// the split drops them exactly as the `[^a-z0-9]+` substitution does.
pub fn normalize_title(value: &str) -> String {
    value
        .to_lowercase()
        .replace('&', " and ")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A 0–1 similarity score used to pick the Steam match.
///
/// Port of `score_title` (`covers.py:123-145`): word-set coverage of the
/// query times a precision term, with an edition-or-sequel prefix rule
/// (`"Half-Life 2"` is still the same series) and the extra-words charge that
/// keeps someone else's art off the tile. The arithmetic order matches the
/// reference term for term, so the doubles — and the six-decimal rounding —
/// agree with it bit for bit.
pub fn score_title(query: &str, candidate: &str) -> f64 {
    let left = normalize_title(query);
    let right = normalize_title(candidate);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_parts: std::collections::BTreeSet<&str> = left.split(' ').collect();
    let right_parts: std::collections::BTreeSet<&str> = right.split(' ').collect();
    let shared = left_parts.intersection(&right_parts).count();
    if left_parts.is_empty() || shared == 0 {
        return 0.0;
    }
    let mut coverage = shared as f64 / left_parts.len() as f64;
    let precision = shared as f64 / right_parts.len() as f64;
    if starts_with_word(&right, &left) || starts_with_word(&left, &right) {
        coverage = coverage.max(0.92);
    }
    round6(coverage * (0.6 + 0.4 * precision))
}

/// `text.startswith(prefix + " ")` without the allocation.
fn starts_with_word(text: &str, prefix: &str) -> bool {
    text.len() > prefix.len() && text.starts_with(prefix) && text.as_bytes()[prefix.len()] == b' '
}

/// `round(value, 6)`, bit for bit.
///
/// Implemented as six-decimal formatting parsed back, because that is what
/// CPython's `round` computes: the correctly rounded six-decimal expansion of
/// the exact binary value (ties to even), read back as the nearest double.
/// Rust's precision formatting is correctly rounded with ties to even —
/// pinned below with an exactly representable halfway double — and
/// `str::parse` is correctly rounded, so the two agree on every input. A
/// multiply-by-1e6 implementation would double-round: `1.0000015 * 1e6` lands
/// exactly on `1000001.5` in doubles while the true value sits below the
/// decimal midpoint, so the cheap version rounds up where the reference
/// rounds down.
fn round6(value: f64) -> f64 {
    format!("{value:.6}")
        .parse()
        .expect("six-decimal formatting of a float always parses back")
}

/// Steam genres mapped onto GameHandler's category list (`covers.py:46-73`).
const GENRE_MAP: &[(&str, &str)] = &[
    ("action", "Action"),
    ("adventure", "Adventure"),
    ("rpg", "RPG"),
    ("role-playing", "RPG"),
    ("strategy", "Strategy"),
    ("simulation", "Simulation"),
    ("racing", "Racing"),
    ("sports", "Sports"),
    ("casual", "Puzzle"),
    ("indie", "Indie"),
    ("free to play", "Indie"),
    ("early access", "Indie"),
    ("massively multiplayer", "Action"),
    ("animation & modeling", "Utility"),
    ("utilities", "Utility"),
    ("design & illustration", "Utility"),
    ("video production", "Utility"),
    ("audio production", "Utility"),
    ("education", "Utility"),
    ("web publishing", "Utility"),
    ("software training", "Utility"),
    ("photo editing", "Utility"),
    ("game development", "Utility"),
    ("violent", "Action"),
    ("gore", "Action"),
    ("nudity", "Adventure"),
];

/// Map Steam genre names onto a GameHandler category.
///
/// Port of `map_steam_genre` (`covers.py:148-154`): the first genre whose
/// stripped, lowercased form hits [`GENRE_MAP`] wins, and anything else —
/// including no genres at all — is `"Uncategorized"`.
pub fn map_steam_genre(genres: &[impl AsRef<str>]) -> &'static str {
    for genre in genres {
        let key = genre.as_ref().trim().to_lowercase();
        if let Some((_, category)) = GENRE_MAP.iter().find(|(mapped, _)| *mapped == key) {
            return category;
        }
    }
    "Uncategorized"
}

/// Normalize a Steam storesearch payload into match dicts.
///
/// Port of `parse_store_search` (`covers.py:157-173`). A hit survives exactly
/// when its id is truthy and its name is non-empty; `tiny_image` and `type`
/// default to `""`. Two lenient divergences, both in inputs the store never
/// sends and the reference answers by crashing: a truthy-but-unparseable id
/// (a non-numeric string, a list, a dict) drops the hit instead of raising
/// `ValueError`/`TypeError` out of `int()`, and a truthy non-string name drops
/// it instead of carrying a non-string into the scorer.
pub fn parse_store_search(payload: &serde_json::Value) -> Vec<StoreHit> {
    let mut items = Vec::new();
    let list = payload.get("items").and_then(serde_json::Value::as_array);
    for item in list.into_iter().flatten() {
        let Some(appid) = item.get("id").and_then(store_id) else {
            continue;
        };
        let Some(name) = item.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        items.push(StoreHit {
            appid,
            name: name.to_string(),
            tiny_image: item
                .get("tiny_image")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            item_type: item
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    items
}

/// `int(item["id"])` with the reference's falsy filter.
///
/// `None` means the hit is dropped: missing, null, `false`, zero, an empty
/// string, or anything `int()` would refuse. Note `"0"` is `Some(0)` — a
/// non-empty string is truthy, so the reference keeps it — while numeric `0`
/// is falsy and dropped. Floats truncate toward zero as `int()` does;
/// unrepresentable magnitudes are dropped rather than saturated.
fn store_id(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(false) => None,
        serde_json::Value::Bool(true) => Some(1),
        serde_json::Value::Number(number) => {
            let int = number.as_i64().or_else(|| {
                number
                    .as_u64()
                    .and_then(|big| i64::try_from(big).ok())
                    .or_else(|| {
                        number.as_f64().and_then(|float| {
                            if float.is_finite()
                                && float >= i64::MIN as f64
                                && float < 9_223_372_036_854_775_808.0_f64
                            {
                                Some(float as i64)
                            } else {
                                None
                            }
                        })
                    })
            })?;
            if int == 0 { None } else { Some(int) }
        }
        serde_json::Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                text.parse::<i64>().ok()
            }
        }
        // Truthy lists and dicts crash `int()` in the reference; a cover
        // lookup that dropped the hit instead is the lenient direction.
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => None,
    }
}

/// Choose the Steam search hit that best matches `query`.
///
/// Port of `pick_best_match` (`covers.py:183-199`) with the default minimum:
/// one-word queries must clear [`GENERIC_QUERY_MINIMUM`], longer ones
/// [`MINIMUM_MATCH_SCORE`]. Non-app hits are skipped, equal scores go to the
/// tighter title rather than to whichever hit the store ranked first, and full
/// ties keep store order — the sort is stable in both languages.
pub fn pick_best_match<'a>(query: &str, items: &'a [StoreHit]) -> Option<&'a StoreHit> {
    pick_best_match_with_minimum(query, items, MINIMUM_MATCH_SCORE)
}

/// [`pick_best_match`] with an explicit minimum score.
pub fn pick_best_match_with_minimum<'a>(
    query: &str,
    items: &'a [StoreHit],
    minimum: f64,
) -> Option<&'a StoreHit> {
    let threshold = if normalize_title(query).split_whitespace().count() < 2 {
        minimum.max(GENERIC_QUERY_MINIMUM)
    } else {
        minimum
    };
    let mut ranked: Vec<(f64, usize, &StoreHit)> = Vec::new();
    for item in items {
        // `item["type"] not in {"app", "game", ""}` with falsy types passing
        // before the membership test — which makes the `""` in the set dead.
        if !item.item_type.is_empty() && item.item_type != "app" && item.item_type != "game" {
            continue;
        }
        let score = score_title(query, &item.name);
        if score >= threshold {
            let words = normalize_title(&item.name).split_whitespace().count();
            ranked.push((score, words, item));
        }
    }
    // Score descending, then word count ascending: `(score, -len)` with
    // `reverse=True`, where the negation turns the descending sort ascending.
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.first().map(|(_, _, item)| *item)
}

/// Every cover URL for a Steam app, in try order (`covers.py:202-210`).
///
/// The outer loop is the asset and the inner loop is the CDN root — every
/// root is tried for `library_600x900.jpg` before anything tries
/// `library_600x900_2x.jpg` — and the search hit's `tiny_image` goes last.
pub fn cover_urls_for_app(appid: i64, tiny_image: &str) -> Vec<String> {
    let mut urls = Vec::with_capacity(COVER_ASSETS.len() * CDN_ROOTS.len() + 1);
    for asset in COVER_ASSETS {
        for root in CDN_ROOTS {
            urls.push(format!("{root}/{appid}/{asset}"));
        }
    }
    if !tiny_image.is_empty() {
        urls.push(tiny_image.to_string());
    }
    urls
}

/// A stable colour bucket for `seed`, so a game's tile never changes shade.
/// `accent_index` (`covers.py:107-110`).
///
/// `sha256(seed.encode("utf-8")).digest()[0] % COVER_ACCENTS` — the **first byte**
/// of the digest, not the first hex digit. Reading the digest as text and
/// taking one character would give a value in `0..16` bucketed against 8 and so
/// happen to produce the same answer, which is exactly the kind of coincidence
/// that stops being harmless the moment the bucket count changes. The
/// arithmetic is done on a byte here, as in Python.
///
/// `seed` being empty is handled by hashing the empty string rather than by a
/// special case, because that is what `(seed or "")` does and the two agree:
/// `sha256(b"")` is a real digest whose first byte is 227, so the empty seed is
/// bucket 3, not bucket 0. A port that short-circuited the empty case would put
/// every nameless game in the first shade.
///
/// UTF-8 rather than the platform encoding, so the bucket for a non-ASCII name
/// is the same on every machine — which is what makes this reproducible at all.
pub fn accent_index(seed: &str) -> usize {
    accent_index_in(seed, COVER_ACCENTS)
}

/// [`accent_index`] with an explicit bucket count.
///
/// Exists because Python's signature is `accent_index(seed, buckets=COVER_ACCENTS)`
/// and the default is the *only* thing that differs: the corpus exercises other
/// counts to reach the clamp below, and a caller that needs a different number
/// of shades should not have to re-derive the rule to get one.
///
/// `buckets` is clamped to at least 1, as `max(1, buckets)` does. A zero would
/// otherwise be a division by zero — in Python it raises, here it would panic,
/// and neither is a useful behaviour for a caller that computed a count from a
/// config file. Negative counts cannot reach the clamp from Python's `int`, but
/// they can from a `usize`-underflowing caller, so the clamp is written on the
/// way in rather than assumed.
/// Writing an executable's icon into the covers directory failed.
///
/// Port of the two failures `save_exe_icon` (`covers.py:307-317`) surfaces:
/// the executable carries no icon (a `RuntimeError` there, [`SaveIconError::NoIcon`]
/// here, with the reference's message byte for byte), or the filesystem
/// refused the write.
#[derive(Debug)]
pub enum SaveIconError {
    /// `RuntimeError(f"{name} carries no icon to use as a cover")`.
    ///
    /// `exe` is the executable's file name, as `Path(exe_path).name` is —
    /// including when there is no icon because the path is not a PE at all,
    /// since [`crate::exe_icons::extract_icon`] folds every unreadable input
    /// into `None` before this error is built.
    NoIcon { exe: String },
    /// The game id could not be turned into a filename without the possibility
    /// of leaving the covers directory. See [`cover_stem`].
    ///
    /// **Deliberate divergence from `covers.py:307-317`** (`SECURITY.md`
    /// `SEC-04`). The reference interpolates `game_id` into a destination path
    /// unvalidated; this port refuses an id that contains a path separator,
    /// because that is the only thing in the interpolation that can traverse.
    UnsafeId { id: String },
    /// Creating the covers directory, writing the temporary file, or the
    /// rename into place failed.
    Io(std::io::Error),
}

impl From<std::io::Error> for SaveIconError {
    fn from(error: std::io::Error) -> Self {
        SaveIconError::Io(error)
    }
}

impl fmt::Display for SaveIconError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveIconError::NoIcon { exe } => {
                write!(formatter, "{exe} carries no icon to use as a cover")
            }
            SaveIconError::UnsafeId { id } => formatter.write_str(&unsafe_id_message(id)),
            SaveIconError::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SaveIconError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveIconError::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Turn a game id into the stem of a filename inside the covers directory, or
/// refuse it. **Deliberate divergence from the reference** (`SECURITY.md`
/// `SEC-04`).
///
/// Every cover write builds its destination as `covers_dir.join(format!("{id}…"))`,
/// and the id comes from a `games.json` entry — which the app loads verbatim,
/// so it is untrusted input in the sense of the brief's category C. The
/// reference interpolates it with no check at all, and this port copied that
/// until this function existed. An id of `"../../.config/autostart/x"` then
/// writes `x.ico`, `x.jpg` or `x.png` there instead, which is an arbitrary-file
/// write anywhere the sandbox can reach.
///
/// **The rule is one condition, not a sanitiser.** A path separator is the only
/// thing in the interpolation that can traverse: the id is always followed by
/// `.{ext}`, so `".."` becomes `"...ico"` — a legal, harmless filename — and an
/// empty id becomes `".ico"`. Rewriting the id rather than rejecting it would
/// rename every existing user's cover files, so nothing here alters a value that
/// passes; `desktop::id_prefix`, which *does* rewrite, is the right shape for a
/// desktop entry whose name is this port's to choose and the wrong shape for a
/// filename the reference already picked.
///
/// `\` is rejected alongside `/` even though it is an ordinary character in a
/// Unix filename, so that the rule matches
/// [`safe_install_id`](crate::runners::archive::safe_install_id) and so that a
/// library carried between machines cannot change meaning.
///
/// Both `covers_dir()` and the directory argument the tests pass are subject to
/// this, and neither caller can be reached with a separator by the application
/// itself: ids are minted by [`crate::models::new_id`], which is 32 lowercase
/// hex characters. Refusing is therefore a foreign-or-hand-edited-`games.json`
/// path, and it surfaces as an error the user is told about rather than as a
/// write somewhere else.
fn cover_stem(game_id: &str) -> Result<&str, &'static str> {
    if game_id.contains('/') || game_id.contains('\\') {
        return Err("it contains a path separator");
    }
    Ok(game_id)
}

/// The one message both [`SaveIconError::UnsafeId`] and
/// [`CoverError::UnsafeId`] render, so the refusal reads identically whichever
/// flow reached it.
fn unsafe_id_message(id: &str) -> String {
    format!("{id:?} cannot be used as a cover filename: it contains a path separator")
}

/// Write the icon embedded in a Windows executable into the covers directory.
///
/// Port of `save_exe_icon` (`covers.py:307-317`): the icon bytes come from
/// [`crate::exe_icons::extract_icon`], the destination is
/// `{covers_dir()}/{game_id}.ico`, and the write is atomic — a temporary
/// `{game_id}.ico.tmp` beside the target, renamed into place — so an
/// interrupted save leaves the previous cover (or nothing), never half a file.
pub fn save_exe_icon(exe_path: &Path, game_id: &str) -> Result<PathBuf, SaveIconError> {
    save_exe_icon_to(exe_path, game_id, &crate::paths::covers_dir())
}

/// [`save_exe_icon`] into an explicit directory, as the reference's tests do
/// by patching `config.covers_dir` (`tests/test_covers.py:142-146`).
pub fn save_exe_icon_to(
    exe_path: &Path,
    game_id: &str,
    covers_dir: &Path,
) -> Result<PathBuf, SaveIconError> {
    let icon = crate::exe_icons::extract_icon(exe_path).ok_or_else(|| SaveIconError::NoIcon {
        exe: exe_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })?;
    let stem = cover_stem(game_id).map_err(|_| SaveIconError::UnsafeId {
        id: game_id.to_string(),
    })?;
    let destination = covers_dir.join(format!("{stem}.ico"));
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("ico.tmp");
    std::fs::write(&temporary, icon)?;
    std::fs::rename(&temporary, &destination)?;
    Ok(destination)
}

/// A cover lookup or download failed.
///
/// Every `RuntimeError` the reference raises is a variant here, and each
/// [`fmt::Display`] is the reference's message byte for byte: these messages
/// reach the user through toasts, and `tests/test_covers.py` asserts on their
/// substrings (`"no icon"`, `"No Steam cover found for X"`), so the wording is
/// behaviour rather than prose.
#[derive(Debug)]
pub enum CoverError {
    /// `RuntimeError(f"No Steam cover found for “{query}”")`
    /// (`covers.py:338`) — note the curly quotes.
    NoMatch { query: String },
    /// `RuntimeError("Cover download was empty")` (`covers.py:275`): fewer
    /// than [`MIN_COVER_BYTES`] arrived, which is an error page, not artwork.
    EmptyDownload,
    /// `RuntimeError(f"Could not download a cover: {last_error}")`
    /// (`covers.py:292`): every URL failed, and `reason` is the last
    /// failure's message — or `"No cover URLs"` when there was nothing to try.
    DownloadFailed { reason: String },
    /// `RuntimeError(f"Response from {url} exceeded {limit} bytes")`
    /// (`covers.py:224`): the transfer crossed the byte cap and was aborted.
    ResponseTooLarge { url: String, limit: usize },
    /// The bytes were not UTF-8 or not JSON. Python raises
    /// `UnicodeDecodeError`/`JSONDecodeError` with messages this port does not
    /// reproduce word for word; `message` carries the decoder's own report,
    /// which plays the same role — the `steam_error` [`fetch_cover`]
    /// surfaces when the exe fallback also fails.
    InvalidResponse { message: String },
    /// `RuntimeError(steam_error)` (`covers.py:382`): Steam failed *and* the
    /// exe fallback failed or was absent, so the Steam failure's message is
    /// what surfaces. Verbatim by construction — it is the [`fmt::Display`]
    /// of the error Steam returned.
    SteamFailed { message: String },
    /// Steam failed **and** the executable's own icon could not be *written* to
    /// the covers directory.
    ///
    /// **Deliberate divergence from `covers.py:379-381`** (`BUGS.md` `BUG-13`,
    /// `FEATURES.md` `P-61`). The reference's bare `pass` folds a full disk, a
    /// read-only `covers_dir` and a `covers` path that is not a directory into
    /// "the icon fallback did not apply", and then reports the Steam failure —
    /// so the user is told the artwork could not be *found* when it was found
    /// and could not be *saved*. Two different subsystems, one message, and the
    /// one that names a fixable condition is the one discarded. The reference
    /// has no value to express this with; this port has
    /// [`SaveIconError::Io`], and this variant is what carries it.
    ///
    /// Only [`SaveIconError::Io`] reaches here — see [`fetch_cover`] for why a
    /// genuinely absent icon stays at the reference's message.
    IconWriteFailed {
        /// The Steam failure's message, exactly as [`Self::SteamFailed`]
        /// carries it.
        steam_error: String,
        /// The directory the icon was to be written into. The `io::Error`
        /// carries no path, and this is the one the user has to act on.
        covers_dir: PathBuf,
        icon_error: SaveIconError,
    },
    /// The injected client refused the transfer: a transport failure or a
    /// non-success status, which the [`HttpClient`] contract reports as `Err`.
    Http(RunnerError),
    /// A filesystem operation failed — the covers directory could not be
    /// created, a temporary file could not be written, a rename into place
    /// failed, or a custom-cover source was missing.
    Io(std::io::Error),
    /// The game id could not be turned into a filename without the possibility
    /// of leaving the covers directory. See [`cover_stem`] for the rule and for
    /// why it is a refusal rather than a rewrite.
    ///
    /// **Deliberate divergence from the reference** (`SECURITY.md` `SEC-04`):
    /// the reference interpolates the id unvalidated and would write outside
    /// `covers_dir`.
    UnsafeId { id: String },
}

impl From<std::io::Error> for CoverError {
    fn from(error: std::io::Error) -> Self {
        CoverError::Io(error)
    }
}

impl From<RunnerError> for CoverError {
    fn from(error: RunnerError) -> Self {
        CoverError::Http(error)
    }
}

impl fmt::Display for CoverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoverError::NoMatch { query } => {
                write!(
                    formatter,
                    "No Steam cover found for \u{201c}{query}\u{201d}"
                )
            }
            CoverError::EmptyDownload => formatter.write_str("Cover download was empty"),
            CoverError::DownloadFailed { reason } => {
                write!(formatter, "Could not download a cover: {reason}")
            }
            CoverError::ResponseTooLarge { url, limit } => {
                write!(formatter, "Response from {url} exceeded {limit} bytes")
            }
            CoverError::InvalidResponse { message } | CoverError::SteamFailed { message } => {
                formatter.write_str(message)
            }
            CoverError::IconWriteFailed {
                steam_error,
                covers_dir,
                icon_error,
            } => write!(
                formatter,
                "{steam_error}; the executable's own icon could not be written to {}: {icon_error}",
                covers_dir.display()
            ),
            CoverError::Http(error) => error.fmt(formatter),
            CoverError::Io(error) => error.fmt(formatter),
            CoverError::UnsafeId { id } => {
                write!(formatter, "{}", unsafe_id_message(id))
            }
        }
    }
}

impl std::error::Error for CoverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CoverError::Http(error) => Some(error),
            CoverError::Io(error) => Some(error),
            // The write is the *cause* of this variant, so it is the source —
            // the Steam error is a peer, not a parent.
            CoverError::IconWriteFailed { icon_error, .. } => Some(icon_error),
            _ => None,
        }
    }
}

/// GET `url` with the GameHandler user agent, capped at `limit` bytes.
///
/// Port of `_request` (`covers.py:217-225`): the status check lives in the
/// injected client (the [`HttpClient`] contract reports non-success as `Err`),
/// and the byte cap is enforced incrementally in the sink — a CDN that
/// streams without end is cut off at the cap rather than buffered. The
/// declared `Content-Length` is deliberately *not* consulted: a lying header
/// must not refuse a body the reference would accept.
fn request_bytes(
    client: &dyn HttpClient,
    url: &str,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>, CoverError> {
    let mut body = Vec::new();
    let mut over_limit = false;
    let result = client.get(
        url,
        &[("User-Agent", USER_AGENT), ("Accept", "*/*")],
        timeout,
        &mut |_head| Ok(()),
        &mut |chunk| {
            body.extend_from_slice(chunk);
            if body.len() > limit {
                over_limit = true;
                // The message is replaced below; the `Err` is what stops the
                // transfer, so its payload only has to be an error.
                return Err(RunnerError::Http {
                    message: String::new(),
                });
            }
            Ok(())
        },
    );
    if over_limit || body.len() > limit {
        return Err(CoverError::ResponseTooLarge {
            url: url.to_string(),
            limit,
        });
    }
    result.map_err(CoverError::Http)?;
    Ok(body)
}

/// `urllib.parse.quote_plus` for a search term: unreserved bytes pass through,
/// spaces become `+`, everything else is `%XX` over the UTF-8 bytes.
fn encode_term(query: &str) -> String {
    let mut out = String::new();
    for byte in query.as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Search Steam for `query` and normalize the hits.
///
/// Port of `search_steam` (`covers.py:228-234`). A non-dict payload yields no
/// hits rather than an error; undecodable bytes and malformed JSON are
/// [`CoverError::InvalidResponse`], which [`fetch_cover`] carries as its
/// `steam_error`.
pub fn search_steam(
    client: &dyn HttpClient,
    query: &str,
    timeout: Duration,
) -> Result<Vec<StoreHit>, CoverError> {
    let url = format!(
        "{STORE_SEARCH_URL}?term={}&l=english&cc=US",
        encode_term(query)
    );
    let raw = request_bytes(client, &url, timeout, MAX_RESPONSE_BYTES)?;
    let text = String::from_utf8(raw).map_err(|error| CoverError::InvalidResponse {
        message: error.to_string(),
    })?;
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| CoverError::InvalidResponse {
            message: error.to_string(),
        })?;
    if !payload.is_object() {
        return Ok(Vec::new());
    }
    Ok(parse_store_search(&payload))
}

/// Fetch the name, header image and genres Steam holds for `appid`.
///
/// Port of `fetch_app_details` (`covers.py:237-250`). A missing entry, a
/// falsy `success`, or a missing `data` object yields [`AppDetails::default`]
/// — the `{}` [`steam_cover`] tolerates — while transport and decoding
/// failures propagate to [`steam_cover`]'s own catch-all.
pub fn fetch_app_details(
    client: &dyn HttpClient,
    appid: i64,
    timeout: Duration,
) -> Result<AppDetails, CoverError> {
    let url = format!("{APPDETAILS_URL}?appids={appid}&l=english");
    let raw = request_bytes(client, &url, timeout, MAX_RESPONSE_BYTES)?;
    let text = String::from_utf8(raw).map_err(|error| CoverError::InvalidResponse {
        message: error.to_string(),
    })?;
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| CoverError::InvalidResponse {
            message: error.to_string(),
        })?;
    let data = payload
        .get(appid.to_string())
        .filter(|entry| entry.get("success").is_some_and(json_truthy))
        .and_then(|entry| entry.get("data"))
        .and_then(serde_json::Value::as_object);
    let Some(data) = data else {
        return Ok(AppDetails::default());
    };
    let name = data
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let header_image = data
        .get("header_image")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let genres: Vec<String> = data
        .get("genres")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    item.get("description")
                        .and_then(serde_json::Value::as_str)
                        .filter(|description| !description.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let category = map_steam_genre(&genres).to_string();
    Ok(AppDetails {
        name,
        header_image,
        genres,
        category,
    })
}

/// Python truthiness for a JSON value, as `if not entry.get("success")` reads it.
fn json_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(flag) => *flag,
        serde_json::Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
    }
}

/// Download `url` into `destination`, refusing error pages and endless streams.
///
/// Port of `download_image` (`covers.py:272-280`): under [`MIN_COVER_BYTES`]
/// is [`CoverError::EmptyDownload`], and the write is atomic — a `.tmp`
/// sibling renamed into place — so an interrupted download leaves the previous
/// cover (or nothing), never half a file.
pub fn download_image(
    client: &dyn HttpClient,
    url: &str,
    destination: &Path,
    timeout: Duration,
) -> Result<PathBuf, CoverError> {
    let data = request_bytes(client, url, timeout, MAX_RESPONSE_BYTES)?;
    if data.len() < MIN_COVER_BYTES {
        return Err(CoverError::EmptyDownload);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = destination.as_os_str().to_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, destination)?;
    Ok(destination.to_path_buf())
}

/// Download the first reachable cover in `urls` as `{covers_dir}/{game_id}.jpg`.
///
/// Port of `save_cover_from_urls` (`covers.py:283-292`): every URL is tried in
/// order, each failure's message replaces the last, and when nothing works the
/// error names the final failure — or `"No cover URLs"` when there was nothing
/// to try.
pub fn save_cover_from_urls(
    client: &dyn HttpClient,
    urls: &[String],
    game_id: &str,
    covers_dir: &Path,
    timeout: Duration,
) -> Result<(PathBuf, String), CoverError> {
    let stem = cover_stem(game_id).map_err(|_| CoverError::UnsafeId {
        id: game_id.to_string(),
    })?;
    let destination = covers_dir.join(format!("{stem}.jpg"));
    let mut last_error = "No cover URLs".to_string();
    for url in urls {
        match download_image(client, url, &destination, timeout) {
            Ok(path) => return Ok((path, url.clone())),
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(CoverError::DownloadFailed { reason: last_error })
}

/// Copy a user-selected image into the covers directory.
///
/// Port of `copy_custom_cover` (`covers.py:295-304`): a missing source is a
/// `NotFound` io error carrying the path, as the reference's
/// `FileNotFoundError` does, and only `png`/`jpg`/`jpeg`/`webp` keep their
/// suffix — everything else is stored as `.jpg`. One deliberate divergence:
/// `shutil.copy2` also copies timestamps, `std::fs::copy` does not, and
/// nothing reads a cover file's timestamps.
pub fn copy_custom_cover(
    source: &Path,
    game_id: &str,
    covers_dir: &Path,
) -> Result<PathBuf, CoverError> {
    if !source.is_file() {
        return Err(CoverError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            source.to_string_lossy().into_owned(),
        )));
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let stem = cover_stem(game_id).map_err(|_| CoverError::UnsafeId {
        id: game_id.to_string(),
    })?;
    let file_name = if ["png", "jpg", "jpeg", "webp"].contains(&extension.as_str()) {
        format!("{stem}.{extension}")
    } else {
        format!("{stem}.jpg")
    };
    let destination = covers_dir.join(file_name);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, &destination)?;
    Ok(destination)
}

/// Build a [`CoverHit`] from a Windows executable's own icon.
///
/// Port of `icon_cover` (`covers.py:320-330`): the icon is saved by
/// [`save_exe_icon_to`], whose error passes through unwrapped exactly as the
/// reference lets it rise, and an empty name falls back to the exe's stem.
pub fn icon_cover(
    name: &str,
    exe_path: &Path,
    game_id: &str,
    covers_dir: &Path,
) -> Result<CoverHit, SaveIconError> {
    let path = save_exe_icon_to(exe_path, game_id, covers_dir)?;
    let name = if name.is_empty() {
        exe_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        name.to_string()
    };
    Ok(CoverHit {
        appid: 0,
        name,
        category: "Uncategorized".to_string(),
        cover_path: path,
        source_url: exe_path.to_string_lossy().into_owned(),
        source: ICON_SOURCE.to_string(),
    })
}

/// Search Steam for `query` and download the best matching cover.
///
/// Port of `steam_cover` (`covers.py:333-356`): no match is
/// [`CoverError::NoMatch`], unreadable details degrade to [`AppDetails::default`]
/// (the reference's `except (...)` around `fetch_app_details`), and the header
/// image jumps the URL queue when details supply one.
pub fn steam_cover(
    client: &dyn HttpClient,
    query: &str,
    game_id: &str,
    covers_dir: &Path,
    timeout: Duration,
) -> Result<CoverHit, CoverError> {
    let items = search_steam(client, query, timeout)?;
    let found = pick_best_match(query, &items).ok_or_else(|| CoverError::NoMatch {
        query: query.to_string(),
    })?;
    let details = fetch_app_details(client, found.appid, timeout).unwrap_or_default();
    let mut urls = cover_urls_for_app(found.appid, &found.tiny_image);
    if !details.header_image.is_empty() {
        urls.insert(0, details.header_image.clone());
    }
    let (path, source) = save_cover_from_urls(client, &urls, game_id, covers_dir, timeout)?;
    Ok(CoverHit {
        appid: found.appid,
        name: if details.name.is_empty() {
            found.name.clone()
        } else {
            details.name.clone()
        },
        category: if details.category.is_empty() {
            "Uncategorized".to_string()
        } else {
            details.category.clone()
        },
        cover_path: path,
        source_url: source,
        source: STEAM_SOURCE.to_string(),
    })
}

/// Find artwork for `query`, preferring Steam and falling back to the exe.
///
/// Port of `fetch_cover` (`covers.py:359-382`). Steam goes first because it
/// has real portrait library art for the games it sells; the executable's own
/// icon is second because it is offline, unambiguous, and belongs to the thing
/// being launched. Any Steam failure is kept as `steam_error`, the icon is
/// tried when `exe_path` names a real file, and the Steam failure's message is
/// what surfaces as [`CoverError::SteamFailed`].
///
/// # Where this diverges: a *write* failure is not a missing icon
///
/// `covers.py:379-381` swallows whatever the icon path raised
/// (`except (OSError, RuntimeError): pass`) and re-raises `steam_error`
/// unchanged, so a full disk, a read-only `covers_dir` and an executable that
/// carries no icon at all are one outcome to the user, and the message names
/// Steam. This port keeps them apart (`BUGS.md` `BUG-13`, `FEATURES.md`
/// `P-61`):
///
/// * [`SaveIconError::Io`] — the artwork was there and could not be saved —
///   becomes [`CoverError::IconWriteFailed`], which carries both messages. The
///   filesystem refused a write the lookup had already earned, and that is the
///   one fact in this pair the user can act on; reporting it as a failed
///   *search* discards it. Failure paths in this port are not allowed to
///   discard the reason a user-visible step did not happen.
/// * [`SaveIconError::NoIcon`] stays silent, exactly as the reference leaves
///   it. The executable has no icon; nothing failed and nothing is fixable,
///   and the Steam message is accurate about the outcome. The same is true
///   when `exe_path` names a file that is not there, which is the case
///   `tests/test_covers.py:186-190` pins against Python.
pub fn fetch_cover(
    client: &dyn HttpClient,
    query: &str,
    game_id: &str,
    exe_path: Option<&Path>,
    covers_dir: &Path,
    timeout: Duration,
) -> Result<CoverHit, CoverError> {
    let steam_error = match steam_cover(client, query, game_id, covers_dir, timeout) {
        Ok(hit) => return Ok(hit),
        Err(error) => error.to_string(),
    };
    // `exe_path and Path(exe_path).is_file()` — the icon is a candidate only
    // when the caller named a real file, so the `NoIcon` arm below is reached
    // only for an executable that exists and has no icon.
    if let Some(exe) = exe_path.filter(|exe| exe.is_file()) {
        match icon_cover(query, exe, game_id, covers_dir) {
            Ok(hit) => return Ok(hit),
            Err(SaveIconError::NoIcon { .. }) => {}
            Err(icon_error) => {
                return Err(CoverError::IconWriteFailed {
                    steam_error,
                    covers_dir: covers_dir.to_path_buf(),
                    icon_error,
                });
            }
        }
    }
    Err(CoverError::SteamFailed {
        message: steam_error,
    })
}

pub fn accent_index_in(seed: &str, buckets: usize) -> usize {
    let buckets = buckets.max(1);
    let digest = crate::hash::sha256_hex(seed.as_bytes());
    // The digest is hex; the first byte is its first two characters. Taking the
    // byte rather than parsing a `u64` keeps this equal to Python's
    // `digest[0]` for every seed, including the ones whose first byte is above
    // `0x7f` — where a naive `hex[0]` would be an ASCII letter.
    let first = u8::from_str_radix(&digest[0..2], 16)
        .expect("sha256_hex always writes at least two hex characters");
    usize::from(first) % buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_seed_is_hashed_rather_than_short_circuited() {
        // `(seed or "")` hashes the empty string. The value below is CPython's:
        // `int.from_bytes(hashlib.sha256(b"").digest()[:1])` is 227, and
        // `227 % 8` is 3. A port that returned 0 for the empty seed would put
        // every unnamed game in the first shade, and this is the assertion that
        // says so.
        assert_eq!(accent_index(""), 3);
        assert_ne!(accent_index(""), 0);
    }

    #[test]
    fn the_bucket_is_the_first_digest_byte_not_the_first_hex_digit() {
        // `sha256("")` is `e3b0c442…`. Its first byte is `0xe3` = 227, bucket 3.
        // Reading the digest as text and taking `'e'` would give 14, bucket 6 —
        // a value this test rejects. The two differ only because 227 and 14 are
        // incongruent mod 8, so the case has to be one where they are.
        assert_eq!(crate::hash::sha256_hex(b"")[0..2], *"e3");
        assert_eq!(accent_index(""), 227 % COVER_ACCENTS);
    }

    #[test]
    fn known_seeds_land_in_cpython_s_buckets() {
        // Every pair below was produced by running the Python implementation.
        // They are here as a smoke test with readable values; the exhaustive
        // check is the vector corpus, which carries the same cases plus the
        // awkward ones (non-ASCII, long ids, the clamp).
        assert_eq!(accent_index("a"), 2);
        assert_eq!(accent_index("bb"), 3);
        assert_eq!(accent_index("ccc"), 4);
        assert_eq!(accent_index("game-a"), 7);
        assert_eq!(accent_index("a-very-long-id"), 3);
        assert_eq!(accent_index("Half-Life"), 5);
        assert_eq!(accent_index("0".repeat(32).as_str()), 4);
    }

    #[test]
    fn a_non_ascii_seed_is_hashed_as_utf8() {
        // The seed is a game id or name, either of which can be non-ASCII. The
        // encoding is what makes the bucket reproducible across machines, so it
        // is pinned rather than assumed: CPython's `"😀".encode("utf-8")` is
        // `f0 9f 98 80` and this crate hashes the same four bytes.
        assert_eq!(accent_index("\u{1f600}"), 0);
        assert_eq!(crate::hash::sha256_hex("\u{1f600}".as_bytes())[0..2], *"f0");
    }

    #[test]
    fn the_result_is_always_a_usable_shade_index() {
        // The property that matters to the view: whatever the seed, the value
        // indexes `COVER_GRADIENTS` without wrapping. `shade` wraps defensively,
        // but it should never have to.
        for seed in ["", "a", "\u{1f600}", &"x".repeat(500)] {
            assert!(
                accent_index(seed) < COVER_ACCENTS,
                "{seed:?} escaped the range"
            );
        }
    }

    #[test]
    fn the_same_seed_always_gets_the_same_shade() {
        // Stability is the whole point of hashing here: a tile that changed
        // colour between sessions would be worse than a shared colour.
        let first = accent_index("Half-Life");
        for _ in 0..8 {
            assert_eq!(accent_index("Half-Life"), first);
        }
    }

    #[test]
    fn a_bucket_count_of_zero_is_clamped_rather_than_dividing_by_zero() {
        // `max(1, buckets)` in Python. Reaching this needs an explicit count,
        // which is why `accent_index_in` is public — a caller that computed zero
        // from a config file must not panic.
        assert_eq!(accent_index_in("a", 0), 0);
        assert_eq!(accent_index_in("a", 1), 0);
        for buckets in [2usize, 3, 5, 8, 100] {
            assert!(accent_index_in("a", buckets) < buckets);
        }
        // The value for a real bucket count is still Python's.
        assert_eq!(accent_index_in("a", 3), 1);
    }

    #[test]
    fn seeds_that_look_alike_do_not_share_a_shade_by_accident() {
        // Guards against a stand-in that keys off the *shape* of the string
        // rather than its hash — the view's placeholder did exactly that before
        // it was wired up, and its own test caught it. The seeds here are
        // deliberately pairwise different in length as well as content, so a
        // `len() % buckets` implementation cannot pass.
        let seeds = ["", "a", "bb", "ccc", "game-a", "a-very-long-id"];
        let buckets: Vec<usize> = seeds.iter().map(|seed| accent_index(seed)).collect();
        let mut distinct = buckets.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() > 1,
            "every seed got the same bucket ({buckets:?}) — this is not hashing"
        );
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gh-covers-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// A real PE carrying one icon (`tests/test_covers.py:24-29`).
    fn windows_executable(path: &Path) -> PathBuf {
        use crate::exe_icons::builders::{build_pe, dib_icon, group_icon, resource_section};
        let payload = dib_icon(32, 0x5A);
        let section = resource_section(
            &[(1, payload.clone())],
            &group_icon(&[(32, payload.len(), 1)]),
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the exe directory");
        }
        std::fs::write(path, build_pe(&section, false, None)).expect("write the fixture");
        path.to_path_buf()
    }

    #[test]
    fn saves_the_executables_icon_into_the_covers_directory() {
        let root = scratch_dir("save");
        let covers = root.join("covers");
        let exe = windows_executable(&root.join("Steam").join("steam.exe"));
        let path = save_exe_icon_to(&exe, "abc123", &covers).expect("a saved icon");
        assert_eq!(path, covers.join("abc123.ico"));
        let saved = std::fs::read(&path).expect("read the saved icon");
        assert!(
            saved.starts_with(b"\x00\x00\x01\x00"),
            "an .ico header, not whatever the exe happened to hold"
        );
        // Saving is atomic: the temporary file is renamed away, not left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&covers)
            .expect("read the covers directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a .tmp file was left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_executable_without_an_icon_is_reported_not_silently_empty() {
        let root = scratch_dir("noicon");
        let covers = root.join("covers");
        let bare = root.join("bare.exe");
        let mut bytes = b"MZ".to_vec();
        bytes.resize(4098, 0);
        std::fs::write(&bare, bytes).expect("write the fixture");
        let error = save_exe_icon_to(&bare, "abc123", &covers).expect_err("no icon to save");
        assert!(
            error.to_string().contains("no icon"),
            "the reference's message, byte for byte: {error}"
        );
        assert_eq!(
            error.to_string(),
            "bare.exe carries no icon to use as a cover"
        );
        assert!(!covers.join("abc123.ico").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_executable_names_itself_in_the_error() {
        // `extract_icon` folds a missing file into `None`, so the error is
        // still `NoIcon` — and it still names the file, as `Path(name).name`
        // does when there is a name to take.
        let root = scratch_dir("missing");
        let error = save_exe_icon_to(&root.join("gone.exe"), "abc123", &root.join("covers"))
            .expect_err("no file, no icon");
        assert_eq!(
            error.to_string(),
            "gone.exe carries no icon to use as a cover"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn initials_match_the_reference() {
        // The vectors the view asserted before the rule moved here
        // (`view/cover.rs`), plus the empty and digit edges from
        // `covers.py:97-104`.
        assert_eq!(initials("Half-Life 2"), "HL");
        assert_eq!(initials("Celeste"), "CE");
        assert_eq!(initials("The Witcher 3: Wild Hunt"), "TW");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("!!"), "?");
        assert_eq!(initials("A"), "A");
        assert_eq!(initials("007"), "00");
        // `é` is not in `[^0-9A-Za-z]`, so it splits rather than joins.
        assert_eq!(initials("Pokémon"), "PM");
    }

    #[test]
    fn titles_normalize_to_word_lists() {
        assert_eq!(normalize_title("Battle.net"), "battle net");
        assert_eq!(normalize_title("Portal 2\u{2122}"), "portal 2");
        assert_eq!(normalize_title("R&D"), "r and d");
        assert_eq!(normalize_title("  A  B  "), "a b");
        assert_eq!(normalize_title(""), "");
        assert_eq!(normalize_title("Café — Über"), "caf ber");
    }

    #[test]
    fn exact_match_is_perfect() {
        assert_eq!(score_title("Half-Life", "Half-Life"), 1.0);
    }

    #[test]
    fn punctuation_and_trademark_do_not_cost_score() {
        assert!(score_title("Portal 2", "Portal 2\u{2122}") > 0.9);
    }

    #[test]
    fn unrelated_titles_score_low() {
        assert!(score_title("Celeste", "Age of Empires") < 0.3);
    }

    #[test]
    fn extra_words_in_the_candidate_cost_score() {
        // A launcher's name inside a longer game title is not a match.
        // (`tests/test_covers.py:42-56`.)
        assert!(score_title("Half-Life", "Half-Life 2") > 0.8);
        assert!(score_title("Steam", "DCS World Steam Edition") < 0.8);
        assert!(score_title("Discord", "Bot Maker For Discord") < 0.8);
        assert!(
            score_title(
                "Battle.net",
                "Mega Man Battle Network Legacy Collection Vol. 1"
            ) < 0.45
        );
        assert!(
            score_title(
                "EA App",
                "Creatry \u{2014} Easy Game Maker & Game Builder App"
            ) < 0.45
        );
    }

    #[test]
    fn scores_match_the_reference_bit_for_bit() {
        // Exact doubles, not inequalities: the arithmetic order matches the
        // reference term for term, so these are what CPython computes too.
        assert_eq!(score_title("Half-Life", "Half-Life 2"), 0.866667);
        assert_eq!(score_title("Steam", "DCS World Steam Edition"), 0.7);
        assert_eq!(score_title("Celeste", "Age of Empires"), 0.0);
        assert_eq!(score_title("", "Half-Life"), 0.0);
    }

    #[test]
    fn six_decimal_rounding_matches_cpython_bit_for_bit() {
        // Every value below was verified against CPython's `round(x, 6)`.
        // `0.0078125` is exactly representable (2^-7) and exactly halfway
        // between two six-decimal grid values: the tie branch, pinned.
        assert_eq!(round6(0.0078125), 0.007812);
        assert_eq!(round6(1.0000005), 1.000001);
        // The double-rounding trap: `1.0000015 * 1e6` is exactly `1000001.5`
        // in doubles, but the true value sits below the decimal midpoint, so
        // the reference rounds down and a multiply-then-round port rounds up.
        assert_eq!(round6(1.0000015), 1.000001);
        assert_eq!(round6(0.8666666666666667), 0.866667);
    }

    #[test]
    fn steam_genres_map_onto_gamehandler_categories() {
        assert_eq!(map_steam_genre(&["Action", "Adventure"]), "Action");
        assert_eq!(map_steam_genre(&["Role-Playing"]), "RPG");
        assert_eq!(map_steam_genre(&["Utilities"]), "Utility");
        assert_eq!(map_steam_genre(&["  ACTION  "]), "Action");
        assert_eq!(map_steam_genre(&["Unknown Genre"]), "Uncategorized");
        let empty: [&str; 0] = [];
        assert_eq!(map_steam_genre(&empty), "Uncategorized");
    }

    fn store_hit(name: &str) -> StoreHit {
        StoreHit {
            appid: 1,
            name: name.to_string(),
            tiny_image: String::new(),
            item_type: String::new(),
        }
    }

    fn store_hits(names: &[&str]) -> Vec<StoreHit> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| StoreHit {
                appid: index as i64 + 1,
                name: (*name).to_string(),
                tiny_image: String::new(),
                item_type: String::new(),
            })
            .collect()
    }

    #[test]
    fn one_word_queries_reject_partial_matches() {
        // Store launchers are not Steam products; no hit beats the wrong hit.
        assert!(
            pick_best_match(
                "Steam",
                &store_hits(&["DCS World Steam Edition", "Steamworld Dig"])
            )
            .is_none()
        );
        assert!(pick_best_match("Discord", &store_hits(&["Bot Maker For Discord"])).is_none());
    }

    #[test]
    fn a_one_word_title_still_matches_its_own_store_page() {
        let hits = store_hits(&["Celeste Classic", "Celeste"]);
        let best = pick_best_match("Celeste", &hits).expect("the exact title");
        assert_eq!(best.name, "Celeste");
    }

    #[test]
    fn multi_word_launcher_names_reject_lookalikes() {
        assert!(
            pick_best_match(
                "Battle.net",
                &store_hits(&["Mega Man Battle Network Legacy Collection Vol. 1"]),
            )
            .is_none()
        );
        assert!(
            pick_best_match(
                "EA App",
                &store_hits(&["Creatry \u{2014} Easy Game Maker & Game Builder App"]),
            )
            .is_none()
        );
    }

    #[test]
    fn the_exact_title_wins_over_a_longer_superset() {
        // Ranking must not just take whichever hit the store listed first.
        let hits = store_hits(&["Portal 2 Sixense Perceptual Pack", "Portal 2"]);
        let best = pick_best_match("Portal 2", &hits).expect("the exact title");
        assert_eq!(best.name, "Portal 2");
    }

    #[test]
    fn non_app_hits_are_skipped() {
        let mut music = store_hit("Half-Life");
        music.item_type = "music".to_string();
        let mut game = store_hit("Half-Life 2");
        game.item_type = "game".to_string();
        let binding = [music];
        assert!(pick_best_match("Half-Life", &binding).is_none());
        let binding = [game];
        let best = pick_best_match("Half-Life", &binding).expect("games pass the filter");
        assert_eq!(best.name, "Half-Life 2");
    }

    #[test]
    fn store_search_payloads_normalize_to_hits() {
        let payload = serde_json::json!({
            "total": 2,
            "items": [
                {"id": 70, "name": "Half-Life", "tiny_image": "https://example/tiny.jpg", "type": "app"},
                {"id": 220, "name": "Half-Life 2", "tiny_image": "", "type": "app"},
                {"id": 1, "name": "Something else", "type": "music"},
            ],
        });
        let items = parse_store_search(&payload);
        assert_eq!(
            items.iter().map(|item| item.appid).collect::<Vec<_>>(),
            [70, 220, 1]
        );
        // The type filter lives in the picker, not the parser.
        let best = pick_best_match("Half-Life", &items).expect("a match");
        assert_eq!(best.appid, 70);
    }

    #[test]
    fn nameless_and_idless_hits_are_dropped() {
        let payload = serde_json::json!({
            "items": [
                {"id": 70, "name": "Half-Life"},
                {"id": 0, "name": "Zero"},
                {"id": 71, "name": ""},
                {"id": null, "name": "Null"},
                {"name": "Missing"},
                {"id": "abc", "name": "Unparseable"},
                {"id": "72", "name": "String id"},
            ],
        });
        let items = parse_store_search(&payload);
        assert_eq!(
            items.iter().map(|item| item.appid).collect::<Vec<_>>(),
            [70, 72]
        );
    }

    #[test]
    fn cover_urls_try_every_root_per_asset_with_tiny_last() {
        let urls = cover_urls_for_app(70, "https://example/tiny.jpg");
        assert_eq!(urls.len(), COVER_ASSETS.len() * CDN_ROOTS.len() + 1);
        assert_eq!(
            urls[0],
            "https://cdn.akamai.steamstatic.com/steam/apps/70/library_600x900.jpg"
        );
        // The inner loop is the root: the second URL is the same asset
        // elsewhere, not the next asset.
        assert_eq!(
            urls[1],
            "https://cdn.cloudflare.steamstatic.com/steam/apps/70/library_600x900.jpg"
        );
        assert!(urls.iter().any(|url| url.contains("library_600x900.jpg")));
        assert_eq!(
            urls.last().map(String::as_str),
            Some("https://example/tiny.jpg")
        );
        assert!(
            !cover_urls_for_app(70, "").iter().any(|url| url.is_empty()),
            "no tiny image, no empty URL"
        );
    }

    use crate::runners::proton::ResponseHead;
    use std::cell::{Cell, RefCell};

    /// What one routed URL answers.
    enum FakeRoute {
        /// A body plus the `Content-Length` the head declares for it.
        Body(Vec<u8>, Option<String>),
        /// A transport failure that never reaches the callbacks.
        Fail(String),
        /// An 8-bit pattern streamed forever, until the sink aborts.
        Endless(u8),
    }

    /// An [`HttpClient`] routed by URL substring, recording every request.
    ///
    /// Routes match in order on `url.contains(key)`; an unrouted URL fails the
    /// transfer rather than hanging it, which is what keeps a test that
    /// requests the wrong URL loud instead of slow.
    /// One recorded request: URL, headers, and timeout.
    type SeenRequest = (String, Vec<(String, String)>, Duration);

    struct FakeClient {
        routes: Vec<(String, FakeRoute)>,
        seen: RefCell<Vec<SeenRequest>>,
        delivered: Cell<usize>,
    }

    impl FakeClient {
        fn new(routes: Vec<(&str, FakeRoute)>) -> Self {
            Self {
                routes: routes
                    .into_iter()
                    .map(|(key, route)| (key.to_string(), route))
                    .collect(),
                seen: RefCell::new(Vec::new()),
                delivered: Cell::new(0),
            }
        }

        fn requests(&self) -> Vec<SeenRequest> {
            self.seen.borrow().clone()
        }

        fn urls(&self) -> Vec<String> {
            self.requests().into_iter().map(|(url, _, _)| url).collect()
        }
    }

    impl HttpClient for FakeClient {
        fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            self.seen.borrow_mut().push((
                url.to_string(),
                headers
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                    .collect(),
                timeout,
            ));
            let route = self.routes.iter().find(|(key, _)| url.contains(key));
            match route {
                None => Err(RunnerError::Http {
                    message: format!("no route for {url}"),
                }),
                Some((_, FakeRoute::Fail(message))) => Err(RunnerError::Http {
                    message: message.clone(),
                }),
                Some((_, FakeRoute::Body(body, declared))) => {
                    on_head(&ResponseHead {
                        content_length: declared.clone(),
                        final_url: url.to_string(),
                    })?;
                    sink(body)?;
                    self.delivered.set(self.delivered.get() + body.len());
                    Ok(())
                }
                Some((_, FakeRoute::Endless(fill))) => {
                    on_head(&ResponseHead {
                        content_length: None,
                        final_url: url.to_string(),
                    })?;
                    // The sink's `Err` is the abort; the cap below only stops
                    // a client that ignored it from hanging the test, and the
                    // delivered count then fails the assertion loudly.
                    for _ in 0..(64 * 1024 * 1024 / 4096) {
                        sink(&[*fill; 4096])?;
                        self.delivered.set(self.delivered.get() + 4096);
                    }
                    Ok(())
                }
            }
        }
    }

    const TIMEOUT: Duration = Duration::from_secs(20);

    fn search_payload() -> Vec<u8> {
        serde_json::json!({
            "total": 2,
            "items": [
                {"id": 70, "name": "Half-Life", "tiny_image": "https://example/tiny.jpg", "type": "app"},
                {"id": 220, "name": "Half-Life 2", "tiny_image": "", "type": "app"},
            ],
        })
        .to_string()
        .into_bytes()
    }

    fn details_payload() -> Vec<u8> {
        serde_json::json!({
            "70": {
                "success": true,
                "data": {
                    "name": "Half-Life",
                    "header_image": "https://example/header.jpg",
                    "genres": [{"description": "Action"}, {"description": "Adventure"}],
                },
            },
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn search_sends_an_encoded_query_and_parses_the_hits() {
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(search_payload(), None),
        )]);
        let items = search_steam(&client, "Half-Life 2 &Co", TIMEOUT).expect("parsed hits");
        assert_eq!(
            items.iter().map(|item| item.appid).collect::<Vec<_>>(),
            [70, 220]
        );
        let urls = client.urls();
        assert_eq!(urls.len(), 1);
        assert!(
            urls[0].starts_with(&format!("{STORE_SEARCH_URL}?term=")),
            "the search endpoint with a term: {}",
            urls[0]
        );
        assert!(
            urls[0].contains("term=Half-Life+2+%26Co"),
            "quote_plus encoding: {}",
            urls[0]
        );
        assert!(
            urls[0].ends_with("l=english&cc=US"),
            "locale params: {}",
            urls[0]
        );
        let (_, headers, timeout) = &client.requests()[0];
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "User-Agent" && value == "GameHandler"),
            "the GameHandler user agent rides every request: {headers:?}"
        );
        assert_eq!(*timeout, TIMEOUT);
    }

    #[test]
    fn search_treats_a_non_dict_payload_as_no_hits() {
        for body in [b"[1, 2]".to_vec(), b"null".to_vec(), b"\"x\"".to_vec()] {
            let client = FakeClient::new(vec![("storesearch", FakeRoute::Body(body, None))]);
            assert_eq!(
                search_steam(&client, "Half-Life", TIMEOUT).expect("no hits"),
                []
            );
        }
    }

    #[test]
    fn search_reports_undecodable_and_malformed_payloads() {
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(b"{not json".to_vec(), None),
        )]);
        assert!(matches!(
            search_steam(&client, "Half-Life", TIMEOUT),
            Err(CoverError::InvalidResponse { .. })
        ));
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(vec![0xFF, 0xFE], None),
        )]);
        assert!(matches!(
            search_steam(&client, "Half-Life", TIMEOUT),
            Err(CoverError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn details_read_name_header_and_category() {
        let client = FakeClient::new(vec![(
            "appdetails",
            FakeRoute::Body(details_payload(), None),
        )]);
        let details = fetch_app_details(&client, 70, TIMEOUT).expect("parsed details");
        assert_eq!(details.name, "Half-Life");
        assert_eq!(details.header_image, "https://example/header.jpg");
        assert_eq!(details.genres, ["Action", "Adventure"]);
        assert_eq!(details.category, "Action");
        assert!(
            client.urls()[0].contains("appids=70"),
            "{}",
            client.urls()[0]
        );
    }

    #[test]
    fn details_degrade_to_empty_when_the_entry_is_missing_or_failed() {
        for body in [
            serde_json::json!({"70": {"success": false}})
                .to_string()
                .into_bytes(),
            serde_json::json!({"71": {"success": true, "data": {}}})
                .to_string()
                .into_bytes(),
            serde_json::json!({"70": {"success": true}})
                .to_string()
                .into_bytes(),
        ] {
            let client = FakeClient::new(vec![("appdetails", FakeRoute::Body(body, None))]);
            let details = fetch_app_details(&client, 70, TIMEOUT).expect("tolerated");
            assert_eq!(details, AppDetails::default());
        }
    }

    #[test]
    fn downloads_refuse_short_bodies_and_endless_streams() {
        let root = scratch_dir("download");
        // An error page, not artwork.
        let client = FakeClient::new(vec![("cdn", FakeRoute::Body(vec![0u8; 512], None))]);
        let error = download_image(&client, "https://cdn/x.jpg", &root.join("x.jpg"), TIMEOUT)
            .expect_err("too short");
        assert_eq!(error.to_string(), "Cover download was empty");
        assert!(!root.join("x.jpg").exists());
        assert!(!root.join("x.jpg.tmp").exists());
        // A stream without end is cut off at the cap, not buffered.
        let client = FakeClient::new(vec![("cdn", FakeRoute::Endless(0xAB))]);
        let error = download_image(&client, "https://cdn/x.jpg", &root.join("y.jpg"), TIMEOUT)
            .expect_err("over the cap");
        assert_eq!(
            error.to_string(),
            format!("Response from https://cdn/x.jpg exceeded {MAX_RESPONSE_BYTES} bytes")
        );
        assert!(
            client.delivered.get() <= MAX_RESPONSE_BYTES + 4096,
            "aborted at the cap, delivered {}",
            client.delivered.get()
        );
        assert!(!root.join("y.jpg").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn downloads_write_atomically_and_ignore_a_lying_length() {
        let root = scratch_dir("atomic");
        let body = vec![7u8; 2048];
        // The head declares a gigabyte; the body is two kilobytes. The
        // transfer succeeds, because the cap is enforced on bytes received —
        // the reference reads `limit + 1` rather than trusting the header.
        let client = FakeClient::new(vec![(
            "cdn",
            FakeRoute::Body(body.clone(), Some("1073741824".to_string())),
        )]);
        let path = download_image(&client, "https://cdn/x.jpg", &root.join("x.jpg"), TIMEOUT)
            .expect("saved");
        assert_eq!(std::fs::read(&path).expect("read back"), body);
        assert!(
            !root.join("x.jpg.tmp").exists(),
            "renamed away, not left behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn url_lists_fall_through_to_the_first_reachable_cover() {
        let root = scratch_dir("urls");
        let covers = root.join("covers");
        let body = vec![9u8; 2048];
        let client = FakeClient::new(vec![
            ("first", FakeRoute::Fail("connection reset".to_string())),
            ("second", FakeRoute::Body(vec![0u8; 10], None)),
            ("third", FakeRoute::Body(body.clone(), None)),
        ]);
        let (path, source) = save_cover_from_urls(
            &client,
            &[
                "https://cdn/first.jpg".to_string(),
                "https://cdn/second.jpg".to_string(),
                "https://cdn/third.jpg".to_string(),
            ],
            "abc123",
            &covers,
            TIMEOUT,
        )
        .expect("the third URL works");
        assert_eq!(path, covers.join("abc123.jpg"));
        assert_eq!(source, "https://cdn/third.jpg");
        assert_eq!(std::fs::read(&path).expect("read back"), body);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn url_lists_report_the_last_failure_or_no_urls() {
        let root = scratch_dir("urlfail");
        let client = FakeClient::new(vec![("cdn", FakeRoute::Fail("boom".to_string()))]);
        let error = save_cover_from_urls(
            &client,
            &[
                "https://cdn/a.jpg".to_string(),
                "https://cdn/b.jpg".to_string(),
            ],
            "abc123",
            &root,
            TIMEOUT,
        )
        .expect_err("nothing reachable");
        assert_eq!(error.to_string(), "Could not download a cover: boom");
        let error = save_cover_from_urls(&client, &[], "abc123", &root, TIMEOUT)
            .expect_err("nothing to try");
        assert_eq!(
            error.to_string(),
            "Could not download a cover: No cover URLs"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn custom_covers_keep_image_suffixes_and_reject_missing_sources() {
        let root = scratch_dir("custom");
        let covers = root.join("covers");
        let source = root.join("art.png");
        std::fs::write(&source, b"\x89PNG fake").expect("write the source");
        let dest = copy_custom_cover(&source, "abc123", &covers).expect("copied");
        assert_eq!(dest, covers.join("abc123.png"));
        assert_eq!(
            std::fs::read(&dest).expect("read back"),
            std::fs::read(&source).expect("read source")
        );
        let odd = root.join("art.bmp");
        std::fs::write(&odd, b"BM fake").expect("write the source");
        let dest = copy_custom_cover(&odd, "def456", &covers).expect("copied as jpg");
        assert_eq!(dest, covers.join("def456.jpg"));
        let error = copy_custom_cover(&root.join("gone.png"), "abc123", &covers)
            .expect_err("missing source");
        assert_eq!(error.to_string(), root.join("gone.png").to_string_lossy());
        assert!(matches!(error, CoverError::Io(_)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_game_id_that_can_traverse_is_refused_by_every_cover_write() {
        // `SECURITY.md` SEC-04. The id is loaded verbatim from a `games.json`
        // entry — untrusted input — and every cover write builds its destination
        // by interpolating it. Before this check existed all three sites below
        // wrote outside `covers_dir`: with the id used here the icon landed at
        // `<root>/escape.ico` and the custom cover at `<root>/escape.png`, both
        // one level above the directory the caller named.
        //
        // `scratch_dir` returns the directory whose *sibling* the traversal
        // reaches, so the assertion is on an absolute path the write would
        // otherwise have created — not merely on the error variant. A test that
        // only checked `matches!(error, UnsafeId { .. })` would pass against an
        // implementation that refused everything and wrote nothing for any id,
        // which is why the accepted cases below are in the same test.
        let root = scratch_dir("traversal");
        let covers = root.join("covers");
        let escaped = root.join("escape");
        let id = "../escape";

        let exe = windows_executable(&root.join("app.exe"));
        let error = save_exe_icon_to(&exe, id, &covers).expect_err("an escaping id");
        assert!(matches!(error, SaveIconError::UnsafeId { .. }));
        assert!(
            error.to_string().contains("path separator"),
            "the refusal must say why: {error}"
        );
        assert!(!escaped.with_extension("ico").exists());
        assert!(!covers.join(format!("{id}.ico")).exists());
        // The `.ico.tmp` sibling `save_exe_icon_to` writes beside its target is
        // the other half of this: refusing after the write would still leave
        // that file at the traversed location.
        assert!(!escaped.with_extension("ico.tmp").exists());

        let mut client = FakeClient::new(vec![]);
        let error = save_cover_from_urls(
            &client,
            &["https://cdn/a.jpg".to_string()],
            id,
            &covers,
            TIMEOUT,
        )
        .expect_err("an escaping id");
        assert!(matches!(error, CoverError::UnsafeId { .. }));
        assert!(!escaped.with_extension("jpg").exists());
        // The check runs before the transfer, so a doomed id does not reach the
        // network. An empty route table fails every transfer, so a refusal that
        // happened *after* the loop would surface as `DownloadFailed`.
        assert_eq!(
            client.urls(),
            Vec::<String>::new(),
            "no request may be made for an id that cannot be written"
        );

        let source = root.join("art.png");
        std::fs::write(&source, b"\x89PNG fake").expect("write the source");
        let error = copy_custom_cover(&source, id, &covers).expect_err("an escaping id");
        assert!(matches!(error, CoverError::UnsafeId { .. }));
        assert!(!escaped.with_extension("png").exists());

        // A backslash is an ordinary character in a Unix filename and still
        // refused, so that the rule matches `safe_install_id` and a library
        // carried between machines cannot change meaning.
        let error = copy_custom_cover(&source, "..\\escape", &covers).expect_err("a backslash");
        assert!(matches!(error, CoverError::UnsafeId { .. }));
        let _ = &mut client;

        // The other half of the rule, and the reason it is one condition rather
        // than a sanitiser: `..` on its own is *not* a traversal, because the
        // id is always followed by `.` and an extension. Refusing it would
        // refuse a filename the reference picks happily, and rewriting it would
        // rename every existing user's covers.
        let dotdot = save_exe_icon_to(&exe, "..", &covers).expect("`..` is a legal stem here");
        assert_eq!(dotdot, covers.join("...ico"));
        let empty = save_exe_icon_to(&exe, "", &covers).expect("the empty id is a legal stem here");
        assert_eq!(empty, covers.join(".ico"));
        // A real id, minted by `models::new_id`, is 32 lowercase hex characters
        // and passes untouched — the value is not rewritten on the way through.
        let real = crate::models::new_id();
        let path = save_exe_icon_to(&exe, &real, &covers).expect("a real id");
        assert_eq!(path, covers.join(format!("{real}.ico")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn icon_covers_carry_the_exe_name_when_no_name_is_given() {
        let root = scratch_dir("iconcover");
        let covers = root.join("covers");
        let exe = windows_executable(&root.join("app.exe"));
        let hit = icon_cover("Battle.net", &exe, "abc123", &covers).expect("an icon hit");
        assert_eq!(hit.source, ICON_SOURCE);
        assert_eq!(hit.origin_label(), "the app icon");
        assert_eq!(hit.appid, 0);
        assert_eq!(hit.name, "Battle.net");
        assert!(hit.cover_path.is_file());
        let unnamed = icon_cover("", &exe, "def456", &covers).expect("a stem-named hit");
        assert_eq!(unnamed.name, "app");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn steam_covers_prefer_the_header_and_report_the_hit() {
        let root = scratch_dir("steam");
        let covers = root.join("covers");
        let body = vec![3u8; 2048];
        let client = FakeClient::new(vec![
            ("storesearch", FakeRoute::Body(search_payload(), None)),
            ("appdetails", FakeRoute::Body(details_payload(), None)),
            ("example/header.jpg", FakeRoute::Body(body.clone(), None)),
            ("steamstatic", FakeRoute::Fail("unreachable".to_string())),
            ("akamaihd", FakeRoute::Fail("unreachable".to_string())),
            ("example/tiny", FakeRoute::Fail("unreachable".to_string())),
        ]);
        let hit = steam_cover(&client, "Half-Life", "abc123", &covers, TIMEOUT).expect("a hit");
        assert_eq!(hit.appid, 70);
        assert_eq!(hit.name, "Half-Life");
        assert_eq!(hit.category, "Action");
        assert_eq!(hit.cover_path, covers.join("abc123.jpg"));
        assert_eq!(hit.source_url, "https://example/header.jpg");
        assert_eq!(hit.source, STEAM_SOURCE);
        assert_eq!(hit.origin_label(), "Steam");
        let urls = client.urls();
        assert_eq!(
            urls[2], "https://example/header.jpg",
            "the header jumps the queue: {urls:?}"
        );
        assert_eq!(std::fs::read(&hit.cover_path).expect("read back"), body);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn steam_covers_without_a_match_name_the_query() {
        let root = scratch_dir("nomatch");
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(search_payload(), None),
        )]);
        let error = steam_cover(&client, "Some Unknown Game", "abc123", &root, TIMEOUT)
            .expect_err("no match");
        assert_eq!(
            error.to_string(),
            "No Steam cover found for \u{201c}Some Unknown Game\u{201d}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn steam_covers_tolerate_unreadable_details() {
        let root = scratch_dir("nodetails");
        let covers = root.join("covers");
        let client = FakeClient::new(vec![
            ("storesearch", FakeRoute::Body(search_payload(), None)),
            ("appdetails", FakeRoute::Fail("500".to_string())),
            ("steamstatic", FakeRoute::Body(vec![5u8; 2048], None)),
        ]);
        let hit = steam_cover(&client, "Half-Life", "abc123", &covers, TIMEOUT).expect("a hit");
        // The match's own name and an uncategorized shelf, as `{}` degrades.
        assert_eq!(hit.name, "Half-Life");
        assert_eq!(hit.category, "Uncategorized");
        assert!(hit.source_url.contains("steamstatic"), "{}", hit.source_url);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_cover_prefers_steam_artwork_when_it_exists() {
        let root = scratch_dir("fetchsteam");
        let covers = root.join("covers");
        let exe = windows_executable(&root.join("app.exe"));
        let client = FakeClient::new(vec![
            ("storesearch", FakeRoute::Body(search_payload(), None)),
            ("appdetails", FakeRoute::Body(details_payload(), None)),
            ("example/header.jpg", FakeRoute::Body(vec![6u8; 2048], None)),
        ]);
        let hit = fetch_cover(&client, "Half-Life", "abc123", Some(&exe), &covers, TIMEOUT)
            .expect("a steam hit");
        assert_eq!(hit.source, STEAM_SOURCE);
        assert_eq!(hit.origin_label(), "Steam");
        assert!(
            !covers.join("abc123.ico").exists(),
            "the exe was never read"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_cover_falls_back_to_the_icon_when_steam_has_nothing() {
        let root = scratch_dir("fetchicon");
        let covers = root.join("covers");
        let exe = windows_executable(&root.join("app.exe"));
        let client = FakeClient::new(vec![("storesearch", FakeRoute::Fail("dns".to_string()))]);
        let hit = fetch_cover(&client, "EA App", "abc123", Some(&exe), &covers, TIMEOUT)
            .expect("an icon hit");
        assert_eq!(hit.source, ICON_SOURCE);
        assert!(hit.cover_path.is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_neither_source_the_steam_failure_is_what_surfaces() {
        let root = scratch_dir("fetchfail");
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(search_payload(), None),
        )]);
        let error = fetch_cover(
            &client,
            "X",
            "abc123",
            Some(&root.join("missing.exe")),
            &root,
            TIMEOUT,
        )
        .expect_err("nothing to fall back to");
        assert_eq!(
            error.to_string(),
            "No Steam cover found for \u{201c}X\u{201d}"
        );
        // A bare executable (no icon) fails the fallback the same way: the
        // icon error is swallowed and the steam message surfaces. Kept as the
        // reference has it, and as the divergence note on `fetch_cover`
        // argues: nothing was found *and* nothing failed, so there is nothing
        // for the user to act on. The write failure is the half that does not
        // stay silent — `a_cover_write_failure_is_not_reported_as_a_steam_...`.
        let bare = root.join("bare.exe");
        std::fs::write(&bare, b"MZ").expect("write the fixture");
        let error = fetch_cover(&client, "X", "abc123", Some(&bare), &root, TIMEOUT)
            .expect_err("a bare exe is no fallback");
        assert_eq!(
            error.to_string(),
            "No Steam cover found for \u{201c}X\u{201d}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cover_write_failure_is_not_reported_as_a_steam_search_failure() {
        // The covers directory is an ordinary *file*, so `create_dir_all`
        // fails with `EEXIST` whatever the process umask and whatever uid the
        // suite runs as — a `0500` directory would be writable to a root
        // runner and this fixture would then pass vacuously.
        let root = scratch_dir("fetchwritefail");
        let covers = root.join("covers");
        std::fs::write(&covers, b"not a directory").expect("write the blocker");
        let exe = windows_executable(&root.join("app.exe"));
        // The search succeeds and matches nothing, so Steam's own failure is a
        // real message rather than "no route": both halves of the outcome are
        // observable.
        let client = FakeClient::new(vec![(
            "storesearch",
            FakeRoute::Body(search_payload(), None),
        )]);
        let error = fetch_cover(&client, "X", "abc123", Some(&exe), &covers, TIMEOUT)
            .expect_err("the icon was found and could not be saved");
        let message = error.to_string();

        let CoverError::IconWriteFailed {
            steam_error,
            covers_dir,
            icon_error,
        } = &error
        else {
            panic!("a cover *write* failure was reported as: {message}");
        };
        assert_eq!(steam_error, "No Steam cover found for \u{201c}X\u{201d}");
        assert_eq!(covers_dir, &covers);
        assert!(
            matches!(icon_error, SaveIconError::Io(_)),
            "a directory that cannot be created is an io failure, not a missing icon: {icon_error}"
        );
        // The rendered message carries both, and the directory the user has to
        // fix — the `io::Error` alone names neither the path nor the cause.
        let SaveIconError::Io(io) = icon_error else {
            unreachable!("asserted above");
        };
        assert!(
            message.contains("No Steam cover found for \u{201c}X\u{201d}"),
            "the Steam failure is still named: {message}"
        );
        assert!(
            message.contains(&covers.display().to_string()),
            "the directory the icon could not be written to is named: {message}"
        );
        assert!(
            message.contains(&io.to_string()),
            "the io failure itself is named, not just its absence: {message}"
        );

        // The paired case, differing only in whether the write can succeed:
        // the same fixture with a real directory falls back to the icon. This
        // is what makes the assertion above about the *write* and not about
        // the exe.
        let saved_root = scratch_dir("fetchwriteok");
        let saved_covers = saved_root.join("covers");
        let saved_exe = windows_executable(&saved_root.join("app.exe"));
        let hit = fetch_cover(
            &client,
            "X",
            "abc123",
            Some(&saved_exe),
            &saved_covers,
            TIMEOUT,
        )
        .expect("a writable covers directory takes the icon");
        assert_eq!(hit.source, ICON_SOURCE);
        assert!(hit.cover_path.is_file());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&saved_root);
    }
}
