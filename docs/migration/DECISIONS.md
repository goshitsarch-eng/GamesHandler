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
