"""Generate the JSON-compatibility oracle from the *Python* implementation.

This is the executable ground truth the Rust port must reproduce. It is
generated from the Python code precisely so that it does not encode our
*assumptions* about Python's behaviour — several of which turned out to be
wrong (see FINDINGS.md).

Regenerate with:

    python3 docs/migration/oracle/gen_oracle.py

writes `oracle.json` plus `.in.json` / `.out.json` byte-level fixture pairs
into `docs/migration/oracle/fixtures/`. Requires nothing but the stdlib and
this repository; the Python app is imported from the repo root.

Declared in DECISIONS D-06 as the compatibility contract for T-02/T-03.

**Deterministic by construction.** `Game.added` defaults to `time.time()`, and
`from_dict` regenerates it whenever the incoming value is invalid — so a naive
run produces different bytes every time. Since these fixtures are checked in
and compared against, the clock is frozen to `FROZEN_NOW` for the whole run.
Everything else in the output is a pure function of the input.
"""
import json, math, sys, pathlib, importlib, time as _time

# Freeze the clock *before* the app is imported, so every `time.time()` call the
# implementation makes (the `added` default factory, `mark_played`,
# `format_last_played`'s `now=None` path) resolves to one fixed instant. Without
# this the fixtures change on every regeneration and cannot be committed.
FROZEN_NOW = 1_700_000_000.0
_time.time = lambda: FROZEN_NOW

REPO = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

from gamehandler.models import Game, Library, format_last_played, UNCATEGORIZED, SORT_MODES
from gamehandler.settings import Settings, COLOR_SCHEMES, VIEW_MODES

assert _time.time() == FROZEN_NOW, "clock freeze failed; fixtures would be nondeterministic"

OUT = pathlib.Path(__file__).resolve().parent / "fixtures"
OUT.mkdir(parents=True, exist_ok=True)

results = {}

# ---------------------------------------------------------------- 1. from_dict
# Adversarial Game.from_dict inputs -> normalized field values.
from_dict_cases = {
    "clean": {"id": "a"*32, "name": "Half-Life", "added": 1700000000.5, "last_played": 1700000001.25},
    "nan_added": {"id": "b"*32, "name": "NaN added", "added": float("nan"), "last_played": 5.0},
    "inf_last": {"id": "c"*32, "name": "Inf last", "added": 1.0, "last_played": float("inf")},
    "neg_inf_last": {"id": "d"*32, "name": "NegInf last", "added": 1.0, "last_played": float("-inf")},
    "bool_true": {"id": "e"*32, "name": "Bool true", "added": True, "last_played": True},
    "bool_false": {"id": "f"*32, "name": "Bool false", "added": False, "last_played": False},
    "str_ts": {"id": "g"*32, "name": "Str ts", "added": "soon", "last_played": "later"},
    "null_ts": {"id": "h"*32, "name": "Null ts", "added": None, "last_played": None},
    "negative": {"id": "i"*32, "name": "Negative", "added": -5.0, "last_played": -1.0},
    "int_ts": {"id": "j"*32, "name": "Int ts", "added": 42, "last_played": 7},
    "unknown_keys": {"id": "k"*32, "name": "Unknown", "legacy_field": 1, "another": [1,2]},
    "unicode": {"id": "l"*32, "name": "Pokémon — Ünïcode ✓", "category": "日本語"},
    "empty_category": {"id": "m"*32, "name": "Blank cat", "category": "   "},
    "linux_game": {"id": "n"*32, "name": "Native", "kind": "linux"},
}
results["from_dict"] = {}
for label, data in from_dict_cases.items():
    g = Game.from_dict(dict(data))
    d = g.to_dict()
    # `added` is regenerated from time.time() when invalid -> non-deterministic.
    results["from_dict"][label] = {
        "input": {k: (repr(v) if isinstance(v, float) else v) for k, v in data.items()},
        "added_regenerated": label in ("nan_added","inf_last","neg_inf_last","bool_true",
                                       "bool_false","str_ts","negative"),
        "output": {k: v for k, v in d.items() if k != "added"},
        "added_value": d["added"],
        "added_is_recent": d["added"] is not None and abs(d["added"] - _time.time()) < 60,
        "is_linux": g.is_linux,
        "display_category": g.display_category,
        # `null` is the one input where Rust must NOT copy this result — see
        # rust_divergences and DECISIONS D-14. Flagged inline so a test writer
        # reading only this entry cannot mistake None for the expected output.
        "rust_must_diverge": label == "null_ts",
    }

# ------------------------------------------------- 2. deserialize -> serialize bytes
def roundtrip(label, raw_text, is_library=True):
    """Feed raw bytes to Library.load then Library.save; capture exact output."""
    p = OUT / f"{label}.in.json"
    p.write_text(raw_text, encoding="utf-8")
    lib = Library(path=p)
    out = OUT / f"{label}.out.json"
    # save() writes to with_suffix('.json.tmp') then replaces
    lib.path = out
    lib.save()
    return {
        "input": raw_text,
        "count": len(lib),
        "names": [g.name for g in lib.all()],
        "output_bytes": out.read_text(encoding="utf-8"),
    }

libcases = {
    "empty_file": "[]",
    "not_a_list": '{"name": "oops"}',
    "malformed": "{not json at all",
    "nan_values": '[{"id": "%s", "name": "NaN game", "added": NaN, "last_played": Infinity}]' % ("p"*32),
    "non_dict_entries": '["a string", 42, null, {"id": "%s", "name": "Real"}]' % ("q"*32),
    "empty_name": '[{"id": "%s", "name": ""}, {"id": "%s", "name": "Kept"}]' % ("r"*32, "s"*32),
    "duplicate_ids": '[{"id": "%s", "name": "First"}, {"id": "%s", "name": "Second"}]' % ("t"*32, "t"*32),
    "unicode_out": '[{"id": "%s", "name": "Pokémon — ✓", "category": "日本語"}]' % ("u"*32),
    "blank_category": '[{"id": "%s", "name": "A", "category": "  "}, {"id": "%s", "name": "B", "category": "RPG"}]' % ("v"*32, "w"*32),
    "trailing_garbage": '[{"id": "%s", "name": "X"}] trailing' % ("x"*32),
}
results["roundtrip"] = {}
for label, text in libcases.items():
    try:
        results["roundtrip"][label] = roundtrip(label, text)
    except Exception as exc:
        results["roundtrip"][label] = {"error": f"{type(exc).__name__}: {exc}"}

# ---------------------------------------------------------------- 3. sorting
sort_cases = [
    {"id": "1"*32, "name": "zeta", "added": 100.0, "last_played": 50.0},
    {"id": "2"*32, "name": "Alpha", "added": 300.0, "last_played": 0.0},
    {"id": "3"*32, "name": "beta", "added": 200.0, "last_played": 900.0},
    {"id": "4"*32, "name": "Alpha", "added": 400.0, "last_played": 10.0},
]
p = OUT / "sort.in.json"
p.write_text(json.dumps(sort_cases), encoding="utf-8")
lib = Library(path=p)
results["sorting"] = {mode: [g.name for g in lib.all(sort=mode)] for mode in SORT_MODES}

# ------------------------------------------------------- 4. search + categories
search_cases = [
    {"id": "1"*32, "name": "Portal 2", "category": "Puzzle"},
    {"id": "2"*32, "name": "Doom", "category": "Shooter"},
    {"id": "3"*32, "name": "Puzzle Quest", "category": ""},
    {"id": "4"*32, "name": "Hades", "category": "  roguelike  "},
]
p = OUT / "search.in.json"
p.write_text(json.dumps(search_cases), encoding="utf-8")
lib = Library(path=p)
results["search"] = {
    "categories": lib.categories(),
    "q_portal": [g.name for g in lib.search("portal")],
    "q_puzzle": [g.name for g in lib.search("puzzle")],
    "q_blank": [g.name for g in lib.search("")],
    "cat_puzzle": [g.name for g in lib.search("", category="Puzzle")],
    "cat_all": [g.name for g in lib.search("", category="All")],
    "cat_uncat": [g.name for g in lib.search("", category=UNCATEGORIZED)],
    "q_case": [g.name for g in lib.search("DOOM")],
    "q_trim": [g.name for g in lib.search("  doom  ")],
}

# --------------------------------------------------- 5. format_last_played
NOW = 1_700_000_000.0
fp_cases = [
    ("never_zero", 0.0), ("never_falsy", 0.0),
    ("just_now_0", NOW), ("just_now_59s", NOW - 59), ("min_boundary_119s", NOW - 119),
    ("min_2", NOW - 120), ("min_59", NOW - 59*60),
    ("hour_1", NOW - 3600), ("hour_2", NOW - 7200), ("hour_23", NOW - 23*3600),
    ("yesterday", NOW - 24*3600), ("days_2", NOW - 2*24*3600), ("days_29", NOW - 29*24*3600),
    ("month_1", NOW - 30*24*3600), ("month_2", NOW - 60*24*3600), ("month_11", NOW - 330*24*3600),
    ("year_1", NOW - 360*24*3600), ("year_2", NOW - 800*24*3600),
    ("future", NOW + 10000),
]
results["format_last_played"] = {
    "now": NOW,
    "cases": {label: format_last_played(ts, now=NOW) for label, ts in fp_cases},
}

# ---------------------------------------------------------------- 6. Settings
settings_cases = {
    "clean": {},
    "bad_scheme": {"color_scheme": "neon"},
    "bad_view": {"view_mode": "carousel"},
    "bad_sort": {"sort_mode": "chaos"},
    "unknown_key": {"legacy": True, "color_scheme": "light"},
    "null_scheme": {"color_scheme": None},
    "all_defaults": {"color_scheme": "dark"},
}
results["settings"] = {}
for label, data in settings_cases.items():
    p = OUT / f"settings_{label}.in.json"
    p.write_text(json.dumps(data), encoding="utf-8")
    s = Settings.load(path=p)
    out = OUT / f"settings_{label}.out.json"
    s.save(path=out)
    results["settings"][label] = {"input": data, "to_dict": s.to_dict(),
                                  "output_bytes": out.read_text(encoding="utf-8")}
# non-dict settings file
p = OUT / "settings_notdict.in.json"
p.write_text("[1,2,3]", encoding="utf-8")
results["settings"]["not_a_dict"] = {"to_dict": Settings.load(path=p).to_dict()}

# ------------------------------------------------- 7. float byte-format fidelity
# Rust's serde_json and Python's json differ in how they render f64 to text.
# These cases pin the Python side so the port can be tested against it.
# (See FINDINGS.md F-F: exponents, and F-G: out-of-range literals.)
float_cases = [
    ("exp_single_digit", 1e-7), ("exp_single_digit_pos", 1e7),
    ("exp_two_digit", 1e-10), ("exp_two_digit_pos", 1e23),
    ("whole_large", 1e16), ("whole_large2", 1e17),
    ("frac", 0.1 + 0.2), ("third", 1.0 / 3.0),
    ("max_f64", 1e308), ("min_subnormal", 5e-324),
    ("int_precision_limit", 9007199254740992.0),
    ("typical_ts", 1700000000.5), ("zero", 0.0), ("small_whole", 100.0),
]
p = OUT / "floats.in.json"
p.write_text(json.dumps([{"id": "%032x" % i, "name": label,
                          "added": v, "last_played": 0.0}
                         for i, (label, v) in enumerate(float_cases)]), encoding="utf-8")
lib = Library(path=p)
out = OUT / "floats.out.json"
lib.path = out
lib.save()
results["float_format"] = {
    "note": "Python's rendering of each f64 in save() output, plus the raw dumps() form",
    "cases": {label: {"value": repr(v), "json_dumps": json.dumps(v)} for label, v in float_cases},
    "saved_bytes": out.read_text(encoding="utf-8"),
}

# Out-of-f64-range and huge-integer literals in the source file. Python accepts
# these; serde_json rejects `1e400` outright. Reachability matters: a rejection
# discards the whole library, an acceptance normalizes one field.
oob_cases = {
    "overflow_to_inf": '[{"id": "%s", "name": "Ov", "added": 1e400, "last_played": 1e400}]' % ("1"*32),
    "neg_overflow": '[{"id": "%s", "name": "NegOv", "added": -1e400, "last_played": -1e400}]' % ("2"*32),
    "big_int": '[{"id": "%s", "name": "BigInt", "added": 123456789012345678901234567890}]' % ("3"*32),
    "i64_max": '[{"id": "%s", "name": "I64Max", "added": 9223372036854775807}]' % ("4"*32),
    "u64_max_plus": '[{"id": "%s", "name": "U64P", "added": 18446744073709551616}]' % ("5"*32),
}
results["out_of_range"] = {}
for label, text in oob_cases.items():
    p = OUT / f"{label}.in.json"
    p.write_text(text, encoding="utf-8")
    lib = Library(path=p)
    notes = {"parsed": len(lib) > 0, "count": len(lib)}
    if len(lib):
        g = lib.get({"overflow_to_inf":"1"*32, "neg_overflow":"2"*32, "big_int":"3"*32,
                     "i64_max":"4"*32, "u64_max_plus":"5"*32}[label])
        notes["added"] = repr(g.added)
        notes["last_played"] = repr(g.last_played)
        try:
            lib.all(sort="added")
            notes["sort_ok"] = True
        except Exception as exc:
            notes["sort_ok"] = f"{type(exc).__name__}: {exc}"
    results["out_of_range"][label] = notes

# ------------------------------------------------- 8. where Rust must DIVERGE
# Everything above is behaviour the Rust port must reproduce. This section is
# the opposite: the few places where matching Python would propagate a defect
# or a divergence we have deliberately accepted. Stated machine-readably so the
# T-02 tests do not have to infer it from prose.
results["rust_divergences"] = {
    "note": (
        "Cases where the Rust port MUST NOT match the Python result recorded "
        "elsewhere in this file. Each entry names the governing decision."
    ),
    "cases": [
        {
            "id": "null_timestamp_normalized",
            "decision": "D-14",
            "python": {
                "input": '{"id": "h"*32, "name": "Null ts", "added": null, "last_played": null}',
                "observed": "added stays None, last_played stays None",
                "consequence": (
                    'sort="recent"/"added" raise TypeError: bad operand type for '
                    "unary -: 'NoneType'"
                ),
            },
            "rust": {
                "expected": "added regenerated from the clock, last_played = 0.0",
                "rationale": (
                    "null carries no information; from_dict already normalizes every "
                    "other invalid value this way, so null slipping through is an early "
                    "`continue` bug, not a policy. Never hold a nullable timestamp."
                ),
            },
        },
        {
            "id": "float_exponent_spelling",
            "decision": "D-15",
            "python": {"observed": "1e-07 and 10000000.0 (C printf %g rules)"},
            "rust": {
                "expected": "1e-7 and 1e7 (serde_json/Ryu)",
                "rationale": (
                    "Both are valid JSON and reparse identically. Byte-equality is not "
                    "required here; assert numeric equality after reparse instead of "
                    "emulating printf."
                ),
            },
        },
    ],
}

# ---------------------------------------------------------------- 9. constants
results["constants"] = {
    "COLOR_SCHEMES": list(COLOR_SCHEMES), "VIEW_MODES": list(VIEW_MODES),
    "SORT_MODES": list(SORT_MODES), "UNCATEGORIZED": UNCATEGORIZED,
    "default_color_scheme": Settings().color_scheme,
}

(OUT / "oracle.json").write_text(json.dumps(results, indent=2, default=str), encoding="utf-8")
print("wrote", OUT / "oracle.json")
print("cases:", {k: (len(v) if isinstance(v,(dict,list)) else v) for k,v in results.items()})
