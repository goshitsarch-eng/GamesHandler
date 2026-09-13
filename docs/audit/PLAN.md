# GameHandler — consolidated audit plan

**What this file is.** The audit brief asks for one plan holding every finding the
specialist passes produced: an ID, a severity, evidence, an owner, its dependencies,
how the fix will be verified, and where it stands. The six specialist documents
(`BUGS.md`, `COSMIC-UX.md`, `PERFORMANCE.md`, `SECURITY.md`, `ARCHITECTURE.md`,
`PACKAGING.md`) hold the *arguments* — each finding with its `file:line` evidence,
its root cause and its impact. This file holds the *plan*. It does not restate the
evidence; it schedules it. Read the specialist row before acting on a plan row.

**Baseline.** Every finding was raised against `d56782d`, the tip of `main` when
the audit's reconnaissance began. That branch is where the fixes land.

`audit-hardening` was cut three commits later, at `93b6278`. Those three commits
touch only the walk harness — `scripts/parity-walk.sh` and the new files under
`scripts/walk/` — and no application code, so no finding here is invalidated by
them. Stated because this file cites a commit, and a citation that was exact
when written and quietly drifted is the failure this audit is about.

## Severity

`BUGS.md`'s scale, applied across all six documents so the merged table sorts:

| Level | Meaning |
|---|---|
| **P0** | Silent, irreversible data loss, or a failure the user cannot perceive at all |
| **P1** | A user-visible false statement (success reported for a failure), a primary flow unreachable, or a whole-suite check that cannot fail |
| **P2** | A real defect with a bounded or recoverable impact |
| **P3** | Edge case, cosmetic divergence, or a comment or test that describes something the code does not do |

## Status

| Status | Meaning |
|---|---|
| `FIXED` | Root cause understood, fix implemented and committed, and a test **shown to fail without it** — by restoring the pre-fix body and observing the failure, except where the fix has no restorable pre-state, which was verified end-to-end against the real binary instead |
| `PARTIAL` | Part of the finding is closed; the rest is recorded rather than papered over |
| `WITHDRAWN` | Refuted by measurement. The row stays, with what refuted it |
| `OPEN` | Not yet fixed |

The status here and the `Status:` tail on the specialist row are updated **in the
same commit** that changes them. A summary that drifts from the rows it summarises
is this project's own named defect class — a statement that stops being inspected
— so `REPORT.md` is generated from these rows rather than written beside them.

**Which document is the authority for which family, stated here because the row
above used to claim `BUGS.md` was the authority for every family.** It is the
authority for `BUG-xx` only. `ARCH-xx` rows are authoritative in
`ARCHITECTURE.md`, `UX-xx` in `COSMIC-UX.md`, `PERF-xx` in `PERFORMANCE.md`,
`SEC-xx` in `SECURITY.md` and `PKG-xx` in `PACKAGING.md`. The strongest reading
of that claim — that a `PKG` fix needs no tail because `BUGS.md` owns the
status — is the reading that produced the gap this pass had to repair: three
`PKG` rows went `FIXED` in this file while the specialist row they summarise
still described the defect, because the fix's commit touched `PACKAGING.md`
in a *later* commit than the one that changed the status. A rule about two
places staying in step is satisfied by a same-commit pair and violated by two
commits a minute apart, and nothing here could tell the difference.

**Where the tails actually live, measured rather than assumed.** Re-counted for
this revision by matching the ID cell of every table row in each specialist
document (`^\|\s*\*{0,2}\`?(BUG|ARCH|UX|PERF|SEC|PKG)-\d+`) and asking which of
those rows contain `Status:`:

| Document | Rows | `Status:` tails | Kinds |
|---|---|---|---|
| `BUGS.md` | 47 | 26 | 23 `FIXED`, 1 `CLOSED`, 2 `PARTIAL` |
| `ARCHITECTURE.md` | 25 | 7 | 7 `FIXED` |
| `COSMIC-UX.md` | 30 | 1 | 1 `FIXED` |
| `SECURITY.md` | 10 | 1 | 1 `FIXED` (inside `SEC-01`'s suggested-fix cell) |
| `PACKAGING.md` | 9 | 4 | 4 `FIXED` |
| `PERFORMANCE.md` | 8 | 3 | 3 `FIXED` |

`BUGS.md`'s 21 tail-less rows are the 20 open `P3` rows and `BUG-11`, whose
withdrawal is recorded in its ID cell rather than as a tail. The seven rows in
`ARCHITECTURE.md`, `COSMIC-UX.md`, `SECURITY.md`, `PACKAGING.md` and
`PERFORMANCE.md` that gained tails since the previous revision did so in the
commits that fixed them (`9e10566`, `be31a7b`, `372b86e`, `8abac0b`, `e2c6476`), so
the pair rule was kept where it applies. The paragraph above it, and the first
revision of this table, are kept in the record because both were instances of the
defect this file names: the table said `PACKAGING.md` carried 1 `FIXED` tail while
the document carried 4, and `PERFORMANCE.md` carried none while it carried 3 —
computed from the document as it stood several commits earlier and then read as
current. Re-counted here from the files on disk.

## Owners

The seven specialist roles the brief specifies. Each row names one. The devil's
advocate reviews every row before it is called done and owns no row.

| Owner | Role |
|---|---|
| **S1** | Bugs, reliability and feature completeness |
| **S2** | libcosmic / COSMIC UX |
| **S3** | Performance and resource |
| **S4** | Security and robustness |
| **S5** | Architecture and code quality |
| **S6** | Packaging, platform and QA |
| **S7** | Devil's advocate / red team (reviewer) |

## Summary

| Severity | Findings | Fixed | Withdrawn | Remaining |
|---|---|---|---|---|
| P0 | 4 | 4 | 0 | 0 |
| P1 | 23 | 18 | 0 | 5 |
| P2 | 51 | 17 | 0 | 34 |
| P3 | 49 | 0 | 0 | 49 |
| **Total** | **127** | **39** | **0** | **88** |

| Family | Document | Findings | Fixed | Withdrawn | Remaining |
|---|---|---|---|---|---|
| `ARCH-xx` | `ARCHITECTURE.md` | 25 | 7 | 0 | 18 |
| `BUG-xx` | `BUGS.md` | 45 | 23 | 0 | 22 |
| `PERF-xx` | `PERFORMANCE.md` | 8 | 3 | 0 | 5 |
| `PKG-xx` | `PACKAGING.md` | 9 | 4 | 0 | 5 |
| `SEC-xx` | `SECURITY.md` | 10 | 1 | 0 | 9 |
| `UX-xx` | `COSMIC-UX.md` | 30 | 1 | 0 | 29 |
| **Total** | | **127** | **39** | **0** | **88** |

These figures are computed from the rows below — by `### Pn` section for the
severity table and by ID prefix for the family table — rather than maintained
beside them. The two refuted rows (`BUG-11`, `BUG-15`) sit in their own `### Not a
defect` section and in no severity bucket, which is why the family table counts 45
`BUG-xx` rows against the 47 the document holds. `Remaining` counts `PARTIAL` as
remaining, because a half-fixed finding is not closed; `Fixed` therefore excludes
them and the two `PARTIAL` rows appear in `Remaining` until they are finished.

**The 129 in the previous revision of these tables was wrong**, and the way it was
wrong is worth one sentence because the fix is a shape change rather than a
recount. `BUG-11` and `BUG-15` were counted both as findings *and* as withdrawn,
so the two totals disagreed with each other by exactly the two rows — and `BUG-15`
additionally sat inside `P2` while its own tail said the defect did not exist. Two
refuted rows now sit outside the severity tree entirely and the arithmetic in both
tables is the same arithmetic. The row count for the whole audit is **129 findings
raised, of which 127 are defects**; `BUGS.md`'s own `## Counts` carries the same
distinction.

## Findings

One table, sorted by severity then ID rather than by family, because the work order
is the brief's priority order — security and data loss first — and that order reads
across families, not within them.

### P0 — 4

| ID | Finding | Owner | Deps | Verification | Status |
|---|---|---|---|---|---|
| `ARCH-01` | A library that fails to load is indistinguishable from an empty library, and the next save writes the empty state over the file | S5 | duplicate of `BUG-01` | As `BUG-01`; the same defect found independently from the architecture side. | FIXED with `BUG-01` |
| `BUG-01` | A games.json the app cannot parse is reported and treated as an empty library, and the next write makes the loss permanent | S1 | — | Live: a truncated `games.json` under `GAMEHANDLER_CONFIG_HOME` — `--list` exits 1 naming the file and leaves it byte-identical afterwards; plus the `LoadStatus` unit tests. | FIXED `26d56d3` |
| `BUG-02` | A games.json that the reference Python app writes cannot be read by the port at all: serde_json rejects a lone UTF-16 surrogate in a \uXXXX escape, CPython's json.loads accepts it | S1 | — | A fixture the real CPython 3.14.7 wrote with a lone `\uD83D` escape: pre-fix `serde_json` rejects it, post-fix parses it. | FIXED `67b022e` |
| `BUG-35` | Every runner install of a real Proton build is refused after the archive has been downloaded and extracted, because the symlink check computes each link's parent directory relative to the *current scan directory* instead of the candidate root. walk_links(root) recurses as walk_links(&path), so root is always the directory being scanned and path is always its direct child — path.strip_prefix(root) yields the bare file name and .parent() is always "". The escape test is then normpath("" + "/" + target), i.e. normpath(target), so any target whose first component is .. is judged to escape, however far inside the tree it lands. Wine builds are full of exactly those. *Sub-audit (agents a4a7776ae5fb4959f); claimed figure re-measured by me.* | S1 | — | Pre-fix: a real Proton tree is refused wholesale (measured: 1,818 of 2,068 symlinks rejected; Python refuses 0). Post-fix: the archive symlink tests plus that tree re-run. | FIXED `0622f93` |

### P1 — 23

| ID | Finding | Owner | Deps | Verification | Status |
|---|---|---|---|---|---|
| `ARCH-02` | The view layer's stated contract is false, and the code that violates it is the code the contract was written to describe | S5 | — | Either the two `view` `update` bodies move behind `Message` and `view/mod.rs:5-11` becomes true, or the paragraph is rewritten to describe the split the code has. A test walking `view/` for `&mut State` parameters enforces whichever is chosen. | OPEN |
| `ARCH-03` | uninstall reports success for a removal that did not happen | S5 | duplicate of `BUG-03` | As `BUG-03`. The remaining `!target.is_dir()` arm returns `Ok(())` for an absent target, which is the intended convergence rather than the defect. | FIXED with `BUG-03` |
| `ARCH-04` | One of the two "open something for the user" helpers discards its spawn failure and returns Ok(()); the other reports | S5 | duplicate of `BUG-04` | As `BUG-04`. | FIXED with `BUG-04` |
| `ARCH-05` | A core module's own header declares the live easy-install path dead | S5 | — | Rewrite `installers.rs:15-23` in the present tense naming `easy_install_worker`; verify every function the header calls callerless has a non-test call site. | FIXED `75fc739` (re-fixed after the line citations went stale) |
| `ARCH-06` | A comment states that the toolkit cannot do something it can, and the paragraph is the recorded reason a known defect is deferred | S5 | duplicate of the comment half of `BUG-47` | The paragraph now records that the claim was false and what the pinned iced actually exposes; `name_ellipsize` holds the strategy. | FIXED with `BUG-47` |
| `BUG-03` | "Removed {runner}" is toasted for a removal that never happened | S1 | — | `uninstall` on a symlinked build: pre-fix the link survives and `Ok(())` is returned, post-fix the link is unlinked. | FIXED `cf0434e` |
| `BUG-04` | open_prefix_folder discards the xdg-open spawn result, so the prefix-folder menu item reports success when nothing opened | S1 | — | Verified end-to-end against the real binary — the helper was new, so there is no restorable pre-state. | FIXED `318d558` |
| `BUG-05` | A game's additional_app helper that never started is reported as a launch that honoured the configuration | S1 | — | A helper argv naming a nonexistent program: pre-fix the launch proceeds, post-fix it fails naming the helper. | FIXED `7fc9ad9` |
| `BUG-06` | An empty GAMEHANDLER_DXVK_ROOT silently launches without DXVK where the reference refuses | S1 | — | `GAMEHANDLER_DXVK_ROOT=""`: pre-fix DXVK is silently skipped, post-fix the launch is refused. | FIXED `032108f` |
| `BUG-07` | The Library's primary create flow — "Add game" — has no visible control once the library holds at least one game | S2 | — | `the_toolbar_offers_add_game_with_a_non_empty_library` walks the built widget tree (`drawn_strings`), not the constants. | FIXED `7482043` |
| `BUG-08` | The Plugins page never re-detects the host | S1 | — | `arriving_at_the_plugins_page_re_detects_the_host`: pre-fix the host is unchanged, post-fix it is re-detected. | FIXED `17b463b` |
| `BUG-09` | The Runners page has no Refresh control, so after a failed fetch the error is on screen with nothing to press | S2 | — | `the_runners_page_offers_refresh_when_a_fetch_has_failed`, over the drawn strings of the error state. | FIXED `8e6181c` |
| `BUG-10` | stage_test has no floor on the number of tests run: an emptied or fully #[ignore]d workspace is green | S6 | — | `require_tests_ran` against a zero-test probe: pre-fix green, post-fix FAIL. Floors 900 (workspace) / 500 (core alone). | FIXED `ec10910` |
| `PERF-01` | Every game's cover is classified on every frame — two filesystem syscalls per game with a cover, per frame — regardless of whether the tile is visible | S3 | — | Classify once at load or change and cache on the game; verify the per-frame syscall count (`strace -c -e trace=statx,newfstatat`) on a 500-game fixture. | FIXED `be31a7b` |
| `PERF-02` | Cover images are decoded at full source resolution and retained, and the cache is bounded only by "what was drawn in the last frame" — which, with no virtualization (PERF-03), is every game in the library | S3 | `PERF-03` | Bound the cache by bytes and decode at the drawn size; verify RSS on a 500-cover fixture (baseline 753 MB / 333 covers, measured). | FIXED `be31a7b` |
| `PERF-03` | Nothing is virtualized: every game in the filtered library gets a built Element every frame | S3 | — | Virtualize the grid and row lists; verify the built-element count per frame against a 500-game fixture. | FIXED `be31a7b` |
| `PKG-01` | The only check that executes the built artefact can execute a stale, previously installed build instead | S6 | — | Resolve which artefact `scripts/smoke-test.sh` executes after the build, or refuse to fall back when a tree build exists; verify by planting a stale installed build and observing the runner name it. | FIXED `8abac0b` |
| `SEC-01` | --device=all has no justification in launcher code | S4 | — | Narrow to `--device=dri`, then run the controller hotplug test `docs/migration/packaging.md` Q-2 asks for and record that test as the grant's justification. | FIXED `e2c6476` |
| `UX-01` | Dropdowns are in no keyboard focus ring and contribute no accessibility node | S2 | — | A test asserting the `Operation` from a built dropdown contains a focusable node. Upstream `Dropdown` has its `operate` hook commented out, so this needs a wrapper or a carried patch. | OPEN |
| `UX-02` | Togglers are mouse-only | S2 | — | A test asserting a focused toggler responds to space/enter. Upstream `toggler` has **no** `operate` at all, so this needs a local widget or an upstream patch. | OPEN |
| `UX-03` | Text inputs emit no accessibility node at all, so the entire add/edit-game form is invisible to a screen reader | S2 | — | A test asserting `a11y_nodes` is non-empty for a built `text_input`. Neither implementation defines it. | OPEN |
| `UX-04` | The application configures no minimum window size, so the window can be dragged to 1×1 and every page becomes unusable | S2 | — | Set the window minimum and assert it is at least what the narrowest page layout needs (`UX-08`, `UX-09`, `UX-21`). | OPEN |
| `UX-05` | "Add Game" has no always-visible control | S2 | duplicate of `BUG-07` | As `BUG-07`; the toolbar button is the always-visible control. | FIXED with `BUG-07` |

### P2 — 51

| ID | Finding | Owner | Deps | Verification | Status |
|---|---|---|---|---|---|
| `ARCH-07` | cargo fmt --check fails, and formatting is not a stage in the gate | S5 | — | Add `cargo fmt --check` as a gate stage and make the tree clean; verify by running it at HEAD (must exit 0) and by re-introducing a formatting diff (must fail). | OPEN |
| `ARCH-08` | README.md documents a property the command it names does not deliver | S5 | — | Correct the README paragraph to what `cargo test` does, or make the command deliver the claim; verify by running the documented command with the variables set. | OPEN |
| `ARCH-09` | Errors lose their context at several boundaries, and one of them makes a local write failure indistinguishable from a legitimate "not found" | S5 | — | A regression test per boundary asserting the error names the operation and the path, and that a destination-side failure is not reported as "not found". | OPEN |
| `ARCH-10` | The error taxonomy is strong inside core and collapses at two boundaries | S5 | — | A test per boundary asserting the `core` error enum survives to the UI message rather than being flattened to `String`. | OPEN |
| `ARCH-11` | Module organisation: the four large files are large because of their test modules, and the production halves have concrete seams that no one has cut | S5 | — | Structural: verify with `cargo test` green plus the seam actually used by both callers. No behavioural test can see a moved definition. | OPEN |
| `ARCH-12` | Two god-objects: a 968-line dispatcher and a 33-field state struct | S5 | — | Structural: verify with `cargo test` green plus a per-page count of `Shell::update` arms after the split. No behavioural test can see it. | OPEN |
| `ARCH-13` | A hand-rolled Python lexer used as a test oracle is duplicated byte-for-byte between the two crates | S5 | — | Delete one copy and have both crates call the survivor; verify with a grep for the second body and `cargo test` green in both crates. | OPEN |
| `ARCH-14` | A comment says a shared helper becomes warranted when a third copy appears; the third copy already exists | S5 | — | Extract the third copy into the shared helper its own comment asks for; verify with a grep that no local copy remains. | OPEN |
| `ARCH-15` | The repo-root test helper is copied four times with three different derivations, and one copy's doc cites a precedent that does not use its derivation | S5 | — | One helper in one place; verify with a grep for the four derivations and `cargo test` green. | OPEN |
| `ARCH-16` | A comment embeds a grep transcript as its own proof, and the transcript's line numbers no longer land | S5 | — | Re-derive the transcript against the current tree, or replace it with a citation that does not age; verify the cited lines contain what the paragraph says. | FIXED `0c92f9e` |
| `ARCH-17` | Three copies of the card surface, two of which hardcode the radius that a comment says is what keeps them in sync | S5 | — | One card surface in one place, or three that cannot drift; verify with a grep for the hardcoded radius outside the definition. | OPEN |
| `ARCH-18` | Six comments across four files cite the settings-validation fallback at line numbers that do not contain it, and the citations are ambiguous in a workspace with two settings.rs files | S5 | — | Re-point each of the six citations at the line that holds the fallback and disambiguate the two `settings.rs` files by path; verify each citation lands. | FIXED `0c92f9e` |
| `BUG-12` | The Settings page advertises four keyboard shortcuts; three of them do not fire while a text field has focus, which is the state a user is most often in when reaching for one | S1 | — | Guard must assert the caveat is *rendered*, not that a word occurs in `main.rs`. The behavioural half needs a raw-event subscription and is not attempted. | PARTIAL with `BUG-21` |
| `BUG-13` | A cover-art write failure is reported to the user as "no Steam cover found" | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | FIXED |
| `BUG-14` | read_metadata collapses four distinct failures into an empty map, and the consequences are silent | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | FIXED |
| `BUG-16` | Both desktop-metainfo and the cargo-sources coverage half pass without inspecting what they claim | S1 | — | Strengthen the guard so it fails on the input it claims to reject; demonstrate the pre-fix pass first. | FIXED |
| `BUG-17` | The placeholder guard cannot see a renamed placeholder, and the "no-results" test for the installers page asserts nothing about what is drawn | S1 | — | Strengthen the guard so it fails on the input it claims to reject; demonstrate the pre-fix pass first. | FIXED |
| `BUG-18` | The notify-voice guard's haystack contains its own needle: read_crates walks back in the same #[cfg(test)] mod tests that holds the NOTIFY_VOICES literal table, so port.contains(voice) is satisfied by the table itself and cannot fail | S1 | — | Strengthen the guard so it fails on the input it claims to reject; demonstrate the pre-fix pass first. | FIXED |
| `BUG-36` | SystemLaunchEnv::environ() panics on any non-UTF-8 environment variable, aborting the process on the way into a launch | S1 | — | A non-UTF-8 variable in the environment: pre-fix the process panics with rc 101, post-fix it launches (rc 0) and passes the rest of the environment through. | FIXED `0c0b3ff` |
| `BUG-37` | which_in answers /bin:/usr/bin when PATH is set but empty, where CPython returns None — so the port finds and injects wrapper programs the reference refuses to find | S1 | — | Both halves asserted: `PATH=""` must miss (probed on `sh`, which is on the default path, so the assertion separates the two readings), and `PATH=":"` must find the cwd. | FIXED `192be4f` |
| `BUG-38` | in_flatpak short-circuits on an empty FLATPAK_ID where Python falls through to the marker file, so inside a sandbox that sets the variable empty the app does not believe it is sandboxed and offers the host's package manager | S1 | — | Empty `FLATPAK_ID` **with** the `/.flatpak-info` marker present: pre-fix the app does not believe it is sandboxed, post-fix it does. | FIXED `192be4f` |
| `BUG-39` | create_partial names the mode it does not set: the doc argues the temp file is 0600 "which mkstemp gives and create_new does not — and it is set explicitly below", but nothing below sets it | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | FIXED |
| `BUG-40` | bounded_attempt's "a bounded parse that yields an icon is final" claim is false, and the result it returns is silently the wrong icon | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | FIXED |
| `BUG-41` | file_offset's EOF check rejects a whole resource the reference reads by clamping, so a truncated executable loses an icon Python recovers | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | FIXED |
| `BUG-46` | Navigating away from the open game form leaves the form on screen | S1 | — | Two tests: `navigating_away_closes_the_layers_the_page_was_covering`, and `clear_overlays_names_every_overlay_field_on_state`, which holds the method against the fields it must cover (verified by deleting one assignment). | FIXED `d9d6863` |
| `BUG-47` | A card's and a row's title and subtitle have no ellipsis marker, so a name too long for its column is cut mid-glyph with nothing to say it was truncated | S2 | — | The comment half is fixed; the wiring cannot be asserted because iced offers no downcast and `Text::format` is private. Recorded rather than papered over. | PARTIAL `ff26a50` |
| `PERF-04` | The project's claim that startup performs only two filesystem reads is false, and the comment asserting it sits three lines above the calls that contradict it | S3 | — | Correct the comment and the README claim, and enumerate the reads that actually happen; verify by counting the syscalls at startup (`strace -c -e trace=openat,statx`). | OPEN |
| `PERF-05` | search() re-sorts and re-filters the entire library, and categories() rebuilds and re-sorts the category list, on every frame — and both allocate Strings inside sort comparators | S3 | — | Cache the filtered/sorted result against the inputs it depends on, and drop the comparator allocations; verify the per-frame allocation count on a 500-game fixture. | OPEN |
| `PERF-06` | RunnerManager::label is uncached, walks the filesystem, and is called once per shown game per frame — and it reads and JSON-parses each runner's metadata only to discard the parsed value | S3 | — | Cache the label against the runner directory's mtime, or read the id without parsing metadata; verify the per-frame file opens on a 5-runner fixture. | OPEN |
| `PKG-02` | Nothing in the repository runs the verification chain automatically | S6 | — | Add CI (or a hook, or a `just`/`make` entry) that runs `scripts/verify.sh`; verify by breaking a test and watching the wiring fail. | FIXED `372b86e` |
| `PKG-03` | Both the Flatpak build and the cargo-sources freshness check depend on things a clean checkout does not contain, and one of them needs the network by default | S6 | — | Make the Flatpak build and the cargo-sources check either self-provisioning or an explicit, reported skip with the reason; verify on a clean checkout. | OPEN |
| `PKG-04` | flatpak-contents verifies placement and bytes but never runs the binary, and its one binary check is a text search inside the ELF rather than an execution | S6 | — | Execute the installed binary in `flatpak-contents` (a `--version` is enough); verify by planting a binary that cannot run and watching the stage fail. | FIXED `ead99e7`, re-fixed `f278b7d` |
| `PKG-05` | The application ships a single 128×128 SVG and no PNG icon at any size | S6 | — | Ship PNGs at the sizes the specification names; verify with the AppStream validator and `flatpak-contents`. | FIXED `9e10566` |
| `SEC-02` | --filesystem=home grants read-write to the entire home directory, which is broader than the directories the code touches | S4 | — | Narrow to the XDG roots the code actually touches plus the seven fixed search roots; verify by running the app in the narrowed sandbox and exercising every path-touching flow. | OPEN |
| `SEC-03` | The Authenticode authenticity decision is weaker than it reads | S4 | — | Compare the publisher against the verifier's structured field rather than a case-folded substring of merged output; verify with a fixture whose output contains a matching substring in an unrelated field. | OPEN |
| `SEC-04` | game_id is interpolated into a destination filename with no sanitisation at three sites in covers.rs, while the same class of bug was deliberately fixed in desktop.rs | S4 | — | A game id containing `/` or `\` is refused by all three writes with `UnsafeId`, asserted on the *absolute path the write would have created* and on the transfer not being made; plus the accepted cases (`..`, ``, a real 32-hex id) in the same test, so a refusal-everything implementation fails it | FIXED `88578db` |
| `UX-06` | The confirmation dialogs are not modal — the page behind them stays live | S2 | — | Make the dialogs modal (an overlay, or an interaction gate on the page); verify that a click behind the dialog reaches nothing. | OPEN |
| `UX-07` | Escape does not close either dialog | S2 | — | Implement `on_escape` to close the open dialog; verify by opening each dialog and pressing Escape. | OPEN |
| `UX-08` | Both dialogs are a fixed 570 px wide and are laid out inside the page column, so below roughly 570 px plus the nav bar they are clipped on both sides — including their buttons | S2 | — | Size the dialogs from the window rather than a fixed 570 px; verify by resizing to 500 px and reaching every button. | OPEN |
| `UX-09` | The Installers search field is a hard 396 px, the only Length::Fixed used for a *field* anywhere in the views, so it overflows the page below about 430 px of content width | S2 | — | Let the search field fill its row; verify at 400 px content width. | OPEN |
| `UX-10` | Installers and Runners have no outer gutter | S2 | — | Give both views the same outer padding the other five have; verify by comparing the five view tails. | OPEN |
| `UX-11` | A capability the UI advertises is not drawn: the game form's cover preview | S2 | — | Draw the cover preview and the "No cover yet" placeholder the reference has; verify with tree-walking assertions on the form. | OPEN |
| `UX-12` | Three icon-only buttons have no accessible name and no tooltip | S2 | — | Give all three `button::icon` sites an accessible name and a tooltip; verify the drawn strings and the a11y nodes. | OPEN |
| `UX-13` | Re-picking a custom cover silently destroys the previous one | S2 | — | Refuse or confirm an overwrite, or keep both files; verify by picking a cover twice and checking the first still exists. | OPEN |
| `UX-14` | Toasts are the app's universal error channel, and they are invisible to assistive technology and expire after 5 s | S2 | — | Give the toaster an accessibility node and make the duration reasonable for a screen reader; verify the node list from a built toaster. | OPEN |
| `UX-15` | A cover-fetch failure surfaces as a bare, unprefixed error string with no game name and no indication that a cover lookup is what failed — e.g | S2 | — | Prefix the message with the game and the operation; verify the drawn toast string for a forced failure. | OPEN |
| `UX-16` | Per-game context menus have no keyboard route | S2 | — | Add a keyboard route to the context menu (Menu key / Shift+F10) or duplicate its actions into the focused tile; verify by driving the key. | OPEN |
| `UX-17` | Plate initials are drawn white-on-accent at roughly 3:1, below the 4.5:1 required at the size they are drawn | S2 | — | Raise the initials' contrast to 4.5:1 at the drawn size; verify the computed ratio against the darkest gradient stop. | OPEN |
| `UX-18` | No text_input in the app is ever given .label() or .helper_text(), and the visible labels are unassociated sibling text widgets in a Row, so they identify a field to a sighted user and to nothing else | S2 | — | Give each `text_input` a label or helper text; verify the field's accessible name. | OPEN |
| `UX-19` | Card surfaces are hand-built three times as a local card_style closure, and two of the three hardcode the radius their own comment says is kept in sync | S2 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `UX-20` | libcosmic's purpose-built settings widgets are used nowhere in the app | S2 | — | Adopt `settings::section`/`item`/`item_row`, or record why the hand-built rows are kept; verify by reading the settings view. | OPEN |
### P3 — 49

| ID | Finding | Owner | Deps | Verification | Status |
|---|---|---|---|---|---|
| `ARCH-19` | Three Python citations land on a blank line | S5 | — | Re-point the three citations; verify each lands on the code it names. | OPEN |
| `ARCH-20` | Two comments record the size of a mutation-testing result, the two numbers disagree with each other, and neither matches the current suite | S5 | — | Re-measure the mutation result and record the method with the number; verify by re-running the measurement. | OPEN |
| `ARCH-21` | The shortcut guard test is not enforced by the gate that is supposed to enforce it | S5 | — | Delete one of the two constants and observe the gate stay green (pre-fix), then fail (post-fix) once the guard is wired into a stage. | OPEN |
| `ARCH-22` | A 58-line dialog docblock describes the runner dialog but sits above the game dialog, leaving the runner dialog undocumented | S5 | — | Move the docblock onto the runner dialog and write one for the game dialog; verify both dialogs have a doc comment naming them. | OPEN |
| `ARCH-23` | The page-handler convention is inconsistent: two page modules own an update, the third does not | S5 | — | Adopt one convention across the three page modules, or record why Plugins differs; verify by reading the three signatures. | OPEN |
| `ARCH-24` | Three launch settings are written, persisted and never read by anything | S5 | — | Either read the three settings at launch, or stop persisting and surfacing them; verify with a call site for each, or none in the UI. | OPEN |
| `ARCH-25` | A module-level #[allow(dead_code)] covers the whole view layer, suppressing fifteen never-used items, including an entire dead widget stack | S5 | — | Narrow the allow to the items that need it and delete the dead widget stack; verify the crate still builds with `-D warnings`. | OPEN |
| `BUG-19` | sanitize accepts a bare - as the number null, so [-], [-]-style malformed numbers and {"last_played":-} parse where Python raises JSONDecodeError | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-20` | Library::all("recent") / all("added") do not treat -0.0 and 0.0 as equal, so two games whose timestamps tie present in a different order than the reference | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-21` | Four .trim()-where-Python-has-a-truthiness-test divergences on paths and identifiers, each of which shifts a value in the permissive direction | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-22` | merge_dll_overrides trims ; from both ends where Python rstrips only, so a leading ; survives the reference and is removed here | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `BUG-23` | Three ways a launch failure is turned into a non-failure | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `BUG-24` | The rolling stderr buffer trims to exactly limit bytes where Python keeps the trailing chunk, and the port's chunks.len() > 1 clause is dead code: Python compares *chunk counts* and keeps the last chunk even when it overshoots, while the Rust buffer is one flat Vec<u8> where the same expression is a byte count, subsumed by chunks.len() > limit. The test pins the port's behaviour and its docstring describes Python's guard | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-25` | Two functions apply opposite policies to the same input | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-26` | The archive's containment root falls back to an unresolved path: if canonicalize(destination) fails, the root stays unresolved while the member path goes through resolve_missing, which resolves as far as it can — so a resolved member is starts_with-compared against an unresolved root. Members are lexically stripped of .. first, so the reachable outcome is a *false rejection* rather than an accepted escape; I could not construct the accepting case and am not claiming one | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-27` | A typo in the Steam AppID field is silent: "abc" or "12 34" parses to 0, i.e | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-28` | Six worker-to-UI sends discard their message with let _ =, including the two that carry *why* a launch failed | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-29` | One production expect on a value a future page addition invalidates: activate_page runs on every nav-bar click and panics if a Page variant is absent from Page::ALL | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-30` | Two tests in the suite are tautologies — they compare a value against the expression that defines it, so they can only fail if the delegation they are made of is edited | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `BUG-31` | Three scripts/verify.sh hygiene defects, all of which make a run look cleaner than it was | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-32` | Two smaller unbounded checks | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-33` | The EasyInstall toast's Play action launches the game but does not dismiss the toast, so a stale "Installed … ▶ Play" toast (duration Long, 15 s) sits over the Library the user was just navigated to | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-34` | The Library card's and row's "More actions" button and tooltip have no port equivalent | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-42` | Three path/identifier conversions that do not do what the code around them says | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-43` | Two more "the doc says the two agree" pairs, both verified sound-but-for-the-claim | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `BUG-44` | An unreadable runners directory renders as a normal, empty, system-Wine-only list | S1 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `BUG-45` | python_repr's string branch is not repr() and duplicates — incorrectly — the python_str_repr twelve lines above it | S1 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `PERF-07` | The 27 threads are all demand-spawned runtime workers; none polls, and two of them exit on their own | S3 | — | Recorded as not-a-finding unless a poller is found; verify by re-running the thread-naming probe over a 60 s idle run. | OPEN |
| `PERF-08` | Minor RSS drift that is not attributable to a leak on the evidence gathered | S3 | — | Re-measure with the window held at one size; verify RSS is flat across forced redraws at a constant size. | OPEN |
| `PKG-06` | The metainfo has no screenshots and no keywords | S6 | — | Add a screenshot and keywords to the metainfo; verify with `appstreamcli validate --pedantic`. | OPEN |
| `PKG-07` | The AppStream validator is not clean under --pedantic: it prints a warning while exiting 0, and the validator the project's own meson test names could not be run at all | S6 | — | Resolve the warning under `--pedantic` and run the meson test the project names; verify both exit clean. | OPEN |
| `PKG-08` | A comment in data/meson.build asserts a packaging regression that does not exist, and its cited evidence is falsified by the manifest | S6 | — | Correct or delete the comment; verify the cited evidence against the manifest. | OPEN |
| `PKG-09` | flatpak-contents and cargo-sources both shell out to a bare python3 for their comparison helpers, independently of the interpreter chosen to run the generator | S6 | — | Use the same interpreter the generator was run with; verify by running both stages under a PATH without a bare `python3`. | OPEN |
| `SEC-05` | The Proton runner archive is downloaded from a URL taken verbatim out of the releases JSON, with no scheme and no origin check — unlike the installer download path, which applies both | S4 | — | Apply the same scheme and origin check the installer path applies; verify with a releases payload naming a `file:` and an off-origin URL. | OPEN |
| `SEC-06` | RunnerManager::get joins a games.json-sourced runner_id onto the runners directory without validating it, where three other sites validate the same kind of string | S4 | — | Validate the `runner_id` the way the three other sites do; verify with a `games.json` carrying `../../etc`. | OPEN |
| `SEC-07` | open_url performs no scheme validation, and its message payload is a plain String rather than a catalogue-constant type | S4 | — | Validate the scheme in `open_url` itself and give the payload a catalogue-constant type; verify with a non-`http(s)` producer. | OPEN |
| `SEC-08` | Three manifest permissions are justified only by the child process, with no direct launcher-code tie; the permissions that *do* tie to code are recorded here so the distinction is auditable | S4 | — | Record each permission's justification in the manifest beside it, or drop the ones with none; verify by reading the manifest against the code. | OPEN |
| `SEC-09` | Two of the app's process spawns inherit the launcher's entire environment, and they are the two security-relevant ones: the package-manager install and the Authenticode verifier | S4 | — | Clear the environment at the two spawn sites and pass back only what the child needs; verify by dumping the child's environment. | FIXED |
| `UX-21` | Grid cells are fixed at 200×300 with a fixed column count, so at narrow widths the library grid runs off the right edge and the cards there cannot be reached | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-22` | Two heading levels do the same job | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-23` | Page padding is a hardcoded 18 and the theme's spacing tokens are never consulted | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-24` | The app's only widget id is the library search box, so only that one field can ever be focused programmatically, and no dialog sets an initial focus | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-25` | No page bounds its content width, so at 2560 px and above the settings rows become "label … far-away control" pairs and the credits prose runs to an unreadably long measure | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-26` | Long names and subtitles are hard-cut mid-glyph with no ellipsis marker, where the reference elides | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-27` | There is no indeterminate or loading indicator, and a running install cannot be cancelled from the UI | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-28` | The Plugins page has no empty-state branch, unlike the other three list pages | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `UX-29` | A dead Option<()> field and a stale comment about theme inertness | S2 | — | Re-read the cited lines after the edit; `scripts/verify.sh` green. No behavioural test applies. | OPEN |
| `UX-30` | A page body's scrollable does not put the page's own primary action within reach, and the page reports this as a layout failure twice | S2 | — | A regression test that fails without the fix, plus `scripts/verify.sh` green. | OPEN |
| `SEC-10` | Dependency hygiene: five live RustSec advisories against the locked graph, none reachable from the shipped binary, and no unused or duplicated direct dependency | S4 | — | Controls for the advisory matcher (it flags `time 0.1.0` and `openssl 0.10.0`; it clears `time 0.3.44` and `openssl 0.10.99`), plus `strings` over the shipped binary for the renderer reachability claim (44,912 `tiny_skia` matches, 0 `wgpu`) and a `grep` over `xkbcommon`'s source for the six affected `memmap2` functions | OPEN |

### Not a defect — 2

Refuted by measurement rather than fixed. Both rows are kept because a reader who
has heard the claim should see it was tested. They carry a severity cell of `—` in
`BUGS.md` for the same reason they sit outside the severity counts here: a refuted
row that stays inside a severity bucket gets scored as a repaired one, which is
exactly what had happened to `BUG-15`.

| ID | Finding | Owner | Deps | Verification | Status |
|---|---|---|---|---|---|
| `BUG-15` | An unknown runner family id silently becomes the default family instead of erroring, so fetch_available fetches and returns *a different family's releases* | S1 | — | Three assertions pinning default, named and unknown; reintroducing the conflation (a `match` falling back to the default for any unresolvable id) fails the test | CLOSED — **not a defect.** the recorded behaviour does not occur. `resolve_family` applies `unwrap_or` to the `Option` *inside* a call returning `Result`, so `None` takes the default and `Some("nonsense")` propagates Python's text byte-identically. The function had no test at all, which is how the row was written without the measurement to settle it; it now pins default, named and unknown as three separate assertions |
| ~~`BUG-11`~~ | Case-variant categories are lost by `dedup` after the sort, where Python's `set` keeps them | S1 | — | Refuted by measurement: both implementations return 39 entries on a 40-game fixture, with the same ten fold-groups. The regression the finding was reaching for is real and *is* guarded — changing the sort's primary term to `a.cmp(b)` reproduces the symptom exactly (28 groups instead of 10) and `case_variant_categories_fold_into_the_same_groups_as_python` fails on it | WITHDRAWN |

## Cross-referenced pairs

Two specialist passes found the same defect from different directions. Both rows
stay — each carries evidence the other does not — but they are one fix and one commit.

| Row | Same finding as | Why both are kept |
|---|---|---|
| `ARCH-01` | `BUG-01` | `ARCHITECTURE.md` reached it from the state contract; `BUGS.md` reproduced it against the real binary |
| `ARCH-03` | `BUG-03` | Architecture reached it from the error taxonomy; bugs from the user-visible toast |
| `ARCH-04` | `BUG-04` | Architecture from the two helpers' asymmetry; bugs from the menu item |
| `ARCH-06` | the comment half of `BUG-47` | The false comment **was** the recorded bound on the ellipsis fix, so closing one closes both |
| `UX-05` | `BUG-07` | Both are "the create flow has no always-visible control" |
| `UX-19` | `ARCH-17` | Both are the three hand-built card surfaces |
| `UX-26` | the behavioural half of `BUG-47` | Same divergence, one from the toolkit side and one from the reference side |

## Execution order

The brief's four phases, with what each is gated on:

1. **Reconnaissance** — done. Six specialist documents plus `BASELINE.md`.
2. **Consolidated plan** — this file.
3. **Fix.** Every row is worked the same way: reproduce, root-cause, fix, tests,
   build, tests, exercise the UI, devil's-advocate review, update the specialist row
   **and** this table, commit. Small reviewable commits, conventional messages.
4. **Full red-team pass** over the finished tree, looping back to 3 on any failure.

## Why the remaining count is not zero yet, and what closes it

The brief allows a non-zero remaining count only where a fix is blocked on
something outside this repository. Three classes qualify, and each is named here
rather than left as a silent residue:

- **Upstream toolkit gaps** — `UX-01`, `UX-02`, `UX-03`, `UX-16`. libcosmic's
  `Dropdown` has its `operate` and `a11y_nodes` hooks commented out upstream, iced's
  `toggler` has no `operate` at all, and neither `text_input` implementation emits an
  accessibility node. A fix means reimplementing widgets the toolkit owns or carrying
  a patch against a pinned revision — decisions the project has to take deliberately,
  and not ones to take unilaterally inside an audit. `UX-04`, which *is* in the
  project's control and is the compensating change for the same class of failure, is
  scheduled and is not blocked.
- **The manifest narrowing** — `SEC-01`, `SEC-02`. Making the grant narrower is safe
  to attempt and unsafe to *assert* without running a game inside the sandbox, which
  needs a GPU, a controller and a title. The plan is therefore: make the narrowest
  change the launcher's own code justifies, then run the hotplug and launch tests
  `docs/migration/packaging.md` names, and record the result as the grant's
  justification.
- **The P3 tail.** Each P3 row is either a faithful copy of the reference's own
  behaviour — where the fix direction is a deliberate divergence that needs a
  recorded decision rather than a silent patch — or a cosmetic divergence with no
  user-visible failure path. They are recorded, not dropped, and they are worked
  after every P0, P1 and P2 row is closed.
