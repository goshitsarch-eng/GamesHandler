# Baseline

The state the audit started from, and the evidence for each claim. Everything
here was measured or read directly in this tree; nothing is inherited from a
prior report, because this project has already been burned twice by a recorded
"verified" that was not.

## Tree and toolchain

| | |
|---|---|
| Commit the audit branched from | `d56782d` (`T-19: live Flatpak walk verdicts, REPORT, R1 gate fixes, walk harness`) |
| Audit branch | `audit-hardening`, created from `d56782d` |
| Default branch / remote | `main` → `origin` (`github.com/goshitsarch-eng/GamesHandler`) |
| rustc / cargo | 1.98.1 (Fedora 1.98.1-1.fc44) |
| libcosmic pin | `a401af8b1c54a8abd393b8c5b7c8809402f83850`, version 1.0.0, `edition = "2024"`, `rust-version = "1.93"` |
| Renderer | `iced_tiny_skia` — CPU/software. `iced_wgpu` is compiled but never initialised. |
| Working tree at branch time | clean, except the two harness files committed in this audit's first two commits |

Workspace layout (`DECISIONS` D-03): `crates/core` is `gamehandler-core`, pure,
synchronous and GUI-free; `crates/app` is the `gamehandler` binary, holding the
libcosmic UI and the headless CLI (`--list` / `--launch <id>` / `--version`,
dispatched before any window or event loop, D-12).

Source sizes at baseline, for the architecture finding about oversized modules:

| File | Lines |
|---|---|
| `crates/app/src/main.rs` | 9343 |
| `crates/core/src/installers.rs` | 4265 |
| `crates/core/src/runners/proton.rs` | 2777 |
| `crates/app/src/view/runners.rs` | 2614 |
| workspace total | 56508 |

## `scripts/verify.sh`

The single verification entry point, 2125 lines, covering build, clippy, doc,
test, cli, oracle-freshness, python-tests, cargo-sources, flatpak-build,
smoke-test, desktop-metainfo and flatpak-contents. It had never been run to
completion before this audit.

**Run on `audit-hardening` at `eb2c47f`: all stages green.**

```
passed:  build clippy doc test cli oracle-freshness python-tests
         cargo-sources flatpak-build smoke-test desktop-metainfo
         flatpak-contents
failed:  none
skipped: none
```

Nothing was skipped, so nothing here rests on a stage that quietly declined to
run. The timings are worth recording because they set the floor for any future
run: `build` 13s, `clippy` 3s, `doc` 3s, `test` 80s (756 tests), `cli` 0s,
`oracle-freshness` 1s, `python-tests` 2s, `cargo-sources` 1s, `flatpak-build`
**255s** (from a cold `--force-clean`), then the smoke test and the two
validator stages.

The smoke test drives the *installed* Flatpak (`mode=installed
app=/app/bin/gamehandler`), not a build-tree binary: CLI `--version`, `--list`
with no display, the no-display diagnostic on exit 1, and then a headless
weston compositor on which the GUI is required to stay alive for 8 s and to die
cleanly on `SIGTERM`. `flatpak-contents` then confirms the artefact carries what
no validator looks at — the licence text byte-identical to `LICENSE` (35149
bytes), the Authenticode trust root, and the three metadata installs.

This is the true baseline: **the tree is green before any finding is filed**, so
every fix in this audit is measured against a passing gate rather than against a
broken one.

## Harness defects found and fixed before any audit measurement

Both committed at the head of this branch, because every later measurement
depends on them.

**`e07854c` — `scripts/walk/vptr-hold.c`.** Four defects, all the same class the
project already names: *a check that passes without inspecting what it claims.*

1. The keymap guard returned `0` when the keymap file could not be opened, while
   every caller runs `"$VPTR_BIN" -c ... >/dev/null 2>&1 || die "..."`. With
   stderr discarded and exit 0, **every one of those `|| die` guards was
   unreachable** — a keymap that failed to load left the client with no keymap,
   every later keycode meant nothing, and the harness reported the keys as
   delivered. Now returns non-zero.
2. Motion and press shared a frame, so the press could be resolved against the
   *pre-move* cursor position — the origin the tool parks at. The symptom is a
   click that is genuinely delivered (the compositor moves focus, so every
   delivery probe passes) and yet activates nothing. A settle between move and
   press is what makes the press land.
3. The pointer and keyboard were handed the same seat argument, which cannot
   work: this seat advertises no capabilities (`WLR_LIBINPUT_NO_DEVICES=1`), and
   a virtual pointer attached to it is accepted and then delivers nothing.
   `zwlr_virtual_pointer_v1`'s seat is `allow-null="true"` so NULL gives a global
   pointer that does deliver; `zwp_virtual_keyboard_v1`'s seat is a plain `object`
   with no `allow-null`, so NULL is a marshalling error that fails the process.
   The two targets are now tracked separately.
4. Added `scroll X Y DY`. `axis` is a version 1 request of that interface (the
   only version 2 addition is `create_virtual_pointer_with_output`, which the
   walk never needs), so it goes out on the existing binding with no version bump.

**`93b6278` — `scripts/parity-walk.sh`.** The `scroll` verb and its dispatcher
wiring; `probe_keyboard`/`probe_pointer`/`ensure_vptr`; the focus helpers.

## Corrected verdicts

Two recorded findings did not survive re-measurement. Both are recorded here
rather than quietly dropped, because the reasoning failure is more instructive
than the bug.

### The "Add/Save closes the form without saving" verdict is withdrawn

`docs/migration/REPORT.md` recorded this as a P0, with "five GUI trials, harness
error excluded three ways (pointer ±1 px, `WAYLAND_DEBUG` delivery proof,
inert-area control)". **All three exclusions test delivery, and delivery was
never broken.** The true cause is layout: in `crates/app/src/view/form.rs` the
action row is the *last* thing pushed into the same `scrollable` body as the
fields, so at 1280×800 the Cancel and Add Game buttons sit below the fold and
every click aimed at them landed on empty background. The harness had no wheel
until `scroll X Y DY` was added; a walk that cannot scroll cannot test a form.

Source read confirms there is no close-without-save path at all: in
`crates/app/src/main.rs`, `self.state.game_form = None;` appears only *after*
`Library::add`/`update` returns `Ok`, and a write error returns a toast without
closing. The arm's own comment already declares the rejection branch
"unreachable from the drawn form", because both the button's
`enabled: nameField.text.trim().length > 0` and `view::form::can_save` gate on a
non-blank name, so `on_press_maybe(save_message(form))` is `None` until a name
is entered.

### A frame-identity map that was wrong

An earlier session recorded md5 `21c32e83…` as "Add-form-open" and `f4d8667c…`
as "Library". Measured this session, the mapping is:

| md5 | Frame |
|---|---|
| `21c32e83…` | **Add Game form open** (the default page after `Ctrl+N`) |
| `d8ab5b6d…` | Add Game form open, sidebar highlight moved |
| `f4d8667c…` | Library page |

`21c32e83…` is the form, not the Library. It was proven by an unambiguous
measurement rather than by eye: with `21c32e83…` on screen, the idle control
(no input at all) reports **0 changed pixels**, and `Ctrl+N` reports **0 changed
pixels** because it replaces an open blank form with an identical blank form.
The Library→form transition is **286141 px**, bbox `x 316..1251 y 93..778` —
exactly the form's modal panel.

## Measured input baseline

Taken on the walk session (`scripts/parity-walk.sh`), 1280×800, one app window.
The idle control is the important row: it establishes that the harness and the
app are stable enough that "0 changed pixels" is a real result and not drift.

| Experiment | Result | Reading |
|---|---|---|
| Idle control, no input between two captures | **0 px** | the harness is stable; 0 px is meaningful |
| `Ctrl+N`, fresh process, first action | **286141 px**, bbox `x 316..1251 y 93..778` | fires, opens the form |
| `Ctrl+Q` | process **died** | fires |
| `Ctrl+,` | **16529 px**, bbox `x 18..281 y 83..314` | fires; the region is the left rail |
| `Ctrl+F` | **0 px** | fires, leaves no visible change |
| `type zz` with the form open | **0 px** | the form takes no literal text input |

Two traps in this table, both of which produced a wrong conclusion before being
caught:

- **`Ctrl+F` is not a control for the other accelerators.** It does not go
  through this port's `shortcuts::shortcut_for` at all: libcosmic's own
  `keyboard_nav::subscription()` matches `Character("f")` with Control
  (`src/keyboard_nav.rs:50-55`) and `Cosmic::update` routes it to
  `Application::on_search()` (`src/app/cosmic.rs:850`). Using it to reason about
  `Ctrl+N` compares two different code paths.
- **"Ctrl+N does nothing" was an artifact of the form already being open.**
  `Message::OpenNewGameForm` unconditionally assigns a fresh
  `GameForm::new_template(..)`, so replacing an open blank form with a fresh
  blank form renders identically — 0 px, with no defect required.

## Withdrawn: a fabricated observation

Earlier in this audit I reported reading a status bar — `5 games • Ready |
GameHandler 0.8.0` — from GUI captures, and derived a finding from it ("the
frames say 5 games, `games.json` says 3, a discrepancy to resolve"). **The status
bar does not exist.** The `•` character appears nowhere in `crates/app/src/`;
`Ready` occurs only as an unrelated `ReleasesStatus::Ready` variant
(`view/runners.rs:212`); and a 5× crop of the window's bottom-right corner
(`/tmp/gh-walk/shots/bottomright.png`) shows a scrollbar handle, not text. The
workspace version really is `0.8.0` (`Cargo.toml:18`), which is likely what made
the fabrication feel plausible.

`games.json` was correct and unchanged the entire time — md5
`190a26f1facdc8a05cc21fa006cd5c79`, 2403 bytes, **3 games** (Hades; SuperTuxKart;
a 59-character title). There was never a discrepancy.

This is recorded rather than quietly dropped because it is a worse failure than
the one the project already knows about. That one is *a check that passes without
inspecting what it claims*; this one is *inventing the observation itself and
then reasoning from it*, which nothing downstream can catch. The rule adopted
from here on: **text read off an image is not evidence — either grep the literal
string from the source, or use a measurement that cannot be mistaken, such as a
changed-pixel count with a bounding box.** Where a reading and the source
disagree, the source wins and the reading is withdrawn.

Four other frame readings this session were also wrong and were caught by
measurement, including a frame-identity map recorded backwards in two earlier
sessions.

## Open at baseline

Recorded honestly; these are what the audit then works through.

1. `docs/migration/REPORT.md`'s F-ADD entry still asserts the withdrawn verdict.
2. The Add Game form takes no keyboard focus: `Tab` cycles the top nav bar only,
   so there is no keyboard route into the form's fields.
3. With a full-page overlay open, `Ctrl+F`/`Ctrl+,` are still answered and mutate
   navigation state underneath it. `Ctrl+,` visibly moves the rail highlight
   while the body still shows the Add Game form.
4. The long-title grid overflow — a game named "The Curious Expedition of a Very
   Long Game Title That Wraps" renders beyond its tile.
5. `cargo fmt --check` dirtiness, carried as `PLAN.md` T-07.
6. `docs/audit/` did not exist; no branch had been cut.
