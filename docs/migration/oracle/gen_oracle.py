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


def _sha(path):
    """SHA-256 of a fixture file.

    `oracle.json` deliberately references its fixtures by name and hash rather
    than embedding their contents: the byte-level fixtures are already written
    alongside it, and duplicating them made the summary larger than everything
    it summarised.
    """
    return __import__("hashlib").sha256(pathlib.Path(path).read_bytes()).hexdigest()

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
        "out_file": out.name,
        "out_sha256": _sha(out),
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
                                  "out_file": out.name, "out_sha256": _sha(out)}
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
    "out_file": out.name,
    "out_sha256": _sha(out),
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

# --------------------------------------- 7b. realistic timestamps and their bits
# Added after the adversarial review found that the float fixtures below were
# composed entirely of "nice" values that sit in serde_json's accurate range,
# so they could not catch its 1-ULP f64 reader error on real data. These are
# seeded random *realistic* timestamps plus their exact IEEE-754 bit patterns,
# which is the only thing that pins the decode path.
import random, struct
# 300 is ample: serde_json's default reader errs on ~15-20% of realistic
# timestamps, so P(a buggy port matches all 300) is below 1e-20. Keeping the
# corpus small keeps the committed fixtures small -- each entry is a full
# 31-field Game, so the cost is linear in the count.
_rng = random.Random(0x6772_6864)  # fixed seed: the fixture must not move
_ts = [_rng.uniform(1.5e9, 2.0e9) for _ in range(300)]
_realistic = [{"id": "%032x" % i, "name": "ts%04d" % i, "added": v, "last_played": v}
              for i, v in enumerate(_ts)]
p = OUT / "floats_roundtrip.in.json"
p.write_text(json.dumps(_realistic, indent=2), encoding="utf-8")
_lib = Library(path=p)
out = OUT / "floats_roundtrip.out.json"
_lib.path = out
_lib.save()
results["float_roundtrip"] = {
    "note": (
        "Realistic time.time()-shaped values. serde_json's DEFAULT f64 reader is "
        "1 ULP wrong on a large fraction of these; `float_roundtrip` fixes it. A "
        "correct port re-saves these bytes unchanged. Compare BITS, not decimals."
    ),
    "count": len(_ts),
    "expected_bits_be_hex": [struct.pack(">d", v).hex() for v in _ts],
    "in_file": "floats_roundtrip.in.json",
    "out_file": "floats_roundtrip.out.json",
    "out_sha256": _sha(out),
    "out_sha256_note": "regression guard; the byte-level check reads out_file",
}

# ------------------------------------------- 7b-ii. the exponent-notation band
# The 300 realistic timestamps above sit in the 1e9 range, where the only
# divergence is Python zero-padding a negative exponent (`1e-07` vs `1e-7`) and
# the values agree after reparse. That is NOT the only way the two writers can
# disagree, and the discovery is worth recording because the first predicate
# written against it was too narrow to see the difference.
#
# `repr(float)` switches between fixed and exponential notation at a different
# threshold than `serde_json` does. Measured on the pinned serde_json 1.0.151:
#
#     value       serde_json     Python repr      divergence
#     1e-4        0.0001         0.0001           none
#     1e-5        0.00001        1e-05            NOTATION
#     2.5e-5      0.000025       2.5e-05          NOTATION
#     1.2345e-5   0.000012345    1.2345e-05       NOTATION
#     1e-6        1e-6           1e-06            padding only
#     1e-7        1e-7           1e-07            padding only
#     1e7         10000000.0     10000000.0       none
#
# So `1e-5 <= |x| < 1e-4` diverges by *notation*, not by padding: the port
# writes a fixed-point literal where Python writes an exponential one. Both are
# valid JSON and reparse to the same double, so this is not a correctness bug,
# but "the port cannot byte-round-trip a file in this band" is a real parity
# limitation and belongs in REPORT.md rather than in a test's blind spot.
#
# Reachability is bounded and worth stating: `added`/`last_played` carry
# `time.time()` values in the 1e9 range, and every other float field is written
# by us, so a file in this band is hand-edited or third-party. The band is
# covered here so the class is *known* rather than latent.
_boundary = [
    ("exp_agrees_at_1e-4", 1e-4),
    ("exp_notation_at_1e-5", 1e-5),
    ("exp_notation_at_2_5e-5", 2.5e-5),
    ("exp_notation_at_1_2345e-5", 1.2345e-5),
    ("exp_notation_at_9_99e-5", 9.99e-5),
    ("exp_padding_at_1e-6", 1e-6),
    ("exp_padding_at_1e-7", 1e-7),
    ("exp_agrees_at_1e7", 1e7),
    ("exp_agrees_at_zero", 0.0),
]
_boundary_entries = [
    {"id": "%032x" % (0xB0 + i), "name": label, "added": value, "last_played": value}
    for i, (label, value) in enumerate(_boundary)
]
p = OUT / "exp_notation_boundary.in.json"
p.write_text(json.dumps(_boundary_entries, indent=2), encoding="utf-8")
_lib = Library(path=p)
out = OUT / "exp_notation_boundary.out.json"
_lib.path = out
_lib.save()
results["exp_notation_boundary"] = {
    "note": (
        "The band where serde_json and Python choose different float NOTATION. "
        "Values in [1e-5, 1e-4) reparse identically but are spelled differently: "
        "the port writes 0.00001 where Python writes 1e-05. Only `1e-6`/`1e-7` are "
        "padding-only. A test that admits any differing float line must reparse "
        "before comparing, because the guard `ours.contains('e-')` is false for "
        "`0.00001`."
    ),
    "cases": {label: value for label, value in _boundary},
    "bit_patterns_be_hex": {label: struct.pack(">d", value).hex() for label, value in _boundary},
    "in_file": "exp_notation_boundary.in.json",
    "out_file": "exp_notation_boundary.out.json",
    "out_sha256": _sha(out),
}

# ---------------------------------------------- 7c. wrong-typed / hostile fields
# Only `added`/`last_played` are validated by from_dict; every other field is
# stored as-is. `{"name": 123}` is MORE reachable than the null-timestamp bug:
# it takes the library view down at the DEFAULT sort, with no setting change.
# NOTE: each body must be a JSON *list* of objects. Library.load rejects a bare
# dict as "not a list" and yields an empty library, so a missing [ ] here would
# silently produce a fixture that tests nothing (this happened once).
#
# The fourth element records whether the entry is expected to SURVIVE loading.
# `name: null` does not: `if game.name:` is falsy for None, so the entry is
# dropped. That is real behaviour and worth pinning, but it means "loaded
# nothing" is not by itself a sign of a broken fixture — hence the flag.
_typed = [
    ("name_is_int", '[{"id": "%s", "name": 123}]' % ("a"*32), "a"*32, True),
    ("name_is_null", '[{"id": "%s", "name": null}]' % ("b"*32), "b"*32, False),
    ("name_is_list", '[{"id": "%s", "name": ["x"]}]' % ("c"*32), "c"*32, True),
    ("appid_is_str", '[{"id": "%s", "name": "G", "steam_appid": "42"}]' % ("d"*32), "d"*32, True),
    ("appid_huge", '[{"id": "%s", "name": "G", "steam_appid": %d}]' % ("e"*32, 10**40), "e"*32, True),
    ("appid_inf", '[{"id": "%s", "name": "G", "steam_appid": Infinity}]' % ("f"*32), "f"*32, True),
    ("category_is_int", '[{"id": "%s", "name": "G", "category": 42}]' % ("1"*32), "1"*32, True),
    ("exe_path_is_null", '[{"id": "%s", "name": "G", "exe_path": null}]' % ("2"*32), "2"*32, True),
    ("toggles_are_str", '[{"id": "%s", "name": "G", "mangohud": "yes"}]' % ("3"*32), "3"*32, True),
]
results["wrong_types"] = {}
for label, text, gid, expect_survives in _typed:
    p = OUT / f"wrongtype_{label}.in.json"
    p.write_text(text, encoding="utf-8")
    lib = Library(path=p)
    game = lib.get(gid)
    # A fixture that loads nothing when the entry was expected to survive means
    # the body is malformed and the case would silently assert nothing.
    assert (game is not None) == expect_survives, (
        f"fixture {label}: expected survives={expect_survives}, got {game is not None}: {text!r}"
    )
    notes = {"count": len(lib), "entry_survives": game is not None}
    if game is not None:
        # Record the Python type *and* the value separately. The earlier
        # single-string form was ambiguous in exactly the case that matters:
        # `name_is_int` stores the int 123, which renders as "123" — byte
        # identical to how a genuine string would render, so the one field
        # whose whole point is "Python let a non-string through" read as if it
        # had not. A test author consumed it as data and asserted on the
        # prefix instead of the type. `python_type` is the fact; `repr` is the
        # rendering.
        notes["stored"] = {
            k: {"python_type": type(v).__name__, "repr": repr(v)}
            for k, v in [("name", game.name), ("steam_appid", game.steam_appid),
                         ("category", game.category), ("exe_path", game.exe_path),
                         ("mangohud", game.mangohud)]
        }
        # Which sorts survive? This is the user-visible consequence.
        notes["sorts"] = {}
        for mode in SORT_MODES:
            try:
                lib.all(sort=mode)
                notes["sorts"][mode] = "ok"
            except Exception as exc:
                notes["sorts"][mode] = f"{type(exc).__name__}: {exc}"
    results["wrong_types"][label] = notes

# ------------------------------------- 7d. encoding, BOM, dup keys, nesting, link
def _probe(label, raw_bytes, ident=None, as_settings=False):
    p = OUT / f"{label}.in.json"
    p.write_bytes(raw_bytes)
    notes = {}
    if as_settings:
        try:
            notes["settings_default"] = Settings.load(path=p).to_dict()["color_scheme"]
        except Exception as exc:
            notes["settings_raised"] = f"{type(exc).__name__}: {exc}"
    else:
        try:
            lib = Library(path=p)
            notes["count"] = len(lib)
            notes["names"] = [g.name for g in lib.all()]
            out = OUT / f"{label}.out.json"
            lib.path = out
            lib.save()
            notes["out_file"] = out.name
            notes["out_sha256"] = _sha(out)
        except Exception as exc:
            notes["raised"] = f"{type(exc).__name__}: {exc}"
    return notes

_ident = ("a"*32)
deep_body = '[{"id": "%s", "name": "Deep", "junk": %s}, {"id": "%s", "name": "Sib"}]' % (
    "b"*32, "["*1000 + "]"*1000, "c"*32)
results["encoding_and_shape"] = {
    # Python catches UnicodeDecodeError in Library.load but NOT in Settings.load.
    # The settings asymmetry is a startup crash in the Python app (bridge.py
    # calls Settings.load in Backend.__init__); the port must not copy it.
    "bad_utf8_library": _probe("bad_utf8_library",
                               b'[{"id": "' + b'd'*32 + b'", "name": "Bad\xff"}]'),
    "bad_utf8_settings": _probe("bad_utf8_settings",
                                b'{"color_scheme": "dark"\xff}', as_settings=True),
    # BOM: Python rejects -> empty library -> the NEXT save() writes "[]" over
    # the user's file. Data loss in both implementations; pinned so the port is
    # not made *worse* than the original.
    "bom": _probe("bom", b'\xef\xbb\xbf[{"id": "' + b'e'*32 + b'", "name": "X"}]'),
    # Duplicate JSON *keys* (not duplicate ids): Python keeps the last. A typed
    # serde derive would error here, so parsing via Value first is load-bearing.
    "duplicate_keys": _probe("duplicate_keys",
                             b'[{"id": "' + b'f'*32 + b'", "name": "first", "name": "second"}]'),
    # Nesting: Python handles 50k deep. serde_json's recursion limit is 128.
    "deep_nesting": _probe("deep_nesting", deep_body.encode()),
}

# Save replaces a symlink rather than following it, so the link is destroyed and
# the original target keeps stale content. Python behaves this way; pinned so
# the port is not *newer* than the original in a surprising direction.
_sym = OUT / "symlink_target.json"
_sym.write_text("[]", encoding="utf-8")
_link = OUT / "symlink.in.json"
if _link.is_symlink() or _link.exists():
    _link.unlink()
_link.symlink_to(_sym)
_lib = Library(path=_link)
# Explicit id: Game's default_factory is uuid4, which would make this fixture
# differ on every run (caught by the two-run determinism check).
_lib.add(Game(id="9" * 32, name="Through the link"))
results["symlink_save"] = {
    "note": "save() writes target.with_suffix('.json.tmp') then replace(path)",
    "link_still_symlink_after_save": _link.is_symlink(),
    "target_bytes": _sym.read_text(encoding="utf-8"),
    "link_bytes": _link.read_text(encoding="utf-8"),
}

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
            "id": "wrong_typed_scalars_coerced",
            # Note the *third* rule, discovered while implementing this: a value
            # with no usable string form (an array, an object) leaves the name
            # empty, and Library.load skips an unnamed entry — so the game is
            # dropped. Python keeps it and then fails every sort on it. The
            # option set in D-18 named only "hold the raw value" and "coerce";
            # dropping is a third behaviour and is recorded here so a test
            # author does not expect the entry to appear.
            "also_affects_count": (
                "name:[\"x\"] — Python keeps the entry (count 1) and every sort "
                "raises AttributeError; the port drops it (count 0)"
            ),
            "decision": "D-18",
            "python": {
                "input": '{"id": "a"*32, "name": 123}',
                "observed": (
                    "Game.name becomes the int 123; Library.all() then calls "
                    "g.name.lower() and raises AttributeError: 'int' object has no "
                    "attribute 'lower' for EVERY sort mode, including the default"
                ),
                "consequence": (
                    "the library view is empty/unrenderable for any hand-edited or "
                    "tool-written games.json with a non-string name -- no setting "
                    "change needed, unlike D-14"
                ),
                "also": (
                    "name/list variants fail identically; steam_appid, category, "
                    "exe_path and the boolean toggles are equally accepting but "
                    "happen not to be touched by the sort key"
                ),
            },
            "rust": {
                "expected": (
                    "parse every scalar field to its declared type, coercing or "
                    "defaulting rather than holding an arbitrary JSON value: a "
                    "non-string name becomes its string form, a non-bool toggle "
                    "becomes its default"
                ),
                "rationale": (
                    "a wrong type in one entry must never make the whole library "
                    "unusable; this is the same class of bug as D-14 and is fixed "
                    "the same way"
                ),
            },
        },
        {
            "id": "float_exponent_spelling",
            "decision": "D-15",
            "python": {"observed": "1e-07 (C printf %g pads the exponent to 2 digits)"},
            "rust": {
                # Measured on the pinned serde_json 1.0.151, which writes
                # floats via `zmij`, not `ryu`. Only the *negative* single-digit
                # exponent differs; `1e7` renders as `10000000.0` on both sides.
                # An earlier version of this entry claimed "1e7" — a spelling
                # the port never produces, so a test written from it could not
                # pass. Corrected after adversarial review.
                "expected": "1e-7 (the only divergent case in the fixture)",
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
