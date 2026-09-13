//! Persisted application preferences. A port of `gamehandler/settings.py`.
//!
//! Like [`crate::models::Game`], the field order here is the wire format:
//! `dataclasses.asdict` and `serde` both emit in declaration order, so the 18
//! fields must stay in the order `settings.py` declares them or a file written
//! by one implementation will not be byte-identical to the other's.
//!
//! # Type tolerance
//!
//! Python's `from_dict` is `cls(**cleaned)`, which does **no** type checking:
//! a hand-edited `"close_on_launch": "yes"` is stored as the string `"yes"` and
//! written back out that way. The three enum-ish fields survive that because
//! they are validated by membership afterwards (`"yes" not in COLOR_SCHEMES`
//! resets it), but the other fifteen do not — `Settings` is a dataclass with no
//! runtime types.
//!
//! This port has real types, so a value of the wrong type is treated as
//! *absent*: the field keeps its default and a valid value for a neighbouring
//! key still applies. That is the only sane reading of a file the app itself
//! never writes, and it is unreachable from a file we produced. The oracle
//! fixtures do not cover it, so nothing measurable diverges.
//!
//! # Two deliberate divergences from `Settings.load`
//!
//! * **Invalid UTF-8 yields the defaults instead of raising.** Python's
//!   `Library.load` catches `UnicodeDecodeError`, but `Settings.load` catches
//!   only `json.JSONDecodeError` and `OSError` — so a single bad byte in
//!   `settings.json` raises out of `Settings.load`, which `Backend.__init__`
//!   calls, and the Python app dies at startup with a traceback. Refusing to
//!   start is not a behaviour worth reproducing: a preferences file that cannot
//!   be read is exactly the case the defaults exist for. Same reasoning as
//!   D-14, and recorded by the oracle's `encoding_and_shape` section.
//! * **A leading BOM is stripped.** Python does not strip one, so a
//!   BOM-prefixed `settings.json` parses to nothing and the next `save` writes
//!   the defaults over the user's real preferences — one byte, added by an
//!   editor or a sync tool, resets every setting. D-21 rejects that, in
//!   `settings` for exactly the same reason it rejects it in the library.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::json::{self, PersistenceError};
use crate::models::{SORT_MODES, SYSTEM_WINE};
use crate::paths;

/// The colour schemes the UI knows. `settings.py:13`.
pub const COLOR_SCHEMES: [&str; 3] = ["system", "light", "dark"];

/// The library layouts the UI knows. `settings.py:14`.
pub const VIEW_MODES: [&str; 2] = ["grid", "list"];

/// User preferences stored under the XDG config directory.
///
/// The field order is the wire format. Do not reorder.
#[derive(Clone, Debug, Serialize)]
pub struct Settings {
    pub color_scheme: String,
    pub view_mode: String,
    pub sort_mode: String,
    pub default_runner: String,
    pub default_mangohud: bool,
    pub default_gamemode: bool,
    pub default_prefer_sdl: bool,
    pub default_esync: bool,
    pub default_fsync: bool,
    pub default_dxvk: bool,
    pub default_vkd3d: bool,
    pub default_nvapi: bool,
    pub default_fsr: bool,
    pub default_battleye: bool,
    pub default_eac: bool,
    pub default_gamescope: bool,
    pub default_virtual_desktop: bool,
    pub close_on_launch: bool,
    /// Where this value lives on disk. Not on the wire.
    ///
    /// [`Library`](crate::models::Library) carries its path for the same
    /// reason: the value and its store must not be separable, or a test
    /// writes the user's file while asserting about a fixture. [`Settings::load`]
    /// stores the path it read (or was given), [`Settings::save`] writes it
    /// back, and `#[serde(skip)]` keeps `settings.json` byte-identical to
    /// what the reference writes.
    #[serde(skip)]
    path: PathBuf,
}

impl Settings {
    /// Port of `Settings.from_dict` (`settings.py:40-51`).
    ///
    /// Unknown keys are dropped. The three enumerated fields are reset to their
    /// defaults when the stored value is not one of the accepted strings — the
    /// fallbacks are the fixed literals `"dark"`, `"grid"` and `"name"`
    /// (FINDINGS F-E), *not* the first member of each list.
    pub fn from_dict(data: &Map<String, Value>) -> Self {
        let mut settings = Self::default();

        if let Some(value) = text(data, "color_scheme") {
            settings.color_scheme = value;
        }
        if let Some(value) = text(data, "view_mode") {
            settings.view_mode = value;
        }
        if let Some(value) = text(data, "sort_mode") {
            settings.sort_mode = value;
        }
        if let Some(value) = text(data, "default_runner") {
            settings.default_runner = value;
        }

        if let Some(value) = flag(data, "default_mangohud") {
            settings.default_mangohud = value;
        }
        if let Some(value) = flag(data, "default_gamemode") {
            settings.default_gamemode = value;
        }
        if let Some(value) = flag(data, "default_prefer_sdl") {
            settings.default_prefer_sdl = value;
        }
        if let Some(value) = flag(data, "default_esync") {
            settings.default_esync = value;
        }
        if let Some(value) = flag(data, "default_fsync") {
            settings.default_fsync = value;
        }
        if let Some(value) = flag(data, "default_dxvk") {
            settings.default_dxvk = value;
        }
        if let Some(value) = flag(data, "default_vkd3d") {
            settings.default_vkd3d = value;
        }
        if let Some(value) = flag(data, "default_nvapi") {
            settings.default_nvapi = value;
        }
        if let Some(value) = flag(data, "default_fsr") {
            settings.default_fsr = value;
        }
        if let Some(value) = flag(data, "default_battleye") {
            settings.default_battleye = value;
        }
        if let Some(value) = flag(data, "default_eac") {
            settings.default_eac = value;
        }
        if let Some(value) = flag(data, "default_gamescope") {
            settings.default_gamescope = value;
        }
        if let Some(value) = flag(data, "default_virtual_desktop") {
            settings.default_virtual_desktop = value;
        }
        if let Some(value) = flag(data, "close_on_launch") {
            settings.close_on_launch = value;
        }

        if !COLOR_SCHEMES.contains(&settings.color_scheme.as_str()) {
            settings.color_scheme = "dark".to_string();
        }
        if !VIEW_MODES.contains(&settings.view_mode.as_str()) {
            settings.view_mode = "grid".to_string();
        }
        if !SORT_MODES.contains(&settings.sort_mode.as_str()) {
            settings.sort_mode = "name".to_string();
        }

        settings
    }

    /// Where this value will be written by [`Settings::save`].
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Port of `Settings.load` (`settings.py:56-67`).
    ///
    /// `None` means the configured path. A missing file, an unreadable one, one
    /// that is not valid UTF-8, invalid JSON, trailing garbage and a top-level
    /// value that is not an object all yield the defaults. The UTF-8 case is a
    /// deliberate divergence — Python raises there — and `read_to_string` gives
    /// it to us for free, since it rejects invalid UTF-8 rather than replacing
    /// it.
    ///
    /// Whatever is returned remembers `path`: a settings value that forgot
    /// where it was loaded from could only ever save to the configured file,
    /// and a test driving a mutation would write the user's `settings.json`.
    pub fn load(path: Option<PathBuf>) -> Self {
        let target = path.unwrap_or_else(paths::settings_file);
        let stored = std::fs::read_to_string(&target)
            .ok()
            .and_then(|source| json::parse_lenient(&source).ok())
            .and_then(|value| match value {
                Value::Object(fields) => Some(Self::from_dict(&fields)),
                _ => None,
            })
            .unwrap_or_default();
        Self {
            path: target,
            ..stored
        }
    }

    /// Port of `Settings.save` (`settings.py:69-74`) to the loaded path.
    pub fn save(&self) -> Result<(), PersistenceError> {
        self.save_to(&self.path)
    }

    /// [`Self::save`] to an explicit path, as `Settings.save(path)` allows.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        json::write_python_file(path.as_ref(), self)
    }
}

/// Same preferences, same value — wherever each was loaded from.
///
/// A manual impl rather than a derive, because the derive would compare
/// [`Settings::path`] and two values holding the same preferences from
/// different files would be unequal. The store is not the value (compare
/// [`Library`](crate::models::Library), which excludes its path from identity
/// by not implementing this trait at all). Every value field is listed, and
/// `every_field_participates_in_equality` fails if one is added without
/// joining this list.
impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.color_scheme == other.color_scheme
            && self.view_mode == other.view_mode
            && self.sort_mode == other.sort_mode
            && self.default_runner == other.default_runner
            && self.default_mangohud == other.default_mangohud
            && self.default_gamemode == other.default_gamemode
            && self.default_prefer_sdl == other.default_prefer_sdl
            && self.default_esync == other.default_esync
            && self.default_fsync == other.default_fsync
            && self.default_dxvk == other.default_dxvk
            && self.default_vkd3d == other.default_vkd3d
            && self.default_nvapi == other.default_nvapi
            && self.default_fsr == other.default_fsr
            && self.default_battleye == other.default_battleye
            && self.default_eac == other.default_eac
            && self.default_gamescope == other.default_gamescope
            && self.default_virtual_desktop == other.default_virtual_desktop
            && self.close_on_launch == other.close_on_launch
    }
}

impl Default for Settings {
    /// The dataclass defaults, including the `"dark"`/`"grid"` fallbacks.
    ///
    /// The path is the configured one: a default-built value that is saved
    /// goes where the reference saves, and a test that must not touch it
    /// loads from an explicit path instead.
    fn default() -> Self {
        Self {
            color_scheme: "dark".to_string(),
            view_mode: "grid".to_string(),
            sort_mode: "name".to_string(),
            default_runner: SYSTEM_WINE.to_string(),
            default_mangohud: false,
            default_gamemode: false,
            default_prefer_sdl: false,
            default_esync: true,
            default_fsync: true,
            default_dxvk: true,
            default_vkd3d: true,
            default_nvapi: false,
            default_fsr: false,
            default_battleye: true,
            default_eac: true,
            default_gamescope: false,
            default_virtual_desktop: false,
            close_on_launch: false,
            path: paths::settings_file(),
        }
    }
}

/// A string field, or `None` when absent or not a string.
fn text(data: &Map<String, Value>, key: &str) -> Option<String> {
    data.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A boolean field, or `None` when absent or not a boolean.
fn flag(data: &Map<String, Value>, key: &str) -> Option<bool> {
    data.get(key).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("expected an object, got {other}"),
        }
    }

    fn from(value: Value) -> Settings {
        Settings::from_dict(&object(value))
    }

    #[test]
    fn defaults_match_the_python_dataclass() {
        let settings = Settings::default();
        assert_eq!(settings.color_scheme, "dark");
        assert_eq!(settings.view_mode, "grid");
        assert_eq!(settings.sort_mode, "name");
        assert_eq!(settings.default_runner, "wine-system");
        for (name, value) in [
            ("default_mangohud", settings.default_mangohud),
            ("default_gamemode", settings.default_gamemode),
            ("default_prefer_sdl", settings.default_prefer_sdl),
            ("default_nvapi", settings.default_nvapi),
            ("default_fsr", settings.default_fsr),
            ("default_gamescope", settings.default_gamescope),
            ("default_virtual_desktop", settings.default_virtual_desktop),
            ("close_on_launch", settings.close_on_launch),
        ] {
            assert!(!value, "{name} should default to false");
        }
        for (name, value) in [
            ("default_esync", settings.default_esync),
            ("default_fsync", settings.default_fsync),
            ("default_dxvk", settings.default_dxvk),
            ("default_vkd3d", settings.default_vkd3d),
            ("default_battleye", settings.default_battleye),
            ("default_eac", settings.default_eac),
        ] {
            assert!(value, "{name} should default to true");
        }
    }

    #[test]
    fn field_order_matches_the_python_dataclass() {
        // Asserted on the written text, not on a `Value`: a `serde_json::Map` is
        // ordered only when `preserve_order` is unified on, which depends on
        // which crates are in the build. See the same note in `models`.
        let text = crate::json::to_python_string(&Settings::default()).unwrap();
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
                "color_scheme",
                "view_mode",
                "sort_mode",
                "default_runner",
                "default_mangohud",
                "default_gamemode",
                "default_prefer_sdl",
                "default_esync",
                "default_fsync",
                "default_dxvk",
                "default_vkd3d",
                "default_nvapi",
                "default_fsr",
                "default_battleye",
                "default_eac",
                "default_gamescope",
                "default_virtual_desktop",
                "close_on_launch",
            ]
        );
        assert_eq!(keys.len(), 18);
    }

    #[test]
    fn unknown_keys_are_dropped() {
        let settings = from(json!({"legacy": true, "color_scheme": "light"}));
        assert_eq!(settings.color_scheme, "light");
        let written = serde_json::to_value(&settings).unwrap();
        assert!(written.get("legacy").is_none());
    }

    #[test]
    fn invalid_enum_values_fall_back_to_fixed_literals() {
        // FINDINGS F-E: the fallbacks are literals, not `COLOR_SCHEMES[0]` — so
        // "neon" becomes "dark" even though "system" is the first member.
        assert_eq!(from(json!({"color_scheme": "neon"})).color_scheme, "dark");
        assert_eq!(from(json!({"view_mode": "carousel"})).view_mode, "grid");
        assert_eq!(from(json!({"sort_mode": "chaos"})).sort_mode, "name");
        // Every accepted value survives.
        for scheme in COLOR_SCHEMES {
            assert_eq!(from(json!({"color_scheme": scheme})).color_scheme, scheme);
        }
        for mode in VIEW_MODES {
            assert_eq!(from(json!({"view_mode": mode})).view_mode, mode);
        }
        for mode in SORT_MODES {
            assert_eq!(from(json!({"sort_mode": mode})).sort_mode, mode);
        }
    }

    #[test]
    fn a_null_enum_falls_back_too() {
        // `null not in COLOR_SCHEMES` in Python, so the same path.
        assert_eq!(from(json!({"color_scheme": null})).color_scheme, "dark");
        assert_eq!(from(json!({"view_mode": null})).view_mode, "grid");
        assert_eq!(from(json!({"sort_mode": null})).sort_mode, "name");
    }

    #[test]
    fn a_value_of_the_wrong_type_keeps_the_default() {
        // Python would store the wrong-typed value verbatim; see the module
        // note. The neighbouring valid key must still apply.
        let settings = from(json!({"close_on_launch": "yes", "color_scheme": "light"}));
        assert!(!settings.close_on_launch);
        assert_eq!(settings.color_scheme, "light");

        let settings = from(json!({"default_runner": 42, "default_esync": "no"}));
        assert_eq!(settings.default_runner, "wine-system");
        assert!(settings.default_esync);
    }

    #[test]
    fn booleans_are_read_in_both_directions() {
        let settings = from(json!({"default_esync": false, "close_on_launch": true}));
        assert!(!settings.default_esync);
        assert!(settings.close_on_launch);
    }

    #[test]
    fn load_returns_defaults_for_every_broken_file() {
        let directory = std::env::temp_dir().join(format!("gh-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let cases: [(&str, &str); 5] = [
            ("missing", ""),             // never written
            ("empty", ""),               // zero bytes
            ("malformed", "{not json"),  // unparseable
            ("trailing", "{}\ngarbage"), // extra data
            ("notdict", "[1,2,3]"),      // parses, wrong shape
        ];
        for (label, source) in cases {
            let path = directory.join(format!("{label}.json"));
            if label != "missing" {
                std::fs::write(&path, source).unwrap();
            }
            assert_eq!(
                Settings::load(Some(path)),
                Settings::default(),
                "load({label}) should fall back to the defaults"
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn invalid_utf8_falls_back_to_the_defaults_instead_of_raising() {
        // Python lets `UnicodeDecodeError` escape `Settings.load`, which
        // `Backend.__init__` calls — the app dies at startup on one bad byte.
        // The port returns the defaults (see the module note).
        //
        // **Two byte positions, and the second is the one with teeth.** The
        // first draft had only `between_string_and_brace`, which passes for two
        // independent reasons and so could not fail for its own: after lossy
        // decoding the text is still unparseable *and* the corrupted
        // `color_scheme` is not in `COLOR_SCHEMES`. Swapping the
        // `read_to_string` guard for `read` + `from_utf8_lossy` left the whole
        // suite green — the mutation survived.
        //
        // `inside_a_free_field_value` closes it. `default_runner` is a free
        // field with no allowlist, so lossy decoding parses cleanly and yields
        // `/usr/bin/lig\u{fffd}ht` — silently accepting a corrupted path where
        // the defaults are correct. That case fails under the mutation. Verified
        // both ways: Python raises `UnicodeDecodeError` for the 0xff byte in
        // *either* position, and the divergence is therefore about position, not
        // about which byte.
        let cases: &[(&str, &[u8])] = &[
            (
                "between_string_and_brace",
                b"{\"color_scheme\": \"light\"\xff}",
            ),
            (
                "inside_a_free_field_value",
                b"{\"default_runner\": \"/usr/bin/lig\xffht\"}",
            ),
        ];

        let directory =
            std::env::temp_dir().join(format!("gh-settings-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        for (label, bytes) in cases {
            let path = directory.join(format!("{label}.json"));
            std::fs::write(&path, bytes).unwrap();
            let loaded = Settings::load(Some(path));
            assert_eq!(
                loaded,
                Settings::default(),
                "load({label}) should fall back to the defaults"
            );
            // The specific field, asserted separately: the whole-struct compare
            // above is what fails under the mutation, but naming the field here
            // says *why* the case exists — this is the value that lossy decoding
            // would quietly produce.
            assert_eq!(
                loaded.default_runner, SYSTEM_WINE,
                "load({label}) must not accept a corrupted default_runner"
            );
            assert!(
                !loaded.default_runner.contains('\u{fffd}'),
                "load({label}) produced a replacement character"
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_leading_bom_is_stripped_rather_than_discarding_the_file() {
        // DECISIONS D-21. `json.loads` does not strip a UTF-8 BOM, so Python
        // reads nothing, `Settings.load` answers with the defaults, and the next
        // `save` writes those defaults over the user's preferences. One BOM —
        // which an editor or a sync tool can add without the user seeing it —
        // silently resets every setting.
        //
        // The port strips the BOM, because the alternative is a preference file
        // that forgets itself. This is a deliberate divergence; the oracle's
        // `encoding_and_shape.bom` case records Python's outcome, and
        // `oracle_tests.rs` asserts the port's.
        let directory =
            std::env::temp_dir().join(format!("gh-settings-bom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, b"\xef\xbb\xbf{\"color_scheme\": \"light\"}").unwrap();

        assert_eq!(
            Settings::load(Some(path)).color_scheme,
            "light",
            "the BOM should be stripped, not treated as corrupt JSON"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn load_reads_a_well_formed_file() {
        let directory = std::env::temp_dir().join(format!("gh-settings-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"color_scheme": "light", "sort_mode": "recent"}"#).unwrap();

        let settings = Settings::load(Some(path));
        assert_eq!(settings.color_scheme, "light");
        assert_eq!(settings.sort_mode, "recent");
        assert_eq!(settings.view_mode, "grid", "unmentioned keys keep defaults");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn save_round_trips_and_stays_atomic() {
        let directory =
            std::env::temp_dir().join(format!("gh-settings-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("settings.json");

        // Loaded from the path, so `save` writes it back there: this is the
        // call the `update` arms make, and it had no test pinning the target.
        let mut settings = Settings::load(Some(path.clone()));
        settings.color_scheme = "light".to_string();
        settings.default_runner = "proton-ge".to_string();
        settings.save().unwrap();

        assert_eq!(Settings::load(Some(path.clone())), settings);
        assert!(
            std::fs::read_to_string(&path).unwrap().ends_with('}'),
            "Python writes no trailing newline"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn every_field_participates_in_equality() {
        // The manual `PartialEq` lists every value field and skips the path.
        // Flipping each field one at a time must break equality; flipping the
        // path must not. And the `Debug` shape carries the field count, so a
        // nineteenth value field fails here until it joins the impl.
        let base = Settings::default();
        let mut flipped = base.clone();
        flipped.color_scheme = "light".to_string();
        assert_ne!(flipped, base);
        let mut flipped = base.clone();
        flipped.view_mode = "list".to_string();
        assert_ne!(flipped, base);
        let mut flipped = base.clone();
        flipped.sort_mode = "recent".to_string();
        assert_ne!(flipped, base);
        let mut flipped = base.clone();
        flipped.default_runner = "proton-ge".to_string();
        assert_ne!(flipped, base);
        for name in [
            "default_mangohud",
            "default_gamemode",
            "default_prefer_sdl",
            "default_esync",
            "default_fsync",
            "default_dxvk",
            "default_vkd3d",
            "default_nvapi",
            "default_fsr",
            "default_battleye",
            "default_eac",
            "default_gamescope",
            "default_virtual_desktop",
            "close_on_launch",
        ] {
            let mut flipped = base.clone();
            let slot = match name {
                "default_mangohud" => &mut flipped.default_mangohud,
                "default_gamemode" => &mut flipped.default_gamemode,
                "default_prefer_sdl" => &mut flipped.default_prefer_sdl,
                "default_esync" => &mut flipped.default_esync,
                "default_fsync" => &mut flipped.default_fsync,
                "default_dxvk" => &mut flipped.default_dxvk,
                "default_vkd3d" => &mut flipped.default_vkd3d,
                "default_nvapi" => &mut flipped.default_nvapi,
                "default_fsr" => &mut flipped.default_fsr,
                "default_battleye" => &mut flipped.default_battleye,
                "default_eac" => &mut flipped.default_eac,
                "default_gamescope" => &mut flipped.default_gamescope,
                "default_virtual_desktop" => &mut flipped.default_virtual_desktop,
                "close_on_launch" => &mut flipped.close_on_launch,
                _ => unreachable!("the table above lists every flag"),
            };
            *slot = !*slot;
            assert_ne!(flipped, base, "{name} is not compared");
        }
        let moved = Settings {
            path: std::path::PathBuf::from("elsewhere.json"),
            ..base.clone()
        };
        assert_eq!(moved, base, "the store is not the value");

        // Eighteen value fields plus the path, no more: a new field must
        // join the `PartialEq` above, and this count says so.
        let comma_free = Settings {
            path: std::path::PathBuf::from("x"),
            ..Settings::default()
        };
        let debug = format!("{comma_free:?}");
        assert_eq!(
            debug.split(", ").count(),
            19,
            "a field was added without joining PartialEq: {debug}"
        );
    }

    #[test]
    fn the_loaded_path_is_remembered_and_saved_back() {
        let directory =
            std::env::temp_dir().join(format!("gh-settings-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("settings.json");

        // Even a missing file yields a value that knows where it came from:
        // the defaults with this path, not the configured one.
        let settings = Settings::load(Some(path.clone()));
        assert_eq!(settings.path(), path.as_path());
        assert_eq!(settings.color_scheme, "dark");

        settings.save().unwrap();
        assert!(path.is_file(), "save writes the loaded path");
        assert_eq!(Settings::load(Some(path.clone())), settings);

        // And the wire format carries no trace of it.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("path"),
            "the store is not on the wire: {text}"
        );

        assert_eq!(
            Settings::default().path(),
            crate::paths::settings_file().as_path()
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
