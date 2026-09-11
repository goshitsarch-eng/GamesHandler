//! Fixture-driven compatibility tests against the executable oracle.
//!
//! `docs/migration/oracle/gen_oracle.py` runs the **Python** implementation
//! against a frozen clock and records what it actually does into
//! `docs/migration/oracle/fixtures/`: `oracle.json` for parsed values, and
//! `.in.json` / `.out.json` byte pairs for the load-save round trip. These
//! tests replay every one of those cases through the Rust port.
//!
//! Three outcomes are checked, and they are not the same thing:
//!
//! 1. the parsed result equals what Python parsed (`oracle.json`);
//! 2. the re-saved bytes equal what Python wrote (`*.out.json`) — this is
//!    `dataclasses.asdict` field order plus `json.dumps(indent=2,
//!    ensure_ascii=True)`, and it is where a silently reordered struct shows up;
//! 3. `oracle.json` and the `.out.json` files agree with each other, so a stale
//!    fixture cannot hide behind a stale oracle.
//!
//! Two entries in `oracle.json` are recorded as places the port **must not**
//! match Python — `rust_divergences`, governed by D-14 (a `null` timestamp is
//! normalized rather than left as `None`) and D-15 (exponent spelling). Those
//! are honoured here, not worked around.
//!
//! `rust_divergences.cases` carries a third, D-18 (wrong-typed scalars are
//! coerced to their declared type rather than stored as whatever the file
//! said), and two more divergences live outside that list because Python has no
//! recorded outcome to compare against — D-20 (invalid UTF-8 in `settings.json`
//! raises in Python and must not here) and D-21 (a leading BOM, and depth past
//! what a recursive parse can survive). Where a divergence means the port's
//! count, name or bytes differ from Python's, the test below says so in the
//! assertion message rather than silently asserting the nicer answer.
//!
//! Where the fixtures live is not configurable: `CARGO_MANIFEST_DIR` is
//! `crates/core`, so the fixtures are two levels up. If they are missing the
//! tests fail loudly with the regeneration command rather than skipping.
//!
//! Owned here rather than in `crates/core/tests/` because that directory
//! belongs to QA (DECISIONS D-05); QA can adopt the same fixtures from there
//! without touching this file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::hash::sha256_hex;
use crate::json as python_json;
use crate::models::{format_last_played, Game, Library, SORT_MODES, UNCATEGORIZED};
use crate::settings::{Settings, COLOR_SCHEMES, VIEW_MODES};

/// The instant `gen_oracle.py` freezes its clock to. Every regenerated
/// timestamp in the fixtures is exactly this, so the port must be handed the
/// same instant rather than reading the real clock.
const FROZEN_NOW: f64 = 1_700_000_000.0;

/// `docs/migration/oracle/fixtures`.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("migration")
        .join("oracle")
        .join("fixtures")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}\n\
             Regenerate the fixtures with `python3 docs/migration/oracle/gen_oracle.py`.",
            path.display()
        )
    })
}

/// The parsed `oracle.json`.
fn oracle() -> Value {
    let path = fixtures_dir().join("oracle.json");
    match python_json::parse_lenient(&read(&path)) {
        Ok(value @ Value::Object(_)) => value,
        Ok(other) => panic!("oracle.json should hold an object, found {other}"),
        Err(error) => panic!("oracle.json is not readable JSON: {error}"),
    }
}

/// A nested object, with a message that names the key rather than panicking
/// inside index syntax.
fn object_at<'a>(value: &'a Value, key: &str) -> &'a Map<String, Value> {
    match value.get(key) {
        Some(Value::Object(fields)) => fields,
        Some(other) => panic!("{key} should be an object, found {other}"),
        None => panic!("{key} is missing from the oracle"),
    }
}

/// A nested string.
fn string_at(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(other) => panic!("{key} should be a string, found {other}"),
        None => panic!("{key} is missing from the oracle"),
    }
}

/// A nested array of strings, as a `Vec`.
fn names_at(value: &Value, key: &str) -> Vec<String> {
    let Some(Value::Array(items)) = value.get(key) else {
        panic!("{key} should be an array");
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => text.clone(),
            other => panic!("{key} should hold strings, found {other}"),
        })
        .collect()
}

/// The bytes the oracle recorded for a fixture's output file.
///
/// The oracle writes the file (`out_file`, relative to the fixtures directory)
/// and a `out_sha256` of its contents. Both are checked: the file is read for
/// the comparison itself, and the hash proves the file has not been edited
/// behind the oracle's back — which a stale-fixture check needs, since the
/// oracle's own copy of the bytes is no longer stored inline.
fn expected_output(case: &Value, label: &str) -> String {
    let name = string_at(case, "out_file");
    let path = fixtures_dir().join(&name);
    let bytes = read(&path);
    assert_eq!(
        sha256_hex(bytes.as_bytes()),
        string_at(case, "out_sha256"),
        "{name} does not match the oracle's out_sha256 — the fixture or the oracle is stale"
    );

    // A second, independent statement of the same fact: the label in the case
    // name has to match the file the oracle names. `gen_oracle.py` builds one
    // from the other, so a mismatch means a hand-renamed fixture.
    if label != "not_a_dict" {
        assert!(
            name.contains(label),
            "oracle case {label} points at {name}, which does not name it"
        );
    }
    bytes
}





/// A fresh, empty scratch directory. Fixtures are never written to: the
/// implementation under test writes here and the two are compared.
fn scratch(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("gh-oracle-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", directory.display()));
    directory
}

/// The names of a sorted game list, in order.
fn names(games: &[&Game]) -> Vec<String> {
    games.iter().map(|game| game.name.clone()).collect()
}

/// Serialise a game the way the port writes it, but as a `Value`, so it can be
/// compared field by field against an oracle entry.
fn game_object(game: &Game) -> Map<String, Value> {
    match serde_json::to_value(game).expect("a game serialises") {
        Value::Object(fields) => fields,
        other => panic!("a game should serialise to an object, found {other}"),
    }
}

// ---------------------------------------------------------------------------
// 1. Game::from_dict
// ---------------------------------------------------------------------------

/// The `from_dict` fixture inputs, written as the JSON text Python was given.
///
/// `gen_oracle.py` stores its inputs with `repr()` for floats, so `NaN` arrives
/// as the string `"nan"` in `oracle.json` and cannot be replayed from there.
/// The text below is the same input, in the form `Library.load` would read from
/// a file — which is also what exercises the lenient parser.
const FROM_DICT_INPUTS: [(&str, &str); 14] = [
    (
        "clean",
        r#"{"id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "name": "Half-Life",
            "added": 1700000000.5, "last_played": 1700000001.25}"#,
    ),
    (
        "nan_added",
        r#"{"id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "name": "NaN added",
            "added": NaN, "last_played": 5.0}"#,
    ),
    (
        "inf_last",
        r#"{"id": "cccccccccccccccccccccccccccccccc", "name": "Inf last",
            "added": 1.0, "last_played": Infinity}"#,
    ),
    (
        "neg_inf_last",
        r#"{"id": "dddddddddddddddddddddddddddddddd", "name": "NegInf last",
            "added": 1.0, "last_played": -Infinity}"#,
    ),
    (
        "bool_true",
        r#"{"id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "name": "Bool true",
            "added": true, "last_played": true}"#,
    ),
    (
        "bool_false",
        r#"{"id": "ffffffffffffffffffffffffffffffff", "name": "Bool false",
            "added": false, "last_played": false}"#,
    ),
    (
        "str_ts",
        r#"{"id": "gggggggggggggggggggggggggggggggg", "name": "Str ts",
            "added": "soon", "last_played": "later"}"#,
    ),
    (
        "null_ts",
        r#"{"id": "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh", "name": "Null ts",
            "added": null, "last_played": null}"#,
    ),
    (
        "negative",
        r#"{"id": "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii", "name": "Negative",
            "added": -5.0, "last_played": -1.0}"#,
    ),
    (
        "int_ts",
        r#"{"id": "jjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjj", "name": "Int ts",
            "added": 42, "last_played": 7}"#,
    ),
    (
        "unknown_keys",
        r#"{"id": "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk", "name": "Unknown",
            "legacy_field": 1, "another": [1, 2]}"#,
    ),
    (
        "unicode",
        r#"{"id": "llllllllllllllllllllllllllllllll", "name": "Pokémon — Ünïcode ✓",
            "category": "日本語"}"#,
    ),
    (
        "empty_category",
        r#"{"id": "mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm", "name": "Blank cat",
            "category": "   "}"#,
    ),
    (
        "linux_game",
        r#"{"id": "nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn", "name": "Native", "kind": "linux"}"#,
    ),
];

/// Parse one fixture input into the object `Game::from_dict` takes.
fn input_object(text: &str) -> Map<String, Value> {
    match python_json::parse_lenient(text) {
        Ok(Value::Object(fields)) => fields,
        Ok(other) => panic!("a from_dict input should be an object, found {other}"),
        Err(error) => panic!("a from_dict input is not readable JSON: {error}\n{text}"),
    }
}

#[test]
fn from_dict_cases_match_the_oracle_field_for_field() {
    let oracle = oracle();
    let cases = object_at(&oracle, "from_dict");

    for (label, source) in FROM_DICT_INPUTS {
        let case = cases
            .get(label)
            .unwrap_or_else(|| panic!("the oracle has no from_dict case {label:?}"));
        let game = Game::from_dict_at(&input_object(source), FROZEN_NOW);

        // The oracle records the whole Game minus `added`, which is regenerated
        // and therefore recorded separately as `added_value`.
        let mut expected = object_at(case, "output").clone();

        // DECISIONS D-14: on this one case the port must NOT copy Python. The
        // oracle flags it inline so a reader cannot mistake `None` for the
        // expected value.
        let diverges = case
            .get("rust_must_diverge")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let added = if diverges {
            FROZEN_NOW
        } else {
            case.get("added_value")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| panic!("from_dict[{label}].added_value is not a number"))
        };
        expected.insert("added".to_string(), json!(added));
        if diverges {
            // `last_played` goes the same way: Python left `None` there too.
            expected.insert("last_played".to_string(), json!(0.0));
        }

        assert_eq!(
            Value::Object(game_object(&game)),
            Value::Object(expected),
            "from_dict[{label}] does not match the oracle"
        );

        // The two derived properties the oracle also records.
        assert_eq!(
            game.is_linux(),
            case.get("is_linux").and_then(Value::as_bool).unwrap_or(false),
            "from_dict[{label}].is_linux"
        );
        assert_eq!(
            game.display_category(),
            case.get("display_category")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "from_dict[{label}].display_category"
        );
        if case.get("added_is_recent").and_then(Value::as_bool) == Some(true) {
            assert!(
                (game.added - FROZEN_NOW).abs() < 60.0,
                "from_dict[{label}].added should be recent"
            );
        }
    }

    assert_eq!(
        cases.len(),
        FROM_DICT_INPUTS.len(),
        "every oracle from_dict case should be replayed"
    );
}

#[test]
fn a_null_timestamp_never_reaches_the_sort() {
    // The bug D-14 exists to prevent: Python keeps `None`, and then
    // `sorted(key=lambda g: (-g.last_played, ...))` raises
    // `TypeError: bad operand type for unary -: 'NoneType'`. Rust has no
    // nullable timestamp, so both sorts must simply work.
    let library = Library::new_at(
        Some(fixtures_dir().join("nan_values.in.json")),
        FROZEN_NOW,
    );
    assert_eq!(library.len(), 1);

    let directory = scratch("null-ts");
    let path = directory.join("games.json");
    std::fs::write(
        &path,
        r#"[{"id": "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh", "name": "Null ts",
             "added": null, "last_played": null}]"#,
    )
    .unwrap();

    let library = Library::new_at(Some(path), FROZEN_NOW);
    assert_eq!(library.all("recent").len(), 1, "recent must not panic");
    assert_eq!(library.all("added").len(), 1, "added must not panic");
    let game = library.all("name")[0];
    assert_eq!(game.added, FROZEN_NOW, "added regenerated from the clock");
    assert_eq!(game.last_played, 0.0, "last_played zeroed");

    let _ = std::fs::remove_dir_all(&directory);
}

// ---------------------------------------------------------------------------
// 2. load -> save byte equality
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_fixtures_reproduce_pythons_bytes_exactly() {
    let oracle = oracle();
    let cases = object_at(&oracle, "roundtrip");

    for (label, case) in cases {
        // The oracle records an `error` entry when even Python raised; none of
        // the committed fixtures do, and silently skipping one would defeat the
        // test.
        assert!(
            case.get("error").is_none(),
            "the oracle recorded an error for {label}: {case:?}"
        );

        let input = fixtures_dir().join(format!("{label}.in.json"));
        let expected = expected_output(case, label);

        let library = Library::new_at(Some(input), FROZEN_NOW);
        assert_eq!(
            library.len(),
            case.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
            "roundtrip[{label}] loaded the wrong number of games"
        );
        assert_eq!(
            names(&library.all("name")),
            names_at(case, "names"),
            "roundtrip[{label}] loaded the wrong games"
        );

        let directory = scratch(label);
        let written = directory.join(format!("{label}.out.json"));
        library
            .save_to(&written)
            .unwrap_or_else(|error| panic!("roundtrip[{label}] save failed: {error}"));
        assert_eq!(
            read(&written),
            expected,
            "roundtrip[{label}] re-saved different bytes"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}

#[test]
fn every_roundtrip_fixture_pair_is_covered() {
    // A new fixture added by the oracle generator must not go untested.
    let oracle = oracle();
    for label in object_at(&oracle, "roundtrip").keys() {
        for suffix in ["in.json", "out.json"] {
            let path = fixtures_dir().join(format!("{label}.{suffix}"));
            assert!(
                path.is_file(),
                "the oracle names roundtrip case {label} but {} is missing",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. sorting, search, format_last_played
// ---------------------------------------------------------------------------

#[test]
fn sorting_fixtures_match_the_oracle() {
    let oracle = oracle();
    let expected = &oracle["sorting"];
    let library = Library::new_at(Some(fixtures_dir().join("sort.in.json")), FROZEN_NOW);

    for mode in SORT_MODES {
        assert_eq!(
            names(&library.all(mode)),
            names_at(expected, mode),
            "sort mode {mode:?}"
        );
    }
    assert_eq!(
        expected.as_object().map(Map::len),
        Some(SORT_MODES.len()),
        "every sort mode should be covered"
    );
}

#[test]
fn search_and_categories_match_the_oracle() {
    let oracle = oracle();
    let expected = &oracle["search"];
    let library = Library::new_at(Some(fixtures_dir().join("search.in.json")), FROZEN_NOW);

    assert_eq!(library.categories(), names_at(expected, "categories"));

    // Each entry is a `(query, category)` pair; the oracle used the defaults for
    // everything it did not pass, which is `""` and `"name"`.
    let queries: [(&str, &str, &str); 8] = [
        ("q_portal", "portal", ""),
        ("q_puzzle", "puzzle", ""),
        ("q_blank", "", ""),
        ("cat_puzzle", "", "Puzzle"),
        ("cat_all", "", "All"),
        ("cat_uncat", "", UNCATEGORIZED),
        ("q_case", "DOOM", ""),
        ("q_trim", "  doom  ", ""),
    ];
    for (label, query, category) in queries {
        assert_eq!(
            names(&library.search(query, category, "name")),
            names_at(expected, label),
            "search({query:?}, {category:?}) should match the oracle's {label}"
        );
    }
}

#[test]
fn every_format_last_played_branch_matches_the_oracle() {
    let oracle = oracle();
    let expected = &oracle["format_last_played"];
    let now = expected
        .get("now")
        .and_then(Value::as_f64)
        .expect("the oracle records its frozen now");
    let cases = object_at(expected, "cases");

    // The timestamps `gen_oracle.py` builds, in full: `now - offset` for
    // everything except the two "never" cases, which are literally `0.0` (the
    // falsy case *is* zero, which is why the pair of labels looks redundant but
    // is not).
    let minute = 60.0;
    let hour = 60.0 * minute;
    let day = 24.0 * hour;
    let offsets: [(&str, f64); 17] = [
        ("just_now_0", 0.0),
        ("just_now_59s", 59.0),
        ("min_boundary_119s", 119.0),
        ("min_2", 120.0),
        ("min_59", 59.0 * minute),
        ("hour_1", hour),
        ("hour_2", 2.0 * hour),
        ("hour_23", 23.0 * hour),
        ("yesterday", day),
        ("days_2", 2.0 * day),
        ("days_29", 29.0 * day),
        ("month_1", 30.0 * day),
        ("month_2", 60.0 * day),
        ("month_11", 330.0 * day),
        ("year_1", 360.0 * day),
        ("year_2", 800.0 * day),
        ("future", -10_000.0),
    ];

    let mut replayed = 0usize;
    for (label, timestamp) in [("never_zero", 0.0), ("never_falsy", 0.0)]
        .into_iter()
        .chain(offsets.into_iter().map(|(label, offset)| (label, now - offset)))
    {
        let expected_label = cases
            .get(label)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("the oracle has no format_last_played case {label:?}"));
        assert_eq!(
            format_last_played(timestamp, now),
            expected_label,
            "format_last_played({timestamp}, {now}) should match the oracle's {label}"
        );
        replayed += 1;
    }
    assert_eq!(
        cases.len(),
        replayed,
        "every format_last_played case should be replayed"
    );
}

// ---------------------------------------------------------------------------
// 4. Settings
// ---------------------------------------------------------------------------

/// The `.in.json` stem for a `settings` oracle case.
///
/// `gen_oracle.py` names most of them `settings_<label>.in.json` but writes the
/// non-dict one as `settings_notdict.in.json`, so the two cannot simply be
/// concatenated.
fn settings_fixture_stem(label: &str) -> String {
    match label {
        "not_a_dict" => "settings_notdict".to_string(),
        other => format!("settings_{other}"),
    }
}

#[test]
fn settings_fixtures_match_the_oracle() {
    let oracle = oracle();
    let cases = object_at(&oracle, "settings");

    for (label, case) in cases {
        let input = fixtures_dir().join(format!("{}.in.json", settings_fixture_stem(label)));
        let settings = Settings::load(Some(input));
        let directory = scratch(&format!("settings-{label}"));

        // Every case except `not_a_dict` recorded a saved file. That one is
        // `[1, 2, 3]`, which Python rejects as a non-dict and answers with the
        // defaults, so there is nothing new to assert about its bytes: it must
        // save exactly the default document.
        match case.get("out_file").and_then(Value::as_str) {
            Some(_) => {
                let expected = expected_output(case, label);
                let written = directory.join(format!("{}.out.json", settings_fixture_stem(label)));
                settings
                    .save_to(&written)
                    .unwrap_or_else(|error| panic!("settings[{label}] save failed: {error}"));
                assert_eq!(
                    read(&written),
                    expected,
                    "settings[{label}] saved different bytes"
                );
            }
            None => {
                assert_eq!(
                    settings,
                    Settings::default(),
                    "settings[{label}] should have fallen back to the defaults"
                );
                let written = directory.join("defaults.json");
                settings.save_to(&written).unwrap();
                assert_eq!(
                    read(&written),
                    read(&fixtures_dir().join("settings_all_defaults.out.json")),
                    "the defaults document should be identical to the one Python wrote"
                );
            }
        }

        // `to_dict` — the parsed values, independent of byte layout.
        if let Some(expected) = case.get("to_dict") {
            let written = serde_json::to_value(&settings).expect("settings serialise");
            assert_eq!(
                written, *expected,
                "settings[{label}].to_dict does not match the oracle"
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    // Every `settings_*.in.json` on disk must belong to a case above, and every
    // case must have a file — the two sets are checked in both directions so
    // neither a stray fixture nor a typo'd label can pass unnoticed.
    let mut on_disk: BTreeMap<String, ()> = BTreeMap::new();
    for entry in std::fs::read_dir(fixtures_dir()).expect("the fixtures directory is readable") {
        let name = entry.expect("a readable entry").file_name();
        let name = name.to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix("settings_")
            && let Some(label) = rest.strip_suffix(".in.json")
        {
            on_disk.insert(label.to_string(), ());
        }
    }
    assert!(!on_disk.is_empty(), "the settings fixtures should exist");
    for label in on_disk.keys() {
        let known = cases
            .keys()
            .any(|case| settings_fixture_stem(case).strip_prefix("settings_") == Some(label));
        assert!(
            known,
            "settings_{label}.in.json exists but the oracle has no case for it"
        );
    }
    for label in cases.keys() {
        let stem = settings_fixture_stem(label);
        let file = stem.strip_prefix("settings_").unwrap_or(&stem);
        assert!(
            on_disk.contains_key(file),
            "{stem}.in.json is missing for the oracle's settings case {label}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. out-of-range and huge numeric literals
// ---------------------------------------------------------------------------

#[test]
fn out_of_range_literals_normalize_instead_of_discarding_the_library() {
    // FINDINGS F-G: `serde_json` rejects `1e400` outright. A rejection at that
    // level discards every game in the file, so this is the difference between
    // one bad field and a wiped library (DECISIONS D-16).
    let oracle = oracle();
    let cases = object_at(&oracle, "out_of_range");
    assert_eq!(cases.len(), 5, "every out-of-range case should be replayed");

    for (label, case) in cases {
        let input = fixtures_dir().join(format!("{label}.in.json"));
        let library = Library::new_at(Some(input), FROZEN_NOW);

        assert_eq!(
            case.get("parsed").and_then(Value::as_bool),
            Some(true),
            "the oracle recorded that {label} parsed"
        );
        assert_eq!(
            library.len(),
            case.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
            "out_of_range[{label}] loaded the wrong number of games"
        );
        assert_eq!(
            case.get("sort_ok").and_then(Value::as_bool),
            Some(true),
            "the oracle recorded that {label} sorts"
        );

        // The oracle stores these as `repr()` strings, e.g. "1.2345678901234568e+29".
        let expected_added: f64 = string_at(case, "added")
            .parse()
            .unwrap_or_else(|error| panic!("out_of_range[{label}].added is unparseable: {error}"));
        let expected_last: f64 = string_at(case, "last_played")
            .parse()
            .unwrap_or_else(|error| panic!("out_of_range[{label}].last_played: {error}"));

        let game = library.all("added")[0];
        assert_eq!(
            game.added, expected_added,
            "out_of_range[{label}].added should be the nearest f64, not 0.0 and not an error"
        );
        assert_eq!(
            game.last_played, expected_last,
            "out_of_range[{label}].last_played"
        );
        // The sort must genuinely run, not merely fail to panic.
        assert_eq!(library.all("recent").len(), 1);
    }
}

#[test]
fn huge_integer_literals_become_the_nearest_f64() {
    // Python converts an arbitrary-precision `int` to the nearest float. These
    // are *valid* timestamps, so they must survive — unlike `1e400`.
    let oracle = oracle();
    let cases = object_at(&oracle, "out_of_range");
    let big_int = cases.get("big_int").expect("the oracle has big_int");

    let library = Library::new_at(Some(fixtures_dir().join("big_int.in.json")), FROZEN_NOW);
    let game = library.all("name")[0];
    assert_eq!(game.added, 123456789012345678901234567890_f64);
    assert!(
        (game.added - FROZEN_NOW).abs() > 1.0e20,
        "an int far outside f64's integer range must not be treated as invalid"
    );
    assert_eq!(
        game.added,
        string_at(big_int, "added").parse::<f64>().unwrap()
    );
}

#[test]
fn u64_max_is_not_mistaken_for_a_negative_timestamp() {
    // `18446744073709551616` does not fit `i64`; a port that read it as a
    // signed integer would see a negative number, call it invalid, and zero the
    // field. The oracle says it survives as 1.8446744073709552e19.
    let oracle = oracle();
    let cases = object_at(&oracle, "out_of_range");
    let expected: f64 = string_at(
        cases.get("u64_max_plus").expect("the oracle has u64_max_plus"),
        "added",
    )
    .parse()
    .unwrap();

    let library = Library::new_at(
        Some(fixtures_dir().join("u64_max_plus.in.json")),
        FROZEN_NOW,
    );
    assert_eq!(library.all("name")[0].added, expected);
    assert!(expected > 0.0);
}

// ---------------------------------------------------------------------------
// 6. constants, and the float-format divergence
// ---------------------------------------------------------------------------

#[test]
fn constants_match_the_python_module_level_values() {
    let oracle = oracle();
    let expected = &oracle["constants"];

    assert_eq!(
        COLOR_SCHEMES.map(str::to_string).to_vec(),
        names_at(expected, "COLOR_SCHEMES")
    );
    assert_eq!(
        VIEW_MODES.map(str::to_string).to_vec(),
        names_at(expected, "VIEW_MODES")
    );
    assert_eq!(
        SORT_MODES.map(str::to_string).to_vec(),
        names_at(expected, "SORT_MODES")
    );
    assert_eq!(UNCATEGORIZED, string_at(expected, "UNCATEGORIZED"));
    assert_eq!(
        Settings::default().color_scheme,
        string_at(expected, "default_color_scheme")
    );
}

#[test]
fn floats_survive_a_roundtrip_numerically_though_the_exponent_is_spelled_differently() {
    // DECISIONS D-15, FINDINGS F-F — whose table is narrower than it first
    // read, so this note records the measured divergence rather than repeating
    // the original claim.
    //
    // Python writes f64 through C's `%g`, which pads a *negative* exponent to
    // at least two digits: `1e-07`. Rust's serializer does not: `1e-7`. That is
    // the whole of it. `1e7` is **not** an example — Python writes `10000000.0`
    // and so does this port. Positive exponents agree in full, `+` sign
    // included (`1e+23`, `1e+308`), as do ordinary decimals and subnormals.
    // These fourteen fixture values were compared one by one, and `1e-7` is the
    // only one spelled differently. (The formatter is `zmij`, not `ryu`; there
    // is no `ryu` in `Cargo.lock` for serde_json 1.0.151.)
    //
    // Both spellings are valid JSON and reparse to the same f64, so the check
    // here is numeric equality after reparse rather than byte equality.
    // Emulating `printf` would be a bug, not fidelity — `floats.out.json` is
    // Python's spelling and the port is expected to differ, there and only
    // there.
    let oracle = oracle();
    let float_format = &oracle["float_format"];
    let library = Library::new_at(Some(fixtures_dir().join("floats.in.json")), FROZEN_NOW);

    let directory = scratch("floats");
    let written = directory.join("floats.out.json");
    library.save_to(&written).expect("save should succeed");
    let produced = read(&written);

    let python_bytes = expected_output(float_format, "floats");

    let Value::Array(ours) = python_json::parse_lenient(&produced).expect("our output parses") else {
        panic!("a saved library is a JSON array");
    };
    let Value::Array(theirs) = python_json::parse_lenient(&python_bytes).expect("Python's parses")
    else {
        panic!("a saved library is a JSON array");
    };

    assert_eq!(ours.len(), theirs.len());
    assert_eq!(
        ours.len(),
        object_at(float_format, "cases").len(),
        "one game per float case"
    );

    for (ours, theirs) in ours.iter().zip(theirs.iter()) {
        let (Value::Object(ours), Value::Object(theirs)) = (ours, theirs) else {
            panic!("each entry of a saved library is an object");
        };
        for (key, value) in theirs {
            match key.as_str() {
                // The two f64 fields are compared as numbers, not as text.
                "added" | "last_played" => assert_eq!(
                    ours.get(key).and_then(Value::as_f64),
                    value.as_f64(),
                    "{key} lost precision: {:?} vs {value:?}",
                    ours.get(key)
                ),
                _ => assert_eq!(ours.get(key), Some(value), "{key} should be identical"),
            }
        }
    }

    // The byte-level difference must be confined to how an f64 is *spelled*:
    // every differing line has to be one of the two float fields, and the two
    // values must be the same double after reparse. Asserting which divergence
    // is allowed is what keeps a real formatting regression from arriving as a
    // new line here and being waved through as "the exponent thing" — the same
    // problem F-F's own table had.
    //
    // # Why the predicate is numeric rather than textual
    //
    // An earlier version asserted the differing line was a negative exponent
    // that Python pads — `contains("e-") && !contains("e-0")`. That is true for
    // `1e-7` vs `1e-07` and **false for the whole notation band**, where the two
    // writers disagree about *whether to use an exponent at all*:
    //
    //     value       this port     Python      divergence
    //     1e-4        0.0001        0.0001      none
    //     1e-5        0.00001       1e-05       NOTATION
    //     1.2345e-5   0.000012345   1.2345e-05  NOTATION
    //     1e-6        1e-6          1e-06       padding
    //
    // So the old guard rejected a divergence the writer genuinely produces. It
    // only survived because this fixture's 14 values happen to sit above and
    // below the band rather than inside it — a test that cannot see a case is
    // not the same as a case that cannot happen. Found by adversarial review;
    // `exp_notation_boundary` is the fixture added to cover the band, and
    // `the_notation_band_diverges_only_in_spelling` runs it.
    assert_eq!(
        produced.lines().count(),
        python_bytes.lines().count(),
        "the two documents should have the same line count"
    );
    let mut differing = 0usize;
    for (ours, theirs) in produced.lines().zip(python_bytes.lines()) {
        if ours != theirs {
            differing += 1;
            assert!(
                ours.trim_start().starts_with("\"added\"")
                    || ours.trim_start().starts_with("\"last_played\""),
                "only the float fields may be spelled differently, but this line differs:\n\
                 rust:   {ours}\n\
                 python: {theirs}"
            );

            let ours_value: f64 = ours
                .trim()
                .trim_start_matches('"')
                .split(": ")
                .nth(1)
                .expect("a `\"key\": value,` line")
                .trim_end_matches(',')
                .parse()
                .unwrap_or_else(|error| panic!("our line is not an f64: {ours}: {error}"));
            let theirs_value: f64 = theirs
                .trim()
                .trim_start_matches('"')
                .split(": ")
                .nth(1)
                .expect("a `\"key\": value,` line")
                .trim_end_matches(',')
                .parse()
                .unwrap_or_else(|error| panic!("Python's line is not an f64: {theirs}: {error}"));
            assert_eq!(
                ours_value.to_bits(),
                theirs_value.to_bits(),
                "a differently spelled float must still be the same double:\n\
                 rust:   {ours}\n\
                 python: {theirs}"
            );
        }
    }
    assert!(
        differing > 0,
        "the exponent divergence should be observable in this fixture, otherwise it no \
         longer tests anything"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn the_notation_band_diverges_only_in_spelling() {
    // The fixture `exp_notation_boundary`, added after the review that found the
    // predicate above to be narrower than the writer. Values in
    // `[1e-5, 1e-4)` reparse to the same double but are *spelled* differently —
    // `0.00001` here, `1e-05` in Python — so this is the band the old textual
    // guard could not describe.
    //
    // What this test is for: stating that the divergence is bounded to
    // spelling. Every value must round-trip to the same bits, and the only
    // lines that may differ are the two float fields. A real regression — a
    // value that changes, or a line of a different shape — fails here.
    let oracle = oracle();
    let case = oracle
        .get("exp_notation_boundary")
        .expect("the boundary fixture is declared in gen_oracle.py");
    let library = Library::new_at(
        Some(fixtures_dir().join("exp_notation_boundary.in.json")),
        FROZEN_NOW,
    );

    let directory = scratch("notation-band");
    let written = directory.join("exp_notation_boundary.out.json");
    library.save_to(&written).expect("save should succeed");
    let python_bytes = expected_output(case, "exp_notation_boundary");

    // Every case's bits survive, which is the property that actually matters:
    // the file is hand-edited, but it must not be *corrupted* by a load/save.
    let declared = case["bit_patterns_be_hex"]
        .as_object()
        .expect("gen_oracle.py records the bit patterns");
    let ours = read(&written);
    assert_eq!(ours.lines().count(), python_bytes.lines().count());

    // The two documents differ only on float lines, and each pair of values is
    // the same double.
    let mut differing = 0usize;
    let mut saw_notation_divergence = false;
    for (ours_line, theirs_line) in ours.lines().zip(python_bytes.lines()) {
        if ours_line == theirs_line {
            continue;
        }
        differing += 1;
        assert!(
            ours_line.trim_start().starts_with("\"added\"")
                || ours_line.trim_start().starts_with("\"last_played\""),
            "only the float fields may be spelled differently, but this line differs:\n\
             rust:   {ours_line}\n\
             python: {theirs_line}"
        );
        let ours_value: f64 = ours_line
            .trim()
            .split(": ")
            .nth(1)
            .unwrap()
            .trim_end_matches(',')
            .parse()
            .expect("our float line");
        let theirs_value: f64 = theirs_line
            .trim()
            .split(": ")
            .nth(1)
            .unwrap()
            .trim_end_matches(',')
            .parse()
            .expect("Python's float line");
        assert_eq!(
            ours_value.to_bits(),
            theirs_value.to_bits(),
            "the divergence must be spelling only:\n\
             rust:   {ours_line}\n\
             python: {theirs_line}"
        );
        // Compare the *value* text, not the line — the key `"added"` contains an
        // `e`, which is exactly the kind of detail that makes a substring test
        // silently never fire.
        let ours_text = ours_line.trim().split(": ").nth(1).unwrap_or("");
        let theirs_text = theirs_line.trim().split(": ").nth(1).unwrap_or("");
        if !ours_text.contains(['e', 'E']) && theirs_text.contains("e-0") {
            saw_notation_divergence = true;
        }
    }

    // The point of the fixture: at least one case diverges by *notation*, which
    // is the class the narrow guard could not describe. If this stops being
    // true — because a serde_json upgrade changes the threshold — the fixture
    // has stopped testing the band and should be re-derived rather than left
    // green.
    assert!(
        saw_notation_divergence,
        "expected at least one case where this port writes a fixed-point literal \
         and Python writes an exponent; the fixture no longer covers the notation band"
    );
    assert!(differing > 0, "the fixture should produce some divergence at all");

    // And the declared bit patterns are the ones on disk, so a change to the
    // fixture regeneration cannot silently move them. Every declared value must
    // appear in our output with exactly those bits — including the ones Python
    // spells differently, which is the pairing this fixture exists to make.
    let produced_bits: Vec<u64> = ours
        .lines()
        .filter_map(|line| {
            line.trim()
                .split(": ")
                .nth(1)?
                .trim_end_matches(',')
                .parse::<f64>()
                .ok()
                .map(f64::to_bits)
        })
        .collect();
    for (label, pattern) in declared {
        let raw = pattern.as_str().expect("a hex bit pattern").to_string();
        let want = u64::from_str_radix(&raw, 16).expect("hex");
        assert!(
            produced_bits.contains(&want),
            "{label} ({raw}) should appear in the output with those exact bits"
        );
    }

    let _ = std::fs::remove_dir_all(&directory);
}

/// Which of F-F's two divergence bands a value falls in.
///
/// The rule, measured on the pinned writer and recorded in FINDINGS F-F: the
/// divergence set is exactly two bands, both closed below and open above.
fn float_band(value: f64) -> &'static str {
    let magnitude = value.abs();
    if (1e-5..1e-4).contains(&magnitude) {
        // Notation: `%g` switches to exponent form below 1e-4, `zmij` does not.
        "notation"
    } else if (1e-9..1e-5).contains(&magnitude) {
        // Padding: Python pads the exponent to two digits, `zmij` does not.
        "padding"
    } else {
        "none"
    }
}

#[test]
fn the_two_float_divergence_bands_are_pinned_at_both_edges() {
    // F-F says the divergence between Python's float rendering and this port's
    // is exactly two bands, and nothing else in the whole `f64` range. This
    // test asserts *that claim* rather than sampling it, because the claim is
    // what the port depends on: it is why a game library full of `~1.7e9`
    // timestamps round-trips byte-for-byte, and why the two bands can be left
    // divergent instead of patched.
    //
    //     |x| in [1e-5, 1e-4)   notation   python "1e-05"    rust "0.00001"
    //     |x| in [1e-9, 1e-5)   padding    python "1e-07"    rust "1e-7"
    //     everything else       agrees     python "0.0001"   rust "0.0001"
    //
    // # Why the edges are the assertion, not an example value
    //
    // The corpus found this band by accident and an earlier version of this
    // test pinned only `1e-5`. That passes if the writer's threshold moves to
    // `1e-6`, or to `1e-4`, or if `zmij` adopts `%g`'s rules outright — the
    // threshold is precisely the thing under test, so a value chosen from the
    // middle of a band cannot see it move. Every case below is therefore
    // either the agreeing value *immediately* outside an edge (`1e-4`,
    // `1e-10`) or the diverging value immediately inside one (`9.9e-5`,
    // `9.9e-6`). F-F's own history is the reason for the suspicion: its first
    // revision stated the divergence was narrower than it was, on the strength
    // of a 14-value probe that happened to contain no value in the notation
    // band at all.
    //
    // # Both rendering paths
    //
    // `python_str` renders a number through `serde_json::Number`'s `Display`,
    // which is *not* the same code as this crate's `PythonFormatter`. A float
    // reaches user-visible text down both routes — `str()` inside
    // `asset_name`, and the JSON writer for a saved library — so both are
    // pinned here. They happen to agree today; that is a fact worth stating
    // rather than assuming, since a change to one would otherwise leave the
    // other silently divergent.
    //
    // Python's answers below were produced by CPython:
    // `str(v) == json.dumps(v)` for every value in the table.
    use crate::runners::families::python_str;
    use serde_json::Value as V;

    let cases: &[(f64, &str, &str, &str)] = &[
        // value      python      rust        band
        (1e-5, "1e-05", "0.00001", "notation"),
        (9.9e-5, "9.9e-05", "0.000099", "notation"),
        (1e-7, "1e-07", "1e-7", "padding"),
        (9.9e-6, "9.9e-06", "9.9e-6", "padding"),
        // The agreeing values immediately outside each edge. These are what
        // make a moved threshold fail: if the writer started rendering
        // `1e-4` as `1e-04`, or stopped rendering `1e-10` as `1e-10`, the
        // band statement above would be false.
        (1e-4, "0.0001", "0.0001", "none"),
        (1e-10, "1e-10", "1e-10", "none"),
        // Far outside both bands, so the "everything else" arm is covered by
        // values rather than by the absence of values.
        (1.7e9, "1700000000.0", "1700000000.0", "none"),
        (1.5, "1.5", "1.5", "none"),
        (1e23, "1e+23", "1e+23", "none"),
    ];

    for (value, python_text, rust_text, named_band) in cases {
        // The table and the rule have to agree with each other, so a case
        // cannot be filed in the wrong band by hand.
        assert_eq!(
            float_band(*value),
            *named_band,
            "{value:e} is filed as {named_band:?} but the band rule says {:?}",
            float_band(*value)
        );

        let through_str = python_str(&V::from(*value));
        let through_writer = python_json::to_python_string(value)
            .expect("a float is serialisable")
            .trim_end()
            .to_string();
        assert_eq!(
            through_str, *rust_text,
            "{value:e} rendered through `str()`"
        );
        assert_eq!(
            through_writer, *rust_text,
            "{value:e} rendered through the JSON writer — the two rendering \
             paths are supposed to agree with each other, and this is the \
             assertion that says so"
        );

        // The divergence, where there is one, is *spelling only*: both texts
        // reparse to the same double. This is the property that matters — a
        // value that came back different would be a lost timestamp, not a
        // differently spelled one.
        let theirs: f64 = python_text.parse().expect("Python's text parses");
        assert_eq!(
            through_str.parse::<f64>().expect("our text parses").to_bits(),
            theirs.to_bits(),
            "{value:e}: {through_str} and {python_text} must be the same double"
        );

        // And the direction of the difference is as stated: a banded value
        // *must* differ, an unbanded one *must not*. Without this, a case
        // could sit in a band and still pass by agreeing, which would make the
        // band claim untested.
        assert_eq!(
            through_str != *python_text,
            *named_band != "none",
            "{value:e}: rust {through_str:?} vs python {python_text:?} is \
             not what the {named_band:?} band predicts"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. floats, pinned by bits (D-19)
// ---------------------------------------------------------------------------

#[test]
fn realistic_timestamps_survive_a_load_and_save_with_their_bits_intact() {
    // DECISIONS D-19. `serde_json`'s default f64 reader is fast but not
    // correctly rounded, and roughly a fifth of realistically shaped
    // `time.time()` values come back one ULP off. That is invisible in a UI and
    // fatal here: the port would read a timestamp Python wrote and re-save
    // different bytes, so every user's library file would churn on every save.
    //
    // This is the fixture that catches it. The comparison is on IEEE-754 bit
    // patterns, not decimal text, because the failure mode is a value that
    // *looks* right and is not.
    let oracle = oracle();
    let case = oracle
        .get("float_roundtrip")
        .expect("the oracle has a float_roundtrip section");
    let count = case
        .get("count")
        .and_then(Value::as_u64)
        .expect("float_roundtrip.count") as usize;
    let bits: Vec<u64> = names_at(case, "expected_bits_be_hex")
        .iter()
        .map(|hex| u64::from_str_radix(hex, 16).expect("a 16-digit hex bit pattern"))
        .collect();
    assert_eq!(bits.len(), count, "one bit pattern per fixture timestamp");

    let input = fixtures_dir().join(string_at(case, "in_file"));
    let library = Library::new_at(Some(input), FROZEN_NOW);
    assert_eq!(library.len(), count, "every timestamp should load");

    // Loaded values first: a wrong read shows up here before any writing.
    let mut wrong = 0usize;
    for (game, expected) in library.all("name").iter().zip(&bits) {
        for (field, value) in [("added", game.added), ("last_played", game.last_played)] {
            if value.to_bits() != *expected {
                wrong += 1;
                if wrong <= 3 {
                    eprintln!(
                        "{}: {field} read as {:#018x}, the oracle says {expected:#018x}",
                        game.name, value.to_bits()
                    );
                }
            }
        }
    }
    assert_eq!(
        wrong, 0,
        "{wrong} timestamps were read with the wrong bit pattern — is `float_roundtrip` \
         still enabled on serde_json in crates/core/Cargo.toml?"
    );

    // And the bytes: a correct port re-saves this file unchanged.
    let directory = scratch("floats-roundtrip");
    let written = directory.join("out.json");
    library.save_to(&written).expect("save should succeed");
    assert_eq!(
        read(&written),
        expected_output(case, "floats_roundtrip"),
        "re-saving realistic timestamps must reproduce Python's bytes exactly"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_value_the_default_reader_gets_wrong_is_read_correctly() {
    // D-19 says a future change to the `serde_json` dependency must not drop
    // `float_roundtrip`. The fixture test above cannot enforce that on its own:
    // it would pass either way if every fixture value happened to survive the
    // default reader, and a green suite proving nothing is precisely the lesson
    // of FINDINGS.md §5.
    //
    // So this names one value the default reader gets *wrong*, by the exact
    // numbers, and asserts the correct bits.
    //
    // `1836229572.9881566` is the first timestamp in
    // `floats_roundtrip.in.json`, and it is the worked example of the bug:
    //
    //   correctly rounded (lexical / Python's `float()`)  41db5ca8f13f3df5
    //   serde_json without `float_roundtrip`              41db5ca8f13f3df6
    //
    // One ULP, from `significand as f64` scaled by a `POW10` table (two
    // roundings) instead of `lexical::parse_concise_float`. 58 of the fixture's
    // 300 timestamps differ that way. With the feature enabled the value below
    // is correct; delete the feature and this test fails on the bit pattern,
    // which is the loud failure D-19 asks for.
    let text = "1836229572.9881566";
    let parsed = python_json::parse_lenient(text).expect("a plain JSON number");
    let value = parsed.as_f64().expect("a number");

    assert_eq!(
        value.to_bits(),
        0x41db_5ca8_f13f_3df5,
        "{text} parsed as {value:?} ({:#018x}) — serde_json's default f64 reader is a \
         ULP off here; is `float_roundtrip` still enabled in crates/core/Cargo.toml?",
        value.to_bits()
    );
    assert_eq!(
        value,
        text.parse::<f64>().unwrap(),
        "Rust's own parser is correctly rounded too, so the two must agree"
    );

    // The value is a game timestamp in the fixture, so it also has to reach the
    // model intact through the full load path — not just the JSON layer.
    let library = Library::new_at(
        Some(fixtures_dir().join("floats_roundtrip.in.json")),
        FROZEN_NOW,
    );
    let first = library.all("name")[0];
    assert_eq!(
        first.added.to_bits(),
        0x41db_5ca8_f13f_3df5,
        "the same value should survive `from_dict` unchanged"
    );
}

// ---------------------------------------------------------------------------
// 8. wrong-typed scalars (D-18)
// ---------------------------------------------------------------------------

/// What the port must make of the `wrongtype_*.in.json` files, one row per
/// fixture: `(label, name, steam_appid, category, mangohud)`.
///
/// The right-hand columns are the *declared type* of each field after D-18's
/// coercion, not a copy of the oracle's `stored` map — the oracle records
/// Python's reprs (`"str:'42'"`, `"inf"`, `"True"`), which is a different
/// question from what the typed port holds. Where the two diverge the port's
/// answer is the one asserted here, and the divergence is named in the comment
/// on the row.
const WRONG_TYPE_EXPECTATIONS: &[(&str, &str, i64, &str, bool)] = &[
    // A number in a string field becomes its text: `123` → `"123"`.
    ("name_is_int", "123", 0, "Uncategorized", false),
    // A numeric string in an integer field is parsed. Python happens to store
    // the string `'42'` and survive, but `steam_appid: i64` cannot hold text,
    // and D-18 says the string form is parsed, not kept as text.
    ("appid_is_str", "G", 42, "Uncategorized", false),
    // 41 digits and `inf` respectively: no `i64` exists to hold either, so the
    // field default is the only honest answer. This is the one place the typed
    // port genuinely cannot represent what Python could.
    ("appid_huge", "G", 0, "Uncategorized", false),
    ("appid_inf", "G", 0, "Uncategorized", false),
    // `"category": 42` → `"42"`, not Python's int.
    ("category_is_int", "G", 0, "42", false),
    // A `null` path has no string form, so the field default (`""`) stands.
    ("exe_path_is_null", "G", 0, "Uncategorized", false),
    // `"yes"` is a string, not a bool: serde's `as_bool` refuses it, and
    // guessing at truthiness is how a toggle silently turns itself on.
    ("toggles_are_str", "G", 0, "Uncategorized", false),
];

#[test]
fn a_wrong_typed_scalar_never_makes_the_library_unusable() {
    // DECISIONS D-18, and the reason it is a decision rather than a bug fix:
    // Python's `from_dict` copies every field except the two timestamps
    // straight into the dataclass with no type check, so `{"name": 123}` loads
    // fine and then `g.name.lower()` raises `AttributeError` for *all three*
    // sort modes — including the default. One wrong-typed scalar in a
    // hand-edited or tool-written `games.json` takes the whole library view
    // down, and the user never touched a setting.
    //
    // The oracle records what Python stored and, per sort mode, what happened.
    // Those recorded exceptions are the whole point: each one is a case the
    // port must survive.
    let oracle = oracle();
    let cases = object_at(&oracle, "wrong_types");
    assert!(cases.len() >= 9, "the wrong_types fixtures should be present");

    let mut repaired = 0usize;
    for (label, name, appid, category, mangohud) in WRONG_TYPE_EXPECTATIONS {
        let case = cases
            .get(*label)
            .unwrap_or_else(|| panic!("the oracle has no wrong_types case {label}"));
        let input = fixtures_dir().join(format!("wrongtype_{label}.in.json"));
        let library = Library::new_at(Some(input), FROZEN_NOW);

        // Rule one: the file loads. Reaching here at all is the assertion —
        // `Library::new_at` returns a library rather than a `Result`, exactly
        // because a decode failure is not the caller's problem to handle.
        assert_eq!(
            library.len(),
            1,
            "wrong_types[{label}] has an intact name, so the entry must survive"
        );

        // Rule two: every sort mode works, including the three Python raised
        // on. The oracle's `sorts` values are the exception strings.
        let sorts = object_at(case, "sorts");
        for mode in SORT_MODES {
            let outcome = sorts.get(mode).and_then(Value::as_str).unwrap_or("ok");
            if outcome != "ok" {
                repaired += 1;
            }
            assert_eq!(
                library.all(mode).len(),
                1,
                "wrong_types[{label}]: Python's {mode:?} sort failed with `{outcome}`; \
                 the port's must succeed"
            );
        }

        // Rule three: the stored values are the declared types (D-18), so a
        // caller can never meet a `Value` where a `String` was promised.
        let game = library.all("name")[0];
        assert_eq!(&game.name, name, "wrong_types[{label}].name");
        assert_eq!(game.steam_appid, *appid, "wrong_types[{label}].steam_appid");
        assert_eq!(&game.category, category, "wrong_types[{label}].category");
        assert_eq!(
            game.mangohud, *mangohud,
            "wrong_types[{label}].mangohud"
        );

        // And everything downstream of those fields still answers.
        let _ = game.display_category();
        let _ = game.is_linux();
        let _ = format_last_played(game.last_played, FROZEN_NOW);
        let _ = library.search("", "All", "name");
        let _ = library.categories();
    }

    assert!(
        repaired >= 3,
        "the fixtures should include at least the three Python-broken sorts this \
         test exists for, found {repaired}"
    );
}

#[test]
fn an_entry_with_no_usable_name_is_dropped() {
    // `name: null` and `name: ["x"]` are the two cases where D-18's coercion has
    // nothing to work with: a `null` is not a scalar, and a list has no string
    // form (`Value::to_string` would hand back `["x"]`, which is JSON syntax,
    // not a name). Both therefore take the field default, `""` — and
    // `Library.load` skips an entry with an empty name, as Python's
    // `if not g.name: continue` does.
    //
    // Python drops the `null` too, but keeps the list and then fails every sort
    // on it. Dropping both is the divergence, and it is deliberate: an entry
    // that cannot be named cannot be shown, searched or launched, so keeping it
    // would only carry the crash forward.
    let oracle = oracle();
    let cases = object_at(&oracle, "wrong_types");

    for (label, python) in [("name_is_null", 0u64), ("name_is_list", 1u64)] {
        let case = cases.get(label).expect("the oracle has this case");
        assert_eq!(
            case.get("count").and_then(Value::as_u64),
            Some(python),
            "the oracle's record of Python's count for {label} has changed"
        );
        let input = fixtures_dir().join(format!("wrongtype_{label}.in.json"));
        let library = Library::new_at(Some(input), FROZEN_NOW);
        assert!(
            library.is_empty(),
            "wrong_types[{label}]: an entry with no name must not reach the library"
        );
        // The sorts still work — on an empty library as much as a full one.
        for mode in SORT_MODES {
            assert!(library.all(mode).is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// 9. encoding, BOM, duplicate keys, deep nesting (D-21)
// ---------------------------------------------------------------------------

#[test]
fn invalid_utf8_gives_defaults_and_an_empty_library() {
    // `Library.load` catches `UnicodeDecodeError` in Python; `Settings.load`
    // does not, and `Backend.__init__` calls it — so the *Python* app dies at
    // startup on one bad byte in `settings.json`. D-20 makes the port tolerant
    // in both places.
    let oracle = oracle();
    let cases = object_at(&oracle, "encoding_and_shape");

    let library_case = cases.get("bad_utf8_library").expect("bad_utf8_library");
    let library = Library::new_at(
        Some(fixtures_dir().join("bad_utf8_library.in.json")),
        FROZEN_NOW,
    );
    assert_eq!(
        library.len(),
        library_case
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize,
        "invalid UTF-8 in the library should yield an empty library, matching Python"
    );

    let settings_case = cases.get("bad_utf8_settings").expect("bad_utf8_settings");
    assert!(
        settings_case.get("settings_raised").is_some(),
        "the oracle should record that Python raised here — that is the divergence"
    );
    // The port must not raise, and must answer with the defaults.
    assert_eq!(
        Settings::load(Some(fixtures_dir().join("bad_utf8_settings.in.json"))),
        Settings::default(),
        "invalid UTF-8 in settings should fall back to the defaults, not raise (D-20)"
    );
}

#[test]
fn a_leading_bom_is_stripped_and_the_games_survive() {
    // DECISIONS D-21. Python does not strip a BOM, so a BOM-prefixed file
    // parses to nothing, the library comes up empty, and the next save writes
    // `[]` over the user's games. The port keeps them.
    let oracle = oracle();
    let case = object_at(&oracle, "encoding_and_shape")
        .get("bom")
        .cloned()
        .expect("the oracle has a bom case");
    assert!(
        case.get("count").and_then(Value::as_u64) == Some(0),
        "the oracle should record Python losing the library — that is the divergence"
    );

    let library = Library::new_at(Some(fixtures_dir().join("bom.in.json")), FROZEN_NOW);
    assert_eq!(
        library.len(),
        1,
        "the port strips the BOM, so the game survives (deliberate divergence, D-21)"
    );
    assert_eq!(library.all("name")[0].name, "X");

    // Settings too: one parser serves both, so the BOM must be a non-issue
    // there as well.
    let directory = scratch("bom-settings");
    let path = directory.join("settings.json");
    std::fs::write(&path, b"\xef\xbb\xbf{\"color_scheme\": \"light\"}").unwrap();
    assert_eq!(Settings::load(Some(path)).color_scheme, "light");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn duplicate_json_keys_keep_the_last_value() {
    // `{"name": "first", "name": "second"}` — Python's `json.loads` keeps the
    // last. Parsing through `Value` gives the same behaviour; a typed serde
    // derive errors on a duplicate field instead, so this pins the choice.
    let oracle = oracle();
    let case = object_at(&oracle, "encoding_and_shape")
        .get("duplicate_keys")
        .cloned()
        .expect("the oracle has a duplicate_keys case");

    let library = Library::new_at(
        Some(fixtures_dir().join("duplicate_keys.in.json")),
        FROZEN_NOW,
    );
    assert_eq!(library.len(), 1);
    assert_eq!(
        library.all("name")[0].name,
        names_at(&case, "names")[0],
        "the last duplicate key should win"
    );
}

#[test]
fn deep_nesting_does_not_lose_the_library() {
    // DECISIONS D-21: Python parses 1,000 nested arrays; `serde_json`'s default
    // limit is 128. Since a parse failure empties the library and the next save
    // writes that emptiness back, being stricter than Python means destroying a
    // library over a nested value the app discards anyway. `parse_lenient`
    // handles it by clamping values deeper than 64 levels to `null` in the raw
    // text instead — a recursive parse is what overflows the stack, and the
    // junk is discarded downstream anyway, so nothing observable is lost.
    let oracle = oracle();
    let case = object_at(&oracle, "encoding_and_shape")
        .get("deep_nesting")
        .cloned()
        .expect("the oracle has a deep_nesting case");

    let library = Library::new_at(Some(fixtures_dir().join("deep_nesting.in.json")), FROZEN_NOW);
    assert_eq!(
        library.len(),
        case.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
        "the deep document should load with the same entries Python found"
    );
    assert_eq!(
        names(&library.all("name")),
        names_at(&case, "names"),
        "the nesting is on an unknown key, so both games must survive it"
    );
}

#[test]
fn a_leading_bom_is_not_stripped_from_a_string_value() {
    // The strip applies to the document, not to the data: a game whose *name*
    // starts with U+FEFF keeps it, because that character is inside a JSON
    // string and never touched by the prefix check.
    let parsed = python_json::parse_lenient("[\"\u{feff}inner\"]").expect("valid JSON");
    assert_eq!(parsed[0].as_str(), Some("\u{feff}inner"));
}

// ---------------------------------------------------------------------------
// 10. saving over a symlink (D-22 — pinned, not fixed)
// ---------------------------------------------------------------------------

#[test]
fn save_replaces_a_symlink_rather_than_writing_through_it() {
    // DECISIONS D-22 records this as inherited behaviour with no action: the
    // temp-file-and-rename that makes the write atomic also means a symlinked
    // `games.json` is replaced by a real file and the original target keeps its
    // stale contents. Pinned so it is a known property rather than a surprise,
    // and so a later "fix" has to be deliberate.
    let oracle = oracle();
    let case = oracle
        .get("symlink_save")
        .expect("the oracle has a symlink_save section");
    assert_eq!(
        case.get("link_still_symlink_after_save")
            .and_then(Value::as_bool),
        Some(false),
        "the oracle should record that the link is replaced"
    );

    let directory = scratch("symlink");
    let target = directory.join("target.json");
    let link = directory.join("games.json");
    std::fs::write(&target, "[]").unwrap();
    std::os::unix::fs::symlink(&target, &link).expect("creating a symlink");

    let mut library = Library::new_at(Some(link.clone()), FROZEN_NOW);
    library
        .add(Game::from_dict_at(
            &input_object(
                r#"{"id": "99999999999999999999999999999999", "name": "Through the link"}"#,
            ),
            FROZEN_NOW,
        ))
        .expect("save should succeed");

    assert!(
        !link.is_symlink(),
        "the link is replaced by a real file, as in Python"
    );
    assert_eq!(read(&target), "[]", "the original target keeps its stale contents");
    assert_eq!(
        read(&link),
        string_at(case, "link_bytes"),
        "the replacement file should hold the same bytes Python wrote"
    );

    let _ = std::fs::remove_dir_all(&directory);
}


// ---------------------------------------------------------------------------
// 12. the runners vector corpus (T-03)
// ---------------------------------------------------------------------------
//
// `docs/migration/oracle/run_runners_vectors.py` answers one question per line
// from the *Python* implementation. `--suite` emits the adversarial case list,
// which is committed as `runners_vectors.cases.json` beside its answers, so
// both are derived from reviewable code rather than hand-maintained.
//
//     python3 docs/migration/oracle/run_runners_vectors.py --suite \
//         > docs/migration/oracle/fixtures/runners_vectors.cases.json
//     python3 docs/migration/oracle/run_runners_vectors.py \
//         < docs/migration/oracle/fixtures/runners_vectors.cases.json \
//         > docs/migration/oracle/fixtures/runners_vectors.answers.json
//
// **Why this exists next to the file-shaped fixtures.** The oracle in §1-§11
// pins JSON *bytes*: it can prove the port writes `games.json` the way Python
// does. It cannot prove anything about the pure helpers — `shlex.split`, the
// `PurePosixPath` basename rule, `or ""` truthiness — and those are where the
// port's own two defects were found. Two of the ops here are direct ports of
// bugs that reached the committed code (`asset_name`, `safe_archive_name`).

/// The ops the corpus carries and this file answers.
///
/// A corpus op is covered by being on exactly one of these two lists, and
/// `the_vector_corpus_is_fully_replayed` asserts the union against what the
/// corpus actually contains. So an op added to the suite without an arm fails
/// loudly, and — the other direction, which a one-list guard misses — an op
/// that quietly *disappears* from the suite fails too. That second case is the
/// degenerate version of "a fixture that only ever checks what it already
/// covers": a corpus that shrank to nothing would otherwise stay green.
const PORTED_VECTOR_OPS: [&str; 10] = [
    "asset_matches",
    "asset_name",
    "install_id_from_parts",
    "looks_like_archive",
    "pick_asset",
    "pure_posix_name",
    "safe_archive_name",
    "safe_install_id",
    "sanitise_release_tag",
    "shell_split",
];

/// The four ops whose port lands with `launch_opts`.
///
/// Named explicitly rather than silently skipped.
const UNPORTED_VECTOR_OPS: [&str; 4] = [
    "parse_env_block",
    "merge_dll_overrides",
    "normalize_desktop_size",
    "virtual_desktop_argv",
];

fn vector_fixture(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    match python_json::parse_lenient(&read(&path)) {
        Ok(value) => value,
        Err(error) => panic!("{} is not readable JSON: {error}", path.display()),
    }
}

/// The answer the Python implementation gave for one case.
fn vector_answer(answers: &[Value], index: usize) -> &Value {
    answers
        .get(index)
        .unwrap_or_else(|| panic!("the answers file has no record {index}"))
}

/// Render Python's answer as the string the Rust side has to produce, so the
/// two are compared as text rather than as two JSON shapes.
///
/// A `Value` answer is rendered with the port's own writer, which is what makes
/// this a comparison of *behaviour* and not of serde's float formatting: the
/// float cases in `install_id_from_parts` never produce one, but a future op
/// that does will be compared with the port's own Python-compatible writer
/// rather than with `serde_json`'s.
fn python_answer_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => python_json::to_python_string(other)
            .expect("the value is serialisable")
            .trim_end()
            .to_string(),
    }
}

fn rust_vector_answer(op: &str, args: &Value) -> Result<Value, String> {
    use crate::runners::families;
    use crate::runners::{archive, shell};

    let text = |key: &str| -> String {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{op} needs a string {key:?}"))
            .to_string()
    };

    match op {
        "shell_split" => match shell::split_posix(&text("text")) {
            Ok(words) => Ok(json!(words)),
            // Python raises `ValueError`; the recorded answer is
            // `"ValueError: <message>"` and the message is user-visible.
            Err(error) => Err(format!("ValueError: {error}")),
        },
        "pure_posix_name" => Ok(json!(crate::runners::pure_posix_name(&text("text")))),
        "safe_archive_name" => Ok(json!(archive::safe_archive_name(
            &text("name"),
            args.get("fallback")
                .and_then(Value::as_str)
                .unwrap_or("runner.tar.gz")
        ))),
        "asset_name" => {
            let asset = args.get("asset").expect("asset_name needs an asset");
            Ok(json!(families::asset_name(asset)))
        }
        "asset_matches" => {
            let (require, exclude) = vector_tokens_by_family(args);
            Ok(json!(families::asset_matches_tokens(
                &text("asset_name"),
                &borrowed(&require),
                &borrowed(&exclude)
            )))
        }
        "looks_like_archive" => Ok(json!(families::looks_like_archive(&text("name")))),
        "pick_asset" => {
            let (require, exclude, prefer) = vector_tokens_by_family_full(args);
            let assets = args
                .get("assets")
                .and_then(Value::as_array)
                .expect("pick_asset needs assets")
                .clone();
            Ok(
                match families::pick_asset_tokens(
                    &assets,
                    &borrowed(&require),
                    &borrowed(&exclude),
                    &borrowed(&prefer),
                ) {
                    None => Value::Null,
                    // The op records the chosen *name*, so a missing or falsy
                    // name is the empty string rather than a null.
                    Some(asset) => json!(families::asset_name(&asset)),
                },
            )
        }
        "safe_install_id" => match archive::safe_install_id(&text("install_id")) {
            Ok(id) => Ok(json!(id)),
            Err(error) => Err(format!("ValueError: {error}")),
        },
        "sanitise_release_tag" => match archive::sanitise_release_tag(&text("tag")) {
            Ok(tag) => Ok(json!(tag)),
            Err(error) => Err(format!("ValueError: {error}")),
        },
        "install_id_from_parts" => {
            // The op builds a `ReleaseInfo` from raw parts and calls
            // `install_id`, which reads only these two fields — see
            // `families::install_id_for`.
            match families::install_id_for(&text("tag"), &text("family_id")) {
                Ok(id) => Ok(json!(id)),
                Err(error) => Err(format!("ValueError: {error}")),
            }
        }
        "wine_prefix_root" => Ok(json!(crate::runners::wine_prefix_root(
            Path::new(&text("prefix"))
        )
        .to_string_lossy())),
        "prefix_drive_cs" => Ok(json!(crate::runners::prefix_drive_cs(Path::new(
            &text("prefix")
        ))
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>())),
        other => panic!("no Rust arm for vector op {other:?}"),
    }
}

/// The `require`/`exclude` token lists a selection case names.
///
/// A case carries either a catalogue `family_id` or an inline `family` spec.
/// The inline form is not a convenience: the corpus needs token combinations no
/// shipped family has (both a `require` and an `exclude` that bite), and
/// `RunnerFamily`'s fields are `&'static`, so a synthetic one cannot be built
/// here at all. That is why `families` exposes the token-level entry points.
fn vector_tokens(args: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    use crate::runners::families;

    if let Some(id) = args.get("family_id").and_then(Value::as_str) {
        let family = families::family_by_id(id).expect("a catalogue family id");
        return (
            family.require.iter().map(|t| (*t).to_string()).collect(),
            family.exclude.iter().map(|t| (*t).to_string()).collect(),
            family.prefer.iter().map(|t| (*t).to_string()).collect(),
        );
    }
    let spec = args.get("family").expect("needs family_id or family");
    let strings = |key: &str| -> Vec<String> {
        spec.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    (strings("require"), strings("exclude"), strings("prefer"))
}

fn vector_tokens_by_family(args: &Value) -> (Vec<String>, Vec<String>) {
    let (require, exclude, _) = vector_tokens(args);
    (require, exclude)
}

fn vector_tokens_by_family_full(args: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    vector_tokens(args)
}

/// `&[&str]` over owned token lists, for the duration of one call.
fn borrowed(tokens: &[String]) -> Vec<&str> {
    tokens.iter().map(String::as_str).collect()
}

#[test]
fn the_vector_corpus_is_fully_replayed() {
    // Guards the guard: every op in the corpus is either replayed below or
    // named in `UNPORTED_VECTOR_OPS`. Without this, an op that quietly stopped
    // being replayed would leave a green suite covering less than it claims.
    let cases = vector_fixture("runners_vectors.cases.json");
    let cases = cases
        .get("cases")
        .and_then(Value::as_array)
        .expect("the corpus is a `cases` array");

    let mut seen: Vec<String> = Vec::new();
    for case in cases {
        let op = case
            .get("op")
            .and_then(Value::as_str)
            .expect("every case names an op")
            .to_string();
        if !seen.contains(&op) {
            seen.push(op);
        }
    }
    seen.sort();

    let mut expected: Vec<String> = PORTED_VECTOR_OPS
        .iter()
        .chain(UNPORTED_VECTOR_OPS.iter())
        .map(|op| (*op).to_string())
        .collect();
    expected.sort();
    assert_eq!(
        seen, expected,
        "the corpus and the arm lists are out of step: a corpus op has no Rust \
         arm, or an op named here is no longer in the suite"
    );
    assert!(
        !seen.is_empty(),
        "the vector corpus is empty — regenerate it"
    );
    for op in UNPORTED_VECTOR_OPS {
        assert!(
            seen.iter().any(|present| present == op),
            "{op} is listed as unported but the corpus no longer carries it"
        );
    }
}

#[test]
fn every_runner_vector_matches_python() {
    // The main replay. Each case is dispatched to the port and compared with
    // the answer `run_runners_vectors.py` recorded from CPython, as text so a
    // list, a string and a null are all comparable.
    let cases = vector_fixture("runners_vectors.cases.json");
    let cases = cases
        .get("cases")
        .and_then(Value::as_array)
        .expect("the corpus is a `cases` array");
    let answers = vector_fixture("runners_vectors.answers.json");
    let answers = answers.as_array().expect("answers is an array");
    assert_eq!(
        cases.len(),
        answers.len(),
        "the corpus and its answers are out of step — regenerate both"
    );

    let mut replayed = 0usize;
    let mut deferred = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let op = case.get("op").and_then(Value::as_str).expect("an op");
        if UNPORTED_VECTOR_OPS.contains(&op) {
            // Counted rather than skipped silently, so the assertion below is
            // "every case in the corpus is either replayed or acknowledged as
            // deferred" instead of a magic number that drifts.
            deferred += 1;
            continue;
        }
        let args = case.get("args").cloned().unwrap_or(Value::Null);
        let answer = vector_answer(answers, index);

        let outcome = rust_vector_answer(op, &args);
        let (expected_ok, expected_text) = match answer.get("ok") {
            Some(Value::Bool(true)) => (
                true,
                python_answer_text(answer.get("result").unwrap_or(&Value::Null)),
            ),
            Some(Value::Bool(false)) => (
                false,
                answer
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ),
            other => panic!("malformed answer record at {index}: {other:?}"),
        };

        match (&outcome, expected_ok) {
            (Ok(value), true) => {
                let produced = python_answer_text(value);
                if produced != expected_text {
                    failures.push(format!(
                        "#{index} {op}({args})\n  rust:   {produced}\n  python: {expected_text}"
                    ));
                }
            }
            (Err(error), false) => {
                if error != &expected_text {
                    failures.push(format!(
                        "#{index} {op}({args})\n  rust:   {error}\n  python: {expected_text}"
                    ));
                }
            }
            (Ok(value), false) => failures.push(format!(
                "#{index} {op}({args}) should have raised {expected_text}, \
                 but produced {}",
                python_answer_text(value)
            )),
            (Err(error), true) => failures.push(format!(
                "#{index} {op}({args}) should have produced {expected_text}, \
                 but raised {error}"
            )),
        }
        replayed += 1;
    }

    assert_eq!(
        failures.len(),
        0,
        "{} of {replayed} vector cases disagree with Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        replayed + deferred,
        cases.len(),
        "every case should be either replayed or deferred"
    );
    assert!(
        replayed >= 480,
        "expected the whole ported corpus, replayed {replayed} of {} \
         ({deferred} deferred to launch_opts)",
        cases.len()
    );
}
