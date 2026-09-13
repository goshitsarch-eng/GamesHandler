# Audit report

**What this file is.** The audit brief's consolidated report: what was found,
what was fixed, and what is still open, in one table. `PLAN.md` is the schedule
— every finding with its owner, dependencies and verification method. This file
is the accounting. The six specialist documents hold the arguments.

**State at this writing.** Branch `audit-hardening`, commit `6187ff2`. The
numbers below are generated from `PLAN.md`'s rows and were recomputed rather
than carried over; where an earlier revision of a document disagreed with them,
the disagreement is recorded in `DECISIONS.md` D-59 rather than smoothed away.

**This is not a final report.** 99 of 129 findings are still open, most of them
because the work has not been done yet rather than because anything blocks it,
and the section *What remains, honestly* says which is which. A report that
claimed otherwise would be the defect this audit exists to find.

## Summary

| Category | Role | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|---|
| `BUG-xx` | Bugs, reliability, feature completeness | 47 | 23 | 2 | 22 |
| `ARCH-xx` | Architecture, code quality | 25 | 4 | 0 | 21 |
| `UX-xx` | libcosmic / COSMIC UX | 30 | 1 | 0 | 29 |
| `PERF-xx` | Performance, resource | 8 | 0 | 0 | 8 |
| `SEC-xx` | Security, robustness | 10 | 0 | 0 | 10 |
| `PKG-xx` | Packaging, platform, QA | 9 | 0 | 0 | 9 |
| **Total** | | **129** | **28** | **2** | **99** |

By severity:

| Severity | Found | Fixed | Not a defect | Remaining |
|---|---|---|---|---|
| P0 | 4 | 4 | 0 | 0 |
| P1 | 23 | 12 | 0 | 11 |
| P2 | 53 | 12 | 2 | 39 |
| P3 | 49 | 0 | 0 | 49 |
| **Total** | **129** | **28** | **2** | **99** |

`Not a defect` is not a euphemism for "wontfix": both rows were **refuted by
measurement** and are kept, marked, with what refuted them (`BUG-11`,
`BUG-15`). Two of the rows counted as `Remaining` above are `PARTIAL` rather
than untouched — `BUG-12` and `BUG-47` — and each row names the half that is
still missing.

## What this audit found that matters

Four findings changed how the application behaves for a user, and they are the
ones to read first:

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
is **not met**, and the reason is that the work is unfinished, not that it is
impossible. Stating that plainly is the point of this section.

Where the 99 rows are:

| Band | Count | What it is |
|---|---|---|
| P0 | 0 | **Closed.** All four fixed and each verified by restoring the pre-fix body and watching the new test fail. |
| P1 | 11 | Real and actionable. Five are `UX` (keyboard and accessibility gaps in dropdowns, togglers and text inputs — these are upstream libcosmic widget gaps, and the work is a local widget or an upstream patch, both of which this repository can do). The rest are architecture, performance and packaging rows of the same kind as those already fixed. |
| P2 | 39 | Actionable. The largest concentration is `UX` (15) and `ARCH` (12). |
| P3 | 49 | Edge cases, cosmetic divergences, and comments or tests that describe something the code does not do. The 20 open `BUG` rows are all here. |

Three findings are worth flagging as *not* ordinary work, so no reader mistakes
them for a backlog item:

* **`UX-01`, `UX-02`, `UX-03` — the only rows whose cause is outside this
  repository.** Dropdowns, togglers and text inputs have no keyboard focus ring
  and contribute no accessibility node, and the cause is in the *pinned*
  libcosmic widgets: `toggler` has no `operate` at all. Closing these is a local
  widget or an upstream patch rather than an edit to this code, and the audit
  has argued both. Three agents are on them now.
* **`SEC-10`** — five live RustSec advisories, none reachable from the shipped
  binary, all transitive pins owned by libcosmic. There is no fix available at
  this layer, and the brief forbids upgrading for version numbers alone. The
  actionable part is two version comparisons at the next libcosmic bump.
* **`SEC-03` cannot be *settled* in this environment, and no re-run will change
  that.** The claim it rests on is whether a system `osslsigncode` without a CA
  file accepts a self-signed certificate, and `osslsigncode` is not installed
  here. The row says its conclusion is an unverified half rather than a finding,
  which is the honest form: a reader with the tool can settle it in one command.

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
| `BUGS.md` | 47 findings on correctness, reliability and completeness |
| `COSMIC-UX.md` | 30 findings on libcosmic and COSMIC conformance |
| `PERFORMANCE.md` | 8 findings on CPU, memory and frame cost |
| `SECURITY.md` | 10 findings on the sandbox, process launching and the dependency graph |
| `ARCHITECTURE.md` | 25 findings on structure, contracts and code quality |
| `PACKAGING.md` | 9 findings on the Flatpak, the desktop entry and the gate |
| `PLAN.md` | All 129 findings: owner, dependencies, verification, status |
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
