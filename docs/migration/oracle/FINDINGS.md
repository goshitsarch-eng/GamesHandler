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

Verified by running both against the pinned `serde_json` (1.0.151 — it renders
floats via `zmij`; there is **no `ryu` in `Cargo.lock` at all**, an attribution
an earlier version of this finding got wrong). Python uses shortest-repr with
**C-style `printf("%g")` exponent rules**. There are **two** divergence classes,
and the second was found only after the first had been written up.

**(a) Exponent padding.** Python pads the exponent to at least two digits.

| f64 | Python `json.dumps` | `serde_json` |
|---|---|---|
| `1e-7` | `1e-07` | **`1e-7`** |
| `1e-10` | `1e-10` | `1e-10` |
| `1e23` | `1e+23` | `1e+23` |
| `1e16` | `1e+16` | `1e+16` |
| `0.1+0.2` | `0.30000000000000004` | `0.30000000000000004` |
| `1700000000.5` | `1700000000.5` | `1700000000.5` |
| `5e-324` | `5e-324` | `5e-324` |
| `1e7` | `10000000.0` | `10000000.0` |

**(b) Notation switch.** `%g` selects exponent notation when the decimal
exponent is below −4; `serde_json` does not switch until much lower. So the
whole band `1e-5 ≤ |x| < 1e-4` differs by **notation** — a different rendering
of the number, not a respelling of the same digits:

| f64 | Python `json.dumps` | `serde_json` |
|---|---|---|
| `1e-5` | `1e-05` | **`0.00001`** |
| `2.5e-5` | `2.5e-05` | **`0.000025`** |
| `1.2345e-5` | `1.2345e-05` | **`0.000012345`** |
| `9.9e-5` | `9.9e-05` | **`0.000099`** |
| `1e-4` | `0.0001` | `0.0001` |

(`1e-6` is **not** in this table: at `exp = -6` both sides already write exponent
form, so the difference there is zero-padding — class (a), not (b). It appeared
here in an earlier revision, which is how a notation claim came to be
illustrated by a padding case.)

**The complete divergence set — both bands, and nothing else.** The tables above
mix diverging rows with agreeing ones, which leaves the actual rule implicit. It
was measured directly on the pinned writer, and it is exactly two bands:

| Band on `|x|` | Class | Python | Rust | Example |
|---|---|---|---|---|---|
| `[1e-5, 1e-4)` | (b) notation | exponent | decimal | `1e-05` vs `0.00001` |
| `[1e-9, 1e-5)` | (a) padding | `e-0N` | `e-N` | `1e-07` vs `1e-7` |
| everything else | — | identical | identical | `1e-4`, `1e-10`, `1e14`, `1e16`, `1e23` |

Both lower edges are exact and both were probed: at `1e-10` the two agree
(`exp = -10` needs no padding), and at `1e-4` they agree (both decimal). The
band is closed below and open above at each boundary — `9.9e-6` diverges,
`1e-10` does not; `9.9e-5` diverges, `1e-4` does not. There is no third band
anywhere in the `f64` range: positive exponents are single-digit only for
`|x| < 1e10`, where neither side uses exponent form at all.

The practical upshot is the load-bearing part: **both bands are confined to
`|x| < 1e-4`, and both are numerically lossless** — `0.00001` and `1e-05`
reparse to the same `f64`. Every timestamp (`~1.7e9`) is far outside the bands,
so no realistic game library can reach either one.

**How (b) was missed, recorded because it is the defect §5 exists to correct.**
The original probe set was 14 hand-picked values and **contained no value in the
`1e-5` band**, so the finding generalised "exactly one differs" from a sample
that could not have shown otherwise. The claim was true of the 14 cases and
false as a statement about the contract. Every row above is from a direct probe
of the pinned version (`serde_json = "=1.0.151"`, `float_roundtrip`).

**Correction (found in review of this section).** This table previously claimed
`1e7` printed as `1e7` in Rust — that is false for the pinned version, where
both sides write `10000000.0`. The same wrong claim had been copied into
`rust_divergences` in `oracle.json`, where it was worse than a prose error — it
told a test author to expect a spelling the port never produces. Both are fixed;
this was the third instance of the defect §5 below exists to correct, and the
first to reach machine-readable data.

**Correction 2 (found when (b) was added).** That first correction then drew the
wrong conclusion from its own fix: it concluded "the real divergence is
**narrower** than stated: only the negative single-digit exponent is spelled
differently". It is not narrower — it is **wider**, in a direction the 14-case
probe could not reveal. The corrected test comment in `oracle_tests.rs` went
further and asserted "That is the whole of it", stating the narrower claim with
*more* confidence than the version it replaced. Both the comment and any
`rust_divergences` text derived from it need to say (a) **and** (b).

**Impact.** Timestamps are the only floats in the format, and `time.time()`
values (~1.7e9) round-trip identically — so in practice this bites only on
hand-edited or synthetic values. It does, however, **break the strict
byte-equality test** that D-15 relies on, so the port cannot simply call
`serde_json::to_string` and compare. Class (b) widens that: the port cannot
byte-round-trip a file holding a value in `[1e-5, 1e-4)`, and unlike (a) the
difference is a change of notation rather than of padding.

**Decision (folds into D-15):** byte-equality is a **goal for realistic data,
not an invariant**, and the test is written to say so — the fixture
`floats.out.json` is compared against Python's output, and where a value
legitimately differs in exponent spelling *or* notation, the test asserts
**numeric equality after reparse** rather than byte equality. Do not contort
the writer to emulate `%g`. Recorded in DECISIONS D-16.

**Test consequence.** The predicate currently in `oracle_tests.rs:868` —

```rust
assert!(ours_value.contains("e-") && !ours_value.contains("e-0"), …)
```

— encodes class (a) only: it demands the Rust side be in exponent form and not
zero-padded. For `0.00001` that is **false**, so the guard rejects a divergence
the writer legitimately produces. It must become "this line is a float field and
the two sides are numerically equal after reparse", and the fixture should gain
an `exp_notation_boundary` case so the class is covered rather than latent.
Assigned to the Architecture owner.

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
- `scripts/verify.sh` **will** regenerate the oracle and fail if the checked-in
  copy is stale, so the contract cannot silently drift from the Python app.
  (**Pending** — this is PLAN task T-17; until it lands, regeneration is a
  manual step. Written in the future tense here because the sentence previously
  read as if the script already existed.)

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
`steam_appid: 10**40` → the 41-digit integer `10000000000000000000000000000000000000000`
(the fixture `wrongtype_appid_huge.in.json` contains that integer *literal*;
`1e40` would have been a float and Python would have stored `1e+40`, so an
earlier version of this sentence named an input the fixture does not contain);
`category: 42`; `exe_path: null` → `None`; `mangohud: "yes"`.

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
onto the target. Measured (`oracle.json` → `symlink_save`):
`link_still_symlink_after_save = false`; the real path now holds the full saved
document (`link_bytes`) while the **former target is left untouched at `[]`**
(`target_bytes`). So the write replaces the link rather than following it, and
the synced file the link pointed at stops being updated.

So a user who symlinks `~/.var/app/…/games.json` to a synced directory has that
link silently replaced with a regular file on the first save — the sync stops
working. This is worth reporting upstream with F-B and F-I.

**Decision:** keep matching Python here (writes go through a temp file and a
replace, which is the correct atomic-write pattern and worth keeping), and pin
the behaviour so it is a known limitation rather than a surprise. Recorded in
REPORT.md's known-limitations section rather than as a divergence — the port
inherits it.

### F-N. A cover's extension does not describe its contents

Found while scoping **R-11** (the webp decoder question); recorded here because
it is a property of the *files the Python app writes*, and it changes what the
port has to do.

The `.jpg` / `.png` / `.ico` suffix on a stored cover is a naming convention, not
the format of the bytes:

- `save_cover_from_urls` (`covers.py:283-292`) loops the candidate URLs and
  writes every one of them to a **hardcoded** destination:
  `config.covers_dir() / f"{game_id}.jpg"`. The suffix is fixed before the
  download, so it cannot reflect what arrived.
- The candidate list (`COVER_ASSETS`, `covers.py:35-43`) includes
  **`portrait.png`**, and `CDN_ROOTS` (covers.py:31-34) is the Steam CDN, which
  serves the literal format named in the path. So a game whose only available
  asset is `portrait.png` stores **PNG bytes in a file called `<id>.jpg`**.
- `_request` (`covers.py:218`) sends `Accept: */*`, so no content negotiation
  narrows the response either.
- `save_exe_icon` (`covers.py:312-322`) writes `<id>.ico` from the icon embedded
  in a Windows executable — a third format, and the `image` crate's `ico`
  feature is what decodes it.
- Only `copy_custom_cover` (`covers.py:296-307`) derives the suffix from the
  source, and only from `{".png", ".jpg", ".jpeg", ".webp"}`.

**Consequence for the port.** Anything that renders a cover must decide the
format by **sniffing the content**, never from the file extension. The combined
requirement is: JPEG, PNG and ICO bytes must all decode, and `.webp` bytes may
additionally appear inside a custom cover. iced decodes through
`ImageReader::open(path).with_guessed_format()`, which sniffs, so the port
satisfies this as long as it does not "optimise" the guess away using the
suffix.

**Consequence for R-11.** The webp question is about *decoder capability*, not
about file naming — a `.webp` name is not needed to have webp bytes present, and
a `.jpg` name does not imply JPEG. R-11's scope is nevertheless still correct:
webp can only reach disk through `copy_custom_cover`, because every other writer
emits JPEG, PNG or ICO bytes.

**Verified against the pinned iced, not assumed.** The sniffing claim above is
correct, and the citation is
`iced/graphics/src/image.rs:126-130` (in the libcosmic checkout at the pinned
rev):

```rust
let image = ::image::ImageReader::open(&path)
    .map_err(|e| image::Error::Inaccessible(Arc::new(e)))?
    .with_guessed_format()
    .map_err(|e| image::Error::Invalid(Arc::new(e)))?
    .decode()
```

`with_guessed_format` reads the magic bytes, so a `<id>.jpg` holding PNG or ICO
bytes decodes correctly. The port therefore gets F-N's requirement for free on
the `Handle::Path` route, provided it passes a path and does not pre-decode the
suffix itself.

**A parity deviation F-N did not predict, found in the same block.** Four lines
below the decode, iced reads the file a *second* time for its EXIF metadata and
applies the orientation transform:

```rust
let operation = std::fs::File::open(path)
    .ok()
    .map(std::io::BufReader::new)
    .and_then(|mut reader| Operation::from_exif(&mut reader).ok())
    .unwrap_or_else(Operation::empty);
let rgba = operation.perform(image).into_rgba8();
```

So a cover JPEG carrying an EXIF `Orientation` tag — reachable through
`copy_custom_cover` (`covers.py:296-307`), the one path that imports a user's
own file, e.g. a phone photo — is rendered **upright** by iced. Qt Quick's
`Image` element exposes no auto-transform property, so the Python app renders
the same file in its stored orientation. Two different pictures from one file.

This is a **deviation, not a regression** — the iced behaviour is the one a user
would call correct, and the priority order puts parity first only for
*behaviour worth keeping*, which this is not. It is recorded rather than
silenced because a parity checklist that does not mention it would later look
like a bug. The iced half is verified above; the Qt half is stated from the
element's documented API and **has not been run**, so it is the weaker of the
two claims. Owner: the UX implementer at T-14, who can settle it by loading one
rotated JPEG through the Python app beside the Rust one.

### F-P. An icon's layout is decided by the file's *name*, not its bytes

Found by the UX owner while implementing the cover component (`view/cover.rs`),
and verified here independently. This is a **defect in the reference
implementation**, so it belongs in the B-series: the port must not copy it.

`bridge.py:313` decides whether a cover is an icon from the suffix alone:

```python
"coverIsIcon": cover.lower().endswith(".ico"),
```

That flag is load-bearing — `CoverArt.qml` uses it for three separate visual
decisions (`:25`, `:55`, `:60`): whether to draw the gradient plate at all,
what inset margin the artwork gets, and whether `fillMode` is
`PreserveAspectFit` (icon, uncropped, on a plate) or `PreserveAspectCrop`
(photo, filling the tile edge to edge). So a wrong flag is not a cosmetic
mislabel; it changes the picture.

**The suffix is the wrong signal, and the app's own writers prove it.**
`copy_custom_cover` (`covers.py:300-301`) keeps a source suffix only from
`{".png", ".jpg", ".jpeg", ".webp"}` and writes **`.jpg` for anything else**.
The cover file chooser (`GameFormPage.qml:352`) offers exactly
`["Images (*.png *.jpg *.jpeg *.webp)"]` — and, unlike the neighbouring
executable chooser at `:337`, it has **no "All files (*)"** fallback. So an
`.ico` is not reachable through the chooser.

Reachability is therefore narrower than it first appears, and is recorded here
exactly rather than rounded up: an ICO can only land under a non-`.ico` name via
a `copy_custom_cover` call that bypasses the chooser — a script, a test, or a
restored profile. Within the shipped UI the misclassification is latent. The
port still does not copy it, because the fix is four bytes of magic number and
the content is the more reliable signal on every path, including the ones the UI
does not currently expose.

**Consequence for the port.** Decide icon-vs-photo by **content**, never by
suffix. `covers.py:305` (`save_exe_icon` → `<id>.ico`) writes real ICO bytes, so
the magic `00 00 01 00` is decisive in the direction that matters most, and
`view/cover.rs` already does this. On the reference side this is a hidden
divergence the Phase 3 pass cannot walk by hand-editing a file, since the chooser
cannot produce the input; it is verified by the Rust unit test instead.

### F-O. `failure()` can silently lose the runner's error text (a race)

Found by the UX teammate during T-18 while chasing an intermittent
`python-tests` failure, and reproduced independently by the lead. Recorded here
because it is a behaviour of the Python app that the port must **not** copy —
tracked as **B-07** in `PLAN.md`.

`LaunchedGame.failure()` (`runners.py:1358-1374`) waits for the child and then
reads the stderr buffer:

```python
code = self.process.wait(timeout=timeout)      # reaps the child
...
detail = _readable_error(self.errors.text())   # reads a buffer filled elsewhere
```

but that buffer is filled by a **separate daemon thread**, `_ErrorTail._drain`
(`:1322-1345`), started in `_ErrorTail.__init__`. `wait()` returning proves the
process was reaped; it proves nothing about the pipe having been drained. The
read can therefore beat the drainer, and `failure()` falls back to the generic
`"the runner exited with status {code}"`, discarding the Wine/Proton diagnostic
that is the entire reason the method exists.

**Measured, twice, independently:**

| Observer | Method | Rate |
|---|---|---|
| UX teammate | real `_ErrorTail`, under load from a concurrent `flatpak-builder` | 12 / 300 |
| Lead | real `_ErrorTail`, `Popen` child writing to stderr then exiting immediately | **90 / 300** |

The rate is load-dependent, which is the signature: this surfaced as a *flaky*
`python-tests` run, and `tests/test_runners.py:761` is the test that fails when
the race is lost. A test that fails one run in ten and passes on re-run is not
noise — it is this.

**Consequence for the port, and the trap in the obvious translation.** The
defect is the *ordering*, not the threading, so a Rust port that spawns a reader
task and then awaits the child inherits the identical unordered pairing. The
port must ensure the reader has completed before the buffer is read —
`tokio::process::Child::wait_with_output()` does this, because draining the
pipes is part of what it waits for; reading a collected buffer *after* `wait()`
without joining the reader does not. The output **format** is unchanged; only
the reliability is. A Rust test must therefore assert the error text is
present, and must **not** accept the fallback string as a valid outcome — the
latter is precisely how a test can be green while the race is live, which is why
the Python suite concealed this.

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
- **`floats_roundtrip` was reduced to 300 entries.** The committed pair is
  44 254 B in / 232 954 B out for 300 values, which is already large for a
  property that settles after a few hundred; the probability of a buggy reader
  matching all 300 is below 1e-20 at the measured 19.3% error rate. (An earlier
  version of this bullet gave those byte counts as the size of the *2 000-entry*
  revision it described. They are the 300-entry sizes; no 2 000-entry corpus
  exists in any commit. Corrected — the original figure was unverifiable, which
  is the defect this section exists to fix.)

Neither correction changes a decision. They are recorded because the first
finding in this document rests on an example that did not demonstrate it, and a
contract that cites its own fixtures as evidence has to cite them accurately.

- **F-F's "exactly one differs" was true of the sample and false of the
  contract.** The 14 probe cases contained **no value in the `1e-5` band**, so
  the claim could not have been falsified by them — and the corrected version
  then concluded the divergence was *narrower* than first stated, when in fact
  it was wider. Found in review and fixed above by adding class (b). This is the
  **fourth** instance of the defect class this section exists to correct, and
  the second where the fix introduced a fresh error of the same kind by
  generalising past its evidence. The pattern worth naming: *a correction that
  states a conclusion the corrected evidence cannot support is the same defect
  wearing the fix's clothes.*

- **D-19 cited a section name that is a fixture filename.** Corrected in
  DECISIONS.md; noted here only because this document is the navigation aid for
  the oracle, and a wrong pointer between the two documents is exactly what a
  reader would use to check the other.

The float contract is also, as of this review, the only place in this document
where the evidence was generated by a probe of **my own construction** rather
than by running `gen_oracle.py` against the real app. The probe answers "what
does `serde_json` do", which is the right question, but it is not the oracle and
does not have the oracle's protection against a hand-picked sample — which is
precisely how the `1e-5` band was missed. The durable fix is the
`exp_notation_boundary` case in `gen_oracle.py` so the class is covered by
generated rather than hand-picked evidence.
