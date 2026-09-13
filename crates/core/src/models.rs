//! The game library: `Game`, `Library` and `format_last_played`.
//!
//! A line-for-line port of `gamehandler/models.py`. Three things about it are
//! compatibility contracts rather than implementation details, and all three
//! are pinned by fixtures under `docs/migration/oracle/fixtures/`:
//!
//! * **Field order.** `Game`'s 31 fields are declared in the same order as the
//!   Python dataclass, and [`Game`]'s `Serialize` impl emits them in that
//!   order. `dataclasses.asdict` and `serde` both preserve declaration order,
//!   so reordering the struct silently breaks byte equality with a file the
//!   Python app wrote even when every value is correct.
//! * **Lenient numbers.** Timestamps are read through [`crate::json`]'s lenient
//!   parser, and normalized by the same table as `Game.from_dict`
//!   (`docs/migration/oracle/FINDINGS.md` F-C, F-G; DECISIONS D-06/D-16).
//! * **Tolerant types.** Only `added` and `last_played` are validated by
//!   Python's `from_dict`; every other field is copied into the dataclass
//!   unchecked, because a dataclass does not enforce its annotations. A
//!   `"name": 123` therefore loads fine and then takes the **whole library
//!   view** down — all three sorts raise `AttributeError: 'int' object has no
//!   attribute 'lower'`, including the default `sort="name"`, with no setting
//!   change and no user action. This port has real types, so every field is
//!   converted on the way in and a `Game` never holds a value that its own
//!   accessors cannot read.
//!
//! # What "tolerant" means here, precisely
//!
//! * a **string** field copies a JSON string verbatim;
//! * a **number** or **bool** where a string belongs is rendered as its JSON
//!   text (`123` → `"123"`), so the game stays in the library and stays
//!   visible. Python would keep the number and then crash on it; dropping the
//!   entry instead would be silent data loss, which is worse than either;
//! * `null`, an array or an object where a string belongs keeps the field's
//!   default. `null` is what D-14 already treats as "no information"; a list is
//!   not a name;
//! * a **bool** field keeps its default unless the value is a JSON boolean;
//! * the integer `steam_appid` keeps its default unless the value can be
//!   represented as an `i64` — the one place where the typed port genuinely
//!   cannot hold what Python could, since Python's `int` is arbitrary
//!   precision.
//!
//! None of this is reachable from a file the app itself wrote, and none of it
//! changes a single byte for well-formed input.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use serde::ser::{Serialize, SerializeStruct, Serializer};
use serde_json::{Map, Value};

use crate::json::{self, PersistenceError};
use crate::paths;

// Case folds performed by `folded`, counted for the PERF-05 tests.
//
// The property those tests pin — that a sort folds each name once rather than
// once per comparison, and that a frame after the first folds nothing at all —
// is invisible from outside: every version of `all` and `search` returns the
// *same* rows in the *same* order and only costs more. The fold is where the
// `String` is allocated, so counting it counts the allocations the finding is
// about. It is `thread_local` rather than a global because `cargo test` runs
// cases concurrently — a shared counter would be incremented by whichever other
// test happened to be sorting at that moment.
//
// `#[cfg(test)]` on the counter *and* on the increment keeps this out of a
// production build entirely, rather than behind a branch that always runs.
#[cfg(test)]
thread_local! {
    static FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Read the fold counter, and reset it when `reset` is set.
///
/// One function rather than a `with` at each test, because the pair is always
/// used together — a count read without a reset is a count of whatever the rest
/// of the suite did first.
#[cfg(test)]
fn folds(reset: bool) -> usize {
    FOLDS.with(|folds| {
        let seen = folds.get();
        if reset {
            folds.set(0);
        }
        seen
    })
}

/// The case-folded form of a name or a category.
///
/// Python's `str.lower()`, which is what `models.py:156-203` sorts and matches
/// on. This is the one place in the module that folds, and routing every fold
/// through it is what makes the work countable — see [`FOLDS`].
fn folded(text: &str) -> String {
    #[cfg(test)]
    FOLDS.with(|folds| folds.set(folds.get() + 1));
    text.to_lowercase()
}

/// A timestamp with its two zeros fused into one, for the two timestamp sorts.
///
/// `f64::total_cmp` is a *total* order and distinguishes `-0.0` from `0.0` — it
/// compares the bit patterns, where IEEE `==` says the two are equal. Python's
/// key is `-g.last_played`, compared with `<`, so it does not distinguish them,
/// and two games whose timestamps tie fall through to the name tie-break there.
/// Here they did not: `total_cmp` ordered them by the sign of a zero and the
/// name was never consulted (`BUG-20`).
///
/// `-0.0` is reachable rather than theoretical. [`timestamp`] admits it —
/// `numeric >= 0.0` is true for `-0.0` — so a hand-edited or third-party
/// `games.json` that spells a timestamp `-0.0` loads with the sign intact. The
/// app itself never writes one (`mark_played` uses the clock), which is why
/// this went unnoticed rather than why it is harmless: the Python app reads the
/// same file, and the two must present one order.
///
/// Negating instead — the literal transcription of `-g.last_played` — would not
/// fix it: `-0.0` negated is `0.0` and `0.0` negated is `-0.0`, so the fold
/// moves the distinction to the other side of the comparison rather than
/// removing it.
fn zero_normalised(timestamp: f64) -> f64 {
    if timestamp == 0.0 { 0.0 } else { timestamp }
}

/// Shown for a game with no category. `models.py:15`.
pub const UNCATEGORIZED: &str = "Uncategorized";

/// The library sort modes. `models.py:16`.
pub const SORT_MODES: [&str; 3] = ["name", "recent", "added"];

/// The built-in Wine runner id.
///
/// Python defines this once in `runners.py:39` and hardcodes the literal in
/// `models.py:25`, which leaves two statements of the same fact that can drift.
/// Here it is defined once and re-exported from `crate::runners` (T-03) so
/// that module's public surface still matches `runners.__all__`.
pub const SYSTEM_WINE: &str = "wine-system";

/// Seconds since the Unix epoch, matching Python's `time.time()`.
///
/// Negative before 1970, as in Python, rather than saturating at zero.
pub fn now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(error) => -error.duration().as_secs_f64(),
    }
}

/// One library entry, mirroring the `Game` dataclass (`models.py:20-53`).
///
/// The field order is the wire format. Do not reorder.
#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub name: String,
    pub exe_path: String,
    pub runner: String,
    pub prefix_path: String,
    pub arguments: String,
    pub cover_path: String,
    pub category: String,
    pub steam_appid: i64,
    pub kind: String,
    pub working_directory: String,
    pub additional_app: String,
    pub mangohud: bool,
    pub gamemode: bool,
    pub prefer_sdl: bool,
    pub wayland: bool,
    pub hdr: bool,
    pub esync: bool,
    pub fsync: bool,
    pub dxvk: bool,
    pub vkd3d: bool,
    pub nvapi: bool,
    pub fsr: bool,
    pub battleye: bool,
    pub eac: bool,
    pub gamescope: bool,
    pub virtual_desktop: bool,
    pub virtual_desktop_size: String,
    pub environment: String,
    pub id: String,
    pub added: f64,
    pub last_played: f64,
}

impl Game {
    /// The dataclass defaults, with neither `id` nor `added` generated.
    ///
    /// [`Self::default`] fills those two in; `from_dict` needs the id and the
    /// timestamp decided *before* the rest is copied, so it starts here.
    fn blank() -> Self {
        Self {
            name: String::new(),
            exe_path: String::new(),
            runner: SYSTEM_WINE.to_string(),
            prefix_path: String::new(),
            arguments: String::new(),
            cover_path: String::new(),
            category: UNCATEGORIZED.to_string(),
            steam_appid: 0,
            kind: "windows".to_string(),
            working_directory: String::new(),
            additional_app: String::new(),
            mangohud: false,
            gamemode: false,
            prefer_sdl: false,
            wayland: false,
            hdr: false,
            esync: true,
            fsync: true,
            dxvk: true,
            vkd3d: true,
            nvapi: false,
            fsr: false,
            battleye: true,
            eac: true,
            gamescope: false,
            virtual_desktop: false,
            virtual_desktop_size: "1920x1080".to_string(),
            environment: String::new(),
            id: String::new(),
            added: 0.0,
            last_played: 0.0,
        }
    }

    /// A new game with Python's `default_factory` values: a fresh id and
    /// `added` set to the current time.
    pub fn new_named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: new_id(),
            added: now(),
            ..Self::blank()
        }
    }

    /// Port of `Game.is_linux` (`models.py:55-57`).
    pub fn is_linux(&self) -> bool {
        self.kind == "linux"
    }

    /// Port of `Game.display_category` (`models.py:59-62`): the category shown
    /// and filtered on, with blanks folded into [`UNCATEGORIZED`].
    pub fn display_category(&self) -> &str {
        let trimmed = self.category.trim();
        if trimmed.is_empty() {
            UNCATEGORIZED
        } else {
            trimmed
        }
    }

    /// Port of `Game.from_dict` (`models.py:64-87`), using the system clock.
    pub fn from_dict(data: &Map<String, Value>) -> Self {
        Self::from_dict_at(data, now())
    }

    /// [`Self::from_dict`] with the clock injected.
    ///
    /// `now` is used only for a regenerated `added`; the oracle fixtures freeze
    /// it so their expected bytes are reproducible.
    pub fn from_dict_at(data: &Map<String, Value>, now: f64) -> Self {
        let mut game = Self {
            id: text(data, "id").unwrap_or_else(new_id),
            added: now,
            ..Self::blank()
        };

        // Strings, via the conversion table in the module docs. Python copies
        // whatever it finds — `"name": 123` is stored as the integer 123 and
        // then breaks every sort, `"category": 42` breaks `display_category`.
        // Converting here is what keeps a `Game` readable by its own accessors.
        if let Some(value) = text(data, "name") {
            game.name = value;
        }
        if let Some(value) = text(data, "exe_path") {
            game.exe_path = value;
        }
        if let Some(value) = text(data, "runner") {
            game.runner = value;
        }
        if let Some(value) = text(data, "prefix_path") {
            game.prefix_path = value;
        }
        if let Some(value) = text(data, "arguments") {
            game.arguments = value;
        }
        if let Some(value) = text(data, "cover_path") {
            game.cover_path = value;
        }
        if let Some(value) = text(data, "category") {
            game.category = value;
        }
        if let Some(value) = text(data, "kind") {
            game.kind = value;
        }
        if let Some(value) = text(data, "working_directory") {
            game.working_directory = value;
        }
        if let Some(value) = text(data, "additional_app") {
            game.additional_app = value;
        }
        if let Some(value) = text(data, "virtual_desktop_size") {
            game.virtual_desktop_size = value;
        }
        if let Some(value) = text(data, "environment") {
            game.environment = value;
        }

        if let Some(value) = integer(data, "steam_appid") {
            game.steam_appid = value;
        }

        // The fifteen launch toggles.
        if let Some(value) = flag(data, "mangohud") {
            game.mangohud = value;
        }
        if let Some(value) = flag(data, "gamemode") {
            game.gamemode = value;
        }
        if let Some(value) = flag(data, "prefer_sdl") {
            game.prefer_sdl = value;
        }
        if let Some(value) = flag(data, "wayland") {
            game.wayland = value;
        }
        if let Some(value) = flag(data, "hdr") {
            game.hdr = value;
        }
        if let Some(value) = flag(data, "esync") {
            game.esync = value;
        }
        if let Some(value) = flag(data, "fsync") {
            game.fsync = value;
        }
        if let Some(value) = flag(data, "dxvk") {
            game.dxvk = value;
        }
        if let Some(value) = flag(data, "vkd3d") {
            game.vkd3d = value;
        }
        if let Some(value) = flag(data, "nvapi") {
            game.nvapi = value;
        }
        if let Some(value) = flag(data, "fsr") {
            game.fsr = value;
        }
        if let Some(value) = flag(data, "battleye") {
            game.battleye = value;
        }
        if let Some(value) = flag(data, "eac") {
            game.eac = value;
        }
        if let Some(value) = flag(data, "gamescope") {
            game.gamescope = value;
        }
        if let Some(value) = flag(data, "virtual_desktop") {
            game.virtual_desktop = value;
        }

        // Timestamps. A missing key keeps the dataclass default (`added` is
        // "now", `last_played` is 0.0); a present but invalid one regenerates
        // `added` and zeroes `last_played`.
        //
        // NOTE (DECISIONS D-14): Python's `from_dict` has a bug here. Its
        // `if value is None: continue` is meant to skip missing keys, but for a
        // key explicitly present as `null` it skips normalization *without
        // removing the key*, so the field becomes `None` and
        // `Library::all("recent"/"added")` later raises `TypeError`. This port
        // treats `null` like any other invalid value — which is what the code
        // below does naturally, since `timestamp()` rejects every non-number.
        if let Some(seconds) = data.get("added").and_then(timestamp) {
            game.added = seconds;
        }
        game.last_played = data.get("last_played").and_then(timestamp).unwrap_or(0.0);

        game
    }
}

impl Default for Game {
    fn default() -> Self {
        Self {
            id: new_id(),
            added: now(),
            ..Self::blank()
        }
    }
}

impl Serialize for Game {
    /// Written by hand, and field order is the point: `serde_json` emits struct
    /// fields in the order the impl requests them, which is what makes the
    /// output byte-comparable with `dataclasses.asdict`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Game", 31)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("exe_path", &self.exe_path)?;
        state.serialize_field("runner", &self.runner)?;
        state.serialize_field("prefix_path", &self.prefix_path)?;
        state.serialize_field("arguments", &self.arguments)?;
        state.serialize_field("cover_path", &self.cover_path)?;
        state.serialize_field("category", &self.category)?;
        state.serialize_field("steam_appid", &self.steam_appid)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("working_directory", &self.working_directory)?;
        state.serialize_field("additional_app", &self.additional_app)?;
        state.serialize_field("mangohud", &self.mangohud)?;
        state.serialize_field("gamemode", &self.gamemode)?;
        state.serialize_field("prefer_sdl", &self.prefer_sdl)?;
        state.serialize_field("wayland", &self.wayland)?;
        state.serialize_field("hdr", &self.hdr)?;
        state.serialize_field("esync", &self.esync)?;
        state.serialize_field("fsync", &self.fsync)?;
        state.serialize_field("dxvk", &self.dxvk)?;
        state.serialize_field("vkd3d", &self.vkd3d)?;
        state.serialize_field("nvapi", &self.nvapi)?;
        state.serialize_field("fsr", &self.fsr)?;
        state.serialize_field("battleye", &self.battleye)?;
        state.serialize_field("eac", &self.eac)?;
        state.serialize_field("gamescope", &self.gamescope)?;
        state.serialize_field("virtual_desktop", &self.virtual_desktop)?;
        state.serialize_field("virtual_desktop_size", &self.virtual_desktop_size)?;
        state.serialize_field("environment", &self.environment)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("added", &self.added)?;
        state.serialize_field("last_played", &self.last_played)?;
        state.end()
    }
}

/// A string field, converted per the module's tolerance table.
///
/// A JSON string is taken verbatim; a number or bool is rendered as its JSON
/// text; `null`, an array and an object count as absent. See the module docs
/// for why each arm is what it is.
fn text(data: &Map<String, Value>, key: &str) -> Option<String> {
    let value = data.get(key)?;
    match value {
        Value::String(text) => Some(text.clone()),
        // `Value`'s `Display` is the JSON text: `123`, `1.5`, `true`.
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

/// A boolean field, or `None` when absent or not a boolean.
fn flag(data: &Map<String, Value>, key: &str) -> Option<bool> {
    data.get(key).and_then(Value::as_bool)
}

/// An integer field, or `None` when the value cannot be held as an `i64`.
///
/// Python's `int` is arbitrary precision, so a hand-edited
/// `"steam_appid": 10**40` survives there and is written back exactly. Here it
/// cannot be represented, and the field keeps its default rather than being
/// silently clamped to `i64::MAX` — a saturated appid is a value the user never
/// wrote. Also accepts a float, truncating toward zero as `int(value)` does,
/// and a numeric string, which Python would store verbatim and then fail to use
/// as an appid.
fn integer(data: &Map<String, Value>, key: &str) -> Option<i64> {
    let number = match data.get(key)? {
        Value::Number(number) => number,
        // A string that is unambiguously an integer keeps its value; anything
        // else is not an appid.
        Value::String(text) => return text.parse::<i64>().ok(),
        _ => return None,
    };
    number.as_i64().or_else(|| {
        number
            .as_f64()
            .filter(|value| value.is_finite() && value.abs() < 9.223_372_036_854_776e18)
            .map(|value| value as i64)
    })
}

/// The timestamp normalization table from `models.py:72-86`.
///
/// Returns `Some(seconds)` only for a finite, non-negative number. Everything
/// else is invalid — including `bool` (Python's `isinstance(value, bool)`
/// guard exists so JSON `true` cannot become `1.0`), strings, `null`, arrays
/// and objects.
fn timestamp(value: &Value) -> Option<f64> {
    // Python evaluates `float(value)` only for a non-bool int/float; every
    // other type short-circuits to `math.nan`, which is then rejected.
    let numeric = match value {
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    (numeric.is_finite() && numeric >= 0.0).then_some(numeric)
}

/// A fresh 32-character lowercase hex id, as `uuid.uuid4().hex` produces.
///
/// Public because two callers outside this module need *the same generator*
/// rather than one of their own: `Game::default`/`new_named`, and the app's
/// `Message::OpenNewGameForm` arm, which ports `newGameTemplate`'s
/// `uuid.uuid4().hex` (`bridge.py:384`) — the form's id is generated when the
/// form opens, not when it is saved. A second generator in the app crate would
/// be a second spelling of the id format, which is the one thing every
/// `Library::get` in both languages depends on.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if fill_random(&mut bytes).is_err() {
        // Unreachable on Linux; a deterministic-enough fallback beats failing
        // to create a game. Time and pid still vary between processes.
        let seed = now().to_bits() ^ u64::from(std::process::id());
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = ((seed >> ((index % 8) * 8)) as u8).wrapping_add(index as u8);
        }
    }
    // Version 4 and the RFC 4122 variant, so an id is a well-formed UUID like
    // the one `uuid.uuid4()` produces.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    use std::fmt::Write as _;
    let mut id = String::with_capacity(32);
    for byte in bytes {
        // Writing to a `String` cannot fail.
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// Fills `bytes` from the kernel CSPRNG.
fn fill_random(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

/// A short, human-readable "last played" label. `models.py:93-117`.
///
/// `now` is a parameter rather than an ambient clock read so the view layer can
/// compute every row against one instant (see the oracle's frozen-clock cases).
pub fn format_last_played(timestamp: f64, now: f64) -> String {
    // Python's `if not timestamp` — false only for 0.0 (and -0.0). NaN is
    // truthy there and unequal to 0.0 here, so both fall through.
    if timestamp == 0.0 {
        return "Never played".to_string();
    }
    // A future timestamp clamps (parity item P-16). Rust's `f64::max` ignores
    // NaN, as Python's `max` does here.
    let seconds = (now - timestamp).max(0.0);
    let minutes = seconds / 60.0;
    if minutes < 2.0 {
        return "Played just now".to_string();
    }
    if minutes < 60.0 {
        return format!("Played {} min ago", minutes as i64);
    }
    let hours = minutes / 60.0;
    if hours < 24.0 {
        let count = hours as i64;
        return if count == 1 {
            "Played 1 hour ago".to_string()
        } else {
            format!("Played {count} hours ago")
        };
    }
    let days = (hours / 24.0) as i64;
    if days == 1 {
        return "Played yesterday".to_string();
    }
    if days < 30 {
        return format!("Played {days} days ago");
    }
    let months = days / 30;
    if months < 12 {
        return if months == 1 {
            "Played 1 month ago".to_string()
        } else {
            format!("Played {months} months ago")
        };
    }
    let years = months / 12;
    if years == 1 {
        "Played 1 year ago".to_string()
    } else {
        format!("Played {years} years ago")
    }
}

/// Port of `format_last_played(timestamp)` with `now` defaulting to the clock.
pub fn format_last_played_now(timestamp: f64) -> String {
    format_last_played(timestamp, now())
}

/// How a [`Library`]'s file read went.
///
/// The reference has no equivalent, and it is the one place this port
/// deliberately diverges from `models.py` — see [`Library::load_at`]. Python's
/// `load` collapses "there is no file" and "there is a file I cannot read" into
/// the same empty list, and every later `save` writes that emptiness back. In
/// Python that is survivable because the file is the app's only state; here the
/// app is the *second* implementation of the format, so a file written by the
/// other one is a normal thing to meet and a parse failure is a thing to report
/// rather than a thing that eats the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadStatus {
    /// No file. A first run, and nothing is wrong.
    #[default]
    Absent,
    /// The file was read and is a list. Individual entries may still have been
    /// skipped, which is the reference's documented tolerance.
    Loaded,
    /// The file exists and could not be read: permissions, I/O, or bytes that
    /// are not UTF-8.
    Unreadable,
    /// The file was read but is not the list this app writes.
    Unparsable,
}

impl LoadStatus {
    /// Whether writing would destroy something the app could not read.
    ///
    /// True for the two failure states and false for the two good ones. This is
    /// the predicate [`Library::save`] gates on, so it is the whole safety
    /// property in one place rather than a condition written out at each call.
    pub fn is_destructive_to_save_over(self) -> bool {
        matches!(self, Self::Unreadable | Self::Unparsable)
    }
}

/// The rows the last [`Library::search`] resolved, and the key they were
/// resolved for.
///
/// # Why the library memoises its own query
///
/// `view()` calls `search` and `categories` at the top of every frame
/// (`crates/app/src/view/library.rs`), and both were O(N log N) in the
/// library's size with a `String` folded inside every comparison. PERF-05
/// measured that cost — for a 500-game library, one `search` folded 4,132 times
/// and one `categories` 4,002, and the page calls both on **every** frame. The
/// answer, though, changes only when the library, the query, the category or the
/// sort changes, and none of those change during a redraw.
///
/// # Why the rows are indices
///
/// A cached `Vec<&Game>` would be a borrow of [`Library::games`] stored inside
/// the `Library` that owns it, which is the self-reference Rust has no safe
/// answer for, and a cached `Vec<Game>` would be a second copy of the library.
/// Positions in `games` are owned data, and rebuilding the rows from them is
/// `k` pointer writes with no comparison and no `String`.
#[derive(Debug, Clone, Default)]
struct RowMemo {
    /// Whether the three fields below describe [`Self::indices`].
    ///
    /// Cleared by [`Library::invalidate`] rather than compared against a
    /// revision counter, because a counter has to be *read* correctly at every
    /// mutation and a flag has to be *written*: one is a step a new mutating
    /// method can omit while still looking right, the other is a call it either
    /// makes or visibly does not.
    valid: bool,
    query: String,
    category: String,
    sort: String,
    indices: Vec<usize>,
}

impl RowMemo {
    /// Whether this memo already holds the rows for that key.
    fn is_current(&self, query: &str, category: &str, sort: &str) -> bool {
        self.valid && self.query == query && self.category == category && self.sort == sort
    }
}

/// The category list the last [`Library::categories`] computed, and whether it
/// still describes `games`.
///
/// Separate from [`RowMemo`] because the key is not the same shape: the
/// category list depends on the library and on nothing else, so tying it to the
/// query would recompute it every time the sort changed and — worse — let a
/// caller that asks for categories first cache them under the wrong key. See
/// [`RowMemo`] for the argument, which is the same one.
#[derive(Debug, Clone, Default)]
struct CategoryMemo {
    /// Whether [`Self::list`] still describes the library.
    valid: bool,
    list: Vec<String>,
}

/// Loads, mutates and persists a collection of [`Game`]s. `models.py:120-209`.
#[derive(Debug, Clone)]
pub struct Library {
    path: PathBuf,
    /// Insertion-ordered, because Python's `dict` is: two entries that tie
    /// under a sort keep their file order, and `Library::all` relies on a
    /// stable sort to preserve that.
    games: Vec<Game>,
    /// What the last read of [`Self::path`] did; [`Self::save`] refuses to
    /// write when it was not a clean read.
    load_status: LoadStatus,
    /// The last query this library answered. See [`RowMemo`].
    rows: RefCell<RowMemo>,
    /// The last category list this library computed. See [`CategoryMemo`].
    category_list: RefCell<CategoryMemo>,
}

impl Library {
    /// Opens the library at `path`, or the configured one when `None`.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::new_loaded(path, None)
    }

    /// [`Self::new`] with the clock injected, for a regenerated `added`.
    pub fn new_at(path: Option<PathBuf>, now: f64) -> Self {
        Self::new_loaded(path, Some(now))
    }

    fn new_loaded(path: Option<PathBuf>, now: Option<f64>) -> Self {
        let mut library = Self {
            path: path.unwrap_or_else(paths::games_file),
            games: Vec::new(),
            load_status: LoadStatus::default(),
            rows: RefCell::new(RowMemo::default()),
            category_list: RefCell::new(CategoryMemo::default()),
        };
        match now {
            Some(now) => library.load_at(now),
            None => library.load(),
        }
        library
    }

    /// Forget the resolved rows and the category list.
    ///
    /// **This is the whole invalidation story** for [`RowMemo`] and
    /// [`CategoryMemo`], and it is one function so that a method added later
    /// has a single thing to call rather than two fields to remember. Every
    /// `&mut self` method that changes [`Self::games`] calls it —
    /// [`Self::load_at`], [`Self::upsert`], [`Self::remove`] and
    /// [`Self::mark_played`]; [`Self::add`] and [`Self::update`] reach it
    /// through `upsert`. A method that changed `games` without calling this
    /// would serve the previous library's rows, which
    /// `every_mutation_is_visible_to_the_next_search` is what prevents.
    fn invalidate(&mut self) {
        self.rows.borrow_mut().valid = false;
        self.category_list.borrow_mut().valid = false;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Port of `Library.load` (`models.py:128-147`).
    ///
    /// Every arm tolerates a broken file: a missing or unreadable file, invalid
    /// JSON, a trailing-garbage file, and a top-level value that is not a list
    /// all yield an empty library; a non-object list entry is skipped on its
    /// own, and so is an entry with an empty name, so the rest of the file
    /// survives.
    ///
    /// # The deliberate divergence: the failure is recorded, not just tolerated
    ///
    /// Tolerating the read and *forgetting* that it failed is what makes the
    /// reference's behaviour destructive here: the in-memory library becomes
    /// empty, and the next `save` — which any add, edit, removal or
    /// `mark_played` performs — writes that emptiness over the user's file. The
    /// user sees an empty library, adds one game, and the original is gone with
    /// no message at any point.
    ///
    /// So this records what happened in [`Self::load_status`] and
    /// [`Self::save`] refuses to write over a file it could not read. The read
    /// itself behaves exactly as the reference does, entry skips included; only
    /// the write is gated.
    pub fn load(&mut self) {
        self.load_at(now());
    }

    /// [`Self::load`] with the clock injected.
    pub fn load_at(&mut self, now: f64) {
        // Before the first early return: every arm below leaves `games` in a
        // state the previous run's rows do not describe, including the three
        // that return having only cleared it.
        self.invalidate();
        self.games.clear();
        let Ok(source) = std::fs::read_to_string(&self.path) else {
            // A file that is not there is a first run. Anything else — an I/O
            // error, a permission denial, bytes that are not UTF-8 — is a file
            // whose contents are unknown, and unknown is not empty.
            self.load_status = if self.path.exists() {
                LoadStatus::Unreadable
            } else {
                LoadStatus::Absent
            };
            return;
        };
        let Ok(Value::Array(entries)) = json::parse_lenient(&source) else {
            self.load_status = LoadStatus::Unparsable;
            return;
        };
        self.load_status = LoadStatus::Loaded;
        for entry in entries {
            let Value::Object(fields) = entry else {
                continue;
            };
            let game = Game::from_dict_at(&fields, now);
            if game.name.is_empty() {
                continue;
            }
            self.upsert(game);
        }
    }

    /// What the last read of the library file did.
    pub fn load_status(&self) -> LoadStatus {
        self.load_status
    }

    /// Inserts, replacing an existing entry with the same id in place.
    ///
    /// `Library::load`'s dict assignment keeps the *first* position of a
    /// duplicated id while taking the *last* value, which is what the
    /// `duplicate_ids` fixture pins.
    fn upsert(&mut self, game: Game) {
        self.invalidate();
        match self
            .games
            .iter_mut()
            .find(|existing| existing.id == game.id)
        {
            Some(slot) => *slot = game,
            None => self.games.push(game),
        }
    }

    /// Port of `Library.save` (`models.py:149-154`).
    ///
    /// The payload is `self.all("name")` — saving re-sorts the in-memory dict
    /// into name order, so the file on disk is always sorted.
    ///
    /// # It refuses to write over a file it could not read
    ///
    /// See [`Self::load_at`]. When the last read failed, the in-memory library
    /// is empty for a reason that has nothing to do with the user's games, and
    /// writing it would turn a recoverable problem into a permanent one. The
    /// caller gets an error it can show, which is the point: the reference
    /// reports nothing and loses the file.
    pub fn save(&self) -> Result<(), PersistenceError> {
        if self.load_status.is_destructive_to_save_over() {
            // A variant, not a formatted string (`ARCH-10`): this is the
            // refusal ARCH-01 is about, and a caller that wants to say
            // something different about it — "your file is safe, here is what
            // to do" rather than "the write failed" — can now match on it
            // instead of testing an `io::ErrorKind` nobody read.
            return Err(PersistenceError::WouldDiscardUnreadable {
                path: self.path.clone(),
                reason: match self.load_status {
                    LoadStatus::Unreadable => "unreadable",
                    _ => "not a game list",
                },
            });
        }
        self.save_to(&self.path)
    }

    /// [`Self::save`] to an explicit path, as Python's `lib.path = out;
    /// lib.save()` allows. Used by the oracle fixture tests so they never write
    /// into the checked-in fixture directory.
    ///
    /// The status gate does **not** apply to an explicit path: a caller writing
    /// somewhere other than the file that failed to load is not overwriting
    /// anything, and that is what the fixture tests rely on. Only
    /// [`Self::save`], which writes back to the source, is gated.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        json::write_python_file(path.as_ref(), &self.all("name"))
    }

    /// Port of `Library.all` (`models.py:156-163`).
    ///
    /// Both timestamp sorts carry an explicit name tie-break; without it
    /// `recent` and `added` diverge from Python on equal keys. `sort_by` is a
    /// stable sort in both languages, so entries equal on both terms keep their
    /// insertion order.
    pub fn all(&self, sort: &str) -> Vec<&Game> {
        self.all_indices(sort)
            .into_iter()
            .map(|index| &self.games[index])
            .collect()
    }

    /// [`Self::all`] as positions in [`Self::games`].
    ///
    /// Split out for [`Self::search`], whose memo holds positions rather than
    /// references — see [`RowMemo`]. The two are one implementation rather than
    /// two: a second sort written for the memo would be a second statement of
    /// the tie-break rules.
    fn all_indices(&self, sort: &str) -> Vec<usize> {
        // Python's key is `g.name.lower()` for every sort — the whole key for
        // "name", and the tie-break for the two timestamps. **The fold is
        // computed once per game and the *precomputed* key is what the
        // comparator reads.** Written inline (`sort_by_key(|a| a.name
        // .to_lowercase())`, which is what this was) the fold runs inside the
        // comparator, on both arguments of every comparison — of which a sort
        // makes O(N log N). PERF-05 measured the difference on a 500-game
        // fixture: 3,242 folds for one `all("name")`, against 500 here. The
        // order and the stability are unchanged, which is what
        // `sorting_matches_python_including_the_name_tie_break` and
        // `timestamp_sorts_break_ties_by_name` hold.
        let keys: Vec<String> = self.games.iter().map(|game| folded(&game.name)).collect();
        let mut order: Vec<usize> = (0..self.games.len()).collect();
        match sort {
            "recent" => order.sort_by(|&a, &b| {
                zero_normalised(self.games[b].last_played)
                    .total_cmp(&zero_normalised(self.games[a].last_played))
                    .then_with(|| keys[a].cmp(&keys[b]))
            }),
            "added" => order.sort_by(|&a, &b| {
                zero_normalised(self.games[b].added)
                    .total_cmp(&zero_normalised(self.games[a].added))
                    .then_with(|| keys[a].cmp(&keys[b]))
            }),
            _ => order.sort_by(|&a, &b| keys[a].cmp(&keys[b])),
        }
        order
    }

    /// Port of `Library.get`.
    pub fn get(&self, game_id: &str) -> Option<&Game> {
        self.games.iter().find(|game| game.id == game_id)
    }

    /// Port of `Library.add` (`models.py:168-171`).
    pub fn add(&mut self, game: Game) -> Result<(), PersistenceError> {
        self.upsert(game);
        self.save()
    }

    /// Port of `Library.remove`: a no-op when the id is unknown, and the file
    /// is only rewritten when something actually changed.
    pub fn remove(&mut self, game_id: &str) -> Result<(), PersistenceError> {
        let Some(position) = self.games.iter().position(|game| game.id == game_id) else {
            return Ok(());
        };
        self.invalidate();
        self.games.remove(position);
        self.save()
    }

    /// Port of `Library.update`.
    pub fn update(&mut self, game: Game) -> Result<(), PersistenceError> {
        self.upsert(game);
        self.save()
    }

    /// Port of `Library.mark_played` (`models.py:182-186`).
    ///
    /// The timestamp is a row's `"Played … ago"` text and one of the sort keys,
    /// not a filter term, so the resolved rows would survive this — but the
    /// memo is about the *library's* contents and not about which fields that
    /// particular query reads, which is not a distinction a future field could
    /// be trusted to keep. It is invalidated like the rest.
    ///
    /// The `position` lookup rather than the `iter_mut().find()` this had
    /// before `PERF-05`: `invalidate` takes `&self` and `iter_mut` holds a
    /// mutable borrow of `self.games` across the call, so the two cannot be
    /// written in one expression. Splitting the lookup from the write is what
    /// lets the memo be dropped *before* the mutation, which is the order that
    /// matters — invalidating afterwards would leave a window in which a
    /// re-entrant read served rows from the old vector.
    pub fn mark_played(&mut self, game_id: &str) -> Result<(), PersistenceError> {
        let Some(position) = self.games.iter().position(|game| game.id == game_id) else {
            return Ok(());
        };
        self.invalidate();
        self.games[position].last_played = now();
        self.save()
    }

    /// Port of `Library.search` (`models.py:188-199`).
    ///
    /// The query matches the name **or** the display category, so searching a
    /// category name finds its games (parity item P-04). `category` equal to
    /// `"All"` is not a filter.
    ///
    /// # The answer is memoised against the three arguments
    ///
    /// See [`RowMemo`]. On a hit this is `k` pointer writes and one `Vec` of
    /// them — no sort, no comparison and no `String`. The rows are identical
    /// either way by construction, and
    /// `a_repeated_query_is_the_same_rows_without_the_work` is what holds the
    /// two against each other.
    pub fn search(&self, query: &str, category: &str, sort: &str) -> Vec<&Game> {
        if !self.rows.borrow().is_current(query, category, sort) {
            let indices = self.compute_rows(query, category, sort);
            *self.rows.borrow_mut() = RowMemo {
                valid: true,
                query: query.to_string(),
                category: category.to_string(),
                sort: sort.to_string(),
                indices,
            };
        }
        self.rows
            .borrow()
            .indices
            .iter()
            .map(|&index| &self.games[index])
            .collect()
    }

    /// [`Self::search`] without the memo: the sort, the two filters and the
    /// query match, exactly as the reference writes them.
    fn compute_rows(&self, query: &str, category: &str, sort: &str) -> Vec<usize> {
        let query = folded(query.trim());
        let mut order = self.all_indices(sort);
        if !category.is_empty() && category != "All" {
            order.retain(|&index| self.games[index].display_category() == category);
        }
        if query.is_empty() {
            return order;
        }
        order.retain(|&index| {
            let game = &self.games[index];
            folded(&game.name).contains(&query) || folded(game.display_category()).contains(&query)
        });
        order
    }

    /// Port of `Library.categories` (`models.py:201-203`): distinct display
    /// categories, case-insensitively sorted with [`UNCATEGORIZED`] last.
    ///
    /// # The answer is memoised whole
    ///
    /// See [`CategoryMemo`]. The filter selector is built from this on every
    /// frame (`crates/app/src/view/library.rs:643`) and nothing in it depends on
    /// the query or the sort, so it is recomputed only when the games change.
    /// The list is returned by value — the caller owns it — so a hit still
    /// copies it; what the memo removes is the `to_owned()` per game, the sort
    /// and the two folds per comparison.
    pub fn categories(&self) -> Vec<String> {
        if !self.category_list.borrow().valid {
            let mut found: Vec<String> = self
                .games
                .iter()
                .map(|game| game.display_category().to_owned())
                .collect();
            found.sort_by(|a, b| {
                (a == UNCATEGORIZED)
                    .cmp(&(b == UNCATEGORIZED))
                    .then_with(|| folded(a).cmp(&folded(b)))
            });
            found.dedup();
            *self.category_list.borrow_mut() = CategoryMemo {
                valid: true,
                list: found,
            };
        }
        self.category_list.borrow().list.clone()
    }

    /// Port of `Library.__len__`.
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// Whether the library holds no games.
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// Every game, in library order.
    pub fn iter(&self) -> impl Iterator<Item = &Game> {
        self.games.iter()
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FROZEN_NOW: f64 = 1_700_000_000.0;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("expected an object, got {other}"),
        }
    }

    #[test]
    fn fresh_games_get_an_id_and_a_timestamp() {
        let game = Game::new_named("Half-Life");
        assert_eq!(game.name, "Half-Life");
        assert_eq!(game.id.len(), 32, "uuid4().hex is 32 hex characters");
        assert!(game.id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!((game.added - now()).abs() < 5.0);
        assert_eq!(game.last_played, 0.0);
    }

    #[test]
    fn ids_are_version_four_uuids_without_dashes() {
        let id = new_id();
        assert_eq!(id.len(), 32);
        assert_eq!(&id[12..13], "4", "version nibble");
        assert!(
            matches!(&id[16..17], "8" | "9" | "a" | "b"),
            "variant nibble"
        );
        assert_ne!(new_id(), id, "ids must not repeat");
    }

    #[test]
    fn field_order_matches_the_python_dataclass() {
        // Byte equality with a Python-written file depends on this order, so it
        // is asserted rather than left to a reader's diligence.
        //
        // The order is read out of the *written text* rather than out of a
        // `Value`. A `serde_json::Map` is a `BTreeMap` unless the workspace
        // happens to have `preserve_order` unified on from a GUI dependency, so
        // asserting on a `Value` would pass in `cargo test` and fail in
        // `cargo test -p gamehandler-core` — a test whose result depends on who
        // else is in the build is worse than no test.
        let text = crate::json::to_python_string(&Game::blank()).unwrap();
        let keys: Vec<&str> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                line.strip_prefix('"')
                    .and_then(|rest| rest.split('"').next())
            })
            .collect();
        assert_eq!(
            keys,
            [
                "name",
                "exe_path",
                "runner",
                "prefix_path",
                "arguments",
                "cover_path",
                "category",
                "steam_appid",
                "kind",
                "working_directory",
                "additional_app",
                "mangohud",
                "gamemode",
                "prefer_sdl",
                "wayland",
                "hdr",
                "esync",
                "fsync",
                "dxvk",
                "vkd3d",
                "nvapi",
                "fsr",
                "battleye",
                "eac",
                "gamescope",
                "virtual_desktop",
                "virtual_desktop_size",
                "environment",
                "id",
                "added",
                "last_played",
            ]
        );
        assert_eq!(keys.len(), 31);
    }

    #[test]
    fn defaults_match_the_python_dataclass() {
        let game = Game::blank();
        assert_eq!(game.runner, "wine-system");
        assert_eq!(game.category, "Uncategorized");
        assert_eq!(game.kind, "windows");
        assert_eq!(game.virtual_desktop_size, "1920x1080");
        for (name, value) in [
            ("mangohud", game.mangohud),
            ("gamemode", game.gamemode),
            ("prefer_sdl", game.prefer_sdl),
            ("wayland", game.wayland),
            ("hdr", game.hdr),
            ("nvapi", game.nvapi),
            ("fsr", game.fsr),
            ("gamescope", game.gamescope),
            ("virtual_desktop", game.virtual_desktop),
        ] {
            assert!(!value, "{name} should default to false");
        }
        for (name, value) in [
            ("esync", game.esync),
            ("fsync", game.fsync),
            ("dxvk", game.dxvk),
            ("vkd3d", game.vkd3d),
            ("battleye", game.battleye),
            ("eac", game.eac),
        ] {
            assert!(value, "{name} should default to true");
        }
    }

    #[test]
    fn unknown_keys_are_dropped() {
        // Parity item P-75: extras are discarded, never an error.
        let game = Game::from_dict_at(
            &object(json!({"name": "Unknown", "legacy_field": 1, "another": [1, 2]})),
            FROZEN_NOW,
        );
        assert_eq!(game.name, "Unknown");
        let written = serde_json::to_value(&game).unwrap();
        assert!(written.get("legacy_field").is_none());
        assert!(written.get("another").is_none());
    }

    #[test]
    fn int_timestamps_become_floats() {
        // FINDINGS F-C: `42` must round-trip as `42.0`.
        let game = Game::from_dict_at(
            &object(json!({"name": "Int ts", "added": 42, "last_played": 7})),
            FROZEN_NOW,
        );
        assert_eq!(game.added, 42.0);
        assert_eq!(game.last_played, 7.0);
        let text = json::to_python_string(&game).unwrap();
        assert!(text.contains("\"added\": 42.0"), "{text}");
        assert!(text.contains("\"last_played\": 7.0"), "{text}");
    }

    #[test]
    fn invalid_timestamps_regenerate_added_and_zero_last_played() {
        let invalid = [
            json!(f64::NAN),
            json!(f64::INFINITY),
            json!(f64::NEG_INFINITY),
            json!(-5.0),
            json!("soon"),
            json!(true),
            json!(false),
            json!([1]),
            json!({}),
        ];
        for value in invalid {
            let game = Game::from_dict_at(
                &object(json!({"name": "X", "added": value, "last_played": value})),
                FROZEN_NOW,
            );
            assert_eq!(game.added, FROZEN_NOW, "added regenerated for {value}");
            assert_eq!(game.last_played, 0.0, "last_played zeroed for {value}");
        }
    }

    #[test]
    fn a_boolean_is_never_a_timestamp() {
        // The `isinstance(value, bool)` rule: JSON `true` must not become 1.0.
        let game = Game::from_dict_at(
            &object(json!({"name": "Bool", "added": true, "last_played": true})),
            FROZEN_NOW,
        );
        assert_eq!(game.added, FROZEN_NOW);
        assert_eq!(game.last_played, 0.0);
    }

    #[test]
    fn a_null_timestamp_is_treated_as_invalid() {
        // DECISIONS D-14: deliberately *not* Python's behaviour, which leaves
        // the field as None and takes the library sort down with a TypeError.
        let game = Game::from_dict_at(
            &object(json!({"name": "Null ts", "added": null, "last_played": null})),
            FROZEN_NOW,
        );
        assert_eq!(game.added, FROZEN_NOW);
        assert_eq!(game.last_played, 0.0);
    }

    #[test]
    fn a_missing_timestamp_keeps_the_dataclass_default() {
        let game = Game::from_dict_at(&object(json!({"name": "Fresh"})), FROZEN_NOW);
        assert_eq!(game.added, FROZEN_NOW);
        assert_eq!(game.last_played, 0.0);
    }

    #[test]
    fn a_supplied_id_wins_over_a_generated_one() {
        let game = Game::from_dict_at(
            &object(json!({"name": "X", "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})),
            FROZEN_NOW,
        );
        assert_eq!(game.id, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn display_category_folds_blanks_but_keeps_the_stored_value() {
        let game = Game::from_dict_at(
            &object(json!({"name": "Blank", "category": "   "})),
            FROZEN_NOW,
        );
        assert_eq!(game.category, "   ", "stored verbatim");
        assert_eq!(game.display_category(), UNCATEGORIZED);

        let padded = Game::from_dict_at(
            &object(json!({"name": "Padded", "category": "  roguelike  "})),
            FROZEN_NOW,
        );
        assert_eq!(padded.display_category(), "roguelike");

        let empty = Game::from_dict_at(
            &object(json!({"name": "Empty", "category": ""})),
            FROZEN_NOW,
        );
        assert_eq!(empty.display_category(), UNCATEGORIZED);
    }

    #[test]
    fn is_linux_follows_the_kind_field() {
        let linux = Game::from_dict_at(&object(json!({"name": "N", "kind": "linux"})), FROZEN_NOW);
        assert!(linux.is_linux());
        let windows = Game::from_dict_at(&object(json!({"name": "W"})), FROZEN_NOW);
        assert!(!windows.is_linux());
    }

    #[test]
    fn every_format_last_played_branch_matches_python() {
        let cases: [(f64, &str); 18] = [
            (0.0, "Never played"),
            // A *negative* timestamp is not falsy, so Python runs the whole
            // branch chain on `now - (-1)` — 54 years — rather than answering
            // "Never played". Unreachable through `from_dict`, which rejects
            // negatives, but part of this function's contract.
            (-1.0, "Played 54 years ago"),
            (FROZEN_NOW, "Played just now"),
            (FROZEN_NOW - 1.0, "Played just now"),
            (FROZEN_NOW - 119.0, "Played just now"),
            (FROZEN_NOW - 120.0, "Played 2 min ago"),
            (FROZEN_NOW - 59.0 * 60.0, "Played 59 min ago"),
            (FROZEN_NOW - 3600.0, "Played 1 hour ago"),
            (FROZEN_NOW - 7200.0, "Played 2 hours ago"),
            (FROZEN_NOW - 23.0 * 3600.0, "Played 23 hours ago"),
            (FROZEN_NOW - 24.0 * 3600.0, "Played yesterday"),
            (FROZEN_NOW - 2.0 * 24.0 * 3600.0, "Played 2 days ago"),
            (FROZEN_NOW - 29.0 * 24.0 * 3600.0, "Played 29 days ago"),
            (FROZEN_NOW - 30.0 * 24.0 * 3600.0, "Played 1 month ago"),
            (FROZEN_NOW - 60.0 * 24.0 * 3600.0, "Played 2 months ago"),
            (FROZEN_NOW - 330.0 * 24.0 * 3600.0, "Played 11 months ago"),
            (FROZEN_NOW - 360.0 * 24.0 * 3600.0, "Played 1 year ago"),
            (FROZEN_NOW - 800.0 * 24.0 * 3600.0, "Played 2 years ago"),
        ];
        for (timestamp, expected) in cases {
            assert_eq!(
                format_last_played(timestamp, FROZEN_NOW),
                expected,
                "format_last_played({timestamp})"
            );
        }
    }

    #[test]
    fn a_future_timestamp_clamps_to_just_now() {
        // Parity item P-16.
        assert_eq!(
            format_last_played(FROZEN_NOW + 10_000.0, FROZEN_NOW),
            "Played just now"
        );
    }

    #[test]
    fn format_last_played_now_uses_the_clock() {
        assert_eq!(format_last_played_now(now()), "Played just now");
        assert_eq!(format_last_played_now(0.0), "Never played");
    }

    fn library_from(entries: Value) -> Library {
        let directory = std::env::temp_dir().join(format!("gh-lib-{}", new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("games.json");
        std::fs::write(&path, json::to_python_string(&entries).unwrap()).unwrap();
        Library::new_at(Some(path), FROZEN_NOW)
    }

    /// A library of `count` games with distinct names and a handful of
    /// categories — the fixture PERF-05's verification method names ("a
    /// 500-game fixture").
    ///
    /// Built through `library_from` rather than by pushing `Game`s so the
    /// fixture is the one the app actually loads: a real file, parsed by the
    /// real loader, in the real order.
    fn library_of(count: usize) -> Library {
        let categories = ["Action", "Puzzle", "Shooter", "Roguelike", "Simulation"];
        let entries: Vec<Value> = (0..count)
            .map(|index| {
                json!({
                    "id": format!("{index:032}"),
                    // Mixed case on purpose: a fold is the work being counted,
                    // and an all-lowercase name would still fold.
                    "name": format!("Game {index}"),
                    "category": categories[index % categories.len()],
                    "added": index as f64,
                    "last_played": (index % 7) as f64,
                })
            })
            .collect();
        library_from(Value::Array(entries))
    }

    /// The rows a `search`/`all` call answered with, as ids.
    fn row_ids(games: &[&Game]) -> Vec<String> {
        games.iter().map(|game| game.id.clone()).collect()
    }

    /// The ids [`Library::compute_rows`] resolves for a key — the answer
    /// [`Library::search`] is supposed to be memoising, computed the long way
    /// round. The oracle for
    /// `the_memo_answers_with_the_rows_the_uncached_body_would_have`.
    fn uncached_rows(library: &Library, query: &str, category: &str, sort: &str) -> Vec<String> {
        library
            .compute_rows(query, category, sort)
            .into_iter()
            .map(|index| library.games[index].id.clone())
            .collect()
    }

    /// One `(query, category, sort)` triple — the three arguments
    /// [`Library::search`] is keyed on.
    type SearchKey = (&'static str, &'static str, &'static str);

    /// Every key worth asking [`Library::search`] for, over the fixture above.
    ///
    /// Each shape of the query is here because each takes a different branch:
    /// `"zzz"` matches nothing, `"game 1"` matches names (and, being a
    /// substring of `"game 10"`, more than one), `"shooter"` matches a
    /// *category* and no name at all (parity item P-04), and `"  "` trims to
    /// empty, which is the no-query path. `"All"` and `""` are the two
    /// spellings of "no category filter" and both appear.
    const SEARCH_KEYS: &[SearchKey] = &[
        ("", "All", "name"),
        ("", "All", "added"),
        ("", "All", "recent"),
        ("", "Puzzle", "name"),
        ("game 1", "All", "name"),
        ("game 1", "All", "recent"),
        ("shooter", "All", "name"),
        ("puzzle", "All", "recent"),
        ("shooter", "Shooter", "added"),
        ("zzz", "All", "name"),
    ];

    /// Keys that are *spelled* differently and mean the same thing, so the memo
    /// is allowed — required — to answer them alike.
    ///
    /// Kept out of [`SEARCH_KEYS`], whose whole purpose is that its entries
    /// disagree, and asserted here instead, because "these two agree" is a
    /// statement about the reference's semantics and not about the memo:
    /// `""` and `"All"` are both "no category filter" (`models.py:188-199`);
    /// `"  "` trims to the empty query; the fold is case-insensitive, so
    /// `"GAME 1"` and `"game 1"` are one query; and `"shooter"` matches only
    /// through the display category, so filtering by `"Shooter"` selects the
    /// same games it already selected (parity item P-04).
    const EQUIVALENT_KEYS: &[(SearchKey, SearchKey)] = &[
        (("", "", "name"), ("", "All", "name")),
        (("  ", "All", "name"), ("", "All", "name")),
        (("GAME 1", "All", "name"), ("game 1", "All", "name")),
        (("shooter", "Shooter", "name"), ("shooter", "All", "name")),
    ];

    #[test]
    fn one_sort_folds_each_name_once_not_once_per_comparison() {
        // PERF-05, the half the memo cannot hide: the fold used to live inside
        // the comparator, so a 500-game sort folded both arguments of every one
        // of its ~2,742 comparisons (3,242 folds, the tie-breaks included).
        // Nothing about the *order* may depend on where the fold happens, which
        // is why this asserts the count and
        // `sorting_matches_python_including_the_name_tie_break` asserts the
        // order.
        let library = library_of(500);

        for sort in ["name", "added", "recent"] {
            folds(true);
            let sorted = library.all(sort);
            assert_eq!(sorted.len(), 500);
            assert_eq!(
                folds(false),
                500,
                "all({sort:?}) folded the library more than once per game, so the fold \
                 is back inside the comparator"
            );
        }
    }

    #[test]
    fn a_redraw_of_an_unchanged_page_folds_nothing() {
        // PERF-05. `view()` calls `search` and `categories` at the top of every
        // frame (`crates/app/src/view/library.rs:643`), so the number that
        // matters is a *later* frame's: the first one legitimately resolves
        // both, and none of the three arguments or the library itself changes
        // in between.
        let library = library_of(500);
        let frame = || {
            let _ = library.search("game 1", "All", "name");
            let _ = library.categories();
        };

        frame();
        folds(true);
        for _ in 0..100 {
            frame();
        }
        let redrawn = folds(false);

        // Before the memo this was 813,400 for the same hundred redraws of the
        // same unchanged page: 500 games re-sorted and re-filtered and the
        // category list rebuilt and re-sorted, on every one of them.
        assert_eq!(
            redrawn, 0,
            "100 redraws of an unchanged page folded {redrawn} names and category lists"
        );
    }

    #[test]
    fn the_memo_answers_with_the_rows_the_uncached_body_would_have() {
        // The cache is only allowed to be a cache. Every key is asked for twice
        // — the first call resolves it, the second is served from the memo —
        // and both answers are held against `compute_rows`, the uncached body,
        // which is what makes this a check on the memo rather than on the sort
        // it shares with it.
        let library = library_of(500);
        for &(query, category, sort) in SEARCH_KEYS {
            let uncached = uncached_rows(&library, query, category, sort);
            for pass in ["resolved", "memoised"] {
                assert_eq!(
                    row_ids(&library.search(query, category, sort)),
                    uncached,
                    "the {pass} answer for {query:?}/{category:?}/{sort:?} is not the one \
                     the uncached body gives"
                );
            }
        }
        for &(first, second) in EQUIVALENT_KEYS {
            assert_eq!(
                row_ids(&library.search(first.0, first.1, first.2)),
                row_ids(&library.search(second.0, second.1, second.2)),
                "{first:?} and {second:?} are two spellings of one query and must not \
                 resolve differently just because they are different memo keys"
            );
        }
    }

    #[test]
    fn the_search_keys_resolve_to_different_rows() {
        // Guards the test above from passing vacuously. A memo keyed on
        // everything, or on nothing, only shows up if the keys in the matrix
        // disagree about the answer — if two of them resolved to the same rows,
        // answering one with the other's memo would look correct.
        let library = library_of(500);
        let answers: Vec<Vec<String>> = SEARCH_KEYS
            .iter()
            .map(|&(query, category, sort)| row_ids(&library.search(query, category, sort)))
            .collect();
        for (index, answer) in answers.iter().enumerate() {
            for (other, other_answer) in answers.iter().enumerate().skip(index + 1) {
                assert_ne!(
                    answer, other_answer,
                    "SEARCH_KEYS[{index}] {:?} and SEARCH_KEYS[{other}] {:?} resolve to the \
                     same rows, so the memo tests cannot tell their answers apart",
                    SEARCH_KEYS[index], SEARCH_KEYS[other]
                );
            }
        }
    }

    #[test]
    fn every_mutation_is_visible_to_the_next_search() {
        // `Library::invalidate` is the whole invalidation story for the two
        // memos, and this is what holds every mutating method to it.
        //
        // **Every case reads back through the key it warmed.** That is the
        // whole point: a mutation followed by a *new* query recomputes whether
        // or not anything was invalidated, so an assertion that changes the key
        // is an assertion that passes without the invalidation being there — it
        // was, until this comment was written, and dropping `invalidate()` from
        // `remove` left the suite green.
        fn by_name(library: &Library) -> Vec<&Game> {
            library.search("", "All", "name")
        }
        fn by_recent(library: &Library) -> Vec<&Game> {
            library.search("", "All", "recent")
        }

        let mut library = library_of(20);

        // Warm both memos, so the previous library's rows and category list are
        // sitting in them waiting to be wrongly served.
        let _ = library.categories();
        assert_eq!(row_ids(&by_name(&library)).len(), 20);

        // `add`, through `upsert`.
        library
            .add(Game::from_dict_at(
                &object(json!({
                    "id": "added-1",
                    "name": "Zzz Added",
                    "category": "Brand New",
                    "added": 1.0,
                    "last_played": 0.0,
                })),
                FROZEN_NOW,
            ))
            .unwrap();
        let added = row_ids(&by_name(&library));
        assert_eq!(added.len(), 21, "the added game is not among the rows");
        assert!(added.contains(&"added-1".to_string()));
        assert!(
            library.categories().iter().any(|name| name == "Brand New"),
            "a category added after the memo was resolved is missing from the selector"
        );

        // `update`, through `upsert`.
        library
            .update(Game::from_dict_at(
                &object(json!({
                    "id": "added-1",
                    "name": "Renamed",
                    "category": "Brand New",
                    "added": 1.0,
                    "last_played": 0.0,
                })),
                FROZEN_NOW,
            ))
            .unwrap();
        let updated: Vec<String> = by_name(&library)
            .iter()
            .map(|game| game.name.clone())
            .collect();
        assert!(
            updated.contains(&"Renamed".to_string()) && !updated.contains(&"Zzz Added".to_string()),
            "the rows still carry the name the game had before the update: {updated:?}"
        );

        // `remove`. This one is a safety matter and not only a freshness one:
        // the memo holds positions in `games`, so a stale memo served after a
        // removal reads a shifted — or, at the end, out-of-bounds — index.
        assert_eq!(row_ids(&by_name(&library)).len(), 21);
        library.remove("added-1").unwrap();
        assert_eq!(
            row_ids(&by_name(&library)).len(),
            20,
            "the removed game is still among the rows"
        );
        assert!(
            !library.categories().iter().any(|name| name == "Brand New"),
            "a category whose last game was removed is still in the selector"
        );

        // `mark_played` — `last_played` is the key the `"recent"` sort reads, so
        // the memoised order itself is what has to change.
        let _ = by_recent(&library);
        let target = library.games[3].id.clone();
        library.mark_played(&target).unwrap();
        assert_eq!(
            row_ids(&by_recent(&library))[0],
            target,
            "the game just played is not first under the recent sort"
        );

        // `load_at` — the file, and with it every row, is replaced.
        let replacement = library_from(json!([
            {"id": "loaded-1", "name": "Loaded", "category": "Loaded Category"}
        ]));
        let _ = by_name(&library);
        replacement.save_to(library.path()).unwrap();
        library.load_at(FROZEN_NOW);
        assert_eq!(library.len(), 1);
        assert_eq!(
            row_ids(&by_name(&library)),
            ["loaded-1"],
            "the rows of the library before the reload are still being served"
        );
        assert_eq!(library.categories(), ["Loaded Category"]);
    }

    /// A library whose file holds `contents` **verbatim**, for the cases where
    /// the point is that the bytes are not a game list.
    fn library_with_raw_file(contents: &str) -> (Library, PathBuf) {
        let directory = std::env::temp_dir().join(format!("gh-lib-raw-{}", new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("games.json");
        std::fs::write(&path, contents).unwrap();
        (Library::new_at(Some(path.clone()), FROZEN_NOW), path)
    }

    /// **A file the app cannot parse is not overwritten by the next write.**
    ///
    /// `BUGS.md` BUG-01, and the reason it is a P0 rather than the cosmetic
    /// divergence it reads like: `load_at` cleared the in-memory library, hit a
    /// parse failure, discarded it, and returned `()`. Nothing downstream could
    /// tell "no games" from "one bad byte", so the next `add` wrote the
    /// emptiness over the user's file. The user saw an empty library, added one
    /// game, and the original was gone with no message at any point.
    ///
    /// The scenario below is the one the audit executed against the built
    /// binary: a truncated file (`[{"id": "alpha-1", "name": "Alpha"` with the
    /// closing brackets cut off), which `--list` reported as an empty library
    /// with exit 0.
    #[test]
    fn an_unparsable_library_is_reported_and_never_overwritten() {
        let (mut library, path) = library_with_raw_file(r#"[{"id": "alpha-1", "name": "Alpha""#);
        let before = std::fs::read_to_string(&path).unwrap();

        assert_eq!(
            library.load_status(),
            LoadStatus::Unparsable,
            "a file that is not a game list must be recorded as such; an empty library and \
             an unreadable one are the same value to every caller otherwise, which is the \
             whole defect"
        );
        assert!(library.is_empty());

        // Every mutating entry point goes through `save`. Each one must fail
        // rather than write, and the file must be byte-identical afterwards.
        let added = Game::new_named("Hades");
        let error = library
            .add(added.clone())
            .expect_err("adding over an unreadable file must not silently succeed");
        // The refusal is its own variant, not an `io::Error` carrying an
        // `InvalidData` kind (`ARCH-10`): a caller can tell "nothing was
        // written because the file was unreadable" from "the write failed"
        // without parsing the message.
        assert!(
            matches!(
                error,
                PersistenceError::WouldDiscardUnreadable {
                    reason: "not a game list",
                    ..
                }
            ),
            "expected the refusal, got {error:?}"
        );
        assert!(
            error.to_string().contains("has not been overwritten"),
            "the error has to tell the user their file was left alone, not just that \
             something failed: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "the file must be byte-identical after a refused write"
        );

        // The other three mutators are the same guard; assert them from a fresh
        // library each so one refusing does not mask the next.
        for (name, mutate) in [
            (
                "update",
                &(|library: &mut Library, game: Game| library.update(game))
                    as &dyn Fn(&mut Library, Game) -> _,
            ),
            ("mark_played", &|library: &mut Library, _game: Game| {
                library.mark_played("alpha-1")
            }),
            ("remove", &|library: &mut Library, _game: Game| {
                library.remove("alpha-1")
            }),
        ] {
            let (mut library, path) =
                library_with_raw_file(r#"[{"id": "alpha-1", "name": "Alpha""#);
            if let Err(error) = mutate(&mut library, added.clone()) {
                assert!(
                    matches!(error, PersistenceError::WouldDiscardUnreadable { .. }),
                    "{name} failed for the wrong reason: {error:?}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                before,
                "{name} wrote over a file the app could not read"
            );
        }
    }

    /// **A file that is not there is a first run, and saving still works.**
    ///
    /// The anti-vacuity half of the test above: a gate that refused every write
    /// would pass it. `Absent` is the state of a fresh install and must stay
    /// writable, or the app could never save anything.
    #[test]
    fn a_missing_library_file_is_absent_and_still_writable() {
        let path = std::env::temp_dir().join(format!("gh-absent-{}.json", new_id()));
        let mut library = Library::new_at(Some(path.clone()), FROZEN_NOW);
        assert_eq!(library.load_status(), LoadStatus::Absent);
        assert!(!LoadStatus::Absent.is_destructive_to_save_over());

        library.add(Game::new_named("Hades")).unwrap();
        assert!(path.exists(), "a fresh library must still be able to save");
    }

    /// **A file that parses is `Loaded`, whatever the entries did.**
    ///
    /// The reference skips a non-object entry and an entry with an empty name,
    /// deliberately, and that tolerance must not be mistaken for a failed read —
    /// otherwise a file with one junk entry would become unwritable.
    #[test]
    fn a_list_with_skipped_entries_is_loaded_not_failed() {
        let library = library_from(json!([
            {"id": "keep-1", "name": "Keep"},
            "not an object",
            {"id": "blank-1", "name": ""},
            {"id": "keep-2", "name": "Keep Two"},
        ]));
        assert_eq!(library.load_status(), LoadStatus::Loaded);
        assert!(!library.load_status().is_destructive_to_save_over());
        assert_eq!(library.len(), 2, "the two valid entries survive");
    }

    /// **Bytes that are not UTF-8 are unreadable, not unparsable.**
    ///
    /// `read_to_string` rejects them before any parser sees them, so the status
    /// has to come from the read failing while the file exists. Both are
    /// refusals, but the message a user gets should name the real cause.
    #[test]
    fn a_file_that_is_not_utf8_is_unreadable() {
        let directory = std::env::temp_dir().join(format!("gh-lib-bytes-{}", new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("games.json");
        std::fs::write(&path, b"[{\"name\": \"Caf\xe9\"}]").unwrap();

        let library = Library::new_at(Some(path.clone()), FROZEN_NOW);
        assert_eq!(library.load_status(), LoadStatus::Unreadable);
        assert!(
            library.save().is_err(),
            "invalid UTF-8 is exactly the file a refused write protects"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"[{\"name\": \"Caf\xe9\"}]");
    }

    /// **The gate is on the source path, not on writing at all.**
    ///
    /// `save_to` writes somewhere the library did not read from, which is how
    /// the oracle fixture tests work and is not an overwrite of anything. A gate
    /// that covered it would break them, so the boundary is pinned here.
    #[test]
    fn save_to_an_explicit_path_is_not_gated() {
        let (library, _path) = library_with_raw_file(r#"[{"id": "alpha-1", "name": "Alpha""#);
        let out = std::env::temp_dir().join(format!("gh-out-{}.json", new_id()));
        library
            .save_to(&out)
            .expect("writing to a path that was never read from must not be gated");
        assert!(out.exists());
    }

    #[test]
    fn a_missing_file_yields_an_empty_library() {
        let path = std::env::temp_dir().join(format!("gh-missing-{}.json", new_id()));
        let library = Library::new_at(Some(path), FROZEN_NOW);
        assert!(library.is_empty());
    }

    #[test]
    fn a_tolerated_load_keeps_the_surviving_entries() {
        let library = library_from(json!([
            "a string",
            42,
            null,
            {"id": "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq", "name": "Real"},
        ]));
        assert_eq!(library.len(), 1);
        assert_eq!(library.all("name")[0].name, "Real");
    }

    #[test]
    fn entries_with_an_empty_name_are_skipped() {
        let library = library_from(json!([
            {"id": "rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr", "name": ""},
            {"id": "ssssssssssssssssssssssssssssssss", "name": "Kept"},
        ]));
        assert_eq!(library.len(), 1);
        assert_eq!(library.all("name")[0].name, "Kept");
    }

    #[test]
    fn duplicate_ids_keep_the_last_value_in_the_first_position() {
        let library = library_from(json!([
            {"id": "tttttttttttttttttttttttttttttttt", "name": "First", "added": 1.0},
            {"id": "uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuu", "name": "Middle", "added": 2.0},
            {"id": "tttttttttttttttttttttttttttttttt", "name": "Second", "added": 3.0},
        ]));
        assert_eq!(library.len(), 2);
        let added_order: Vec<&str> = library
            .all("added")
            .iter()
            .map(|game| game.name.as_str())
            .collect();
        assert_eq!(added_order, ["Second", "Middle"]);
        assert_eq!(
            library
                .get("tttttttttttttttttttttttttttttttt")
                .unwrap()
                .name,
            "Second"
        );
    }

    #[test]
    fn save_round_trips_through_the_file() {
        let directory = std::env::temp_dir().join(format!("gh-save-{}", new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("games.json");

        let mut library = Library::new_at(Some(path.clone()), FROZEN_NOW);
        library
            .add(Game::from_dict_at(
                &object(json!({"id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "name": "Half-Life"})),
                FROZEN_NOW,
            ))
            .unwrap();

        let reloaded = Library::new_at(Some(path.clone()), FROZEN_NOW);
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.all("name")[0].name, "Half-Life");

        // Saving is atomic: the temporary file is renamed away, not left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a .tmp file was left behind");

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn remove_is_a_no_op_for_an_unknown_id() {
        let library = library_from(json!([{"id": "a", "name": "A"}]));
        let mut library = library;
        library.remove("nope").unwrap();
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn mark_played_updates_the_timestamp_and_persists() {
        let mut library = library_from(json!([{"id": "a", "name": "A"}]));
        library.mark_played("a").unwrap();
        let game = library.get("a").unwrap();
        assert!(
            (game.last_played - now()).abs() < 5.0,
            "mark_played stamps the current time"
        );
        // An unknown id is a quiet no-op, as in Python.
        library.mark_played("missing").unwrap();
    }

    #[test]
    fn search_matches_the_name_or_the_display_category() {
        let library = library_from(json!([
            {"id": "1", "name": "Portal 2", "category": "Puzzle"},
            {"id": "2", "name": "Doom", "category": "Shooter"},
            {"id": "3", "name": "Puzzle Quest", "category": ""},
            {"id": "4", "name": "Hades", "category": "  roguelike  "},
        ]));
        let names = |query: &str, category: &str| -> Vec<String> {
            library
                .search(query, category, "name")
                .iter()
                .map(|game| game.name.clone())
                .collect()
        };

        // A category term finds its games (P-04), and a name term finds itself.
        assert_eq!(names("puzzle", ""), ["Portal 2", "Puzzle Quest"]);
        assert_eq!(names("portal", ""), ["Portal 2"]);
        // Case-insensitive, and the query is stripped.
        assert_eq!(names("DOOM", ""), ["Doom"]);
        assert_eq!(names("  doom  ", ""), ["Doom"]);
        // "All" is not a filter; a real category is.
        assert_eq!(names("", "All").len(), 4);
        assert_eq!(names("", "Puzzle"), ["Portal 2"]);
        // A blank category folds to Uncategorized.
        assert_eq!(names("", UNCATEGORIZED), ["Puzzle Quest"]);
        // The padded category is matched in its stripped form.
        assert_eq!(names("", "roguelike"), ["Hades"]);
        assert_eq!(names("", "").len(), 4);
    }

    #[test]
    fn categories_sort_uncategorized_last_then_alphabetically() {
        let library = library_from(json!([
            {"id": "1", "name": "Portal 2", "category": "Puzzle"},
            {"id": "2", "name": "Doom", "category": "Shooter"},
            {"id": "3", "name": "Puzzle Quest", "category": ""},
            {"id": "4", "name": "Hades", "category": "  roguelike  "},
        ]));
        assert_eq!(
            library.categories(),
            ["Puzzle", "roguelike", "Shooter", "Uncategorized"]
        );
    }

    /// **`BUG-11` is withdrawn: case-variant categories are not duplicated.**
    ///
    /// The recorded finding was that `Vec::dedup` after the sort loses what
    /// Python's `set` keeps, because the sort key (`to_lowercase()`) and the
    /// dedup key (`PartialEq`) are different keys. The premise is wrong: the
    /// sort's **primary** key *is* the folding, so every entry whose folded form
    /// is equal lands adjacent, and `dedup` then removes exactly what the `set`
    /// removes. Nothing can sort between two entries that compare equal on the
    /// leading term.
    ///
    /// Measured against the reference on a 40-game fixture (ten base categories
    /// × the spelling, upper, lower and alternating case): the reference returns
    /// **39** entries and so does this — the one collapse is `RPG`, whose upper
    /// form is itself. Same length, same set, and the same ten fold-groups with
    /// the same members. The two lists are *not* identical element-for-element,
    /// and the assertion below deliberately does not claim they are: within a
    /// group of entries that fold together, the reference's order is its `set`'s
    /// hash order, which is arbitrary, while this one keeps insertion order.
    /// A dropdown cannot distinguish them, and pinning the hash order would pin
    /// an implementation detail of CPython.
    #[test]
    fn case_variant_categories_fold_into_the_same_groups_as_python() {
        let bases = [
            "Puzzle", "Action", "RPG", "Sim", "Shooter", "Strategy", "Racing", "Sports", "Horror",
            "Party",
        ];
        let mut entries = Vec::new();
        for base in bases {
            let alternating: String = base
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i % 2 == 0 {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .collect();
            for spelling in [
                base.to_string(),
                base.to_uppercase(),
                base.to_lowercase(),
                alternating,
            ] {
                entries.push(json!({
                    "id": format!("g{}", entries.len()),
                    "name": format!("Game {}", entries.len()),
                    "category": spelling,
                }));
            }
        }
        let library = library_from(Value::Array(entries));
        let found = library.categories();

        // The count is the reference's, and it is 39 rather than 40 because
        // `"RPG".to_uppercase()` is `"RPG"` — the one spelling that collides
        // with its own base.
        assert_eq!(found.len(), 39, "got {found:?}");

        // Grouped by the key the reference sorts on, the two agree exactly.
        // This is the assertion the recorded finding would have failed.
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for name in &found {
            let key = name.to_lowercase();
            match groups.last_mut() {
                Some((last, members)) if *last == key => members.push(name.clone()),
                _ => groups.push((key, vec![name.clone()])),
            }
        }
        assert_eq!(groups.len(), 10, "got {groups:?}");
        // Nine groups hold four spellings each; `rpg` holds three, because
        // `"RPG".to_uppercase()` is `"RPG"` — the one spelling in this fixture
        // that collides with its own base, and the reason the total is 39.
        for (key, members) in &groups {
            let expected = if key == "rpg" { 3 } else { 4 };
            assert_eq!(
                members.len(),
                expected,
                "the {key} group lost or gained a spelling: {members:?}"
            );
        }
        assert_eq!(
            groups.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            [
                "action", "horror", "party", "puzzle", "racing", "rpg", "shooter", "sim", "sports",
                "strategy"
            ]
        );
        // `category` is trimmed and an empty one becomes `Uncategorized`, which
        // this fixture does not exercise — the existing test above does.
    }

    #[test]
    fn sorting_matches_python_including_the_name_tie_break() {
        let library = library_from(json!([
            {"id": "1", "name": "zeta", "added": 100.0, "last_played": 50.0},
            {"id": "2", "name": "Alpha", "added": 300.0, "last_played": 0.0},
            {"id": "3", "name": "beta", "added": 200.0, "last_played": 900.0},
            {"id": "4", "name": "Alpha", "added": 400.0, "last_played": 10.0},
        ]));
        let names = |sort: &str| -> Vec<String> {
            library
                .all(sort)
                .iter()
                .map(|game| game.name.clone())
                .collect()
        };
        assert_eq!(names("name"), ["Alpha", "Alpha", "beta", "zeta"]);
        assert_eq!(names("recent"), ["beta", "zeta", "Alpha", "Alpha"]);
        assert_eq!(names("added"), ["Alpha", "Alpha", "beta", "zeta"]);
    }

    #[test]
    fn timestamp_sorts_break_ties_by_name() {
        // Without the explicit tie-break the two entries below would come out
        // in insertion order, which is what Python's `(key, name.lower())` key
        // prevents.
        let library = library_from(json!([
            {"id": "1", "name": "zeta", "added": 5.0, "last_played": 5.0},
            {"id": "2", "name": "Alpha", "added": 5.0, "last_played": 5.0},
        ]));
        assert_eq!(
            library
                .all("added")
                .iter()
                .map(|game| game.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "zeta"]
        );
        assert_eq!(
            library
                .all("recent")
                .iter()
                .map(|game| game.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "zeta"]
        );
    }

    #[test]
    fn a_negative_zero_timestamp_ties_like_python() {
        // `BUG-20`. `-0.0` survives the load — `timestamp`'s guard is
        // `numeric >= 0.0`, which `-0.0` satisfies — so a hand-edited file
        // reaches the sorts with the sign intact. CPython's key is
        // `(-g.last_played, g.name.lower())`, whose comparison says `-0.0` and
        // `0.0` are equal, so these two tie and the name decides:
        // `sorted` gives `['AAA', 'ZZZ']`. `total_cmp` ordered them by the sign
        // of the zero instead and answered `["ZZZ", "AAA"]`.
        //
        // Written in the reference's own order of events: the fixture is a file
        // the real loader reads, and the assertion is on the names the real
        // sorts return.
        let library = library_from(json!([
            {"id": "1", "name": "ZZZ", "added": 0.0, "last_played": 0.0},
            {"id": "2", "name": "AAA", "added": -0.0, "last_played": -0.0},
        ]));
        // The premise, asserted rather than assumed: if the sign were lost on
        // the way in, this test would pass against the old body too and prove
        // nothing. `all("name")` is the one sort whose key does not read a
        // timestamp, so it is the harmless way to get at the loaded values.
        let loaded = library.all("name");
        assert_eq!(loaded[0].name, "AAA");
        assert_eq!(loaded[0].added, 0.0);
        assert!(
            loaded[0].added.is_sign_negative(),
            "-0.0 must survive the load, or this test has no subject"
        );

        let names = |sort: &str| -> Vec<String> {
            library
                .all(sort)
                .iter()
                .map(|game| game.name.clone())
                .collect()
        };
        assert_eq!(names("recent"), ["AAA", "ZZZ"]);
        assert_eq!(names("added"), ["AAA", "ZZZ"]);
    }

    #[test]
    fn a_whole_library_survives_an_out_of_range_literal() {
        // FINDINGS F-G / DECISIONS D-16: `serde_json` rejects `1e400` outright,
        // which would discard every other game in the file.
        let directory = std::env::temp_dir().join(format!("gh-oob-{}", new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("games.json");
        std::fs::write(
            &path,
            format!(
                r#"[{{"id": "{}", "name": "Ov", "added": 1e400}},
                   {{"id": "{}", "name": "Fine", "added": 5.0}}]"#,
                "1".repeat(32),
                "2".repeat(32)
            ),
        )
        .unwrap();

        let library = Library::new_at(Some(path), FROZEN_NOW);
        assert_eq!(library.len(), 2, "the rest of the library must survive");
        assert_eq!(library.get(&"1".repeat(32)).unwrap().added, FROZEN_NOW);
        assert_eq!(library.get(&"2".repeat(32)).unwrap().added, 5.0);

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
