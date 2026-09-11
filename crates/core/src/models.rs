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

use std::path::{Path, PathBuf};

use serde::ser::{Serialize, SerializeStruct, Serializer};
use serde_json::{Map, Value};

use crate::json;
use crate::paths;

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
fn new_id() -> String {
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

/// Loads, mutates and persists a collection of [`Game`]s. `models.py:120-209`.
#[derive(Debug, Clone)]
pub struct Library {
    path: PathBuf,
    /// Insertion-ordered, because Python's `dict` is: two entries that tie
    /// under a sort keep their file order, and `Library::all` relies on a
    /// stable sort to preserve that.
    games: Vec<Game>,
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
        };
        match now {
            Some(now) => library.load_at(now),
            None => library.load(),
        }
        library
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
    pub fn load(&mut self) {
        self.load_at(now());
    }

    /// [`Self::load`] with the clock injected.
    pub fn load_at(&mut self, now: f64) {
        self.games.clear();
        let Ok(source) = std::fs::read_to_string(&self.path) else {
            return;
        };
        let Ok(Value::Array(entries)) = json::parse_lenient(&source) else {
            return;
        };
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

    /// Inserts, replacing an existing entry with the same id in place.
    ///
    /// `Library::load`'s dict assignment keeps the *first* position of a
    /// duplicated id while taking the *last* value, which is what the
    /// `duplicate_ids` fixture pins.
    fn upsert(&mut self, game: Game) {
        match self.games.iter_mut().find(|existing| existing.id == game.id) {
            Some(slot) => *slot = game,
            None => self.games.push(game),
        }
    }

    /// Port of `Library.save` (`models.py:149-154`).
    ///
    /// The payload is `self.all("name")` — saving re-sorts the in-memory dict
    /// into name order, so the file on disk is always sorted.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&self.path)
    }

    /// [`Self::save`] to an explicit path, as Python's `lib.path = out;
    /// lib.save()` allows. Used by the oracle fixture tests so they never write
    /// into the checked-in fixture directory.
    pub fn save_to(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        json::write_python_file(path.as_ref(), &self.all("name"))
    }

    /// Port of `Library.all` (`models.py:156-163`).
    ///
    /// Both timestamp sorts carry an explicit name tie-break; without it
    /// `recent` and `added` diverge from Python on equal keys. `sort_by` is a
    /// stable sort in both languages, so entries equal on both terms keep their
    /// insertion order.
    pub fn all(&self, sort: &str) -> Vec<&Game> {
        let mut games: Vec<&Game> = self.games.iter().collect();
        match sort {
            "recent" => games.sort_by(|a, b| {
                b.last_played
                    .total_cmp(&a.last_played)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
            "added" => games.sort_by(|a, b| {
                b.added
                    .total_cmp(&a.added)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
            _ => games.sort_by_key(|a| a.name.to_lowercase()),
        }
        games
    }

    /// Port of `Library.get`.
    pub fn get(&self, game_id: &str) -> Option<&Game> {
        self.games.iter().find(|game| game.id == game_id)
    }

    /// Port of `Library.add` (`models.py:168-171`).
    pub fn add(&mut self, game: Game) -> std::io::Result<()> {
        self.upsert(game);
        self.save()
    }

    /// Port of `Library.remove`: a no-op when the id is unknown, and the file
    /// is only rewritten when something actually changed.
    pub fn remove(&mut self, game_id: &str) -> std::io::Result<()> {
        let Some(position) = self.games.iter().position(|game| game.id == game_id) else {
            return Ok(());
        };
        self.games.remove(position);
        self.save()
    }

    /// Port of `Library.update`.
    pub fn update(&mut self, game: Game) -> std::io::Result<()> {
        self.upsert(game);
        self.save()
    }

    /// Port of `Library.mark_played` (`models.py:182-186`).
    pub fn mark_played(&mut self, game_id: &str) -> std::io::Result<()> {
        if let Some(game) = self.games.iter_mut().find(|game| game.id == game_id) {
            game.last_played = now();
            return self.save();
        }
        Ok(())
    }

    /// Port of `Library.search` (`models.py:188-199`).
    ///
    /// The query matches the name **or** the display category, so searching a
    /// category name finds its games (parity item P-04). `category` equal to
    /// `"All"` is not a filter.
    pub fn search(&self, query: &str, category: &str, sort: &str) -> Vec<&Game> {
        let query = query.trim().to_lowercase();
        let mut games = self.all(sort);
        if !category.is_empty() && category != "All" {
            games.retain(|game| game.display_category() == category);
        }
        if query.is_empty() {
            return games;
        }
        games.retain(|game| {
            game.name.to_lowercase().contains(&query)
                || game.display_category().to_lowercase().contains(&query)
        });
        games
    }

    /// Port of `Library.categories` (`models.py:201-203`): distinct display
    /// categories, case-insensitively sorted with [`UNCATEGORIZED`] last.
    pub fn categories(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .games
            .iter()
            .map(|game| game.display_category().to_owned())
            .collect();
        found.sort_by(|a, b| {
            (a == UNCATEGORIZED)
                .cmp(&(b == UNCATEGORIZED))
                .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        });
        found.dedup();
        found
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
        assert!(matches!(&id[16..17], "8" | "9" | "a" | "b"), "variant nibble");
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

        let empty = Game::from_dict_at(&object(json!({"name": "Empty", "category": ""})), FROZEN_NOW);
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
        assert_eq!(library.get("tttttttttttttttttttttttttttttttt").unwrap().name, "Second");
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
