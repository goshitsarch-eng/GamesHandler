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
`ensure_ascii=True` escapes every non-ASCII character:

```
"Pokémon — ✓"   →   "Pokémon — ✓"
"日本語"         →   "日本語"
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
