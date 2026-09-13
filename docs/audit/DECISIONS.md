# Audit decision log

Decisions made while auditing this application and fixing what the audit found.
Same shape as [`../migration/DECISIONS.md`](../migration/DECISIONS.md): the
question, the options considered, the choice, why, and the consequence.

**Why the numbering continues rather than restarting at D-01.** The migration
log already owns `D-01`–`D-56`, and its decisions are still in force — the
workspace split, the injected `HttpClient`, the pin on libcosmic. Two registries
in one repository that both say "D-03" would make every bare `D-xx` citation in
a commit message or a source comment ambiguous, and the citation is the whole
point of a numbered decision. So the audit's decisions begin at **D-57** and
cross-reference the migration log by number where they extend it.

The migration log's resolution order was set by the brief. The audit brief sets
its own, and it is the one used here when two rows compete:
**security and data loss → correctness → feature completeness → a reliable
Flatpak → accessibility → COSMIC conventions → performance → maintainability →
cosmetic.** A decision below that looks like it traded polish for safety is
following that order deliberately.

---

## D-57. The audit lands on `audit-hardening`, cut from `main`, and every finding cites one commit

**Question.** The brief says to branch `audit-hardening`. From where, and what
do the findings point at?

**Options considered.**
1. Branch from the current `main` tip and raise findings against whatever was
   current as each one was found.
2. Branch from a fixed commit and raise every finding against that one commit.
3. Work on `main` directly, since the brief also asks for the work to end up
   there.

**Choice.** Option 2 — branch `audit-hardening`, and raise every finding against
`d56782d`, the `main` tip when reconnaissance began.

**Why.** An audit finding is a claim about a *specific tree*. `BUG-40`'s row
says a truncated icon offset is silently accepted; that is true or false of a
commit, not of a branch that is moving under it. Raising findings against the
live tip means two rows written a day apart describe two different trees and
the summary table adds them together anyway.

**Consequence.** `audit-hardening` was cut three commits *after* `d56782d`
(`93b6278`), because those three commits landed on `main` while reconnaissance
was running. They touch only the walk harness, so no finding is invalidated —
but the gap is recorded in `PLAN.md` rather than smoothed over, because a
citation that was exact when written and drifted afterwards is precisely the
failure mode this audit exists to find.

**Consequence for the brief's last instruction.** The brief also requires the
work to be "commited and pushed to github on main". Both are satisfied: the
branch carries the commits, and `main` receives them at the end. The two are not
in conflict — the branch is how the work is reviewable in the meantime, which is
what option 1 loses and option 3 gives up entirely.

---

## D-58. A withdrawn finding is kept and marked, never deleted — and it ships with the regression it should have caught

**Question.** `BUG-11` was raised, then refuted by measurement: the code it
accused is correct. What happens to the row?

**Options considered.**
1. Delete it. It was wrong; a wrong row is noise.
2. Keep it, marked `WITHDRAWN`, with what refuted it.
3. Keep it marked, *and* add the regression test that the finding's own premise
   was reaching for.

**Choice.** Option 3.

**Why.** Deleting it destroys the only record that someone read that code and
concluded it was broken. The next reader arrives at the same `Vec::dedup` after
a sort, has the same suspicion, and re-derives the whole refutation from
scratch — or, worse, "fixes" it.

Four findings were withdrawn in this audit, and *how* they were wrong is the
argument for keeping them:

| Withdrawn | Where | Why it was wrong |
|---|---|---|
| A status bar reading `5 games • Ready` | `BASELINE.md:177` | **Invented.** No such text exists in the source; the bar was never there. |
| `BUG-11`, case-variant categories | `BUGS.md:83` | An assumed translation. The sort's primary term *is* the fold, so `dedup` removes what Python's `set` removes. |
| "The Add Game form takes no keyboard focus" | `migration/REPORT.md:102`, `BASELINE.md:262` | The pixel count that "proved" it was confounded — an empty focused field also returns ~50 px of caret blink, so 0-of-1024000 did not mean what it was read to mean. |
| `F-ADD`, filed P0: "Add/Save closes the form without saving" | `migration/REPORT.md:31` | Three probes "excluded three ways" all tested *delivery*, and delivery was never broken, so they excluded nothing. The cause was layout — the action row sits below the fold and the walk had no scroll verb. |

Three of the four were not carelessness; they were **probes that did not test
what the finding claimed.** That is the same defect class as a test that passes
without inspecting what it asserts, one level up: an *investigation* that
returns a verdict about something other than the question. Only the first row is
an outright fabrication, and it is recorded in `BASELINE.md` in those words
because nothing downstream could have caught it — unlike a broken test, an
invented observation has no failing assertion to trip over.

Option 3 is the part that is not obvious. `BUG-11`'s premise was wrong, but the
symptom it described — case variants of a category failing to fold together —
is exactly what happens if the sort's primary term changes. So the row ships
with `case_variant_categories_fold_into_the_same_groups_as_python`, and the
withdrawal text records the mutation that makes it fail: changing the primary
sort term to `a.cmp(b)` produces 28 groups instead of 10, with `PUZZLE`,
`puzzle` and `Puzzle` in three separate groups. The finding was wrong; the
guard it wanted was real and is now in place.

**Consequence.** Each withdrawn row states what refuted it *and* what it left
behind. `BUGS.md`'s counts table excludes withdrawn rows from `Remaining` but
keeps them in the total, so the summary cannot be read as "we fixed everything
we found" when part of what was found was not real. The rule the `BASELINE.md`
fabrication produced is binding on every measurement in this audit: **text read
off an image is not evidence** — grep the literal string from the source, or use
a measurement that cannot be mistaken, such as a changed-pixel count with a
bounding box. Where a reading and the source disagree, the source wins and the
reading is withdrawn.

Every claim in this file was re-checked against the repository before it was
committed, which is how the four-row table above replaced an earlier sentence in
this very entry that gave the count as two.

---

## D-59. A status lives once, as a `Status:` tail on the specialist row; the plan and the report are derived from it

**Question.** `PLAN.md` has a status column. `BUGS.md` and the five other
specialist documents have a row per finding. Keeping both in step by hand means
updating two places per fix.

**Options considered.**
1. `PLAN.md` is the authority; the specialist rows stay as first written.
2. The specialist rows are the authority; `PLAN.md`'s column is regenerated.
3. Both are hand-maintained, with a check that they agree.

**Choice.** Option 2.

**Why.** The specialist row is where the evidence is, so a status written there
sits beside the thing that justifies it. A status written *only* in `PLAN.md`
is a claim with the evidence one file away, which is how a plan drifts from the
rows it summarises — a statement that stops being inspected, which is this
project's own named defect class.

**Consequence.** `PLAN.md`'s columns are derived from the specialist tails, and
`REPORT.md` is generated from the same source rather than written beside it.

**The convention was stated before it was implemented, and that is worth
recording.** Only `BUGS.md` ended up carrying tails — 26 of its 47 rows, being
23 `FIXED`, 1 `CLOSED` and 2 `PARTIAL`. The other five specialist documents carry
none, so for those families the plan's status column is the only record and there
is nothing to derive from. Both states were measured, not assumed:
`grep -c 'Status: [A-Z]*' docs/audit/*.md` returns 26 for `BUGS.md`, 1 for
`SECURITY.md` (inside `SEC-01`'s fix cell) and **0** for the other four.

The failure this produced is concrete. Regenerating `PLAN.md` from its own rows
gave **P2: 4 fixed, 47 remaining**, while `BUGS.md` held **13** fixed P2 rows:
the BUG rows' statuses had never been synced from the tails that existed, so
the plan said four when the specialist document said thirteen. The summary was
wrong in exactly the direction that makes an audit look unfinished, and nothing
would have caught it, because the two files were never compared.

Both are now correct and both were recomputed from the rows rather than edited
by hand. The remaining work — writing the five missing sets of tails — is the
first item of the documentation pass, and until it is done `PLAN.md` says so in
its own Status section rather than implying a derivation that is not happening.

---

## D-60. The Flatpak device grant is narrowed to what was measured, not to the test the project had deferred it to

**Question.** `build-aux/flatpak/com.goshapps.GameHandler.json` granted
`--device=all`. `docs/migration/packaging.md` said to keep it that way "until a
real controller hotplug test through a launched game justifies narrowing it
(PLAN.md Q-2)". Should the audit run that test first?

**Options considered.**
1. Keep `--device=all` and run the hotplug test the packaging document asked
   for before changing anything.
2. Narrow to the three device classes games actually need, on the documented
   behaviour of the grants themselves.
3. Leave it. The device is only exposed to a game the user chose to launch.

**Choice.** Option 2 — `--device=dri`, `--device=input`, `--device=usb`.

**Why.** The deferred test answers the wrong question. It would establish
whether controllers work under the narrow grant; it would not establish what
the wide grant exposes, and *that* is the security question. The second half is
already answered by `flatpak-metadata(5)`, which lists what each device class
grants — so the audit measured the sandbox directly instead of running the test
the document had scheduled.

Under `--device=all` the sandbox contained `/dev/mem`, `/dev/kvm`, the raw NVMe
device and its partitions, `/dev/vfio`, `/dev/vhost-net`, the hardware watchdog,
NVRAM and the raw serial ports — inside the sandbox where **third-party game
binaries run**. The sandbox uid is `nobody`, which matches host `nobody:nobody`
ownership on those nodes, so the access is by uid rather than by capability and
the grant is not theoretical.

The deferral's premise was also wrong. `flatpak-metadata(5)` documents that
`--device=input` is what exposes `/dev/input`. `--device=all` was therefore
never what made gamepads work, and the condition the deferral was waiting on
("until a real controller hotplug test justifies narrowing") could not have
come out the other way.

**Consequence.** Three narrow grants replace one wide one. `tests/test_packaging.py`
asserts both halves: the three classes are present, and `--device=all` is
**absent** — the positive subset check alone would be satisfied by a manifest
carrying both. The absence assertion was shown to be load-bearing by re-adding
`--device=all` and watching it fail. `packaging.md` §3 and the README carry the
before/after node list.

**A measurement that was thrown away.** The first access test ran
`head -c1 /dev/dri/card0` and reported "denied". A control `ls /dev/dri/` showed
only `card1` and `card2` exist — `card0` was absent, not denied. The test proved
nothing and was discarded rather than reported. It is recorded here because the
conclusion above rests on the *second* measurement, and a reader is entitled to
know the first one was confounded.

---

## D-61. A `core` module's claim about who calls it is a contract, and gets a test that reads the claim

**Question.** `crates/core/src/installers.rs`'s header said it "has no *caller*"
and that five named functions "have no call site outside their own tests". All
five have production callers. Fix the prose, or fix the prose *and* guard it?

**Options considered.**
1. Correct the prose. It is a comment.
2. Correct the prose and add a test that the named functions have call sites.
3. Correct the prose and add a test that a separately-written expectation list
   agrees with the prose.

**Choice.** Option 2, with the test reading **the header itself** — not a
second copy of the facts (option 3) and not nothing (option 1).

**Why.** Option 1 fails because a wrong "not landed yet" is not neutral. A
maintainer told the install path is unwired may re-wire it, delete it, or
decline to touch it, and `crates/app/src/view/installers.rs:46-50` records that
this exact thing has already cost this project time once.

Option 3 fails because a hand-maintained list beside the header can disagree
with the header, and a disagreement is invisible — the defect class this audit
keeps finding. Reading the header makes the header the thing under test, so the
claim and its check cannot drift apart.

**Consequence.** The header carries a table of `function → path:line`, and
`crates/core/tests/wiring_claims.rs` asserts a live call exists in production
source (test modules cut) for every row.

**The test was measured, and the measurement changed it.** The first version
required the named function to appear *on the cited line*. It failed on the
header it was written for: three of five citations had already drifted, because
the header had just been edited and its own table sits above the calls it cites.
Pinning the line would then fail on every unrelated insertion above it — and a
test that fails on correct code is one someone deletes, which restores the
defect. So: rows into other files are exact; rows into `installers.rs` are
checked for pointing inside the file and no further. That is a measured
consequence of where the table lives, not a tolerance, and it is written in the
test where a reader will hit it.

**And the first version passed while the defect was present.** Deleting the
production call to `wineserver_binary` — leaving its three test calls — left the
test **green**, because the needle `wineserver_binary(` matched the `fn`
definition. A check that passed without inspecting what it claimed, in the test
written to prevent exactly that. The needle now excludes definitions, and two
further tests pin the two helpers: `the_needle_does_not_match_a_definition` and
`the_test_module_cut_removes_test_only_calls`. Both were verified by restoring
the defective state and watching them fail.

---

## D-62. An injected boundary is audited where it is consumed, not where it is declared

**Question.** `crates/core` takes its HTTP client, its `LaunchEnv` and its
environment as injected traits (`D-26`, `D-27` in the migration log). An audit
looking for "what does this app do to the network / the filesystem / the
environment" could read those traits and stop.

**Options considered.**
1. Audit the trait definitions and their documented contract.
2. Audit every production call site, and treat a rule the trait documents but no
   caller enforces as unfixed.
3. Audit the concrete implementations only.

**Choice.** Option 2.

**Why.** A trait that *permits* something is not a finding; a call site that
*does* it is. The migration log's injection decisions buy testability and
portability, and in exchange the interesting behaviour — which URLs are fetched,
which binaries are spawned, which variables are read — moves to the call sites
and out of the module a reader would naturally open. Option 1 would have
audited the permission slip rather than the act.

This is also what made `ARCH-05` visible. The `installers.rs` header's claim was
about wiring, and wiring is a property of call sites; reading the module in
isolation could not have contradicted it.

**Consequence.** Findings in `SECURITY.md` and `PERFORMANCE.md` cite call sites,
and several rows exist only because a documented trait contract turned out to
have no enforcing caller. Conversely, a rule the trait genuinely cannot be made
to enforce — where the trait's signature permits states the caller must
validate — is recorded as a caller-side obligation rather than a defect in the
trait.

---

## D-63. No dependency is upgraded for its version number

**Question.** The audit found stale, duplicated and unused dependencies. The
brief also forbids "risky dependency upgrades merely to obtain newer version
numbers".

**Options considered.**
1. Upgrade everything that is behind, then re-run the suite.
2. Remove what is unused and duplicated; upgrade only where a fix depends on it
   or where an advisory requires it.
3. Change nothing, to avoid the risk entirely.

**Choice.** Option 2.

**Why.** A dependency bump is a behaviour change in someone else's code with no
finding attached to it, and in a port whose correctness rests on matching a
reference implementation byte for byte, "the tests still pass" is the weakest
possible evidence for one — `D-39` and `D-40` in the migration log are both
records of a green suite meaning less than it looked like it meant.

Option 3 is not a real option either: an *unused* dependency is pure attack
surface and pure build time, and removing it is not an upgrade.

**Consequence.** Removals and deduplication are findings with rows. A version
bump is a finding only where the audit can name what it fixes; a newer version
with no such reason is declined and the declination is written down, so the next
audit does not re-open it as an oversight.

The review was run, and its result is `SEC-10`. Summarised here because the
*shape* of the answer is the decision's justification: **no unused and no
duplicated direct dependency**, so options 2 and 3 above did not in the end
differ by much — but five live RustSec advisories were found in the locked
graph, and none of them is fixed by us, because all five are transitive pins
owned by libcosmic/iced. The actionable part is two version comparisons
(`lru >= 0.18.2`, `memmap2 >= 0.9.11`) to make at the next libcosmic bump, and
`SEC-10` records the *reachability argument* — the shipped binary is the
software renderer and links no `wgpu` at all, and `xkbcommon` calls none of the
six affected `memmap2` functions — so that a change to either path reopens the
question deliberately instead of silently.

**How not to run this review.** The first version of the advisory check read
`*.toml` files out of `RustSec/advisory-db`. The advisories are `.md` with TOML
frontmatter, so it opened nothing, examined nothing, and printed "0 of 679
packages affected". It looked like a clean bill of health and was a program that
had never run. It was caught by adding positive controls — versions that are
*known* vulnerable — and watching them come back clean. Any check that reports
"nothing found" needs a control that proves it can find something; this is the
audit's own defect class and it caught the audit.

---

## What is *not* decided here

Decisions about the *port* stay in
[`../migration/DECISIONS.md`](../migration/DECISIONS.md): the workspace split
(`D-03`), the injected boundaries (`D-26`, `D-27`), the renderer choice (`D-11`)
and the rest are inherited by this audit and are not restated or revisited
above. Where an audit decision extends one of them, it cites the number and says
what it is extending.

Open questions that are *not* decisions yet are tracked as `Q-n` in `PLAN.md`.
An entry here is a choice that has been made and can be argued with; a `Q-n` is
a gap that cannot be closed from the repository alone.
