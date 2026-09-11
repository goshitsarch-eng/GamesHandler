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
| 32 | An all-placeholder Flatpak passed `verify.sh` *and* `smoke-test.sh` completely: six page bodies were the app's entire user-visible surface, and nothing could tell scaffold from finished | Reading both scripts for any reference to the placeholder pages — zero | **CLOSED (`50798f9` + UX's in-source test), in three complementary halves.** *Packaging:* `crates/app/tests/pending_pages.rs` — parses the dispatch arms out of `main.rs` and pins the set, independent of any table — plus `verify.sh`'s artifact cross-check (source count vs the shipped ELF). *UX:* `PENDING_PAGES`/`pending_task` (`main.rs:997`/`:1008`) and the rendering test, which asserts each page's body actually draws the placeholder. **The third half was a tautology until UX fixed it** — it compared `pending_task`'s output against `PENDING_PAGES`, which is the list `pending_task` reads (finding #43). | Lead reproduced both directions. Porting Library's arm while leaving it pinned FAILS UX's test with `Library is listed in PENDING_PAGES as T-09 but its body does not draw the placeholder; drawn: ["Library"]`. Packaging captured the artifact pair: **every byte-identity check passes while the stage fails**, which is the point — no installed file is `main.rs`, so nothing else in the stage could have shown it. |
| 33 | `update()` had no coverage: all five real handlers, `NavigateTo` included, could be no-oped under a green suite | Deleting each arm and re-running | Open (UX) — #38/#39/#40 qualify the fix | — |
| 36 | Five DLL-override constants pinned only against themselves in both scopes; the oracle corpus could not catch it | Mutation: flipping a literal survived the whole suite | `e16dd5f` — parses `runners.py` call sites at test time | Lead reproduced in the **narrow** scope: clean passes, dropping `dxgi` fails |
| 37 | `canonicalise_object_keys`'s object-in-object path unpinned in both scopes | Mutation | `e16dd5f` — array path pinned; object path documented as *behaviourally equivalent* narrow, with the honest bound stated | Reproduced by Architecture; the honest bound is the right answer (D-33) |
| 41 | `verify.sh`: a run that *finishes* with stages in `STAGES` never reached exits **0**, while printing that nothing was verified about them. `summary` computed the list as a `local` and discarded it; the exit tail checked only `FAILED` and `SKIPPED_UNREQUESTED` | Advocate, reviewing the gate at `ed59974` | `50798f9` — `UNRUN` is a global, and the exit tail at `:1420` routes a non-empty list to exit 3, with an extra NOTE when `STOPPED` is empty (STAGES and the runner disagreeing, not an abort) | Lead reproduced on a stub harness: all stages stubbed → **rc=0**; last `run_stage` line deleted → **rc=3**, `did not run (the run finished without reaching them): flatpak-contents` + `an unrun stage is not a pass` |
| 43 | The source-side pending-page test was **a tautology**: it asserted `pending == PENDING_PAGES.to_vec()`, where `pending` was computed by `filter_map` over `pending_task` — and `pending_task` *reads* `PENDING_PAGES`. The same list against itself. **LINEAGE CORRECTED (see below): this was an in-flight, never-committed intermediate in `main.rs`, not a defect in shipped code.** `git log --all -S 'PENDING_PAGES.to_vec()'` returns **empty** — the string exists in no commit. It is kept as a finding because observing it is what caused the redesign, and because it is the cleanest example of the class; it must not be read as a bug that reached the tree. **The file it was observed in is not the file it would be attributed to today** | Packaging, building the complementary half and finding it green while the page was ported | UX refactored **before landing** (`PENDING_PAGES` became a slice, the redundant `len()==6` went, and the test renders `view_body` and asserts the drawn strings) — the fix *is* `55a1e0f`'s test, which is a real check: a page landing while still listed fails it, and a `pending_page` arm absent from `PENDING_PAGES` fails it too | Lead reproduced: porting `Page::Library`'s arm while leaving it pinned FAILS with `Library is listed in PENDING_PAGES as T-09 but its body does not draw the placeholder; drawn: ["Library"]` |
| 42 | `verify.sh:517` asserted the `cli` stage "is RED until #31 lands. That is intended". The line was **born false** — `e9ecf5e` (#31's fix) is an ancestor of `398a7c0`, the commit that wrote it | `git log -S` + `git merge-base --is-ancestor` | `50798f9` — replaced, not deleted, because the danger was the *forward* instruction; the replacement says in as many words that **a red cli stage is a defect** | Read at `:530-538`; the replacement states the born-false history and the exception (a missing binary) explicitly |

| 46 | `widgets.rs`: the photo branch of `an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset` asserts on `leaves()`, which is a witness of the laid-out **box**, not of a **decode**. `framed()`'s `Fixed(188)×Fixed(218)` container sets `Limits` `min == max` (`iced/core/src/layout/limits.rs:60-66`), and `Image::layout` resolves `intrinsic.min(max).max(min)` (`iced/widget/src/image.rs:257-268`) — which returns `188×218` for *any* intrinsic, **including the `Size::ZERO` a failed decode produces**. Measured (three inputs, lead's own probe on a HEAD tree): a real 118-byte VP8L `.webp` → `classify=Photo`, `measure_image=Some(24×16)`, `leaves=188×218`; UX's 12-byte PNG-magic fixture → `classify=Photo`, `measure_image=None`, `leaves=188×218`; a nonexistent path → `classify=Plate`, `leaves=80.96×89.6`. So `leaves()` **does** separate `Plate` from `Photo` — the third row differs — but not a `Photo` that decoded from one that did not. **Stated this way deliberately: an overstated defect invites the wrong repair, and asserting on the `Plate` case would let #46 be "closed" by a check satisfied by the thing it cannot distinguish — the #43 shape.** The assertion's *claim* ("a photograph is cropped to the box rather than inset") is true; the evidence offered for it is satisfied by a broken photograph | Packaging, building T-22; **independently reproduced by the lead** with a separate probe | **CLOSED (`7e8f2b3`)** — the repair landed with T-22's `.webp` assertion and on `measure_image` rather than `leaves`, exactly as the finding required; `with_photo` was repointed at bytes that decode, so the photo branch's assertion is no longer satisfiable by undecodable input. The bound is kept in a doc comment on the test itself, with the three-row table, so a later reader cannot re-derive the weaker check | Reproduced by both parties independently, with the same numbers. **And the fail direction measured by the lead, end to end:** with `"animated-image"` removed from `crates/app/Cargo.toml`'s `libcosmic` features and the whole suite re-run, **exactly two tests fail** — `a_webp_cover_is_decoded` and `an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset`, both `left: None, right: Some(24x16)` at the same `assert_decodes` line — and **144 pass**. So the new check is a check, not a claim: it moves only with the decoder. The fixture was also verified independently of the test by decoding the 118 embedded bytes against the **system** libwebp (a different implementation from the Rust `image` crate's VP8L path): `24x16`, 216 distinct RGB triples over 384 pixels, four distinct quadrant corners. It is a picture, not a flat block |
| 47 | `verify.sh:1413-1415` says a `begin X; finish_ok` that never calls the stage, and a `run_stage` naming the wrong function, "both claim a result without producing one, and **both land here**" — the exit-3 `UNRUN` branch. **Both halves of that are wrong.** (a) `begin X; finish_ok` **exits 0** with the stage listed under `passed`: `begin` appends to `ATTEMPTED`, so the stage is never in `UNRUN` and the branch cannot see it. (b) A `run_stage` naming a nonexistent function never gets that far — `run_stage`'s own `declare -F` guard exits **2** before `begin` is reached, which is the *better* outcome and is not this branch | Advocate, reviewing the gate; **reproduced by the lead** on a stub harness with every stage stubbed green | Open (Packaging). `begin cli; finish_ok` → `EXIT=0`, `passed: … cli …`. Cheap fix is the comment; the class-removing fix is `finish_ok` refusing to report `ok` for a stage `run_stage` never ran | Reproduced. **And the ancestry measured, in the form that matters for the repair:** the mutant exits 0 at `50798f9^` *and* at `50798f9` — but for **different reasons**, which "pre-existing" alone would hide. `50798f9^` contains **0** occurrences of `UNRUN`, so it exits 0 because there is no unrun-stage check at all; `50798f9` contains **7**, so it exits 0 because the new check exists and `begin` **bypasses** it. The fix therefore did not introduce the gap, but it **newly claimed** to catch "a stage that claims a result without producing one" — a class its guard cannot see. That is the precise form of #47, and it is why the repair is to make `begin`-without-a-stage detectable rather than to add the branch again. Third case in this project of a comment that was *born* false rather than gone stale |
| 48 | `run_stage <name> <fn>` never compares `<fn>` to `<name>`. #47's fix makes `finish_ok` test `[ -z "$STAGE_RAN" ]` — **non-empty, not *which* function** — and `run_stage` sets `STAGE_RAN="$fn"` to whatever it was handed. So the legitimate-looking edit `run_stage cli stage_test` (a copy-paste slip across eleven near-identical consecutive lines) reports **`cli` passed** while the test suite ran in its place, and exits **0**. The `cli` stage would contribute nothing to the run and the summary would claim it as coverage. **Filed separately from #47 on purpose:** #47's subject is a stage that claims a result *having run nothing*; this is a stage that *ran a different function*. Folding it into #47 would make a closed item's description broader than its subject, which is the class this file exists to record | Advocate, reviewing #47's fix; **reproduced by the lead** | Open (Packaging). The **class-removing** shape is to carry the function name in `STAGES` itself (`name\|fn\|description`) and drop the second argument at all eleven call sites, so the pairing exists in exactly one place and *cannot* disagree — the same move as #39's single message list. A comparison inside `finish_ok` is the detection shape: cheaper, but it leaves the wrong state representable. Deriving the name by convention does **not** work — `oracle-freshness → stage_oracle`, `flatpak-build → stage_flatpak`, `smoke-test → stage_smoke` fail `stage_${name//-/_}` on three of eleven, so it needs a table either way | Lead, independently of the advocate, on a stub harness with every stage stubbed: `run_stage cli stage_cli` → `EXIT=0`; `run_stage cli stage_test` → **`EXIT=0`**, summary byte-identical, `passed: … cli …`. Third instance of the one-directional enumeration reported in the `ed59974` review (finding 4): the deleted last `run_stage` → exit 3 (#41), `begin X; finish_ok` → exit 2 (#47), the wrong function → still 0 |
| 49 | `crates/core/src/runners/launch.rs:1153` — `the_game_environment_block_reaches_the_dxvk_installer_and_not_only_the_child` intermittently fails with `Io(Os { code: 26, kind: ExecutableFileBusy, message: "Text file busy" })`. The test writes `<scratch>/wine` as an executable `/bin/sh` script (`write_script`, `launch.rs:753-760`: `fs::write` then `set_permissions(0o755)`) and `launch` execs it, so the failure is a **write-then-exec race on a freshly created file** — the classic ETXTBSY, which is also a very strong *hypothesis* and not a demonstrated mechanism, and is recorded as one. **The consequence is what makes it a finding rather than a nuisance:** `verify.sh`'s `test` stage is non-deterministic, so a green gate is a probability rather than a fact — and a gate that is green six runs in seven is worse than one that is reliably red, because its red runs get retried away | **Packaging, while validating `889f7a5`; NOT reproduced by the lead.** Packaging saw 1 failure in 7 runs of the whole `--lib` suite (358 passed, then 357/1, then 358) and captured the panic with `code: 26`. The lead's two attempts did **not** reproduce it: 1 in 12 runs of the full git-archive HEAD tree — **and that single count is not trustworthy, see below** — then **0 in 20** runs of the isolated test, warmed first so no run compiled | Open (Architecture). Not filed on the strength of the lead's count: the reproduction that counts is Packaging's, with the panic *observed*. The falsifiable next step is the one the lead's second attempt missed — the race needs the filesystem settled under load, so run the **whole `--lib` suite** (many files created concurrently) rather than the single test in isolation | **Packaging's ruled-out list, kept so nobody re-derives it:** all 62 `scratch("…")` labels in `crates/core` are unique crate-wide; the three `scratch` helpers use three distinct path prefixes (`gh-env-`, `gh-runners-`, `gh-oracle-`); and it reproduces in a `--lib` run, which is a single test binary with no concurrent sibling. **And a self-report against the lead's own instrument:** the 1-in-12 number came from a shell harness whose `grep` for the result line skips a run that produced no parsable line — so its denominator cannot tell "failed" from "did not finish", and it printed a total it could not substantiate per-run. That is the *same* defect class as everything else in this table, committed by the person filing it, and it is why the count above is attributed rather than asserted |

### 2.1 What #32's guards still do not catch

Stated by the author of the primary half, and worth keeping because it bounds
what the three halves above actually establish. **Both source-side tests classify
a page by whether its dispatch arm calls `pending_page`** — Packaging's by parsing
the arm, UX's by checking the drawn strings and the table. So a page "ported" by a
*second placeholder mechanism under another name* still reads as ported to both.
Only the artifact half would notice, and only if the new mechanism's text differs
from the string `verify.sh` greps for — which it would.

The division of labour is nonetheless real and each half covers what the other
cannot:

- **Parsed arm** (Packaging) catches a page whose arm still renders the
  placeholder while the bookkeeping says otherwise. A table-driven test cannot
  catch this, because the table is the thing that agrees with itself.
- **Rendered strings** (UX) catches a table entry claiming a page is done whose
  body does not actually draw real content.
- **Artifact cross-check** (`verify.sh`) catches a shipped binary that disagrees
  with the source about whether any placeholder exists at all — the only half that
  looks inside the artefact the tag ships.

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

- **#41 and #42 are closed** (`50798f9`, in the table above). They are named here
  only because this section listed them as open and the listing outlived the fix:
  leaving a closed item in the open list is the same failure as leaving a stale
  instruction in a comment — a reader acting on it looks for a defect that is not
  there. **Section 4 is walked at T-19 and everything in it must be re-verified
  before it is restated**, because this has now happened once.

- **#32 is closed**, and the two questions it carried are answered. The
  before/after pair was captured for both halves. The reachability question —
  `build.sh:36` passes `--force-clean`, so is the artifact branch reachable? —
  was answered correctly by Packaging: **not under a full run**, but reachable via
  `--skip-flatpak`/`--skip-smoke`, which skip the build and still reach the stage
  against whatever `build-flatpak/` an earlier run left. And porting a page is
  exactly the change nothing else in the stage can see, because
  `crates/app/src/main.rs` is in none of the five `FLATPAK_CONTENTS` entries — so
  the byte-identity checks stay green over a stub. The failure text now
  distinguishes the two cases.
- **#28** — nine mutation survivors in `view/widgets.rs`. **CLOSED (`ee18527`)**,
  by rendering rather than by a seam: the builders now destructure their spec
  (`let spec = card_cover_spec(); cover_box(game, spec)`), so nothing restates a
  number, and the tests read what is observable — widget `Id`s through a real
  `Tree`/`Operation` traversal, and the laid-out `layout::Node` — under the
  software renderer, so no display is needed. Two `CoverSpec` readers were
  removed as dead. 26 mutations, and the two behavioural survivors are caught.
  **Verified by the author, not yet independently reproduced by the lead** — the
  94-test run above is the lead's, and it passes, but that is not the same claim.

- **#33 — CLOSED for the variants that exist today; #39 keeps it open for the
  ones T-09…T-15 will add.** `253498d` moved the dispatcher to `Shell` (whose
  `update` a test *can* call, `App` holding a `Core` that only the framework can
  build) and pinned the written handlers in both directions. The four emptiable
  arms are now caught. `#38` (the redundant length assertion) and `#40`
  (`message_changes_state` observing only `State`, so a `Task`-only handler reads
  as a placeholder) were **closed with it**: `observe` now reports a `Debug`-diff
  of `State` **and** `task.units()`, and a mutation making `FetchCover` return a
  task is caught. That last one is #40's inverse — a *written* handler read as a
  placeholder — which is the shape #32 is about, one level down.

  One blind spot was measured rather than declared and is worth keeping: a
  `ToastId` a test can construct cannot name a live toast (`push` returns only
  the expiry `Task`; the slot map and queue are private), so `DismissToast` is
  written but unobservable. `a_test_cannot_observe_which_toast_was_dismissed`
  measures that and goes red if libcosmic ever grows an accessor — which is the
  right shape: the gap is asserted at the point it would close, not papered over.

- **#39 — RE-MEASURED, and the claim of closure is false.** The doc comment on
  `every_message` (`main.rs:1858-1865`) says the length pin means "once the enum
  compiles again that test is red until the new variant is also driven here".
  It is not. Probe, run by the lead against the committed tree: add
  `Message::LeadProbe(String)` to the enum, an arm to `variant_name` and a real
  handler arm to `Shell::update` (both compile-forced — no wildcard in either),
  and deliberately **not** to `every_message`. Result: **94 passed, 0 failed.**
  Nothing anywhere noticed. The pin is `assert_eq!(every_message().len(), 49)`
  and the hand-written list has 49 entries, so a *missing* entry keeps the count
  correct; the pin only catches the other direction (someone appends to the list
  and forgets to bump the number). So a new variant is driven by no test, and
  `only_the_written_handlers_change_anything` cannot see it either — it iterates
  `every_message`, so the variant is outside its universe.

  **This is the one to fix before the pages land, not after.** T-09…T-15 add
  variants; each one added without an `every_message` entry is a handler covered
  by nothing, and the suite stays green while it is. The class-removing fix is a
  single source of truth — declare the enum through a macro that also emits
  `variant_name` and the sample list, so adding a variant is one edit and drift
  is unrepresentable. The cheaper interim is to keep the list and pin its length
  against a constant that is itself compile-forced, which buys a reminder rather
  than a guarantee. Filed to UX; **not yet decided.**
- **#46 is CLOSED** (`7e8f2b3`) — the photo assertion now reads `measure_image`,
  which is the one observable that moves when a decode fails, and `with_photo`
  no longer feeds the branch bytes that cannot decode. The fail direction was
  measured by the lead rather than taken on report: strip `"animated-image"` and
  **exactly two** tests go red, both at the same `assert_decodes` line, with 144
  still green. Kept in this section rather than only in the table because it was
  listed as open here and a closed item left in an open list is the failure this
  section's first bullet is about.
- **#47 is CLOSED** (`e82c153`) — `finish_ok` now refuses to print `ok` unless
  `run_stage` ran a function for that stage, exiting 2. Packaging's reasoning for
  preferring this over widening the exit tail is the part worth keeping, because
  it rules the alternative out rather than merely declining it: `begin` appends to
  `ATTEMPTED` **before** anything runs, so a stage that announced itself and then
  claimed `ok` without running is *in* `ATTEMPTED`, and no widening of an
  `STAGES`-minus-`ATTEMPTED` computation can find it. The exit tail is
  structurally the wrong place for this family. A `PAST` set filled by the
  `finish_*` functions was rejected on the same grounds — it would record "a
  finish was called", not "the stage ran", and would pass
  `begin cli; stage_test; finish_ok` all the same.
- **#48 is CLOSED** (`889f7a5`, follow-up `030ad18`) — Packaging took the
  removal shape: `STAGES` is `name|function|description`, `run_stage` takes only
  the name and reads the function from the entry, and all eleven call sites are
  one argument. Verified by the lead by *running* it rather than reading it, on a
  stub harness where each stage function echoes its own name so the log records
  which one ran: baseline runs `stage_cli`; appending an old-style second
  argument (`run_stage cli stage_test`) still runs `stage_cli` — the table wins
  outright. The stated residual was measured rather than taken: an entry
  deliberately naming another function (`"cli|stage_test|…"`) exits 0 and reports
  `cli` passed, with the log showing `stage_test` ran. **That residual was then
  closed too**, on the lead's argument that a table cannot check its *mapping* but
  can check the **bijection**: `030ad18` asserts every `stage_*` function defined
  in the script is named by exactly one entry, and caught a real bug in its own
  first draft (the helper was named `stage_table_check`, matched its own
  convention, and failed the baseline). The remaining boundary is a *permutation*
  that preserves the bijection — two coordinated edits — which is measured and
  written into the comment rather than left implied.
- **#49 is open** — `launch.rs:1153`'s ETXTBSY flake, which makes the `test`
  stage non-deterministic. **Reported by Packaging with the panic observed; NOT
  reproduced by the lead** (0 in 20 isolated runs). Not assigned a rate by the
  lead, because the harness that produced the lead's first number could not
  distinguish "failed" from "produced no parsable line" — the defect class of
  this file, committed while writing it. Architecture's file; the next step is
  the whole `--lib` suite under load, not the isolated test.
- **N-01** — the no-display panic, assigned to T-08; `smoke-test`'s only failing
  check.
- **`run_stage` → `finish_fail` exits by default: contract, not defect.** Settled
  (D-42). The item as originally written was *substantially false* — it said a
  failing early stage "**silently** skips later ones … while the summary lists
  only the stages that ran". Measured on a stub harness with `clippy` forced to
  fail and every other stage stubbed green: the summary prints

  ```
  failed:  clippy
  did not run (an earlier stage failed): test cli oracle-freshness python-tests
      cargo-sources flatpak-build smoke-test desktop-metainfo flatpak-contents
    these are neither passes nor skips: nothing was verified about them
  ```

  and exits **1**. Every skipped stage is named, the reason is given, and the
  exit code is non-zero. **The same harness run against `50798f9^` produces the
  identical summary**, so this was true *before* the #41 fix as well as after:
  the description was not stale, it was wrong when written. What #41 actually
  repaired was the run that **finishes** with stages unreached (no `STOPPED`),
  which is the opposite path. Kept as a bullet rather than deleted, because the
  next reader of the header's `--keep-going` line deserves the measurement, and
  because "defect or contract?" is answered by running the case, not by taste.
- **Minor:** the `view/cover.rs` fixture helper creates
  `/tmp/gh-cover-{name}-{pid}` per test per run and never removes them — 1245
  directories accumulated in one session.
- **Python flake, verified unreproducible.** One live-repo run reported
  `FAILED (failures=1)`: `test_a_failing_launch_reports_the_runners_own_error`,
  `'cannot find' not found in 'the runner exited with status 53'`. Six
  consecutive `python3 -m unittest discover -t . -q` runs then returned
  `OK (skipped=1)`, and the test passes alone. Not reproducible, so not a task —
  but the reason string varies with a runner's exit status, which is worth
  watching. **If it recurs, capture the run.** Python is the *reference* rather
  than the shipped code beyond this point, so a sporadic failure here affects
  the oracle's authority, not the product.

## 5. Unmeasured numbers

Commit `09237d9` claims "165 core + 3 app integration tests pass". At that commit
the workspace reports 142 + 3 = 145. **Recorded as an unmeasured number in a
commit message — do not restate it.** The same failure produced D-39's own
correction, where a `--workspace` total was quoted from one binary's line.
