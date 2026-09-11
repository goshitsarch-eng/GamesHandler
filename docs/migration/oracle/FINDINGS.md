# Python behaviour: the executable compatibility contract

`gen_oracle.py` runs the **existing Python implementation** over a set of
adversarial inputs and records exactly what it does. `fixtures/oracle.json`
holds the structured results; `fixtures/*.in.json` / `*.out.json` are
byte-level input→output pairs.

**Why this exists.** The Rust port must read files the Python app wrote and
vice versa (DECISIONS D-06). Rather than reason about what Python's `json`
module does, we asked it. That was the right call: it immediately contradicted
four assumptions the team had written down, and it found two real bugs.

Regenerate any time with `python3 docs/migration/oracle/gen_oracle.py`.

---

## 1. Findings that change the port

### F-A. `ensure_ascii=True` — non-ASCII is escaped in the output

`Library.save` uses `json.dumps(payload, indent=2)`, whose default
`ensure_ascii=True` escapes every non-ASCII character. The evidence is the
fixture pair `unicode_out.in.json` / `unicode_out.out.json` — the input carries
real UTF-8 bytes and the output does not:

```
unicode_out.in.json    "name": "Pokémon — ✓", "category": "日本語"
unicode_out.out.json   "name": "Pok\u00e9mon \u2014 \u2713", "category": "\u65e5\u672c\u8a9e"
```

`serde_json` emits **raw UTF-8** by default. So a straightforward port produces
different bytes for the same data.

**Decision:** match Python's escaping. Both forms are valid JSON and either
reads back identically — but matching means a file written by Rust and one
written by Python are *diff-identical*, which makes the round-trip test a
strict equality check instead of a semantic one, and keeps `git diff` quiet for
users who downgrade. Implement with a custom `serde_json` formatter or by
post-escaping; a test asserts the fixture bytes exactly.

### F-B. An explicit `null` timestamp survives as `None`, and later crashes

`Game.from_dict` (models.py:68-87) starts with:

```python
value = values.get(key)
if value is None:
    continue          # <-- leaves the key as None
```

For a missing key this is correct (the dataclass default applies). For a key
present with an explicit `null`, the `continue` skips normalization *without
popping the key*, so `values["added"] = None` is passed to the constructor and
the field becomes `None` rather than a float.

Confirmed by the oracle:

```
-- null_ts
   added_value   = None
   last_played   = None
```

Verified against the real module:

```
added       = None
last_played = None
format_last_played(None) = 'Never played'
all(sort='name')   -> OK ['NullAdded']
all(sort='recent') -> TypeError: bad operand type for unary -: 'NoneType'
all(sort='added')  -> TypeError: bad operand type for unary -: 'NoneType'
```

Two consequences:

1. **`sort="recent"` and `sort="added"` raise `TypeError`** on such a library —
   `sorted(..., key=lambda g: (-g.added, ...))` cannot negate `None`. `"name"`
   (the default) is safe, because its key never touches a timestamp.

   **This is reachable in the shipped app.** `Backend._get_games`
   (`bridge.py:324-330`) passes `sort=self.settings.sort_mode` into
   `Library.search`, so any user who has selected *Recently played* or *Date
   added* — a normal, persistent setting — gets a `TypeError` raised inside a
   Qt `Property` getter, which takes the library view down. Nothing about it is
   exotic: it only needs a `games.json` where one entry has `"added": null`,
   which a hand-edit, a partially-written file, or another tool can produce.

2. `format_last_played(None, ...)` returns `"Never played"` silently — `not
   None` is `True` — so the bug is invisible for `last_played` and only bites
   for `added`. The row builder at `bridge.py:311` is therefore not the failure
   point; the sort is.

**Decision:** this is a **bug in the Python app**, not behaviour to replicate.
The Rust port treats explicit `null` exactly like an invalid value: regenerate
`added` from the clock, set `last_played` to `0.0`. This is strictly more
robust and cannot lose user data (a `null` timestamp carries no information).
Recorded as DECISIONS **D-14**, and covered by a dedicated unit test.

**Worth reporting upstream** to the Python project independently of this
migration: a hand-edited or interrupted-write `games.json` containing
`"added": null` makes the app's library view throw.

### F-C. `int` and `float` are normalized to `float`

`"added": 42` → `42.0`; `"last_played": 7` → `7.0`. Rust must parse JSON numbers
into `f64` regardless of whether the source literal had a fractional part,
otherwise the round-trip writes `42` where Python writes `42.0`.

### F-D. Unknown keys are silently dropped, and that is load-bearing

`from_dict` filters to `{f.name for f in fields(cls)}`; `Settings.from_dict`
does the same. The oracle confirms `legacy_field` does not survive a
load→save cycle. Parity item **P-75** depends on this: Rust must use a
`#[serde(deny_unknown_fields)]`-free derive that *discards* extras, not one
that errors.

### F-F. Float→text rendering differs between Python and `serde_json`

Verified by running both. `serde_json` uses Ryu; Python uses David Gay/Grisu
shortest-repr with **C-style `printf("%g")` exponent rules**. They agree on
most values but not all — 2 of the 14 probe cases differ:

| f64 | Python `json.dumps` | `serde_json` |
|---|---|---|
| `1e-7` | `1e-07` | `1e-7` |
| `1e7` | `10000000.0` | `1e7` |
| `1e-10` | `1e-10` | `1e-10` |
| `1e23` | `1e+23` | `1e+23` |
| `1e16` | `1e+16` | `1e+16` |
| `0.1+0.2` | `0.30000000000000004` | `0.30000000000000004` |
| `1700000000.5` | `1700000000.5` | `1700000000.5` |
| `5e-324` | `5e-324` | `5e-324` |

Python pads the exponent to at least two digits (`1e-07`) and switches to
exponential notation only at ≥1e16, so whole values like `1e7` print as
`10000000.0` where Ryu says `1e7`.

**Impact.** Timestamps are the only floats in the format, and `time.time()`
values (~1.7e9) round-trip identically — so in practice this bites only on
hand-edited or synthetic values. It does, however, **break the strict
byte-equality test** that D-15 relies on, so the port cannot simply call
`serde_json::to_string` and compare.

**Decision (folds into D-15):** byte-equality is a **goal for realistic data,
not an invariant**, and the test is written to say so — the fixture
`floats.out.json` is compared against Python's output, and where a value
legitimately differs only in exponent spelling, the test asserts
**numeric equality after reparse** rather than byte equality. Do not contort
the writer to emulate `%g`. Recorded in DECISIONS D-16.

### F-G. `1e400` is fatal to a naive `serde_json` port

Python parses an out-of-`f64`-range literal to `inf`, which `from_dict` then
normalizes. `serde_json` **rejects the literal outright**
(`number out of range at line 1 column 5`).

The difference is not cosmetic. Under `Library.load`, Python's tolerance means
one game's `added` is regenerated and **the rest of the library survives**;
`serde_json` erroring means **the entire file is discarded** — the same
failure class D-06 was written to prevent, reached by a different input.

Verified Python behaviour:

| literal | result |
|---|---|
| `1e400` | parses to `inf` → `added` regenerated, `last_played` → `0.0`, sort OK |
| `-1e400` | parses to `-inf` → same normalization, sort OK |
| `123456789012345678901234567890` | arbitrary-precision `int` → `1.2345678901234568e+29` |
| `9223372036854775807` (i64::MAX) | → `9.223372036854776e+18` |
| `18446744073709551616` (u64::MAX+1) | → `1.8446744073709552e+19` |

**Decision:** the port must parse JSON numbers **leniently** — accept any
numeric literal, saturating to `±inf` when it exceeds `f64` range, then apply
the same normalization. `serde_json`'s own number handling is insufficient;
use a lenient parse path (e.g. `serde_json`'s arbitrary-precision or a
pre-pass that rewrites out-of-range literals) and prove it against
`fixtures/out_of_range/*`. This is a **hard requirement**, not a nicety: the
whole point of D-06 is that a file Python tolerates must not take the library
down. Recorded as DECISIONS D-16.

### F-E. Validation-failure fallbacks are fixed constants, not "keep previous"

`Settings.from_dict` resets to literals: an unknown `color_scheme` becomes
`"dark"`, not `"system"`; an unknown `view_mode` becomes `"grid"`; an unknown
`sort_mode` becomes `"name"`. Note the `color_scheme` fallback is the
**hardcoded `"dark"` string**, not the dataclass default — they happen to agree
today, but the Rust port must use the same literal so a future change to one
side cannot silently diverge.

---

## 2. Confirmed-correct behaviour worth pinning

| Behaviour | Oracle result |
|---|---|
| Corrupt file (`{not json`) | whole file rejected → empty library, no crash |
| Valid JSON that is not a list (`{"name":…}`) | rejected → empty library |
| Trailing garbage after valid JSON | `JSONDecodeError` → whole file rejected |
| `NaN` / `Infinity` in the file | **accepted** by Python; `added` regenerated, `last_played` → `0.0` |
| `1e400` / `-1e400` in the file | **accepted** (→ `±inf`) → normalized; see F-G |
| Integers beyond `i64`/`u64`, or beyond `f64` precision | accepted, converted to nearest `f64` |
| Non-dict list entries (`"str"`, `42`, `null`) | skipped individually, rest of file kept |
| Entry with empty `name` | skipped |
| Duplicate `id`s | last one wins (`{"id": t…, "name":"First"}`, then `"Second"` → `Second`) |
| `category: "   "` | stored verbatim as `"   "`, but `display_category` folds to `Uncategorized` |
| Category with padding (`"  roguelike  "`) | `display_category` is stripped → `"roguelike"`, and `categories()` lists the stripped form |
| Empty library saves as | `[]` (no newline) |

### Sorting (`Library.all`)

Input names, with `added`/`last_played`:

| sort | result |
|---|---|
| `name` | `Alpha, Alpha, beta, zeta` (case-insensitive, stable) |
| `recent` | `beta(900), zeta(50), Alpha(0.0), Alpha(0.0)` — never-played sink, ties by name then stable |
| `added` | `Alpha(400), Alpha(300), beta(200), zeta(100)` |

The tie-break is `(key, name.lower())` and Python's `sorted` is stable, so
equal-key equal-name entries keep their insertion order. Rust's `sort_by` is
also stable — but the comparator must include the name tie-break explicitly, or
`recent`/`added` will diverge.

### Search

`Library.search` matches the query against **name OR display_category**,
case-insensitively, after `.strip()`. Observable consequences:

- `search("puzzle")` → `Portal 2` (category match) **and** `Puzzle Quest`
  (name match). Searching a category name finds its games — parity item P-04.
- `search("  doom  ")` → `Doom` (query is stripped).
- `category="All"` is **not** a filter (explicitly special-cased).
- `category="Uncategorized"` matches games whose `display_category` folds to it.

### `categories()`

Sorted by `(name == "Uncategorized", name.lower())` — so `Uncategorized` always
sorts last, and the rest are case-insensitively alphabetical. Oracle:
`['Puzzle', 'roguelike', 'Shooter', 'Uncategorized']`.

### `format_last_played`

Boundaries are exact and the pluralization is irregular — `1 hour ago` but
`2 hours ago`, `Played yesterday` (no number), `1 month ago` / `2 months ago`.
Oracle cases are in `oracle.json` under `format_last_played.cases` and cover
every branch including the boundary pair 119s→"just now" / 120s→"2 min ago".

Two details easy to get wrong:

- `timestamp=0.0` (and any falsy value) → `"Never played"`, *not* "Played just
  now".
- A **future** timestamp clamps: `max(0.0, current - timestamp)` → `"Played just
  now"`. Parity item P-16.

---

## 3. How the Rust port consumes this

- **T-02/T-03** (Architecture) read `fixtures/*.in.json`, assert the parsed
  model matches `oracle.json`, and assert the re-written bytes equal
  `*.out.json`. This is a *strict* comparison, which is only possible because
  of F-A.
- The QA teammate adopts the same fixtures into `crates/core/tests/`.
- `scripts/verify.sh` regenerates the oracle and fails if the checked-in copy
  is stale, so the contract cannot silently drift from the Python app.

Because the fixtures are generated by running the real Python module, they stay
honest: if someone changes `models.py`, regenerating produces a diff, and the
port has to follow.

---

## 4. Findings from adversarial review of §1–3

The sections above were written by the port author and reviewed by the
devil's-advocate teammate. The review's method was to ask of each claim *"what
input would make this wrong?"* and then run it. Six findings, each verified by
executing the Python implementation. Four are bugs in the Python app the port
must not copy; two are traps in the *fixtures themselves* that made earlier
findings look stronger than they were.

### F-H. `serde_json`'s default f64 reader is not correctly rounded — 20% of real data

This is the most consequential finding, because **every earlier fixture passed
while a naive port would still have been wrong on roughly one timestamp in
five.**

`serde_json`'s default number reader computes `mantissa * 10^exp` in `f64`
arithmetic, which is off by one ULP for a substantial fraction of inputs. The
`float_roundtrip` feature switches to a correctly-rounded algorithm.

Measured on this machine, over 20 000 realistic `time.time()`-shaped values in
`[1.5e9, 2.0e9]`:

| configuration | values changed by parse→serialize |
|---|---|
| `serde_json = "1"` | **4070 / 20000 (20.35%)** |
| `serde_json = "1", features = ["float_roundtrip"]` | **0 / 20000** |

Why the original fixtures missed it: they used small round numbers (`100.0`,
`300.0`, `1700000000.5`) that are exactly representable or land on the correct
side of the 1-ULP error. The bug only shows on dense, arbitrary values — i.e.
on every timestamp the app actually writes.

The consequence is not a rounding curiosity. A `games.json` written by Rust
would differ in the last bits from one written by Python, so the round-trip
byte-equality test D-15 depends on would pass on fixtures and fail on user
data — the worst possible failure mode for a compatibility contract.

**Decision:** `serde_json` must be declared with `features = ["float_roundtrip"]`.
Pinned by section 7b (`floats_roundtrip`), which records 300 seeded realistic
timestamps *with their IEEE-754 bit patterns* and asserts the re-saved bytes are
byte-identical. Recorded as DECISIONS **D-19**.

### F-I. A wrong-typed `name` breaks every sort, including the default

`name: 123` is accepted by `from_dict` and stored as the `int` 123. Then every
sort mode calls `g.name.lower()`:

```
name_is_int   survives=True
  sort=name    AttributeError: 'int' object has no attribute 'lower'
  sort=recent  AttributeError: 'int' object has no attribute 'lower'
  sort=added   AttributeError: 'int' object has no attribute 'lower'
```

This is **worse than F-B** and by a wider margin than it first appears:

- F-B needs the user to have selected a non-default sort *and* a null timestamp.
- F-I needs neither. `sort="name"` is the default, so the library is broken on
  startup for anyone whose `games.json` has a non-string `name`, from any cause
  (hand edit, another tool, a partial write).

`name: ["x"]` fails identically. A sibling case is quieter: `name: null` is
*dropped entirely* (`entry_survives=False`, `count=0`) because the constructor's
`if game.name:` guard is falsy for `None` — so the game disappears from the
library instead of crashing it.

Other wrong-typed scalars are accepted without consequence, because the sort key
never touches them: `steam_appid: "42"` → stays the string `"42"`;
`steam_appid: 1e40` → `10000000000000000000000000000000000000000`; `category: 42`;
`exe_path: null` → `None`; `mangohud: "yes"`.

**Decision:** the port parses every scalar into its declared type — coercing or
defaulting rather than holding an arbitrary JSON value — so one malformed entry
cannot make the library unusable. Recorded as DECISIONS **D-18**, and pinned in
`rust_divergences` so a test author is not tempted to match the crash.

### F-J. `Library.load` tolerates invalid UTF-8; `Settings.load` does not

Measured, on a file that is otherwise valid JSON with one `0xff` byte:

| file | result |
|---|---|
| `games.json` | `UnicodeDecodeError` **caught** → empty library, app continues |
| `settings.json` | `UnicodeDecodeError` **propagates** → `'utf-8' codec can't decode byte 0xff in position 23` |

`Settings.load` catches `(json.JSONDecodeError, OSError)`. `UnicodeDecodeError`
is a subclass of `ValueError`, not of either — so it escapes. `Library.load`
catches `UnicodeDecodeError` explicitly, which is why the two disagree.

**This is a startup crash in the Python app**, not a library-view failure:
`Backend.__init__` (`bridge.py`) calls `Settings.load` during construction, so
the process dies before a window appears.

**Decision:** same treatment as F-B — the port tolerates it and falls back to
defaults. The asymmetry is a bug, and copying it would import a crash. Recorded
as DECISIONS **D-20**.

### F-K. A BOM destroys the library on the next save

`\xef\xbb\xbf[{"id": …}]` → `count=0`, `names=[]`, and the re-saved file is
`[]` (`out_sha256` matches the empty-library fixture). Python's `json.loads`
rejects a leading BOM, `Library.load` treats that as a corrupt file and returns
an empty library — and then the next `save()` writes that emptiness over the
user's data.

**Decision:** do **not** make this worse, but do not chase byte-parity with
data loss either. The port strips a leading BOM before parsing. This is a small,
strictly-safer divergence and is safe with respect to D-06 (a BOM'd file is one
Python cannot read either, so nothing that Python wrote is affected). Recorded
as DECISIONS **D-21**.

### F-L. Deep nesting: Python 50 000, `serde_json` 128

`deep_nesting` is a `junk` field of 1 000 nested arrays, and Python parses it
without complaint (`count=2`, `Deep` and `Sib` both survive — the unknown key is
then dropped per F-D). `serde_json` has a default recursion limit of 128 and
returns `recursion limit exceeded`.

Here the port can be **safer than Python at no cost**, because F-D means the
nested value is discarded anyway. The port does not need to materialise it:
skip over unknown values during parsing (or raise the limit) so a junk field
cannot reject a file that Python accepts. Recorded as DECISIONS **D-21**.

### F-M. `save()` replaces a symlinked `games.json` instead of writing through it

`Library.save` writes `path.with_suffix(".json.tmp")` and then `os.replace`s it
onto the target. Measured: after a save, `link_still_symlink_after_save=False`,
the link is gone, and the bytes are at the target path with the target still
holding `[]`.

So a user who symlinks `~/.var/app/…/games.json` to a synced directory has that
link silently replaced with a regular file on the first save — the sync stops
working. This is worth reporting upstream with F-B and F-I.

**Decision:** keep matching Python here (writes go through a temp file and a
replace, which is the correct atomic-write pattern and worth keeping), and pin
the behaviour so it is a known limitation rather than a surprise. Recorded in
REPORT.md's known-limitations section rather than as a divergence — the port
inherits it.

## 5. Corrections to §1–3 found by the same review

Two claims in the sections above were **weaker than they were written to be**,
because the evidence illustrating them was rendered as if it had already been
transformed. Both fixed; recorded here so the correction is visible rather than
silently edited.

- **F-A was self-defeating as illustrated.** The finding is that `ensure_ascii`
  escapes non-ASCII, but both sides of the before/after example were written as
  literal `é` and `日本語`, so the illustration *showed no escaping*. The
  fixture `unicode_out` is the real evidence — its `out.json` contains
  `\\u00e9` and `\\u65e5\\u672c\\u8a9e`. The prose now cites the fixture bytes
  rather than a hand-written example.
- **`floats_roundtrip` was initially 2 000 entries**, encoding 44 KB of input
  and 228 KB of output for a property that is fully determined after a few
  hundred. Reduced to 300, which still puts the probability of a buggy reader
  matching all of them below 1e-20 at the measured 20% error rate.

Neither correction changes a decision. They are recorded because the first
finding in this document rests on an example that did not demonstrate it, and a
contract that cites its own fixtures as evidence has to cite them accurately.
