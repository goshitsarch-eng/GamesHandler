# Migration decision log

Every non-obvious choice made during the Qt 6/Kirigami → libcosmic migration,
with the alternatives that were considered and the reason one won. Resolution
order when the team disagrees (from the project brief):
**feature parity → working in the Flatpak sandbox → accessibility → simplicity
and maintainability → COSMIC conventions.**

---

## D-01. Parity target is the Qt app at HEAD, not the GTK4 app

**Question.** The migration brief described GameHandler as "a desktop app
currently built with GTK4". At HEAD it is not. What should the Rust port
actually be measured against?

**Options considered.**
1. Port the current Qt 6/Kirigami app (HEAD, `cbaf7e6`, version 0.7.2).
2. Port the GTK4/libadwaita app, which exists only at `3c4b735^`.
3. Halt until the brief was clarified.

**Choice.** Option 1 — **Qt → libcosmic, from HEAD**. Confirmed by the
maintainer after the discrepancy was raised.

**Why.** The GTK4 stack was removed in `3c4b735` ("Rewrite the interface on
Qt 6 and Kirigami, replacing GTK/libadwaita entirely", 2026-08-28) and five
further commits and two releases sit on top of it. `git grep` finds zero GTK
imports in application code, and `tests/test_packaging.py:90` actively fails
the build if GTK idioms reappear. The GTK4 app is two releases in the past and
is not what the maintainer runs. Building a port of it would have targeted a
dead branch.

**Consequence.** The brief's "GObject subclasses / signals / callbacks" is a
description of the *pre-rewrite* app. There are no GObject subclasses; there is
one `QObject` subclass (`Backend`, `gamehandler/bridge.py`) plus 9 QML files.

---

## D-02. Branch `cosmic-migration`, granular commits

**Choice.** All work on `cosmic-migration`, one commit per completed task, so
history reads as small reviewable changes rather than one rewrite.

**Why.** Directed by the brief, and it also keeps the Qt app fully intact on
`main` — the migration is abandonable at any commit until the final decision to
merge. Phase 1 docs are committed, not squashed into the first code commit.

---

## D-03. Cargo workspace split: `crates/core` + `crates/app`

**Question.** One crate or a workspace?

**Options considered.**
1. Single crate holding logic + libcosmic UI.
2. `gamehandler-core` (pure logic, no GUI dependencies) + `gamehandler`
   (binary: libcosmic UI + CLI).
3. One crate per domain (`-runners`, `-installers`, …).

**Choice.** Option 2.

**Why.** The existing 241-test Python suite runs headless and must keep doing
so. A single crate would drag `libcosmic`/`iced`/wgpu/windowing into every unit
test build, making "just run the logic tests" impossible on a headless box and
slow everywhere. Option 3 adds publish/coordination overhead for modules that
already import each other freely in Python (`installers.py` imports
`runners.py`; `covers.py` imports `exe_icons.py`). Option 2 keeps those
one-way dependencies intact inside one crate while isolating the GUI.

**Enforcement.** `crates/core/Cargo.toml` must not depend on `libcosmic`,
`iced`, or any GUI crate — checked mechanically, not by convention.

---

## D-04. Pin libcosmic by git rev; do not track a branch

**Question.** libcosmic is not published on crates.io. How is it consumed?

**Choice.** Git dependency pinned to an exact revision:
`rev = "a401af8b1c54a8abd393b8c5b7c8809402f83850"`.

**Why.** libcosmic declares `iced` as a git submodule pointing at
`pop-os/iced` **branch master** — a moving head with no SemVer promise. Under a
branch pin, upstream could break this app without the app changing. A rev pin
makes builds reproducible; upstream drift is then a deliberate, reviewable
bump. (Verified: `cargo build` and `cargo vendor` both work from this rev.)

---

## D-05. Test-ownership split (avoids two teammates editing one tree)

**Question.** Architecture owns the core modules; QA owns `tests/`. Rust
convention puts unit tests *inside* the module file. Who writes the core
unit tests?

**Choice.** Architecture teammate writes **inline `#[cfg(test)]` unit tests**
in `crates/core/src/**` (they are part of the module they test). Packaging/QA
teammate owns **`crates/core/tests/`** (integration tests + fixtures),
**`crates/app`** test scaffolding, **`tests/`**, **`scripts/`**, and
**`flatpak/`**.

**Why.** Prevents two owners writing the same file. The 241 existing Python
tests are the porting checklist: pure-logic suites (models, settings, runners,
installers, covers, exe_icons, netpaths, plugins, credits) become inline unit
tests; flow-level suites become `crates/core/tests/` integration tests.

---

## D-06. JSON compatibility is a hard requirement, including Python's lenient NaN

**Question.** Must the Rust app read files the Python app wrote, and vice
versa?

**Choice.** Yes — **both directions**, byte-compatible where practical.

**Why.** Users must be able to downgrade. Critically, Python's `json.loads`
*accepts* `NaN`/`Infinity`/`-Infinity` while `serde_json` rejects them at parse
time, and `models.py:68-87` then normalizes them. A naive serde port would
**fail the entire library file** on input the Python app tolerates. The port
must parse leniently and apply the same normalization table, including the
`isinstance(value, bool)` rejection rule (JSON `true` must not become timestamp
`1.0`).

**Verification.** Round-trip fixtures written by the Python build, checked in
under `crates/core/tests/fixtures/`, asserted from both directions.

---

## D-07. `Message` + `State` replaces the QObject property/signal contract

**Question.** How is `Backend`'s 33 properties / 23 slots / 16 signals
expressed in iced's Elm architecture?

**Choice.** One `Message` enum (~50–55 variants), a `State` struct, and
`update()`. Derived properties (`games`, `categories`, `installedRunners`, …)
become **pure functions of state**, not cached mutable fields.

**Why.** In iced, `view()` re-runs after every `update()`, so signals like
`gamesChanged` have no equivalent and are not needed. Making the row builders
pure functions moves today's untestable QML-adjacent logic (`_game_row`,
`_plugin_row`, subtitle composition) into headlessly testable code — a
strict improvement.

**Deliberate behavior changes** (only these; everything else faithful):
1. Confirm-before-delete for removing a game (currently instant, and
   irreversible).
2. `formCategories` becomes derived rather than a QML `constant=True` property
   that goes stale after adding a game with a new category.
3. `progress: Option<f32>` replaces the `-1.0` sentinel; `ReleasesStatus` enum
   replaces the `"error: {msg}"` string protocol.
4. `quit()` closes the window through the normal COSMIC path instead of
   `QCoreApplication.quit()`.

---

## D-08. `cargo clippy -- -D warnings` is run only at our workspace root

**Question.** Clippy fails inside libcosmic's own workspace
(`cosmic-config-derive/src/lib.rs:3`, `use syn;` →
`clippy::single_component_path_imports`). Does our DoD gate even hold?

**Choice.** Run clippy **only** at the GameHandler workspace root. Never inside
the libcosmic checkout.

**Why.** Verified empirically: path members of a workspace do not get cargo's
`--cap-lints allow`, so libcosmic lints itself and fails. A *consumer* crate is
unaffected — cargo caps lints for dependencies, so libcosmic's own lints never
surface in our build. `scripts/verify.sh` must therefore scope clippy to our
workspace only.

---

## D-09. `iced` submodule is NOT a vendoring problem — but three other git deps are

**Question.** libcosmic declares `iced` as a *path* dependency and a git
submodule. Does the offline Flatpak build need a hand-written git source for it?

**Options considered.**
1. Separate `flatpak-builder` git source for `pop-os/iced`.
2. Standard `cargo-sources.json` generation and let cargo handle it.

**Choice.** Option 2.

**Why.** Verified: cargo initializes submodules for git sources. A standalone
crate depending on libcosmic by git URL builds successfully (exit 0), the
`iced` tree is present in cargo's checkout at
`~/.cargo/git/checkouts/libcosmic-*/a401af8/iced`, and `cargo vendor` produces
a complete 630-crate offline tree including `iced*`. The **real** vendoring
trap is different: cargo also resolves three further git dependencies
(`pop-os/dbus-settings-bindings`, `wash2/accesskit?tag=cosmic-0.14`,
`iced-rs/cryoglyph`) which `cargo-sources.json` must cover.

---

## D-10. Flatpak runtime: Freedesktop 25.08 + rust-stable extension

**Question.** The KDE runtime currently carries Qt/PySide6. A Rust app needs a
different SDK. Which?

**Choice.** `org.freedesktop.Platform`/`Sdk` **25.08** with
`org.freedesktop.Sdk.Extension.rust-stable//25.08`.

**Why.** Verified via `flatpak remote-ls flathub`: the 25.08 rust-stable
extension ships **Rust 1.98.1**, comfortably above libcosmic's
`rust-version = "1.93"` floor. Freedesktop 25.08 also matches the Wine
BaseApp's `stable-25.08` line, keeping ABI expectations aligned.

**Resolved.** The Wine BaseApp **can** still be layered. `org.winehq.Wine`'s
`stable-25.08` branch already sits on `org.freedesktop.Platform//25.08` — the
same runtime this migration moves to — so there is no ABI mismatch and no
runtime bump to wait for. Verified against `org.winehq.Wine.yml`. Keeping the
BaseApp is mandatory for parity: launching Windows games is the app's core
purpose.

---

## D-11. Renderer: ship the software path (`iced_tiny_skia`) as the default

**Question.** libcosmic renders through iced. `wgpu` is a GPU backend and
`tiny_skia` is a software one; libcosmic's *default* feature set does not
include `wgpu` (it is opt-in). Which do we ship, and does the app still start
where there is no usable GPU (llvmpipe-only, headless CI, no `/dev/dri`)?

**Options considered.**
1. Enable `wgpu`.
2. Take libcosmic's default feature set, i.e. software rasterization via
   `iced_tiny_skia`.

**Choice.** Option 2 — **software by default; leave `wgpu` off.**

**Why.** Settled by spike rather than argument, as the protocol requires. A
minimal libcosmic application built with default features (no `wgpu`) launched
under a display server and **stayed running** (observed by timeout, exit 124) —
which means the CPU rasterizer initialised and painted with no GPU work at all.
This clears the project DoD's headless smoke-test requirement without
lavapipe, a Vulkan loader, or a virtual GPU, and it means a user with a broken
or absent GPU gets a working window instead of a blank one or a panic.

Under the project's resolution order this outcome wins on two counts: it
protects **working correctly in the Flatpak sandbox** (no `/dev/dri` needed for
rendering) and it is **simpler and more maintainable** than shipping a GPU
stack that the smoke test cannot exercise. Revisit only if CPU rasterization
proves too slow in practice, and only with measurements.

**Consequence for packaging.** `packaging.md`'s earlier assumption that the app
renders "via wgpu" is superseded: the manifest does not need `--device=dri`
*for rendering*. A hardware device is still needed to *play* games (gamepad,
GPU access passed through to the launched game), which is a separate question
recorded as Q-2 in `PLAN.md`.

---

## D-12a. No display must produce a diagnostic, not a panic

**Question.** What happens when the app is started with neither
`WAYLAND_DISPLAY` nor `DISPLAY` set — a headless CI box, an SSH session, a
crashed compositor?

**Finding.** The Qt app prints a friendly hint and exits cleanly
(`main.py`'s `run_gui()` catches the QML-engine failure). The libcosmic spike
**panics**: exit 101, with `thread 'main' panicked at
.../iced/winit/src/lib.rs:92:39: Create event loop: NotSupported(... "neither
WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set.")`.

**Choice.** Catch it and print an actionable message, exiting non-zero without
a traceback. Recorded as parity items **N-01/N-02** and scheduled as an early
task (T-08), because it is both a user-visible regression and a testability
prerequisite.

**Why.** Two reasons, in priority order. First, **parity**: the Qt app does not
crash this way. Second, the `--launch` path that every existing desktop shortcut
invokes (D-12) must never require a display; a panic there would break shortcuts
that already exist on users' disks.

---

## D-12. Keep `--list` / `--launch` / `--version` free of any GUI initialisation

**Question.** Today the CLI paths run "without importing Qt"
(`main.py:1-7`). Does that matter enough to constrain the Rust design?

**Choice.** Yes — the CLI paths must not initialise libcosmic, the window, or
the renderer. They run and exit before any GUI code.

**Why.** Desktop shortcuts created by the app invoke `gamehandler --launch
<id>` (`runners.py:1449-1473`). If that path needs a display or GPU, every
shortcut a user already created breaks — and it breaks for users on
headless/remote sessions too. This is also the escape hatch that keeps the app
testable on machines with no GPU. Exit codes and stderr text are preserved
verbatim because shortcuts and scripts may depend on them.

---

## D-13. Theme default: follow the system, not hardcoded dark — **OPEN**

**Question.** The README advertises a **dark interface by default**, and
`settings.py` defaults `colorScheme` to `"dark"` while *also* offering
`"system"`. COSMIC applications follow the system theme, and forcing dark
fights the toolkit (and the user's desktop settings).

**Options considered.**
1. Keep the dark default for faithfulness to the Qt app.
2. Default to the system theme, keeping explicit light/dark overrides
   (`COLOR_SCHEMES` already has all three).

**Status.** **Not yet decided.** This is a deliberate scope call, not a
technical blocker, so it is scheduled rather than spiked: decided at **T-13**
(the Settings page task) and recorded here.

**Constraints on the answer.** The three-value `COLOR_SCHEMES` tuple and the
persisted `colorScheme` key must survive either way, so an existing user's saved
choice is never lost. Whichever way it goes, the README's claim must match the
shipped default — a mismatch here is a documentation defect the advocate will
catch.

**Lead's leaning.** Option 2, because "following COSMIC conventions" and
"simplicity and maintainability" both favour it and neither outranks parity in a
way that matters: the *feature* (three-way theme choice) is preserved exactly;
only the out-of-the-box default changes. But it is user-visible, so it gets an
explicit decision and a README update rather than a silent change.

---

## D-14. Do not replicate the `added: null` crash; fix it

**Question.** The Python app crashes under `sort="recent"`/`"added"` when a game
entry has an explicit `"added": null`. The port must be compatible with Python's
JSON — but must it be compatible with its **bugs**?

**Options considered.**
1. Replicate exactly: accept `null` and carry `None` through, matching Python.
2. Treat `null` as an invalid value and normalize it (regenerate `added` from
   the clock; `last_played` → `0.0`).

**Choice.** Option 2 — **fix it**.

**Why.** Compatibility exists so users can move between versions without losing
data, not so defects propagate. A `null` timestamp carries no information, so
normalizing cannot lose anything a user typed; and `Game.from_dict` already does
exactly this for every *other* invalid value (`NaN`, `Infinity`, `-1`, `"soon"`,
`true`) — `null` slips through only because of an early `continue` that skips
normalization without popping the key. Option 2 makes `null` behave like its
siblings, so it is the *consistent* reading of the existing code, not a new
policy. Under the resolution order, "working correctly" outranks bug-for-bug
fidelity.

**Consequence.** The Rust port never holds a nullable timestamp, so the crash
class cannot occur. A unit test asserts `null` normalizes. Written up in
`docs/migration/oracle/FINDINGS.md` §F-B, and worth reporting upstream to the
Python project independently of this migration.

---

## D-15. Match Python's `ensure_ascii` escaping in saved JSON

**Question.** Python's `json.dumps` defaults to `ensure_ascii=True` and escapes
every non-ASCII character. `serde_json` writes raw UTF-8. Which do we emit?

**Options considered.**
1. Let `serde_json` write raw UTF-8 (the idiomatic Rust choice).
2. Emit `\uXXXX` escapes to match Python byte-for-byte.

**Choice.** Option 2 — **match Python**.

**Why.** Discovered by running the Python implementation rather than reasoning
about it (see FINDINGS §F-A). Both forms are valid JSON and round-trip
identically, so this is purely about testing strength and user experience:
matching makes the round-trip test a **strict byte equality** check instead of a
semantic one, and it keeps a user's `games.json` byte-stable across a downgrade
so `git diff` and file-sync tools stay quiet. The cost is a custom formatter —
small, and paid once.

**Verification.** T-02 asserts the exact fixture bytes from
`docs/migration/oracle/fixtures/*.out.json`.

**Bounded (added after F-F; corrected after review).** Byte-equality is a goal
for realistic data, not an invariant. Python renders floats with C `%g` exponent
rules and the pinned `serde_json` 1.0.151 renders them via **`zmij`** (an earlier
version of this paragraph said Ryu; there is no `ryu` in `Cargo.lock` at all,
and the same paragraph also repeated a retracted claim that Rust writes `1e7`
where Python writes `10000000.0` — both sides write `10000000.0`). They disagree
on two classes:

- **padding** — Python writes `1e-07` where Rust writes `1e-7`;
- **notation** — across `1e-5 ≤ |x| < 1e-4`, Python uses exponent form and Rust
  positional: Python `1e-05`, Rust `0.00001`.

Real timestamps (~1.7e9) round-trip identically, and the escaping part of this
decision (the actual motivation) is unaffected. The round-trip test therefore
asserts byte equality **and**, for float fields whose spellings legitimately
differ, numeric equality after reparse. We do not contort the writer to emulate
`printf`. See FINDINGS §F-F, including the note on why the divergence was
mistakenly reported as narrower than it is.

---

## D-16. Parse JSON numbers leniently; out-of-range literals must not drop the library

**Question.** Python accepts a numeric literal that exceeds `f64` range
(`1e400` → `inf`) and then normalizes it. `serde_json` **rejects** it, which
would discard the whole `games.json`. And Python accepts integers of arbitrary
size, converting to the nearest `f64`; `serde_json`'s default path does not.

**Options considered.**
1. Use `serde_json`'s default number handling; accept that such files are
   rejected.
2. Parse leniently — accept any numeric literal, saturating to `±inf` outside
   `f64` range, then apply the existing normalization table.

**Choice.** Option 2 — **lenient parsing**.

**Why.** This is exactly the failure mode D-06 exists to prevent, reached by a
different input than `NaN`. D-06 chose compatibility so a user's library
survives; a file the Python app loads fine must not be silently emptied by the
Rust app. The blast radius is the deciding factor: Python's tolerance loses one
field (`added` is regenerated from the clock; `last_played` → `0.0`, neither of
which carries information a user entered), whereas a parse error loses every
game in the file. Under the resolution order, working correctly wins.

**Implementation note.** `serde_json` alone is insufficient — its
`arbitrary_precision` feature keeps the literal as a string, which then needs
the same saturating conversion. Either route is acceptable; whichever is chosen
must be proven against `docs/migration/oracle/fixtures/out_of_range/`, which
pins Python's behaviour for `1e400`, `-1e400`, a 30-digit integer, `i64::MAX`,
and `u64::MAX+1`.

---

## D-17. Keep the Python test suite green; update `test_packaging.py` with the manifest

**Question.** `tests/test_packaging.py` asserts the *old* manifest — KDE
runtime, `llvm21`, the `pyside6` module, `PYTHONPATH`. Rewriting the manifest
breaks it. Do we let the Python suite go red, retire it, or update it?

**Observed.** Running the suite after the manifest rewrite: **240 pass, 1 fails**
(`test_flatpak_has_reviewed_launcher_permissions_and_multilib`,
`manifest["runtime"] == "org.kde.Platform"`). Exactly the one test that
described the stack being replaced.

**Options considered.**
1. Let it fail; the Python app is being replaced anyway.
2. Delete `test_packaging.py`.
3. Update it in the same commit as the manifest change, keeping the suite green.

**Choice.** Option 3 — **update it, and keep the suite green throughout**.

**Why.** Two reasons. First, the invariant is worth more than the test: the 241
Python tests are the *behavioural reference* the Rust port is being written
against (D-05), and a permanently-red suite destroys that signal — a genuine
regression would be lost in known noise. Second, only part of the file is
obsolete. Its assertions split cleanly:

- **Survives** — the app identity contract (`APP_ID` matching across desktop,
  metainfo, icon), license, and the *reviewed permissions* set (`--allow=multiarch`,
  `--filesystem=home`, `--filesystem=xdg-run/gvfs`, `--device=all`,
  `inherit-extensions` multilib), plus the `osslsigncode` and `dxvk-runtime`
  modules. These encode deliberate review decisions and must still hold.
- **Obsolete** — `org.kde.Platform`, `llvm21`, the `pyside6` module,
  `--env=PYTHONPATH=…`, and the Python-only `cleanup` entries. These describe
  the stack being removed; they are replaced by the Rust equivalents.

So the update is a retarget, not a deletion: the test keeps asserting *"the
manifest's permissions have been reviewed and the identity is consistent"*,
which is exactly the guarantee worth preserving, and stops asserting the
implementation details that are intentionally changing.

**Note.** `gamehandler/**` is untouched by this migration branch, so every
behavioural test (models, runners, installers, covers, security, …) continues to
pass unchanged and keeps its full value as a porting reference. This is the
strongest available signal that the Rust port has not silently changed
behaviour, and it costs nothing to maintain while the Python app remains on the
branch (D-02).

---

## D-18. Wrong-typed scalar fields are coerced, not stored as-is

**Question.** `Game.from_dict` type-checks nothing but the two timestamps. A
`games.json` containing `{"name": 123}` stores the **int** `123` in
`Game.name`. Every sort mode then evaluates `g.name.lower()`, so `sort="name"`
— the **default** — raises `AttributeError: 'int' object has no attribute
'lower'`. Measured: all three sort modes fail. `{"name": ["x"]}` fails the same
way, and `{"name": null}` is different again but no better — the game is
dropped from the library entirely, because the constructor's `if game.name:`
guard is falsy for `None`.

The same permissiveness applies to `steam_appid`, `category`, `exe_path` and the
boolean toggles, which are equally unvalidated but happen not to be touched by
the sort key. So the bug is latent in five more fields.

**Options considered.**
1. Mirror the permissiveness: hold whatever JSON value arrived in the typed
   field. (For Rust this means `name: Value`, or a hand-rolled enum per field.)
2. Parse each scalar into its declared Rust type, coercing where a lossless
   coercion exists and falling back to the field default otherwise.

**Choice.** Option 2 — **coerce into declared types**.

**Why.** This is the same class of defect as D-14, with a wider blast radius and
a lower trigger. D-14 needs the user to have selected a non-default sort *and* a
null timestamp present. D-18 needs neither: the library is unbootable, on
startup, for any file with a non-string `name`, whatever wrote it. Option 1 would
mean the Rust port reproducing an `AttributeError`-equivalent, which is not
parity worth having.

Note the asymmetry this preserves and the one it drops. Preserved: a wrong type
is *never* an error — loading still succeeds, and the rest of the library still
loads (matching Python's tolerance, F-D/F-G/F-J). Dropped: the specific value
surviving untyped into a field that later crashes the view.

**Consequence.** `rust_divergences` gains a third case so that a test author
cannot mistake the Python result for the expected one.

**Correction (found while implementing it).** The rule as first written —
*"use the string form where one exists, otherwise the default"* — does not
describe what the port does, and the `rust_divergences` entry could not be
implemented from it. There is a **third** behaviour, which the option set above
never named: for an array or object the name has no usable string form, the
field stays empty, and `Library.load` **skips the unnamed entry entirely**.

```
input:  {"name": ["x"]}     Python: count 1, every sort raises AttributeError
                            port:   count 0 — the game is dropped
input:  {"name": null}      Python: count 0 (same reason, `if game.name:`)
```

Dropping is the right answer — the alternative is carrying an entry with no
name into a UI that must display one — but it is a *third option*, not the
"otherwise the default" case, because for `["x"]` a string form arguably does
exist (`Value::to_string` gives `["x"]`). The distinction only shows up for
non-scalar names, which is exactly why it was missed. Both the coercion rule and
this drop case are now recorded in `rust_divergences`, and the drop is pinned by
`an_entry_with_no_usable_name_is_dropped`.

---

## D-19. `serde_json` is required with `float_roundtrip`

**Question.** `serde_json`'s default `f64` reader computes `mantissa * 10^exp`
in `f64`, which is not correctly rounded.

**Measured on the committed fixture, so anyone can reproduce it:**
`floats_roundtrip.in.json` holds 300 realistic `time.time()`-shaped values, and
**58 of them (19.3%) come back one ULP off** through the default reader. With
`float_roundtrip` enabled: **0 of 300**.

An earlier version of this decision quoted "4070/20000 (20.35%)" from a
throwaway corpus whose seed and generator were never recorded. The order of
magnitude is right and was independently reproduced (4095/20000 on a different
LCG), but an unreproducible figure has no place in a document whose purpose is
to justify a pin — the 58/300 count is backed by a committed file.

**Options considered.**
1. Accept the default reader; write the round-trip test against the existing
   fixtures, which all pass.
2. Declare `serde_json = { version = "1", features = ["float_roundtrip"] }`.

**Choice.** Option 2.

**Why.** Option 1 is not merely "slightly wrong" — it is *undetectably* wrong.
The original fixtures used small round values (`100.0`, `300.0`,
`1700000000.5`) that happen to survive; the error needs dense, arbitrary values
to appear, i.e. precisely the timestamps the app writes for real. So option 1
would have produced a suite that was green while every user's `games.json`
drifted in the low bits on the first Rust save — and the byte-equality guarantee
D-15 depends on would have held on the fixtures and failed on real data. That is
the worst shape for a compatibility bug: the test that exists to catch it
reports success.

This was found by adversarial review, not by the port author, and after the
fixtures had been written. It is recorded as a decision rather than a footnote
because it is the single highest-value change to come out of the review.

**Implementation note.** Pinned by the oracle section **`float_roundtrip`**
(`fixtures/oracle.json`) and the fixture pair **`floats_roundtrip.in.json` /
`floats_roundtrip.out.json`** — an earlier version of this note gave the section
as `floats_roundtrip`, which is the *fixture* stem, not the section key: 300
seeded realistic timestamps with their IEEE-754 bit patterns, asserting the
re-saved bytes are identical. **The pin was mutation-verified:** removing the feature
fails three tests — `realistic_timestamps_round_trip_bit_for_bit` (431/2000),
`realistic_timestamps_survive_a_load_and_save_with_their_bits_intact` (58/300),
and `a_value_the_default_reader_gets_wrong_is_read_correctly`, which names a
single value (`1836229572.9881566`) whose bit pattern flips. The last one exists
because the feature changes no API a compiler could check — unlike
`unbounded_depth`, whose removal fails to compile — so the named value *is* the
guard. Every fixture that predated these three passed with the feature off,
which is the whole point. The test compares **bits**, not decimal text, so it cannot
be satisfied by a writer that merely formats prettily. **Any future change to
the `serde_json` dependency must not drop this feature** — a bare `"1"` in
`crates/core/Cargo.toml` reintroduces the bug silently, which is why the reason
is written next to the dependency as well as here.

---

## D-20. `Settings` tolerates invalid UTF-8, like `Library`

**Question.** `Library.load` catches `UnicodeDecodeError`; `Settings.load`
catches only `(json.JSONDecodeError, OSError)`, and `UnicodeDecodeError` is a
subclass of `ValueError`, so it propagates. Measured: a `settings.json` with one
`0xff` byte raises `'utf-8' codec can't decode byte 0xff in position 23`.

This is a **startup crash**. `Backend.__init__` calls `Settings.load` while
constructing the backend, so the process dies before any window appears — the
user's app does not open at all and the cause is a byte in a settings file they
never see.

**Options considered.**
1. Mirror the asymmetry: make `Settings` loading strict, `Library` loading
   tolerant.
2. Make `Settings` as tolerant as `Library`: on any decode failure, fall back to
   defaults.

**Choice.** Option 2 — **tolerate and fall back to defaults**.

**Why.** Same reasoning as D-14 and D-18: this is an unambiguous bug, and
copying it imports a crash. The asymmetry has no defensible reading — there is
no design in which a game library should survive a bad byte but the settings
file beside it should take the app down. Under the resolution order the first
two priorities (parity, then working correctly) conflict, and "working
correctly" wins because a crash is not a behaviour worth preserving. Falling
back to defaults loses at most the user's theme and sort preferences.

**Consequence.** The divergence is deliberate and is recorded in
`rust_divergences` alongside D-14 and D-18, so a porting test asserts the
tolerant behaviour rather than the crash.

---

## D-21. Reject no file that Python reads; specifically, strip a BOM and do not recurse into discarded values

**Question.** Two shapes where strictness on the Rust side would cost the user
data that Python keeps:

1. **BOM.** Python's `json.loads` rejects a leading `\xef\xbb\xbf`, so
   `Library.load` returns an empty library — and the next `save()` writes `[]`
   over the user's `games.json`. Measured: `count=0`, and the output equals the
   empty-library fixture byte for byte.
2. **Deep nesting.** A `junk` field of 1 000 nested arrays parses fine in Python
   (`count=2`, both entries survive) but trips `serde_json`'s default recursion
   limit of 128, which would reject the file. Notably the value is *discarded
   anyway* — unknown keys do not survive (F-D) — so there is no reason to
   materialise it.

**Options considered.**
- **BOM.** (a) Match Python exactly, including the data loss. (b) Reject the
  file as Python does but do not save over it. (c) Strip the BOM and read the
  file.
- **Nesting.** (a) Raise the recursion limit to Python's tolerance. (b) Skip
  unparsed unknown values without recursing.

**Choice.** BOM: **(c) strip it**. Nesting: **(b) skip discarded values**.

**Correction (found while implementing it) — the fallback was unsafe, not
merely worse.** Option (a) "raise the recursion limit" was offered as an
acceptable fallback. It is not: `unbounded_depth` +
`disable_recursion_limit` parses with stack proportional to nesting depth, and
on a 2 MiB stack a **1 500-deep document overflows the stack and aborts the
process** — `fatal runtime error: stack overflow`, SIGABRT, uncatchable, in the
process that owns the user's window. That is strictly worse than rejecting the
file. The margin was not theoretical even at the tolerated 1 000: enabling
`serde_json/preserve_order` — which **`cosmic-theme` does, and Cargo feature-
unifies it into our workspace build** — was enough to abort on the oracle's own
`deep_nesting` fixture. It surfaced only because a workspace-wide `cargo test`
died while `cargo test -p gamehandler-core` passed all 87 tests. Removing
option (a) from consideration; there is no safe recursive path here.

**What the port actually does (a mechanism neither option named).** `json.rs`
runs a **textual pre-pass** (`clamp_depth`) that finds values nested past 64
levels by counting brackets without descending, and replaces them with `null`
before `serde_json` ever sees them. So nothing recurses, and `unbounded_depth`
has been **removed from `Cargo.toml`** — which is a net gain: if the clamp ever
regresses, serde_json's own 128-level limit turns the input into a *catchable*
error instead of an abort.

This makes D-21 a **fourth** case the divergence list has to carry, because it
alters a value rather than merely accepting the file: for a *known* key the
"unknown keys are discarded" argument of F-D does not apply. The oracle's
`deep_nesting` fixture cannot see the difference, since it nests under a junk
key and every depth from 64 to 10 000 produces the same bytes — so the boundary
is pinned instead by `a_pathologically_deep_document_does_not_overflow_the_stack`
(20 000 deep, past where the old code aborted). The test *is* the assertion,
because a stack overflow cannot be caught.

**Why.** Both are cases where the *only* thing Python's behaviour preserves is a
way to lose data, so "match Python" and "do not lose the user's library" point
in opposite directions, and the second wins. Neither choice weakens D-06: a
BOM'd or deeply-nested file is one Python cannot usefully read either, so no
file Python *wrote* is affected, and nothing about a file Python can read
becomes unreadable.

BOM stripping is strictly safer than option (a) at zero cost — the file is
valid JSON after the BOM is removed, so the user's games load instead of being
erased. For nesting, option (b) is both safer *and* cheaper than (a): the
subtree is discarded by F-D regardless, so not recursing into it avoids the
limit and the allocation at once. Raising the limit (a) is the fallback only
because it leaves the port parsing — and bounding recursion on — data it is
about to throw away.

**Consequence.** Pinned by `encoding_and_shape.bom` and
`encoding_and_shape.deep_nesting`. These are the two places in the port where
being *more* tolerant than Python is deliberate rather than incidental; both are
listed in REPORT.md's deviations section.

---

## D-22. `save()` still replaces a symlink — inherited limitation, not a divergence

**Question.** `Library.save` writes `path.with_suffix(".json.tmp")` and
`os.replace`s it onto the target. If `games.json` is a **symlink** — the natural
way to keep a library under version control or in a synced folder — the replace
destroys the link. Measured: `link_still_symlink_after_save=False`, the target
still holds `[]`, and the real bytes now sit at the link's old path as a regular
file. So the user's sync silently stops working after the first save.

**Options considered.**
1. Write through the symlink: resolve the path first, then temp-file-and-replace
   *at the resolved target*.
2. Keep the current behaviour and document it.
3. Write in place, without the temp file, when the path is a symlink.

**Choice.** Option 2 — **keep it, document it, and do not diverge**.

**Why.** Stated plainly: **the port is choosing byte-for-byte parity with Python
over preserving the symlink.** Option 1 does *not* give up atomicity — it
resolves the path and then does the same temp-file-and-replace at the resolved
target, so an interrupted save is still safe. The only thing it gives up is
observable identity with the Python implementation, which is what D-06 exists to
protect: a `games.json` written by the port must be indistinguishable from one
Python wrote.

(An earlier version of this reasoning argued that option 1 "changes where the
bytes land, and a user might rely on the link being replaced". That is not a
credible user and it is not the real reason; the real reason is parity, and the
parity cost is small but real.) Option 3 is genuinely worse — it trades the
atomicity guarantee for a convenience — so it is rejected on its merits.

So this is recorded as a **known limitation** rather than a divergence: the port
inherits it rather than introducing it. It is listed in REPORT.md alongside the
F-B/F-I/F-J/F-K bugs that are worth reporting to the Python project upstream,
since it has the same character — an edge case that silently does the wrong
thing to a user's data — even though the port does not fix it.

**Revisit if** a user reports it. Option 1 is the change to make then; it is
listed here so the analysis is not repeated.

---

## D-23. File dialogs: libcosmic's own portal-backed chooser, not `rfd`

**Question** (PLAN.md Q-3). Does the app need libcosmic's `rfd` feature for the
three file dialogs (P-21 exe picker, P-24 cover picker, P-57 installer source
directory)?

**Options considered.**
1. Enable libcosmic's `rfd` feature.
2. Add `ashpd` to the app crate and drive the XDG portal directly.
3. Use libcosmic's built-in `cosmic::dialog::file_chooser`, gated on the
   `xdg-portal` feature we already enable.

**Choice.** Option 3. **No dependency change is needed at all.**

**Why.** Verified at the pinned rev, not assumed: `libcosmic/src/dialog/
file_chooser/` is a first-class module with `open::Dialog` / `save::Dialog`,
`open_file()` / `open_files()` / `open_folder()` / `open_folders()`, and
`FileFilter` (description + extensions) support for the exe and image filters.
Critically, it re-exports its types from `ashpd::desktop::file_chooser` and is
gated on libcosmic's **`xdg-portal`** feature — which our `crates/app/Cargo.toml`
feature list already contains. So the portal path is available today.

`rfd` is the *alternate* backend, for platforms without a portal. On Linux it is
strictly redundant: it would be a second dialog stack in the dependency graph
and in `cargo-sources.json` for no behaviour we want. `ashpd` directly (option 2)
would mean reimplementing the dialog plumbing libcosmic already ships.

**Verified detail worth keeping.** `rfd` *is* present in `Cargo.lock` — as an
**optional** dependency of libcosmic, recorded by the lockfile but never
compiled: `cargo tree -e features` shows no `rfd` in the actual build graph, and
`cargo metadata` names libcosmic as its only dependent, marked `optional: true`.
So its presence in the lock is not evidence that we depend on it. A reader
tracing dialogs should not conclude from `Cargo.lock` that `rfd` is in play.
(Whether it also appears as an unused entry in `cargo-sources.json` has **not**
been checked; that is a vendoring-size question for T-16/T-17, not a
correctness one.)

**Consequence for T-15.** The dialog API returns a **`url()`**, not a path. That
is the right primitive for P-72 (network-share games via GVFS, R-9), since a
portal selection may be a URI that has no direct filesystem path — T-15 must
keep the `url` → path conversion explicit and handle the failure case rather
than assuming a local path. Also note `Err(file_chooser::Error::Cancelled)` is
a normal outcome, not an error to report to the user.

---

## D-24. `crates/core` may depend on archive, compression, hashing and rename primitives

**Question.** `architecture.md` §1.1 allowed `core` "only `serde`/`serde_json`, an
HTTP client, and std". That allowlist was written before R-3 was understood.
`core::runners` needs tar extraction, gzip/xz/bzip2 decompression, SHA-256, and
an atomic no-replace rename. Is the allowlist amended, and if so which
implementations?

**Options considered.** For compression: pure-Rust backends (`flate2`/
`miniz_oxide`, `lzma-rs`, `bzip2-rs`) versus bindings to the reference C
implementations (`xz2` → liblzma, `bzip2` → libbz2, `flate2` → zlib).

**Choice.** **Amend the allowlist**, and take the **reference backends**.

**Why.** The allowlist's purpose was never "no dependencies" — it is that
`cargo test -p gamehandler-core` stays instant, headless and offline (D-03). A
tar/compression/hash dependency does not compromise that; an async runtime or a
GUI crate would, and both remain excluded. So the rule being defended is intact.

The backend choice is decided by what this code *is*: R-3 marks extraction as
the one place in this port where a defect is a vulnerability rather than a bug,
and it processes untrusted input (a downloaded Proton tarball, a user's installer
archive). Against that, "mature implementation with a decade of adversarial
attention" beats "fewer linked libraries". liblzma and libbz2 are the reference
implementations for their formats; the pure-Rust equivalents are younger and
less exercised on hostile input. Simplicity also favours them — they are thin
wrappers, not reimplementations.

**Consequence — measured, not assumed.** This links C libraries, so the
Flatpak must provide them. Checked against the installed 25.08 runtime and SDK
rather than assuming, and the answer is **mostly yes, with one gap**:

| | `libz` | `liblzma` | `libbz2` |
|---|---|---|---|
| runtime `.so` | `libz.so.1` | `liblzma.so.5` | `libbz2.so.1` |
| SDK header | `zlib.h` | `lzma.h` | `bzlib.h` |
| SDK `pkg-config` | `zlib.pc` ✓ | `liblzma.pc` ✓ | **`bzip2.pc` MISSING** |

So the libraries and headers are all present, and two of three advertise
themselves to `pkg-config`. Bzip2 does not: `libbz2.so` and `bzlib.h` exist but
there is no `bzip2.pc`.

**This is a build-time risk for T-17, not a design problem.** Read the vendored
`bzip2-sys-0.1.13+1.0.8/build.rs` (corrected 2026-09-11; the previous version of
this paragraph named a `BZIP2_SYS_USE_PKG_CONFIG` variable that **does not
exist** in that crate — `build.rs` reads no environment variable at all). The
actual logic is:

```rust
} else if !cfg!(feature = "static") {
    if pkg_config::Config::new().cargo_metadata(true).probe("bzip2").is_ok() {
        return;                     // system libbz2 — not taken, no bzip2.pc
    }
}
// falls through: cc::Build over the vendored bzip2-1.0.8/*.c -> libbz2.a
```

Because the probe result is consumed by `.is_ok()`, a missing `bzip2.pc` is
**not** a failure — it selects the vendored-source build. `lzma-sys` uses the
same `.is_ok()` shape (`probe_library("liblzma")`), so it is equally safe. So
the missing `.pc` costs a slower C compile, nothing more.

What remains genuinely unproven is narrower than before: that the vendored
`bzip2-1.0.8/*.c` **compiles inside the SDK**, which needs a C compiler in the
build sandbox and the sources present in `cargo-sources.json`. T-17's
`flatpak-build` stage is where that becomes a fact. If it fails, the fix is a
small manifest module adding a `.pc` file — not a redesign, and not that
environment variable.

Each format needs a round-trip test regardless; `test_supported_xz_and_bzip2_streams_extract`
is the gate, and it is now a *build* gate as well as a behavioural one.

**Also decided here.** No `unsafe` in our own code, in this path or any other —
see D-25.

---

## D-25. No-replace renames go through `rustix`, not `libc::renameat2`

**Question.** Python reaches `RENAME_NOREPLACE` through `ctypes`
(`runners.py`). In Rust, `libc::renameat2` is an `unsafe extern` call and the
workspace lint sets `unsafe_code = "deny"`. Options: allow `unsafe` here, use
`rustix::fs::renameat_with(.., RenameFlags::NOREPLACE)`, or use
`nix::fcntl::renameat2`.

**Choice.** **`rustix`.**

**Why.** The cheapest correct answer. `rustix` is *already in our dependency
graph* — libcosmic depends on it (`rustix = "1.1"`, features `pipe`/`process`),
so it is already vendored for the Flatpak and adds no new source to
`cargo-sources.json`. That makes it strictly better than `nix` (a new
dependency for one syscall) on simplicity.

Against `#[allow(unsafe_code)]`, the argument is not purity. This is a *security
boundary* — the no-replace rename is what stops an install from clobbering a
path that already exists — and an `unsafe` block there is precisely the kind of
code that needs an invariant written down and maintained. `rustix` already
encapsulates that invariant and is maintained by people who have thought about
it, so we inherit it instead of restating it. Given the choice between
"we promise this is safe" and "this is safe", take the second.

---

## D-26. The HTTP client is injected into `core`, not imported by it

**Question.** `core::runners` needs to fetch Proton releases. §1.1 permits an
HTTP client in `core` directly.

**Options considered.** (1) Depend on a concrete blocking client (`ureq`) in
`core`. (2) Define an `HttpClient` trait in `core` and inject the concrete
client from the binary crate.

**Choice.** **Option 2 — inject it.**

**Why.** It is the same pattern already used twice in this port: `paths.rs`
injects its `Env`, and blocking work takes `progress: &dyn Fn(f32)` so no async
runtime is needed to test it. A trait keeps all 92 runner tests running
**offline** — `test_security.py` mocks responses, so the tests exercise the
retry, size-cap and error paths without a network — and it keeps `core` free of
a networking dependency, so `cargo test -p gamehandler-core` stays instant and
hermetic.

There is a parity benefit too: an injected client makes the response-handling
logic testable against the same mocked inputs the Python tests use, which is
what makes those 92 tests portable at all. A concrete client in `core` would
force those tests to become network tests or to be dropped.

**Note.** This supersedes §1.1's allowance rather than contradicting it: §1.1
*permitted* a client; it did not require one, and the trait is the better fit
for the reason above.

---

## D-27. `LaunchEnv` is injected into `launch_opts`, for the same reason as `Env` and `HttpClient`

**Question.** `apply_launch_options` (`runners.py:1172`) reads three impure
sources **from inside its own body**: `shutil.which` for `mangohud` /
`gamemoderun` / `gamescope` (which decides argv-wrapping vs. env-fallback, and
*raises* when gamescope is enabled but absent), `find_anticheat_runtime` for
BattlEye/EAC (a filesystem probe), and the DXVK root from
`GAMEHANDLER_DXVK_ROOT` or `/app/share/gamehandler/dxvk`.

The Python tests prove the dependency rather than leaving it to be inferred:
`test_gamescope_wraps_command_and_hdr_flag`, `test_gamescope_missing_fails_*` and
`test_additional_app_uses_runner_before_gamescope_wrapping` all
`mock.patch("gamehandler.runners.shutil.which")`. Python can replace a module
global; Rust cannot. So without a seam, roughly a dozen toggle cases become
untestable unless a real `gamescope` happens to be installed on the build box —
which is precisely the hermeticity D-24 and D-26 exist to protect.

**Options considered.** (1) Call `which` directly and accept that those tests
need the binaries present, or drop them. (2) Inject a trait covering the impure
lookups, as `paths.rs` does with `Env` and `core` does with `HttpClient`.

**Choice.** Option 2 — an injected `LaunchEnv`.

**Why.** Same reasoning as D-26, and the same evidence: the mechanism the
*original tests* use to be hermetic is module mocking, so the port must provide
an equivalent seam or it loses the tests. The alternative is worse than it
sounds — a dozen toggle cases dropped, or made conditional on the build host,
which is how a suite starts passing for reasons unrelated to the code. Keeping
`virtual_desktop_argv` and `normalize_desktop_size` pure and un-injected matters
too: they need no seam, and adding one would make their vectors harder to read
for no gain.

**Shape.** `which(&self, name) -> Option<PathBuf>` and
`anticheat_runtime(&self, kind, roots) -> Option<String>`, real implementation in
`core`, table-driven fake in tests, each vector case carrying its own `which`
spec. This is the third seam of the same kind, so it is the point at which the
pattern should be treated as the crate's convention rather than a series of
one-off decisions: **anything `core` needs from outside the process is injected,
so `cargo test -p gamehandler-core` stays instant, headless and offline.**

---

## D-28. One identifier, five places: `App::APP_ID` is the single source of truth

**Question.** The application has one identity string that has to appear
consistently in several artefacts that are validated by different tools, in
different languages, at different times. Nothing in the build enforces that they
agree. Where is the value defined, and what keeps the copies in step?

**The dependency, traced in the pinned sources rather than assumed.**
`libcosmic/src/app/mod.rs:70` sets

```rust
window_settings.platform_specific.application_id = App::APP_ID.to_string();
```

and `iced/winit/src/conversion.rs:202,213` passes that same string to
`WindowAttributesX11::with_name(class, instance)` and to the Wayland
`WindowAttributesWayland::with_name(...)`. So the X11 `WM_CLASS` (both class and
instance) and the Wayland `app_id` are **exactly `App::APP_ID`**, with no case
folding, prefixing or other transformation.

**Choice.** `App::APP_ID` is the single source of truth, and every other
occurrence is derived from the same literal:

| Artefact | Value | Consumed by | Enforced by |
|---|---|---|---|
| `App::APP_ID` (Rust) | `com.goshapps.GameHandler` | winit → `WM_CLASS` + Wayland `app_id` | `cargo test` |
| desktop file basename | `com.goshapps.GameHandler.desktop` | the shell's launcher | `desktop-file-validate` + Flatpak |
| `StartupWMClass` | `com.goshapps.GameHandler` | window↔launcher association | `desktop-file-validate` (weakly) |
| Flatpak manifest `app-id` | `com.goshapps.GameHandler` | `flatpak-builder`, the sandbox | `flatpak-builder` |
| metainfo `<launchable>` | `com.goshapps.GameHandler.desktop` | AppStream | `appstreamcli validate` |

The manifest key is `app-id`, not `id` — an earlier revision of this table
said `id`, which is the key of the module list's entries, not of the manifest.

**Why this needs writing down.** Each of the five is validated, and *none of the
validators compares them to each other*. `desktop-file-validate` will accept a
`StartupWMClass` that matches nothing; `appstreamcli validate` checks that
`<launchable>` ends in `.desktop` but not that the file exists alongside it;
`flatpak-builder` does not read `APP_ID`. So a mismatch is possible with every
tool reporting success — the failure mode is a running app whose window does not
associate with its launcher icon, which no check in `verify.sh` would catch.

**Consequence — a required check, not a convention.** `verify.sh` must assert
the five agree. The load-bearing ones are `App::APP_ID` vs the desktop basename
vs `StartupWMClass`; these are cheap string comparisons against two files and
should be a test in the workspace rather than a shell assertion, so it runs under
plain `cargo test` too. Assigned to the Packaging owner, who owns `verify.sh`,
coordinating with UX for `data/`.

**Note on the name itself.** The `com.goshapps.GameHandler` ID, the `goshapps`
namespace and the icons are unchanged by this migration — D-02 keeps the app
identity, and changing the ID would orphan every installed user's data
directory. The Qt-era `Categories` line is a separate, cosmetic defect being
corrected under T-18.

---

## D-29. `.webp` covers: enable the decoder, do not transcode on import

**Question.** libcosmic pins its `image` dependency to
`features = ["ico","jpeg","png"]` (`libcosmic/Cargo.toml:139-143`), but
`copy_custom_cover` stores a user's `.webp` verbatim and keeps the suffix
(`covers.py:300-301`). How does the port render a `.webp` cover?

**Options considered.** (1) Enable libcosmic's `animated-image` feature, which
turns on `image/webp`, `image/gif` and `dep:async-fs`. (2) Transcode `.webp` →
PNG on import in `core::covers`, adding `image` as a direct `crates/core`
dependency. (3) Do nothing and lose `.webp` silently.

**Choice.** Option 1 — enable the decoder.

**Why.** The full analysis, with a measured probe, is in `ux.md` §12.4. The
decisive argument is parity, and it is worth restating here because it is not
obvious: **option 2 does not fix the library that exists.** A user's `games.json`
may already point `cover_path` at `<id>.webp`, written by the Python app, which
stays on this branch and shares the data directory (D-02). Transcoding on import
only affects covers imported afterwards and does nothing on the *read* path, so
every already-stored `.webp` stays unrenderable. Only a decoder fixes both. Option
2 also stores different bytes under a different name for the same user action,
breaking a byte-parity property the migration's first priority protects, and
would require inventing a conversion policy for files already on disk.

Option 3 is excluded by R-11 itself.

**Accepted costs, recorded so they are not mistaken for benefits.**
`animated-image` is granularity-locked: it also enables GIF and pulls `async-fs`,
neither of which this app uses. The feature name is a misnomer for our purpose
("codec coverage"), so it is **not** a promise of `AnimatedImage` widgets in the
cover UI. Enabling it requires regenerating `build-aux/flatpak/cargo-sources.json`,
since `async-fs` is absent from it today; `scripts/verify.sh` stage 6 enforces that.

**Cost, measured — and corrected.** This paragraph originally said `gif` was
already vendored and that "`async-fs` is the only genuinely new crate". **That
was wrong**, and the way it was wrong is worth recording: `image`'s optional
`gif` dependency is **unversioned**, while `resvg` pins `gif = "0.13.3"`. Cargo
therefore does not reuse the existing crate — it appends a **second, duplicate**
`gif 0.14.2`. Verified: `Cargo.lock` now holds both `gif 0.13.3` and `gif 0.14.2`,
and the regenerated `cargo-sources.json` vendors both.

So enabling this feature adds **two** crates, not one: `async-fs 2.2.0` and
`gif 0.14.2`, and it means `gif` compiles twice in the sandbox. The lesson is the
mirror image of the one this entry was meant to teach: "it is already in
`Cargo.lock`" is not evidence that a feature costs nothing, because a
*differently versioned* copy is a genuinely new crate. Only `image-webp 0.2.4`
was already locked **and reused**. `gif 0.13.3` was locked and vendored too (via
`resvg`, which `iced_tiny_skia` pulls in for SVG), but cargo will not reuse it
across the version mismatch, so `gif 0.14.2` is a genuinely new crate. An earlier
revision of this sentence said only `image-webp` "was already locked and
vendored", which is false as written and conflates *vendored* with *reused* —
the same conflation that produced the original error. (Corrected on the
Packaging owner's report; both `gif` versions are in `Cargo.lock` and in
`cargo-sources.json`.)

The original prediction about `async-fs`'s transitive dependencies was sound —
`blocking`, `futures-lite`, `async-lock`, `polling`, `fastrand`, `event-listener`,
`piper` and `memchr` were all already locked and none had to be added.

**Verified after landing:** `cargo tree -f "{p} {f}" -p image` →
`image v0.25.10 bmp,gif,ico,jpeg,png,webp`, and `cargo tree -e features -i image`
shows the edge `image feature "webp" ← libcosmic feature "animated-image"`. There
is exactly one `image` package in `Cargo.lock`, which `iced`/`iced_graphics`/
`iced_tiny_skia` also depend on, so feature unification is **mechanically forced**
rather than inferred — a stronger result than the reasoning this entry originally
rested on. `flatpak-build` then compiled the whole set offline with zero errors.

**Two adjacent facts established by the same probe, so they are not re-litigated.**
- The `.ico` path needs **no** fix: `image`'s `"ico"` feature implies `bmp` + `png`,
  so `save_exe_icon`'s artefact already decodes. R-11 should not be read as
  implicating it.
- Decoding sniffs **content**, so PNG bytes in a `<id>.jpg` file work. This is a
  normal outcome (F-N), not an edge case, and it is why extension-based branching
  is forbidden in the cover UI (D-13's constraint, restated in `ux.md` §12.5).

**Verification owed, and it is now narrower than when this was written.** The
unification question is **settled**: `cargo tree -e features -i image` shows the
edge `image feature "webp" ← libcosmic feature "animated-image"`, and there is
exactly one `image` package in `Cargo.lock` that `iced` also depends on, so
`image/webp` cannot fail to reach iced's decode path. That was the reasoning this
entry had to flag as unproven; it is now a mechanical fact.

What remains genuinely unverified is the **end-to-end render**: that a real
`<id>.webp` displays through iced's widget path. The cover UI is T-14 and does
not exist, so this cannot be checked yet, and neither the crate-level probe nor
the `cargo tree` output above is evidence for it — they show the decoder is
reachable, not that a render succeeds. Tracked as T-22, owed once T-14 lands.

---

## D-30. Cover icons are classified by content, not by suffix

**Question.** `bridge.py:313` decides whether a cover is an icon from the file
name (`cover.lower().endswith(".ico")`), and `CoverArt.qml` uses that flag for
three visual decisions (plate or not, inset margin, `PreserveAspectFit` vs
`PreserveAspectCrop`). Does the port copy the suffix test for parity, or decide
from the bytes?

**Options considered.** (1) Copy the suffix test, for exact parity. (2) Decide
by content magic (`00 00 01 00`). (3) Decide by content, but only when the
suffix is `.ico`, as a hybrid.

**Choice.** Option 2 — content only. Found by the UX owner in `view/cover.rs`;
verified independently here.

**Why.** The suffix is a *naming convention* on these files, not a property of
them, and the app's own writers are what make that true: `copy_custom_cover`
(`covers.py:300-301`) keeps a source suffix only from
`{".png", ".jpg", ".jpeg", ".webp"}` and writes **`.jpg` for anything else**.
`covers.py:305` writes real ICO bytes to `<id>.ico`. So the content is the
reliable signal in every direction that matters, and the name is reliable in
only some of them. This is the same rule already recorded for *decoding* in
D-29 and F-N; applying it to *classification* is consistency, not a new
principle.

**Reachability, stated exactly rather than rounded up.** The cover chooser
(`GameFormPage.qml:352`) offers `["Images (*.png *.jpg *.jpeg *.webp)"]` with
**no "All files (*)"** fallback — unlike the executable chooser at `:337` — so
the shipped UI cannot produce an `.ico` under another name. The defect is
therefore **latent in the UI** and reachable only via a script, a test, or a
restored profile. It is fixed anyway: the fix is four bytes of magic number, and
a latent misclassification that changes `fillMode` is a poor thing to carry
forward for parity's sake.

**Consequence for verification, and this is the part that matters.** B-08 is a
B-item with **no hand-walkable reproduction**, which makes it unlike B-01…B-07.
It cannot be verified the way the others are (hand-edit the named file, confirm
the app survives), because no UI path produces the input. Its evidence is meant
to be the `view/cover.rs` unit test — `png-in-jpg`, `ico-honest` vs
`ico-mislabelled`, `fake-ico`, `jpeg-in-png` — with T-19 asked to confirm the
test **fails when `classify` is made suffix-aware**, so it is known to be doing
work rather than passing vacuously.

**Correction, made the same session this was written.** That evidence does not
currently exist. `crates/app/src/view/` is **dead code**: `main.rs` never
declares `mod view;`, so `cargo test -p gamehandler` runs 0 tests from the app's
binary target and the 25 tests in `view/` have never executed. Attempting the
mutation check above is how this was found — the mutation appeared to prove
nothing, and the reason was that nothing ran. The module compiles and all 25
tests pass once the declaration exists (verified in an isolated copy of the
tree), so the tests are real; they are simply not built. Filed as task #21,
owned by Architecture (the declaration) and UX (a `clippy::assertions_on_constants`
lint in `metrics.rs` that the gate catches only once the module is wired).

So B-08's stated evidence is **currently absent**, and the row in `PLAN.md` says
"verified by the unit test" in the same tense as B-01…B-07, which is not true
yet. Left standing as written rather than softened, because the discrepancy is
the point: a checklist item citing a test that does not run is precisely the
defect this conversation keeps rediscovering — a claim whose supporting evidence
is not what it appears. B-08 is not verified until task #21 lands and the test
is observed to fail under mutation.

**Deviation from the reference.** Recorded here and in `FINDINGS.md` F-P. The
port renders an icon stored under a non-`.ico` name on a plate, uncropped; the
Python app renders it cropped, filling the tile, with no plate.

---

## D-31. A SKIP is only a pass when the caller asked for it

**Question.** `scripts/verify.sh` exits on `FAILED[]` alone; `SKIPPED[]` never
affects the exit code. So on a machine with no `flatpak-builder`, stage 7 SKIPs,
stage 10 loses its installed-copies half and SKIPs too, and the script exits 0
having never built or inspected the Flatpak. PLAN.md §9 requires that
"`scripts/verify.sh` passes from a clean checkout". Should a skipped stage be
able to satisfy that, and if not, how is the difference expressed?

**Options considered.**

1. Leave it: a SKIP is a SKIP, the summary already says "a SKIP is not a pass",
   and the reader is trusted to notice.
2. Make every SKIP fatal (exit non-zero whenever anything skipped).
3. Split skips by **cause** and exit non-zero only for the ones the caller did
   not ask for: requested skips (via `--skip-flatpak` / `--skip-smoke`) stay a
   legitimate exit 0; a skip caused by a missing prerequisite exits 3.

**Choice.** Option 3.

**Why.** The two kinds of skip are not the same event wearing different labels:
one is an instruction the caller gave, the other is a capability the machine
lacked. Option 1 leaves the DoD claim satisfiable vacuously — a partial run
prints a note in the middle of a wall of output and returns success, and the
note is exactly the sort of thing that gets skimmed past (this project has
already had to correct several claims whose supporting evidence was not what it
appeared to be; a green exit code is the strongest-looking evidence there is).
Option 2 is the opposite overcorrection: it breaks the fast-iteration loop the
skip flags exist for, by failing a run in which the caller deliberately asked
for less work, and it would make the flag unusable in the one context it is
documented for.

Exit **3** rather than 1, so a reader and CI can separate "a stage failed"
(a defect to fix) from "the run was incomplete" (a toolchain to install).
**Not 2**, because 2 is already the unknown-option usage error at the top of the
same script; reusing it would make `verify.sh --oops` and a partial run
indistinguishable by exit code alone.

The default for `finish_skip`'s `requested` argument is **0** (unrequested,
i.e. the code path that can exit 3) on purpose: forgetting to mark a requested
skip makes a run look incomplete — noisy but safe — whereas the opposite default
would let an unrequested skip disappear inside an exit 0, which is the defect
this decision exists to close.

Precedence: a genuine failure (exit 1) outranks an incomplete run (exit 3).
Observed in the wild during this work — a run with both a failed stage and an
unrequested `flatpak-build` skip exited 1, with the incompleteness reported in
the summary alongside it.

**Amendment (tasks #23/#24): a stage has to be able to say "the caller asked".**
The rule above was stated before any stage could express it. `finish_skip` takes
the `requested` flag, but a stage's only channel to the runner is a number, and
99 and 77 were already spoken for as "this did not run" (→ exit 3). Stage 10 is
the stage that needs the distinction: its installed-copies half is missing
*either* because `--skip-flatpak` was passed (requested) *or* because no
flatpak-builder was ever available (not requested) — the same absent
`build-flatpak/files/`, two entirely different meanings. It now returns **98**
for the first and 99 for the second, and `run_stage` maps 98 to
`finish_skip ... 1`. Without that, an explicit `--skip-flatpak` in a checkout
with no build output reported an *unrequested* skip and exited 3: the script
failing the caller for using the flag exactly as documented, which is the
overcorrection Option 2 was rejected for. 98/99 are a stage's private
convention; the exit code is the interface.

**Interaction with the Flatpak build lock (the sibling change in the same
file).** The lock and this decision answer the same underlying worry — "can this
run's result be believed?" — and they must not contradict each other:

- A lock that **cannot be acquired** is a **failure (1)**, not a skip: stages
  7-10 did not run and it was not the caller's doing or an absent tool's, it is
  a broken lock.
- `flock` **not being installed** is deliberately **neither** a failure nor a
  skip-to-exit-3: every stage still runs, so the run is *complete* and remains
  exit 0. It is un-*serialised*, not incomplete, and it says so in the output.
  Exiting 3 there would state something false — that stages did not run — about
  stages that did.
- The lock converts the specific corruption T-23 recorded (`flatpak-contents`
  reporting SKIP because a concurrent `--force-clean` wiped the tree) into a
  wait. That SKIP was never *this* decision's kind of skip: the caller asked for
  a full verification, so it would have been an unrequested skip and exit 3.
  Both fixes point the same way, which is why they landed together.

**Consequence.** "`verify.sh` passes from a clean checkout" now means every
stage either passed or was skipped by explicit request. A run that could not
verify something says so in its exit code, not only in its prose.

---

## D-32. `mod view;` lands now, behind a `dead_code` allow that T-09 deletes

**Question.** `crates/app/src/view/` (T-14) is finished and carries **44 tests**,
but `main.rs` never declares `mod view;`. Wire it in now, or leave it unwired
until T-09 starts calling the components?

*(Count note, added later: the module carries **45** tests as of `e75454f`, which
added `the_plate_shade_defers_to_core` when `accent_index` moved into core. The
44 above was measured at `3083011` and was true then; every number in this entry
is a measurement at that commit, not a standing claim. The move strengthens the
argument rather than weakening it — 45 assertions run nowhere, not 44.)*

**Options considered.**
1. Leave it undeclared. The module and its 44 tests run nowhere until T-09.
2. Declare it now with a scoped `#[allow(dead_code)]`, removed at T-09.
3. Declare it now and drop the `-D warnings` gate until T-09.

**Choice.** Option 2 — declare it now, with one `#[allow(dead_code)]` on the
`mod view;` line and a comment naming T-09 as its removal point.

**Why.** Option 1 was the initial instinct, and it is wrong for the reason #21
exists: a test that does not execute is not a test. The module's whole
justification is that the row/subtitle/gradient logic moved out of QML into
headlessly-testable code, and under Option 1 that claim is false in the tree —
44 assertions, none of them running, while `cargo test` reports green and 44
tests short. That is the same defect as #21 one layer down, just less visible.

Option 3 trades a real gate for a temporary absence of one. Option 2 costs a
single attribute.

**Measured, not assumed** — this was verified on a pristine `git archive HEAD`
in a scratch tree before the decision was taken, not reasoned about:

- Declaring `mod view;` alone: clippy reports **45 error lines, of which every
  one is `dead_code`** (23 functions, 18 constants, 1 enum, 1 associated-items
  group, plus 2 "could not compile" summaries that are not lints). **Zero
  non-`dead_code` lints** — the module is clean apart from being uncalled.
- Declaring it with the scoped allow: `cargo clippy --workspace --all-targets --
  -D warnings` → **exit 0**.
- `cargo test -p gamehandler --bin gamehandler` → **49 passed, 0 failed**, of
  which **44 are `view::`**. So the tests are real and green, and wiring adds 44
  to a workspace count that currently excludes them.

**The cost, stated honestly.** An `#[allow(dead_code)]` is a suppression, and a
suppression that outlives its reason is how a gate quietly stops gating. The
mitigation is that it is one line at the declaration site, it names T-09, and
**removing it is part of T-09's definition of done** — not a follow-up. If T-09
closes without deleting the attribute, that is a defect in T-09, and T-19's
verification pass checks for it.

**Not decided here.** `accent_of` (`view/cover.rs:198`) returns a constant shade
for every seed, because the hash it needs is `pub(crate)` in core
(`hash.rs:20`, `mod hash;` private at `lib.rs:43`). Every placeholder tile is
therefore the same blue. This is a real parity gap against `accent_index`
(`covers.py:107-110`), it is already documented in the code and pinned by a test
that fails the day it changes, and it is Architecture's to close by exposing
`accent_index` from core's `covers` module. It is recorded here so it does not
read as an oversight in the T-14 diff.

## D-33. Reading a test failure that "cannot happen": two mechanisms, one question

**Question.** A test fails in a way that contradicts the code you just read. How
do you tell a real defect from noise, without discarding genuine findings?

**Context — the rule this replaces.** The lead's first guidance to the advocate
was *"an impossible test failure is probably a stale artifact."* That is **too
broad and it is wrong in the direction that matters**: it treats every
unexplained failure as something a rebuild will clear, so a real defect is
discarded precisely because it looks impossible. It was retracted the same
session, before any finding was acted on under it.

**There are two mechanisms, and they are distinguished by one question.**

| | Stale / foreign artifact | Feature unification |
|---|---|---|
| Reproduces? | No | **Yes** |
| Flips with build scope? | No | **Yes** |
| Cleared by `touch` + rebuild? | **Yes** | No |
| Verdict | Noise | **Real defect** |

**The question is therefore not "is this failure real?" but "does it reproduce,
and does it flip with the build scope?"** Yes to either → real.

**The mechanism, verified.** `cosmic-theme` enables `serde_json/preserve_order`,
and Cargo **feature-unifies** across a build, so the feature is on workspace-wide
and **off** under `cargo test -p gamehandler-core`. `serde_json::Map` is
consequently a `BTreeMap` (alphabetical) in the narrow build and
insertion-ordered in the wide one. `cargo tree -p gamehandler-core -e features -i
serde_json` → not enabled; the workspace tree → enabled. Same source, two
behaviours, decided by how you invoked `cargo`.

**This is not a new discovery — it is a new rule about an old one.** The same
mechanism produced D-21: the oracle's `deep_nesting` fixture **aborted the
process** under a workspace-wide `cargo test` while `cargo test -p
gamehandler-core` passed all 87 tests. It has now bitten a second time, on
`84e3a1d`'s vector case #485 (passing narrow, failing wide), which is what
forced the general rule.

**What is *not* affected — checked, not assumed.** Production serialization is
feature-independent: `json::to_python_string` calls `value.serialize()` on the
typed value rather than materialising a `serde_json::Map`, so `Library::save` and
`Settings::save` write identical bytes in both configurations. The exposure is
test-side `to_value` only. This matters because "the oracle disagrees with itself
across scopes" would otherwise read as a product bug; it is a test-harness bug.

**Consequences, both mandatory.**

1. **Never answer mechanism (a) without testing for mechanism (b) first.** A
   rebuild cannot clear (b), so "just rebuild it" destroys exactly the findings
   that are hardest to rediscover.
2. **A verdict without its build scope is not evidence.** Every test result and
   every mutation verdict must state the invocation — `-p gamehandler-core` or
   workspace-wide — the way it already states the commit. Two configurations that
   can disagree are two gates, and reporting one as though it were both is the
   same defect class as #20, #22 and #29: **a check that passes without
   inspecting what it claims.** Task #27 exists to close the gate; this entry
   governs what to do while it is still open.

**Why this way round.** The costs are asymmetric. A false positive costs a few
minutes of a teammate's time. A swallowed finding ships a defect, and this
particular mechanism is invisible to every intuition we have: the code is
correct in one build and wrong in the other, and the *narrower* invocation — the
one that looks more targeted, more careful — is the one that hides it.

---

## D-34. A report of what passed is not a report of what ran

**Question.** `verify.sh` aborts at the first failing stage unless `--keep-going`
is passed, and the summary printed on that path is built from `PASSED`,
`FAILED` and `SKIPPED` alone. Stages the run never reached appear in none of
those three, so they appear nowhere: a failure at stage 4 printed

```
passed:  build clippy test
failed:  oracle-freshness
skipped: none
```

with `flatpak-build`, `smoke-test`, `desktop-metainfo` and `flatpak-contents`
absent entirely — not listed, not skipped, not mentioned. The lead hit this
himself and read `failed: oracle-freshness` as "only the oracle is broken"; the
Flatpak build and the smoke test had not run at all. What should the report say
about work that was never attempted?

**Options considered.**

1. Leave it. The abort is deliberate and the reader can infer the rest.
2. Print the stages that were not reached as their own category, derived from a
   single ordered list of stages that the script also uses for its usage text and
   for ordering.
3. Print only a banner ("this run is incomplete") without naming them.

**Choice.** Option 2, plus deriving `usage()` from the same array and having
`begin()` refuse a stage that is not next in it.

**Why.** The output was not merely incomplete, it was *misleading*, and it is the
instance that hides every other one: a reader who believes three stages ran and
one failed has been told something false about six stages, and the false part is
the reassuring direction. Option 1 relies on the reader reconstructing the stage
list from the header — which is exactly what the lead, who wrote the header, did
not do. Option 3 names the problem without locating it; "some stages did not run"
sends the reader back to the top of a 200-line scrollback to work out which.

Naming them is cheap because the information already exists: the list was
authoritative at the top of the script and **nothing consumed it**. That is the
same defect as #20, #22 and #29 — *a check that does not inspect what it claims*
— one level up, in the script that exists to catch it. A list nothing reads has
the shape of a check and the behaviour of a comment, so it drifts: the usage text
was restated by hand and the run order was implied by the sequence of `run_stage`
calls, with no comparison between them. `STAGES` is now the one copy, and
`begin()` compares each announcement against it, which turns "somebody edited one
of the three" from a silent inconsistency into exit 2 on the run that introduces
it.

**Why exit 2, and not 1 or 3.** A stage whose `name` is not next in `STAGES` is a
bug in this script, not a stage result. Reporting it as 1 would say "a stage
failed" (there is a defect in the tree); as 3, "the run was incomplete" (a
toolchain is missing). Both are false statements about the tree, and both would
send the reader to the wrong place. 2 already means "the invocation or the script
is wrong" — the unknown-option error uses it, and this is the same class.

**What is deliberately unchanged.** The abort policy. `--keep-going` is
documented and fail-fast on a broken build is correct: the later stages are not
merely slower without it, they are meaningless (the Flatpak build would compile
the tree that just failed to compile). Aborting remains the default; the change
is that the report now says what the abort cost.

**Consequence.** `summary()` prints `did not run (<reason>): <stages>` whenever
`ATTEMPTED` is shorter than `STAGES`, with the reason supplied by the path that
stopped the run — an earlier failure, or an unacquirable Flatpak build lock.
Measured on the two live early-exit paths rather than assumed: the fail-fast
summary above, and the lock path, both name the unattempted stages; a run that
reaches the end prints no such line. The same treatment covers the second way a
stage can be invisible, which is a `STAGES` entry with no `run_stage` call: that
is caught by `run_stage` refusing a function that does not exist (2) instead of
reporting `command not found` as a failing stage, and by `begin()` rejecting the
out-of-order announcements that follow a missing call.

---

## D-35. The CLI is a gate, not a smoke check

**Question.** `gamehandler --list`, `--launch <id>` and `--version` are the
headless surface D-12 requires (the app's own `.desktop` shortcuts invoke
`--launch <GAME_ID>`, and those shortcuts are already on users' disks). Nothing
in `verify.sh` mentioned any of the three flags — the smoke test ran
`--version` and `--list`, but only asserted that they *exited 0*. Where should
the CLI be gated, and against what?

**Context — the defect that made this a question.** Built from HEAD, against a
real two-game library at `$XDG_CONFIG_HOME/gamehandler/games.json`:

```
$ gamehandler --list
GameHandler: the library is empty      <- the library was there, unread
rc=0                                    <- and it claims success
```

`--list` was a stub that printed the empty-library line unconditionally and
never opened the file (task #31; T-07 owns the port). It passed every check that
existed, because the only `--list` check asserted an exit code. That is the
defect class this repository keeps rediscovering — *a check that passes without
inspecting what it claims* — in its purest form: **the stub's output and the
correct output are identical on every input the check used.**

**Options considered.**

1. Add the non-empty assertions to `scripts/smoke-test.sh`, where the other CLI
   checks live.
2. A new `verify.sh` stage that owns its library and asserts the non-empty case.
3. Leave it to the Rust test suite once T-07 lands `list_games`'s own tests.

**Choice.** Option 2.

**Why not 1.** The smoke test's whole design is to exercise the *shipped* app —
it prefers `build-flatpak/`'s `/app/bin/gamehandler` and runs the GUI, which is
why it sits inside the Flatpak lock. A stage that needs a real library and a
temp `$XDG_CONFIG_HOME` has nothing to do with that, and putting it there would
make it contingent on a Flatpak build, which is precisely backwards for a check
whose value is that it runs everywhere. The two are complements and they stay
separate: the smoke test says the packaged binary starts, the `cli` stage says
the verbs behave.

**Why not 3.** A crate's own tests cannot catch a *missing dispatch*. If
`main()` never calls into the library at all — which is the state #31 describes
— there is nothing for a unit test of `list_games` to fail on, because the
function that is wrong is `main`'s, and it is wrong by not being connected. Only
a check that runs the built binary can see that.

**The two assertion choices, both deliberate.**

- **The non-empty case is mandatory, and is compared byte for byte.** This is
  the assertion that would have caught #31, and the empty case cannot substitute
  for it: on an empty library the stub and the implementation print the same
  line, which is why "both cases look identical" was not noticed. The fixture's
  names (`apple`, `Banana`) also make it a sorting trap — `Library::all`'s
  default order is by the *lowercased* name (`gamehandler/models.py:156-163`), so
  a byte-wise comparison of the names would put `Banana` first and fail.
- **`--launch <unknown id>` is asserted on the exit code**, with the reason
  required on stderr. The stub printed the correct reason and returned `1`; the
  asserted property is the status, because a check that only matched the text
  would pass a future implementation that printed the right words and returned
  success — the exact shape of #31 with the sign flipped. A *known* id is never
  launched: that would start a real game through Wine, and a verification gate
  must not have side effects of that kind.

**Consequence.** The stage is red until #31 lands, on purpose. A gate that
passes while the thing it gates is broken is not a gate, and this one is worth
more red than green: the failure names the fixture, the expected bytes and what
was actually printed, so the reader gets the diagnosis rather than the news.

**What this does not cover.** `--launch` on a *valid* id, and the
immediate-failure grace check (P-46) behind it, remain ungated — that path starts
a game and cannot be exercised in a gate. It is named here so the gap is visible
rather than implied by a green run.

---

## D-36. The Windows runner label is a tracked gap, not a resting state

**Question.** `view::meta::runner_label(is_linux, manager_label)` is a pure
passthrough — `"Linux native"` for a Linux game, `manager_label` otherwise — and
`a_windows_game_gets_the_managers_label_verbatim` (`view/meta.rs:126-129`) pins
it, including `runner_label(false, "") == ""`. The manager label reaching it is
`""`, because nothing supplies a `RunnerManager` to the view yet. So **every
Windows game currently renders with no runner**, where Python renders
`"System Wine"`. Is that a defect, and whose?

**Not a defect in the function, and not a stale test.** The passthrough is
correct and the assertion is correct: the function's whole contract is "hand the
manager's label through unchanged", and a unit test of a passthrough *has* to
assert that the input arrives unmodified. Verified against Python rather than
read: `RunnerManager.label` (`runners.py:759-761`) returns `"System Wine"` for a
falsy `runner_id`, and `bridge.py:300-302` resolves the label *before* the row
is built, so Python can never hand this function an empty string.

**The defect is that the empty string is what ships, and no check can see it.**
The test passes in both the fixed and unfixed states, because `""` is a legal
input either way. That is the same shape as D-34's summary: a green result that
does not distinguish the state you want from the state you have.

**Choice — three things, because a note alone has already proven insufficient.**

1. **Closing it is a required step of T-11** (the Runners page), not an optional
   improvement. T-11 is the task that owns the runner catalogue in the view, so
   it is the task that can supply a real label. It is *not* done until a Windows
   row shows its runner.
2. **T-19 confirms it in the running Flatpak** — a real Windows game showing its
   actual runner label, observed, not a unit test asserting a string. This is
   the P-item rule applied to the one place it was most tempting to accept a
   test in its place.
3. **This entry**, so the gap cannot quietly become permanent by being
   locally reasonable on every individual review.

**Why the test stays.** Deleting or weakening
`a_windows_game_gets_the_managers_label_verbatim` would remove the only
statement of the passthrough rule, and T-11 needs that rule intact. The fix is
to *feed* it, not to change it. Read the assertion as a contract, not as
sign-off — the distinction this entry exists to make.

**Measured magnitude — larger than "no runner hint", and the record said too
little.** This entry first described the symptom as "renders with no runner".
That understates it in two directions, both found by the audit's second pass and
verified by the lead at `widgets.rs:387-390`:

```rust
fn subtitle_of(game: &Game) -> String {
    let label = meta::runner_label(game.is_linux(), "");
    meta::subtitle(game.display_category(), &label)
}
```

1. The hardcoded `""` reaches `subtitle` as the *runner* argument, so for an
   uncategorised Windows game the port renders **`""`** — an empty subtitle,
   where Python renders `"System Wine"`. Not a missing hint: nothing at all.
2. It hits **every** Windows game, not only uncategorised ones. For a
   categorised Windows game Python gives `"Action · System Wine"` against the
   port's `"Action"` — so the runner is absent from the subtitle in the common
   case too.

The conclusion is unchanged and the fix is unchanged; only the size of the gap
was wrong. Recorded because "no runner hint" reads as cosmetic and "empty
subtitle on every Windows row" does not — and the difference is what makes this
worth a required step rather than a note.

---

## D-37. The blank-category fold is not a divergence — measured, and why the reading that said it was

**Question.** An audit reported that `view::meta::subtitle` is *stronger* than
the reference: that it folds a blank category to nothing, where `bridge.py`
compares the **raw** `row["category"]` against `"Uncategorized"` and so renders
`"    · System Wine"` for a whitespace-only category — a reachable input,
because `from_dict` does not strip `category`. Should the port keep its own
better answer, and how should that be recorded?

**The premise is wrong: there is no divergence, and the port already matches.**
Measured rather than re-read, because the claim turned on which value reaches
the comparison:

* `bridge.py:309` sets `"category": game.display_category` — and
  `display_category` is a **`@property`** (`models.py:60-62`) that already folds:
  `(self.category or "").strip() or UNCATEGORIZED`.
* So `row["category"]` at the comparison on `bridge.py:319` is the *folded*
  value, never the raw one. A whitespace-only category has already become
  `"Uncategorized"`, the comparison is false, and the f-string is not entered.

Running Python's exact expression over the full input set:

| stored `category` | `display_category` | Python's subtitle |
|---|---|---|
| `""`, `"   "`, `"\t"` | `"Uncategorized"` | `"System Wine"` |
| `"Uncategorized"` | `"Uncategorized"` | `"System Wine"` |
| `"Action"` | `"Action"` | `"Action · System Wine"` |
| `"  roguelike  "` | `"roguelike"` | `"roguelike · System Wine"` |

Every blank renders `"System Wine"` — which is what
`subtitle(display_category(), runner_label)` produces, since
`Game::display_category` (`models.rs:174-181`) folds the same way and
`subtitle`'s `has_category` test then excludes it. The raw value *is*
unstripped in storage, as the audit said; it is simply never the value compared.

**Choice — record the negative result, and keep the code unchanged.** The port
keeps its `.trim()` and its `category != UNCATEGORIZED` guard. They are not a
divergence and not a strengthening: they are one more way to reach the same
answer, and `subtitle` is also reachable from callers that pass a category not
routed through `Game::display_category`, so the guard is doing real work against
a *stray separator* — the `"Shooter · "` case named in its doc comment — rather
than against Python.

**Why an entry for something that did not happen.** This is not an isolated
case this session, and not only the audit's: the lead's own blanket rule that an
impossible test failure is "probably a stale artifact" was retracted the same day
(D-33), and so was the lead's "`verify.sh` matches `--list|--launch|--version`
zero times", which was accurate when grepped and false within the hour because
Packaging had already written the stage. The cost of not recording it is that the
same plausible reading is re-derived, and the next reader is asked to choose
between "preserve our better behaviour" and "match the reference" on a case where
the two never differed. A wrong premise written down is not harmless: it invites a
change to correct code. The measurement is the deliverable.

---

## D-38. A doubled separator in the subtitle is the reference's output, not a defect

**Question.** `subtitle("Action", "Proton-GE · Proton")` yields
`"Action · Proton-GE · Proton"` — two middle dots. It reads like a composition
bug, and it is exactly the kind of thing a later reader "fixes". Is it?

**Yes — it is correct, and it is faithful.** The doubling is produced on the
Python side by two independent `·`-joining steps that both fire:

* `RunnerManager.label` (`runners.py:763-765`) returns
  `f"{runner.name} · {family}"` when a `ProtonRunner`'s family label differs
  from its name — so **one** separator is already inside the runner label.
* `bridge.py:319-321` then composes `f"{row['category']} · {meta}"` around that
  same label — **the second**.

Neither step knows about the other, and the user-visible string is their
composition. The port reproduces it by keeping the separator in the label and
joining again in `subtitle`, so from `("Action", "Proton-GE · Proton")` it
produces the same two dots.

**Choice — keep it, and say so here.** Collapsing the two into one would be a
visible divergence for exactly the Proton runners that have a distinct family,
which is the common case, and it would show up as a subtitle that differs from
Python's on a screenshot comparison rather than in any test.

**The general rule this is an instance of.** A composed display string may
contain a separator contributed by a *component* as well as one contributed by
the *composition*. Faithfulness means reproducing both, so the right question
about a strange-looking delimiter is not "is this tidy?" but "which stage put it
there, and does Python's stage do the same?" — and the answer here is that both
stages do.

## D-39. A green `cargo test` is not evidence about the clippy stage, and the two can disagree on one tree

**What happened.** On a single working tree, measured within the same minute:

```
cargo build  --workspace                          -> exit 0
cargo test   --workspace                          -> 414 passed; 0 failed; exit 0
cargo clippy --all-targets -- -D warnings         -> exit 101  (2 errors)
```

`414` is the sum over four test binaries — 78 (`gamehandler`), 3
(`app_id_lockstep`), 333 (`gamehandler_core`), 0 (doc-tests) — and the
breakdown is stated rather than only the total, for the reason in the next
paragraph. It is also a correction: as first written, this entry reported the
`--workspace` figure as **`312`**, which was `gamehandler_core`'s line alone,
read out of a `tail -35` the author had truncated themselves and then quoted as
the suite. The number was never 312-at-the-workspace-scope; it was one binary
presented as all of them. That is this entry's own subject, committed inside
this entry, which is the strongest available argument that the failure mode is
about the reader and not about any check.

Two lints in `crates/app/src/view/widgets.rs` (lines 970 and 1018),
`clippy::unnecessary_mut_passed` on `&mut renderer` passed to `operate` and
`layout`. Both sites are in test-harness code.

**Why both are true at once.** The flagged code is *valid Rust*. The lint is
clippy-only, and `-D warnings` is what promotes it to an error. `cargo test`
compiles the same target without clippy's lints and runs it green. So this is
not one check being wrong; it is two checks measuring different things on the
same bytes, and the green one is silent about the red one's whole subject.

**This is a new shape for the class.** Every earlier instance in this file is a
check that passes *without inspecting what it claims*. This one is subtler: the
green check is honest and its result is true, and the error is inferring that a
green `test` stage covers the `clippy` stage's territory because they build the
same target from the same source. Same target, same source, different question.
The failure is in the *reader*, not in either check.

**How it was found.** By running both stages rather than one and reasoning about
the other — which is the only technique available, since neither stage's output
contains any hint that the other exists.

**The operational rule, which is the load-bearing half.** The two lint sites are
inside the working tree's *uncommitted* changes to `view/widgets.rs`, owned by
another agent, mid-task, in the exact region they are actively editing. The
correct handling was therefore **not** to report a regression and **not** to
edit the file:

* distinguish *in-flight* (uncommitted, inside the recently-edited region) from
  *landed* (committed) — D-33's discipline, applied to construction rather than
  to authorship;
* route the finding to the owner as information, with the measurement, and say
  explicitly that the file was left alone;
* state plainly that a green `cargo test` is not evidence their work is
  complete.

An agent that treats a teammate's mid-edit tree as a reportable defect will
spend the team's time on states that never existed. An agent that silently
"fixes" another agent's file mid-edit destroys work and hides the problem. Both
are worse than the two `mut`s.

## D-40. A green test whose assertion a *different* mechanism satisfies

**What happened.** A 30-mutation battery over the new `runners::proton`
filesystem code found four survivors. Three were vacuous tests, and all three
had one shape: **a guard that fires earlier produced the error being asserted**,
so the guard under test never ran.

* the declared-length test passed a release size over the cap, so the
  release-size guard fired first;
* the streaming-cap test had the same shape;
* the broken-symlink test asserted an error that `RENAME_NOREPLACE` raises
  anyway — so it passed for a guard that runs *after* the entire download and
  extraction. Same error, same surviving link, download wasted.

The fourth was a false survivor: a mutation that did not compile, counted by the
harness as a survivor as well as a compile error. That is a measurement error in
the apparatus rather than in a test, and it belongs to the same family.

**Why this is a new shape, and not D-33 restated.** Every earlier entry in this
file is a check that passes *without inspecting what it claims* — the assertion
targets an input, or a hand-written copy of the expected value, or nothing at
all. Here the assertion targets exactly the right observable. It is simply
reachable by another route. So the test is honest, the code under test may be
perfect, and the suite is green — while the property named in the test's own
title is not being measured.

**The distinguishing question, which is the whole technique.** For each guard,
ask: *is the code under test the only thing that can produce the asserted
outcome?* If a second mechanism can produce it, the assertion cannot separate
them, and the test reports coverage it does not have. The repair is not a
stronger assertion — it is to make the other mechanisms **unreachable in the
test**, which is why `install_with` now takes the download cap as a parameter:
with the real 2 GiB constant no honest test could reach the streaming guard at
all, so the production path and the tested path are now the same code and the
earlier guard can be made non-firing.

**How it is found.** Only by mutating the guard you believe is under test and
observing that nothing changes — the survivors are the signal, and each one must
then be *explained* rather than merely counted. Two of the four survivors here
were not defects in the code; one was a defect in the harness; three were
defects in the tests. A survivor count alone would have conflated all four.

Also required, and now present: a **no-op control mutation** in the battery that
correctly survives, so the harness is shown to be measuring rather than merely
reporting `caught`.

## D-41. "Does the target directory matter?" has no single answer — it is per-mechanism

**Qualifies the earlier correction.** The lead previously asserted, and then
corrected, a claim about `verify.sh`'s `CARGO_TARGET_DIR="$ROOT/target/verify-narrow"`
in the narrow-configuration stage: first that it was the load-bearing part of the
feature gate, then — after testing — that it is **cache hygiene only**, because
Cargo distinguishes feature sets by `-C metadata` rather than by target directory,
so both configurations coexist in one directory. That correction stands, for
feature unification.

**It is not a general truth about target directories, and this entry exists so the
correction is not over-read.** Found by UX while producing the before/after
evidence for #32: `crates/app` has a `[[bin]]` and no library, so an integration
test in `crates/app/tests/` **does not depend on `src/main.rs` and is not rebuilt
when `main.rs` changes**. Share one `target/` between two checkouts — or point
`CARGO_TARGET_DIR` at another checkout's — and cargo may run the test binary that
was compiled in the *other* checkout, which then reads *that* tree's `main.rs` and
reports on it. Observed, not hypothesised: the pending-pages test passed against a
tree whose Library page had been ported, because it was reading the tree where it
had not. A silent pass is the exact outcome that test exists to prevent.

**So both statements are true and they do not conflict:**

* for **feature unification**, the directory is irrelevant — Cargo encodes the
  feature set in `-C metadata`, and two sets coexist in one directory. Hence
  `verify.sh`'s separate directory really is cache hygiene.
* for **test-binary staleness in a bin-only crate**, the directory is load-bearing:
  the binary is keyed to the source cargo built it from, and nothing in the test
  forces a rebuild from the tree being tested.

**The rule.** "Does the target directory matter?" is not a question with an answer.
The answerable form names the mechanism: *matters for what?* The same reasoning
that produced the earlier over-claim — generalising from one measured mechanism to
the whole behaviour of a build flag — produced this one.

**Guard now in place.** `crates/app/tests/pending_pages.rs`'s `assert_right_tree`
turns both cases into hard errors: a compiled-in `CARGO_MANIFEST_DIR` that does not
match the working directory, and a root that is not a GameHandler checkout, each
fail with the advice the case needs (the second is distinguished from "the tree is
gone" by a three-path probe, as `app_id_lockstep.rs` does).

## D-42. A fail-fast abort names every stage it skipped — and the item that said otherwise was wrong when written

**The disagreement, as it stood.** `VERIFY-FINDINGS.md` §4 carried an open item:
*"`run_stage` → `finish_fail` exits by default, so a failing early stage silently
skips later ones including the Flatpak build and smoke test, while the summary
lists only the stages that ran. Use `--keep-going` for a full picture. Raised with
the advocate: defect or contract?"*

**Decision: contract. And the item's description of the behaviour is false.**

**The measurement.** A stub harness — `verify.sh` copied to `/tmp`, its `STAGES`
list intact, every stage body replaced by `return 0` except `stage_clippy`, which
returns 1 — run to completion:

```
passed:  build
failed:  clippy
skipped: none
did not run (an earlier stage failed): test cli oracle-freshness python-tests
    cargo-sources flatpak-build smoke-test desktop-metainfo flatpak-contents
  these are neither passes nor skips: nothing was verified about them
```

exit **1**. Nothing is silent: the eight stages that did not run are named, the
reason is stated in the summary line itself, and the exit code is non-zero. The
Flatpak build and smoke test are named in that list rather than omitted from it.

**And it was never true.** The same harness run against `50798f9^` — the tree
*before* the #41 fix — produces a byte-identical summary. So this is not a
description that went stale; it described a behaviour the script did not have when
it was written. The mechanism was there the whole time, in `finish_fail`'s
`summary` call before `exit 1`, reading `ATTEMPTED` against `STAGES`.

**Why #41 was not this.** #41 is the *other* path: a run that **finishes** with
stages in `STAGES` never reached. There `summary` computed the unrun list into a
`local` and discarded it, and the exit tail checked only `FAILED` and
`SKIPPED_UNREQUESTED` — so a run that ended early *without failing* reported
nothing and exited 0. That was a real defect with a real fix. The two look alike
from the outside (both print "did not run") and are opposite in mechanism, which is
probably how one item absorbed the other's description.

**Consequence for the record.** The item is kept in §4 rather than deleted, with
the measurement and the note that it was wrong when written. Deleting it would
remove the only place the header's `--keep-going` line is justified. It is also
the third time this project has found a claim that was *never* true being read as
a claim that had merely gone out of date — after `verify.sh:517` (#42) and the
`NOTE(T-03)` `ProtonManager` marker — so §4 now carries an explicit instruction to
re-verify every item at T-19 before restating it.

**The rule.** "Defect or contract?" is settled by running the case, not by taste.
And a residual filed against a script should be filed with the command that shows
it; an unrun claim about an unrun path is how this one survived review.

## D-43. `leaves()` witnesses a box, not a decode — and T-22 splits into evidence and landing

**The finding (#46, found by Packaging, independently reproduced by the lead).**
The widget seam `ee18527` introduced reads a builder's output through `leaves()`
— the laid-out leaf `Size`s under a real `Tree`/`Layout`. That is the right
observable for *which box is where*, and it is not an observable of *whether an
image decoded*. Measured, three inputs through `cover_box` on a HEAD tree:

| input | `CoverSource::classify` | `renderer.measure_image` | `leaves()` |
|---|---|---|---|
| a real 118-byte VP8L `.webp` | `Photo` | `Some(24×16)` | `188.0×218.0` |
| UX's `with_photo` fixture: 12 bytes, PNG magic + `\x00\x00\x00\rIHDR` | `Photo` | **`None`** | **`188.0×218.0`** |
| a path that does not exist | `Plate` | `None` | `80.96001×89.6` |

**Mechanism.** `framed()` puts the image inside a container with
`width(Fixed(188)).height(Fixed(218))`. `Limits::width(Length::Fixed(x))` sets
`min == max == x` (`iced/core/src/layout/limits.rs:60-66`), and `Image::layout`
computes `intrinsic.min(max).max(min)` (`iced/widget/src/image.rs:257-268`) —
which returns `188×218` for **any** intrinsic, including the `Size::ZERO` a
failed decode produces. The fixed box erases the intrinsic entirely.

**What this does and does not say.** `leaves()` *does* separate `Plate` from
`Photo` (row 3 differs), so the seam is not useless. It cannot separate a
`Photo` that decoded from a `Photo` that did not. The consequence is specific:
**the photo branch of `an_icons_picture_is_inset_inside_the_plate_by_the_icon_inset`
asserts `leaves() == [188×218]`, and that is satisfied by bytes that are not an
image at all.** The claim it makes ("a photograph is cropped to the box rather
than inset") is true; the evidence offered for it is satisfied by a broken
photograph. Third instance of the project's defect class in a file that had just
been rewritten to remove the first two — see D-40 and [[verification-defect-class]].

**Decision — T-22 splits, and the halves have different owners.**

* **The evidence half is Packaging's and is complete.** A before/after pair on
  two real trees: with `animated-image` present `measure_image` gives
  `Some(24×16)` and the check passes; with the feature removed it gives `None`,
  the check fails with the disagreement named, exit 1, and the other 53 tests in
  the binary stay green. That is what D-29 said T-22 owed. **It is evidence
  without landing in the repository**, and that is a legitimate place for it to
  live for now: the deliverable is the pair and the fixture, not a merged test.
* **The landing half is UX's** — a `.webp` assertion inside `widgets.rs`'s
  private `mod tests`, because `#28`'s seam is unreachable from
  `crates/app/tests/` (`crates/app` has a `[[bin]]` and no lib, so a second copy
  of the seam would be the only alternative, and a copy can drift from the
  original — the same argument that made `ee18527` destructure the spec instead
  of adding a seam). It lands **with #46's repair**, on `measure_image` rather
  than `leaves`, with the reason in a comment: a `.webp` assertion written the
  way the photo test is written would prove nothing.

**Why not option A.** Packaging offered a narrow exception to edit `widgets.rs`
themselves. Declined: Architecture is mid-write with untracked `runners.rs`,
`installers.rs`, `badge.rs` and `view/mod.rs` in the same directory, and R-13
exists precisely to keep one writer per file. A handover costs one message; two
writers cost a rebase whose conflicts are in the code that has no tests yet.

**The rule.** "The test passes" is a claim about the assertion, not about the
picture. When a check's observable is a *laid-out box*, ask what the container
was told to make that box: a `Fixed` dimension is an assertion the layout
already satisfies, and no intrinsic can move it.
