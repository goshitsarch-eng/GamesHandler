# Audit report

**What this file is.** The audit brief's consolidated report: what was found,
what was fixed, and what is still open, in one table. `PLAN.md` is the schedule
— every finding with its owner, dependencies and verification method. This file
is the accounting. The six specialist documents hold the arguments.

**State at this writing.** Branch `audit-hardening`. The numbers below are
generated from `PLAN.md`'s rows and were recomputed rather than carried over;
where an earlier revision of a document disagreed with them, the disagreement is
recorded in `DECISIONS.md` D-59 rather than smoothed away.

**This paragraph used to pin a commit hash, and the hash had gone stale.** It
read `d855015` while the branch stood 36 commits further on, so a reader
following it to "state at this writing" would have read a tree that did not
contain the findings the sentence introduced. A pin is the right instrument for
a document that answers *what did the audit find*; it is the wrong one for
tables that are regenerated whenever a row moves, and this file is the second
kind. The generator is named instead — `scripts/plan-counts.py`, whose `--check`
mode fails if any table line drifts from the rows — because it cannot go stale.
The pin's removal is recorded rather than done quietly, since the whole point of
the exercise is that numbers a reader cannot re-derive are numbers a reader
cannot check. Note what that check does **not** cover: the tables only. Every
prose count beside them in this file and in `PLAN.md` was re-derived by counting
the rows, and three were wrong when this paragraph was written (see the
`PARTIAL` paragraph below and `PLAN.md`'s note on the `BUGS.md` row).

The `Fixed` column counts `FIXED` only; the nine `PARTIAL` rows are counted in
`Remaining`, because a half-fixed finding is not closed.

**This report tracks a moving tree, and the count is recomputed on every move.**
It was first written at `6187ff2` with 28 fixed. Three ARCH findings were then
closed (`ARCH-05`, `ARCH-16`, `ARCH-18`), and `PKG-01`, `PKG-02`, `PKG-04` and
`PKG-05` followed. `PERF-01`, `PERF-02` and `PERF-03` were the next, in
`be31a7b` — and that commit's own documentation pass is the reason to distrust
this file's arithmetic on principle: `PLAN.md`'s summary tables read 28 fixed
where its rows said 32, and this file's `Found` column counted the two refuted
rows twice. Both are now computed from the rows by the same reading, and both
mistakes are recorded rather than quietly overwritten. A report whose figures
lag the rows it claims to summarise is the defect this audit exists to find, one
level up — and `88578db`/`48e036e` are the proof of it: they marked `SEC-04` and
`SEC-09` `FIXED` without regenerating `PLAN.md`'s tables, so this file inherited
a count that was three rows stale. That is now a `plan-counts` stage in
`scripts/verify.sh` rather than an intention.

**The sentence that opened this paragraph read "it has already moved five
times", while the list beneath it numbered its moves only as far as "the fourth
move".** One of the two was wrong and neither could be re-derived from the file,
so the count is gone rather than adjusted: an ordinal tally of how often a moving
target has moved is a present-tense count of a moving target, which is the shape
of error `PLAN.md` records against its own `BUGS.md` row. What is true at every
revision is what the paragraph now says.

**This is not a final report.** 23 of 130 defects are still open, most of them
because the work has not been done yet rather than because anything blocks it,
and the section *What remains, honestly* says which is which.

## Summary

| Category | Role | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|---|
| `BUG-xx` | Bugs, reliability, feature completeness | 46 | 40 | 2 | 6 |
| `ARCH-xx` | Architecture, code quality | 25 | 23 | 1 | 2 |
| `UX-xx` | libcosmic / COSMIC UX | 29 | 20 | 1 | 9 |
| `PERF-xx` | Performance, resource | 8 | 6 | 0 | 2 |
| `SEC-xx` | Security, robustness | 11 | 10 | 0 | 1 |
| `PKG-xx` | Packaging, platform, QA | 11 | 8 | 0 | 3 |
| **Total** | | **130** | **107** | **4** | **23** |

The `Found` column is defects; the four refuted rows are counted in `Not a defect`
and in no other column, which is why `BUGS.md` holds 48 id-bearing rows, two of
them refuted, and this table says 46 defects. Those two rows used to be counted in both, which is what made the two totals
disagree with each other and made a refuted row read as repaired work.

By severity:

| Severity | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|
| P0 | 5 | 5 | 0 | 0 |
| P1 | 24 | 24 | 0 | 0 |
| P2 | 52 | 46 | 0 | 6 |
| P3 | 49 | 32 | 0 | 17 |
| **Total** | **130** | **107** | **0** | **23** |

`Not a defect` is not a euphemism for "wontfix": all four rows were **refuted by
measurement** and are kept, marked, with what refuted them (`BUG-11`, `BUG-15`,
`UX-08`, `ARCH-24`). Nine of the rows counted as `Remaining` above are `PARTIAL` rather
than untouched — the nine `PARTIAL` rows, which `scripts/plan-counts.py` prints
by name on every run — and each names the half that is still missing: `ARCH-12`
(the four dispatcher extractions landed in `3b6ee5a`, and the grouping half did
not: re-measured, `Shell::update` is 965 lines over 59 arms, `State` carries 38
`pub` fields with 367 `.state.<field>` sites in `main.rs`, and the row's own
seven named fields account for 39 of them, so a field-grouped rewrite is a
200-site mechanical change for the eighth-ranked benefit of nine), `BUG-47` (the
ellipsis is commented but cannot be asserted — iced offers no downcast and
`Text::format` is private), `PKG-03` (the freshness check is split and always
runs; the generator half is unvendored because the only copy to hand has no
nameable upstream), `PKG-06` (keywords added, screenshots still absent),
`UX-06` (the scrim blocks the pointer, not Tab), `SEC-05` (the request is
now scheme- and origin-checked, but the redirect target is judged by nothing,
because every Proton-GE asset redirects off `github.com` and no host allowlist
is writable) and `UX-14` (the notice is published as an assertive alert and every
toast lasts 15 s rather than 5, but nothing delivers the node at the pinned rev —
`UserInterface::a11y_nodes` has no caller, the same upstream gap `UX-01`–`UX-03`
sit behind).

**This paragraph read "two… `BUG-12` and `BUG-47`" and named `SEC-05` as "a
third", which was wrong twice over.** `BUG-12` had been fixed outright since it
was written, and the count had grown to five. It was found by counting the
`PARTIAL` status cells in `PLAN.md` against this sentence rather than by
re-reading it — the same method failure this audit keeps recording, here in the
document that reports on the failures.

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

Where the 23 open rows are:

| Band | Count | What it is |
|---|---|---|
| P0 | 0 | **Closed.** All five fixed and each verified by restoring the pre-fix body and watching the new test fail. |
| P1 | 0 | **Closed.** `SEC-11` — the approved-publisher gate reading signer-chosen text as a certificate subject — was the last row here and is fixed; the recipe this report first sketched for it was measured wrong and corrected in the fix. `UX-01`–`UX-03` were the four upstream-widget accessibility gaps plus `ARCH-02`, all fixed, and the three widget rows each record the residue that is upstream's rather than this port's. |
| P2 | 6 | 2 `ARCH`, 2 `UX`, 1 `BUG`, 1 `PKG`. Actionable. |
| P3 | 17 | 7 `UX`, 5 `BUG`, 2 `PERF`, 2 `PKG`, 1 `SEC`. Edge cases, cosmetic divergences, and comments or tests that describe something the code does not do. |
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
| `COSMIC-UX.md` | 30 ids on libcosmic and COSMIC conformance — 29 defects, 1 refuted |
| `PERFORMANCE.md` | 8 findings on CPU, memory and frame cost |
| `SECURITY.md` | 11 findings on the sandbox, process launching and the dependency graph |
| `ARCHITECTURE.md` | 26 ids on structure, contracts and code quality — 25 defects, 1 refuted |
| `PACKAGING.md` | 11 findings on the Flatpak, the desktop entry and the gate |
| `PLAN.md` | All 134 rows: owner, dependencies, verification, status — 130 defects and the 4 refuted rows the tables above exclude |
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
