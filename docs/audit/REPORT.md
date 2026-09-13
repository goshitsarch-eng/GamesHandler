# Audit report

**What this file is.** The audit brief's consolidated report: what was found,
what was fixed, and what is still open, in one table. `PLAN.md` is the schedule
— every finding with its owner, dependencies and verification method. This file
is the accounting. The six specialist documents hold the arguments.

**State at this writing.** Branch `audit-hardening`, commit `d855015`. The
numbers below are generated from `PLAN.md`'s rows and were recomputed rather
than carried over; where an earlier revision of a document disagreed with them,
the disagreement is recorded in `DECISIONS.md` D-59 rather than smoothed away.
The `Fixed` column counts `FIXED` only; the two `PARTIAL` rows are counted in
`Remaining`, because a half-fixed finding is not closed.

**This report tracks a moving tree, and it has already moved five times.** It
was first written at `6187ff2` with 28 fixed. Three ARCH findings were then
closed (`ARCH-05`, `ARCH-16`, `ARCH-18`), and `PKG-01`, `PKG-02`, `PKG-04` and
`PKG-05` followed. `PERF-01`, `PERF-02` and `PERF-03` are the most recent, in
`be31a7b` — and that commit's own documentation pass is the reason to distrust
this file's arithmetic on principle: `PLAN.md`'s summary tables read 28 fixed
where its rows said 32, and this file's `Found` column counted the two refuted
rows twice. Both are now computed from the rows by the same reading, and both
mistakes are recorded rather than quietly overwritten. A report whose figures
lag the rows it claims to summarise is the defect this audit exists to find, one
level up — and the fourth move is the proof of it: `88578db` marked `SEC-04`
`FIXED` and `48e036e` marked `SEC-09` `FIXED`, and neither regenerated `PLAN.md`'s
tables, so this file inherited a count that was three rows stale. That is now a
`plan-counts` stage in `scripts/verify.sh` rather than an intention.

**This is not a final report.** 61 of 130 defects are still open, most of them
because the work has not been done yet rather than because anything blocks it,
and the section *What remains, honestly* says which is which.

## Summary

| Category | Role | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|---|
| `BUG-xx` | Bugs, reliability, feature completeness | 46 | 26 | 2 | 20 |
| `ARCH-xx` | Architecture, code quality | 26 | 13 | 0 | 13 |
| `UX-xx` | libcosmic / COSMIC UX | 29 | 12 | 1 | 17 |
| `PERF-xx` | Performance, resource | 8 | 6 | 0 | 2 |
| `SEC-xx` | Security, robustness | 11 | 7 | 0 | 4 |
| `PKG-xx` | Packaging, platform, QA | 10 | 5 | 0 | 5 |
| **Total** | | **130** | **69** | **3** | **61** |

The `Found` column is defects; the three refuted rows are counted in `Not a defect`
and in no other column, which is why `BUGS.md` holds 48 id-bearing rows, two of
them refuted, and this table says 46 defects. Those two rows used to be counted in both, which is what made the two totals
disagree with each other and made a refuted row read as repaired work.

By severity:

| Severity | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|
| P0 | 5 | 5 | 0 | 0 |
| P1 | 24 | 24 | 0 | 0 |
| P2 | 51 | 36 | 0 | 15 |
| P3 | 50 | 4 | 0 | 46 |
| **Total** | **130** | **69** | **0** | **61** |

`Not a defect` is not a euphemism for "wontfix": both rows were **refuted by
measurement** and are kept, marked, with what refuted them (`BUG-11`,
`BUG-15`). Two of the rows counted as `Remaining` above are `PARTIAL` rather
than untouched — `BUG-12` and `BUG-47` — and each row names the half that is
still missing. `SEC-05` is a third, and it is `PARTIAL` for a measured reason
rather than an unfinished one: the host half of its suggested fix is not
writable, because every Proton-GE asset redirects off `github.com`.

## What this audit found that matters

Five findings changed how the application behaves for a user, and they are the
ones to read first. (The sentence said "four" while listing five for long enough
that it is worth naming: a count in prose beside a list is the same defect shape
as the tables this file has already been corrected for.):

* **`BUG-01` (P0).** A `games.json` the app could not parse was reported *and
  then treated as an empty library*, so the next save made the loss permanent.
  The port now refuses to write over a library it could not read.
* **`BUG-02` (P0).** A `games.json` that the Python app itself writes could not
  be read by the port at all — `serde_json` rejects a lone UTF-16 surrogate in
  a `\uXXXX` escape, which CPython's `json.loads` accepts. Every user of the
  old app had a file the new one would have refused.
* **`BUG-35` (P0).** Every install of a real Proton build was refused *after*
  the download and extraction, because the archive symlink check computed each
  link's parent against the wrong root. Measured: 1,818 of 2,068 symlinks in a
  real Proton tree rejected, where Python rejects 0.
* **`BUG-48` (P0).** The accessibility wrapper added for `UX-01`–`UX-03` reported
  a `Custom` id through `Widget::id`, and the wrapper and its own inner text
  input wore *the same* one — which iced's named-state branch keys its map by, so
  the wrapper consumed the single entry and left the input stateless while its
  tag still claimed state. The GUI died on its second frame with `Downcast on
  stateless state` and never drew one. It was introduced during this audit
  (`8f7269e`), caught by `scripts/verify.sh`'s own `smoke-test` stage, and fixed
  by reporting `None` from `Widget::id` exactly as the toolkit's `Named` widget
  does.
* **`SEC-01` (P1).** The Flatpak granted `--device=all` — the raw NVMe disk,
  `/dev/mem`, `/dev/kvm`, and every input device, inside the sandbox where
  third-party game binaries run. Narrowed to the three classes a launched game
  actually needs, on measurement rather than on the test the packaging document
  had deferred it to.

The single most common *shape* of defect found is worth naming, because it is
the one that survives review: **a check that passes without inspecting what it
claims.** It appears at every level in this repository — `BUG-16` (a stage that
re-validates the repository's own sources and reports success), `BUG-17` (a
placeholder guard that cannot see a renamed placeholder), `BUG-18` (a guard
whose haystack contains its own needle), `ARCH-05` (a module header that
declares the live install path dead), and, in this audit's own work, a
dependency check that read no advisories and reported a clean bill of health
(`SEC-10`) and a new guard whose needle matched its own definition (`D-61`).

## What remains, honestly

**Nothing is blocked by anything outside this repository.** No finding is
waiting on an upstream release, a missing tool, or a decision from anyone else.
So the brief's standard — `Remaining` should be zero for actionable P0/P1/P2 —
is **met for P0 and P1** and **not met for P2**, where the reason is that the
work is unfinished rather than impossible. Stating that plainly is the point of
this section.

Where the 61 rows are:

| Band | Count | What it is |
|---|---|---|
| P0 | 0 | **Closed.** All five fixed and each verified by restoring the pre-fix body and watching the new test fail. |
| P1 | 0 | **Closed.** `SEC-11` — the approved-publisher gate reading signer-chosen text as a certificate subject — was the last row here and is fixed; the recipe this report first sketched for it was measured wrong and corrected in the fix. `UX-01`–`UX-03` were the four upstream-widget accessibility gaps plus `ARCH-02`, all fixed, and the three widget rows each record the residue that is upstream's rather than this port's. |
| P2 | 15 | Actionable. The largest concentration is `UX` (7) and `ARCH` (6); the rest are 1 `BUG` and 1 `PKG`. |
| P3 | 46 | Edge cases, cosmetic divergences, and comments or tests that describe something the code does not do. The 19 open `BUG` rows are all here, along with 10 `UX`, 7 `ARCH`, 4 `SEC`, 4 `PKG` and 2 `PERF`. |

Three findings are worth flagging as *not* ordinary work, so no reader mistakes
them for a backlog item:

* **`UX-01`, `UX-02`, `UX-03` — the rows whose cause is outside this
  repository, and whose residue still is.** Dropdowns, togglers and text inputs
  had no keyboard focus ring and contributed no accessibility node, and the
  cause is in the *pinned* libcosmic widgets: `toggler` never had an `operate`
  at all, and the dropdown's is commented out at the pinned revision. All three
  are now fixed by the local-wrapper route — `crates/app/src/view/a11y.rs` wraps
  each control so the node exists and the focus ring is the app's, rather than
  carrying a patch against the pinned revision, and it reaches all 20 production
  call sites (3 togglers, 4 `input`, 2 `input_with_id`, 11 dropdowns). What no wrapper can fix is upstream's: a dropdown's popup still cannot
  be opened without a pointer, because the two operations that would open it are
  commented out in the pinned widget, and the rows say so rather than claiming a
  completeness they do not have. The verification half is worth keeping in view:
  the page tests assert the state a node *publishes* — a combo's `value`, a
  switch's `selected`, a toggler's activation message — because a suite that
  only counts nodes of the right kind is a check that passes without inspecting
  what it claims, which is the defect shape named two sections above. That shape
  was found in this fix's own first test, which walked the tab ring to a stop it
  never reached.
* **`SEC-10`** — five live RustSec advisories, none reachable from the shipped
  binary, all transitive pins owned by libcosmic. There is no fix available at
  this layer, and the brief forbids upgrading for version numbers alone. The
  actionable part is two version comparisons at the next libcosmic bump.
* **`SEC-03` was settled, and settling it found a second defect.** The row's
  unverified half was whether `osslsigncode` without a CA file accepts a
  self-signed certificate. The tool is absent from the host but present inside
  the Flatpak (2.14), so the question was answerable here after all: without
  `-CAfile` a self-signed certificate exits 1 and prints no success line, so the
  flag is load-bearing and the nine recipes that omit it are weaker but not
  broken. Re-running the probe then showed the `Subject:`-scoped predicate that
  replaced the substring test does **not** exclude the fields the signer chooses
  — `-n` is printed verbatim and may contain newlines, so a continuation line
  shaped like a `Subject:` is accepted as a certificate subject. That is
  `SEC-11`, a P1, tracked and fixed in this audit.

## How a finding is called fixed

The standard is applied to every row and is the same one the audit used on
itself: the pre-fix body is restored, the new test is watched to **fail**, the
fix is restored, and the test is watched to pass. Where a fix has no restorable
pre-state (`BUG-04`, `BUG-36`), it was verified end-to-end against the real
binary instead, and the row says so.

Two guards written during this audit **failed that standard on their first
version** and are recorded rather than quietly corrected: the dependency
matcher read `*.toml` when the advisories are `.md`, so it opened nothing and
reported 0 of 679 packages affected; and the `ARCH-05` guard's needle matched
the function's own `fn` definition, so it passed with the last production caller
deleted. Both were caught by controls — positive cases that must come back
flagging — and both now have those controls pinned in tests.

## The documents

| Document | What it holds |
|---|---|
| `BASELINE.md` | The pre-audit baseline, and the withdrawn fabricated observation |
| `FEATURES.md` | The feature matrix: what is advertised and whether it exists |
| `BUGS.md` | 48 ids on correctness, reliability and completeness — 46 defects, 2 refuted |
| `COSMIC-UX.md` | 30 findings on libcosmic and COSMIC conformance |
| `PERFORMANCE.md` | 8 findings on CPU, memory and frame cost |
| `SECURITY.md` | 11 findings on the sandbox, process launching and the dependency graph |
| `ARCHITECTURE.md` | 26 findings on structure, contracts and code quality |
| `PACKAGING.md` | 10 findings on the Flatpak, the desktop entry and the gate |
| `PLAN.md` | All 133 rows: owner, dependencies, verification, status — 130 defects and the 3 re
| `DECISIONS.md` | The seven decisions this audit made (`D-57`–`D-63`) |

Each document's scope section states whether it is still read-only. That
matters more than it sounds: `SECURITY.md`, `PERFORMANCE.md` and the four
others were written as read-only reconnaissance, and their fixes landed
afterwards, so a scope line reading "No source file was modified" is now
false about the tree a reader is holding.

## Attribution

Findings are attributed to the seven specialist roles the brief specifies (`S1`
bugs, `S2` UX, `S3` performance, `S4` security, `S5` architecture, `S6`
packaging, `S7` devil's advocate). `PLAN.md` names the owner per row. `S7` owns
no row and reviews every one before it is called done.
