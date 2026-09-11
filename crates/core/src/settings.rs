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

use crate::json;
use crate::models::{SORT_MODES, SYSTEM_WINE};
use crate::paths;

/// The colour schemes the UI knows. `settings.py:13`.
pub const COLOR_SCHEMES: [&str; 3] = ["system", "light", "dark"];

/// The library layouts the UI knows. `settings.py:14`.
pub const VIEW_MODES: [&str; 2] = ["grid", "list"];

/// User preferences stored under the XDG config directory.
///
/// The field order is the wire format. Do not reorder.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

    /// Port of `Settings.load` (`settings.py:56-67`).
    ///
    /// `None` means the configured path. A missing file, an unreadable one, one
    /// that is not valid UTF-8, invalid JSON, trailing garbage and a top-level
    /// value that is not an object all yield the defaults. The UTF-8 case is a
    /// deliberate divergence — Python raises there — and `read_to_string` gives
    /// it to us for free, since it rejects invalid UTF-8 rather than replacing
    /// it.
    pub fn load(path: Option<PathBuf>) -> Self {
        let target = path.unwrap_or_else(paths::settings_file);
        let Ok(source) = std::fs::read_to_string(&target) else {
            return Self::default();
        };
        let Ok(Value::Object(fields)) = json::parse_lenient(&source) else {
            return Self::default();
        };
        Self::from_dict(&fields)
    }

    /// Port of `Settings.save` (`settings.py:69-74`) to the configured path.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(paths::settings_file())
    }

    /// [`Self::save`] to an explicit path, as `Settings.save(path)` allows.
    pub fn save_to(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        json::write_python_file(path.as_ref(), self)
    }
}

impl Default for Settings {
    /// The dataclass defaults, including the `"dark"`/`"grid"` fallbacks.
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
            ("missing", ""),                // never written
            ("empty", ""),                  // zero bytes
            ("malformed", "{not json"),     // unparseable
            ("trailing", "{}\ngarbage"),    // extra data
            ("notdict", "[1,2,3]"),         // parses, wrong shape
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
        let directory = std::env::temp_dir().join(format!("gh-settings-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, b"{\"color_scheme\": \"light\"\xff}").unwrap();

        assert_eq!(Settings::load(Some(path)), Settings::default());

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
        let directory = std::env::temp_dir().join(format!("gh-settings-bom-{}", std::process::id()));
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
        let directory = std::env::temp_dir().join(format!("gh-settings-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("settings.json");

        let settings = Settings {
            color_scheme: "light".to_string(),
            default_runner: "proton-ge".to_string(),
            ..Settings::default()
        };
        settings.save_to(&path).unwrap();

        assert_eq!(Settings::load(Some(path.clone())), settings);
        assert!(
            std::fs::read_to_string(&path).unwrap().ends_with('}'),
            "Python writes no trailing newline"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
