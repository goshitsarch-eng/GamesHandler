# Verification findings

**Purpose.** The running record of every defect the verification pass has found,
what fixed it, and how the fix was checked. `REPORT.md` (T-20) consolidates this;
this file exists so findings cannot be lost between here and there, and so T-19
has one list to walk.

**Status.** Live. Not the final report — `REPORT.md` does not exist yet, and the
pages (T-09…T-15) are not built.

**How to read a row.** *Found by* names the mechanism, not the author: what was
run, and what it showed. A finding whose fix has not been independently
reproduced says so.

---

## 1. The defect class

One shape accounts for almost every entry below: **a check that passes without
inspecting what it claims.** The assertion targets an *input*, or a hand-written
copy of the expected value, or nothing at all — never the mechanism.

A test that compares two constants you also wrote proves nothing. A test that
compares `initials("Half-Life 2")` against `NO_COVER_LABEL` without calling
either builder cannot notice the two being swapped.

Two variants were found later:

- **Inverted** — a check that can *never* pass (finding 5 below). Found by the
  same method, from the other direction.
- **Same observable, different mechanism** (D-40) — the assertion targets exactly
  the right observable, and simply reaches it by another route, so a guard that
  ran earlier satisfies it and the guard under test never executes. Mutation
  testing reports "caught". Only mutating the *later* guard exposes it.

A fourth shape is not a defect in a check at all but in its **reader**, and is
recorded because it recurred: a green check is inferred to cover a different
check's territory because they build the same target from the same source (D-39),
and a general claim about a build flag is made from one measured mechanism
(D-41).

## 2. Findings

| # | What was wrong | Found by | Fixed | Verified |
|---|---|---|---|---|
| 20 | `verify.sh` ignored a stage's exit code, so a SKIPPED stage could not fail a run — a toolchain-less run exited 0 without inspecting the Flatpak at all | Running the script with a stage forced to skip | `9c523d0`; exit 3 for unrequested skips | Re-run with a stage skipped: exits 3 |
| 21 | `crates/app/src/view/` held 45 green tests that **ran nowhere** (`mod view;` was never declared), while `cargo test` reported success | Counting tests in the module vs. tests executed | `mod view;` declared | `17 passed; 3 failed; 44 filtered out` observed during the B-08 mutation run |
| 22 | `DECLARES_PY` was a substring match, so a bare `: install …` shell no-op passed as a real install | Reading what the matcher matched against | Rejects both bypasses; real installs still accepted | Both bypasses rejected, real installs accepted |
| 23 | The icon install was covered by no stage at all | Listing stages against manifest installs | `FLATPAK_CONTENTS` lists all five installs; the stage fails for any manifest install not covered | Stage fails on an uncovered install |
| 24 | The manifest's install *destinations* were compared by nothing | Same audit | Fixed | Destination mismatch fails |
| 26 | Stage 4 diffed a **two**-generator directory against **one** generator's output, so it could never pass — the inverted form | Reading what each side of the diff was generated from | Fixed | Independently verified it still FAILS on a genuinely corrupted fixture, so the fix did not trade a permanent failure for a permanent pass |
| 27 | The gate checked only the wide feature configuration; the narrow one could disagree | `cargo tree -p gamehandler-core -e features -i serde_json` → not enabled narrow, enabled wide (3 occurrences) | Two invocations, both configurations | Dropping `dxgi` from `DXVK_DLL_OVERRIDES` is now caught **in the narrow scope**, where the old test provably could not see it (lead, reproduced) |
| 28 | `view/widgets.rs`: no test calls the builders, so 9 mutations survive, 2 of them behavioural | Mutation testing | Open (UX) | — |
| 29 | An early-exit summary omitted the stages that never ran — a report of what passed, presented as a report of what ran | Forcing an early exit | `verify.sh` names skipped stages and exits non-zero | Skipped stage is named; exit non-zero |
| 31 | `--list` printed "the library is empty" and exited **0** without reading the library; `verify.sh` matched `--list\|--launch\|--version` **zero times** | Running the binary against a 2-game library | `e9ecf5e` (Architecture) | CLI stage fails before the fix and passes after; a sorting mutant fails only `cli-list-non-empty` |
| 32 | An all-placeholder Flatpak passed `verify.sh` *and* `smoke-test.sh` completely: six page bodies were the app's entire user-visible surface, and nothing could tell scaffold from finished | Reading both scripts for any reference to the placeholder pages — zero | Two halves: `crates/app/tests/pending_pages.rs` (source pin) + `verify.sh:1146-1190` (artifact cross-check) | Open — before/after pair still owed (see §4) |
| 33 | `update()` had no coverage: all five real handlers, `NavigateTo` included, could be no-oped under a green suite | Deleting each arm and re-running | Open (UX) — #38/#39/#40 qualify the fix | — |
| 36 | Five DLL-override constants pinned only against themselves in both scopes; the oracle corpus could not catch it | Mutation: flipping a literal survived the whole suite | `e16dd5f` — parses `runners.py` call sites at test time | Lead reproduced in the **narrow** scope: clean passes, dropping `dxgi` fails |
| 37 | `canonicalise_object_keys`'s object-in-object path unpinned in both scopes | Mutation | `e16dd5f` — array path pinned; object path documented as *behaviourally equivalent* narrow, with the honest bound stated | Reproduced by Architecture; the honest bound is the right answer (D-33) |

## 3. The hazard: stale test binaries

**This is the highest-risk item for T-19's mutation work.** Concurrent editing
produces *impossible* failures.

`cargo test` reported two failures that vanished on a forced rebuild (`touch` +
retest): `view::cover` tests asserting `Icon` where the code provably returns
`Photo` for a 2-byte fixture, and a `parse_env_block` vector mismatch. In both
cases the fixture on disk was correct, the source was hash-stable across the run,
and the test passed in isolation and after `touch`.

The same family produced `/tmp/wire-test` contamination: a teammate ran
`CARGO_TARGET_DIR=<repo>/target` while building a scratch copy, so a foreign
tree's artifacts — with `/tmp` paths baked into `CARGO_MANIFEST_DIR` — were cached
in the repo and reused.

**Rule.** Before diagnosing a test failure as a defect, confirm the binary is
current: `touch` the source or `cargo clean -p <crate>` and re-run. **A failure
that contradicts the code is more likely a stale artifact than a bug.**

Two mechanisms, and they need different handling — this is D-33:

| | stale artifact | real defect |
|---|---|---|
| reproduces after `touch` + rebuild | no | **yes** |
| flips with build scope (`--workspace` vs `-p`) | no | **yes, if feature unification** |

Never answer "stale artifact" without testing "feature unification". A verdict
without its build scope is not evidence.

The related per-mechanism rule (D-41): **"does the target directory matter?" is
not a question with an answer.** It is cache hygiene for feature unification
(Cargo keys by `-C metadata`), and load-bearing for test-binary staleness in a
bin-only crate like `crates/app`. The answerable form names the mechanism:
*matters for what?*

## 4. Open at the time of writing

- **#32's before/after pair.** Both halves exist but neither is committed, and
  neither has been observed going red. A check that has never failed is not known
  to have teeth. The artifact half also has an open question: `build.sh:36` passes
  `--force-clean`, so the stage's advertised "stale build tree" failure may be
  unreachable on the path it runs.
- **#28** — nine mutation survivors in `view/widgets.rs`.
- **#33** — `update()` coverage, qualified by #38 (redundant length assertion),
  #39 (a new `Message` variant need not be added to `every_message`) and #40
  (`message_changes_state` observes only `State`, so a `Task`-only handler reads
  as a placeholder).
- **N-01** — the no-display panic, assigned to T-08; `smoke-test`'s only failing
  check.
- **`run_stage` → `finish_fail` exits by default**, so a failing early stage
  silently skips later ones *including the Flatpak build and smoke test*, while
  the summary lists only the stages that ran. Use `--keep-going` for a full
  picture. Raised with the advocate: defect or contract?
- **Minor:** the `view/cover.rs` fixture helper creates
  `/tmp/gh-cover-{name}-{pid}` per test per run and never removes them — 1245
  directories accumulated in one session.

## 5. Unmeasured numbers

Commit `09237d9` claims "165 core + 3 app integration tests pass". At that commit
the workspace reports 142 + 3 = 145. **Recorded as an unmeasured number in a
commit message — do not restate it.** The same failure produced D-39's own
correction, where a `--workspace` total was quoted from one binary's line.
