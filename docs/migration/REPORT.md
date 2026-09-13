# T-19 verification report (2026-09-12)

**Owner:** Advocate (T-19). **Tree:** `cosmic-migration` at `a6166b8`
plus the commits listed below. **Standard:** `review-phase1.md` §2
(P-01…P-78) and the PLAN.md non-regression table (B-01…B-08).
**Method:** source + suite re-derivation in
[`T19-PARITY-WALK.md`](T19-PARITY-WALK.md) §§1–4, then the live Flatpak
walk of §5 — every (B) procedure that can run on this machine, executed
by hand against the installed `com.goshapps.GameHandler` via
`scripts/parity-walk.sh`.

## Verdicts

- **B-01…B-07: all VERIFIED in the running app.** B-01/B-02 survived
  all three sort orders live; B-03 opened on defaults; B-04 loaded a
  BOM'd file and a GUI delete-save kept the survivors while stripping
  the BOM; B-05 timestamps are bit-stable across a save; B-06 deep
  nesting exits 0 without aborting; B-07 reports the runner's stderr on
  6/6 attempts. B-08 stays unit-evidenced per PLAN (unreachable in the
  UI by construction).
- **P-46 walked, with a correction.** Failure toast carries the stderr
  tail; prompt reporting on early exit matches the reference's
  `wait(timeout)`; a 12 s title shows no toast (grace positive case).
  The walk doc's own "~6 s" timing expectation was wrong, not the port.
  The hide half is protocol-traced (`set_minimized` on the wire) but
  visually unwitnessable on sway, which ignores minimise.
- **P-15 flipped to (A)** (confirm dialog + removal + toast + save,
  all live). **P-11…P-14 are half-flipped**: the 8-item card menu
  exists; Edit opens; menu Play / Find cover / prefix tools are
  sighted but unwalked.
- **F-ADD (WITHDRAWN — was filed as P0).** This report recorded "the
  form's Add/Save button closes the form without saving", with harness
  error "excluded three ways (pointer ±1 px, `WAYLAND_DEBUG` delivery
  proof, inert-area control)". **All three probes test delivery, and
  delivery was never broken** — so they exclude nothing, and the
  verdict does not survive them. The real cause is layout: in
  `crates/app/src/view/form.rs` the action row is the *last* thing
  pushed into the same `scrollable` body as the fields, so at 1280×800
  the Cancel and Add Game buttons sit below the fold and every click
  aimed at them landed on empty background. The walk had no wheel until
  `scroll X Y DY` was added to `scripts/parity-walk.sh`; a walk that
  cannot scroll cannot test a form. There is also no
  close-without-save path in the source to begin with: in
  `crates/app/src/main.rs`, `self.state.game_form = None;` runs only
  *after* `Library::add`/`update` returns `Ok`, and a write error
  returns a toast without closing. The arm's own comment already
  calls the rejection branch "unreachable from the drawn form",
  because the button's `enabled: nameField.text.trim().length > 0` and
  `view::form::can_save` both gate on a non-blank name, so
  `on_press_maybe(save_message(form))` is `None` until one is entered.
  **P-18's Add half and P-31 are therefore unblocked** — the instrument
  that could not reach the buttons now can — but they have *not* been
  re-walked, and this report does not claim they have.
- **Two observations withdrawn as fabricated.** This report's own
  author also recorded reading a status bar (`5 games • Ready |
  GameHandler 0.8.0`) from captures. **No status bar exists:** `•`
  appears nowhere in `crates/app/src/`, `Ready` is only the unrelated
  `ReleasesStatus::Ready` variant (`view/runners.rs:212`), and a 5×
  crop of that corner shows a scrollbar handle. `games.json` was
  correct throughout (3 games, md5 `190a26f1…`). Separately, the frame
  identified as "Add-form-open" (`21c32e83…`) was carried with the
  wrong labels through two sessions; measured, it is the open form and
  the Library is `f4d8667c…`. See `../audit/BASELINE.md` for both.

## Gate

`bash scripts/verify.sh --keep-going`: 10/12 green at `a6166b8`; the
two reds were pre-existing HEAD defects from `b54fe70`, fixed here in
R1 (stale `launcher_command` rustdoc links; T-07c traceability
deferral — the row itself is the Lead's, D-05). Re-run after R1 is
12/12 green; see the commit messages for the per-stage summary.

## Residuals

1. ~~F-ADD blocks all GUI form-save verification.~~ **Withdrawn** — F-ADD
   was not a defect. P-18's Add half and P-31 are re-walkable and
   outstanding; the harness can now reach the form's buttons. See the
   verdict above.
2. Close-on-launch restore (`minimize(false)` no-op + `gain_focus`)
   is unwitnessed on any compositor.
3. B-06/B-01-class silent coercions hide errors as empty libraries.
4. **CORRECTED and FIXED under this audit.** This report said "long grid
   titles overflow into the neighbour tile (cosmetic)". The *overflow*
   is false: measured, a card laid out at the cell's width draws its
   name at ≤ the available 188 px
   (`crates/app/src/view/widgets.rs`,
   `a_long_name_does_not_squeeze_the_play_control`, with an
   anti-vacuity check that the name fills more than half the cell).
   `Wrapping::None` clamps the run to its limits, so it cannot spill.
   The real defect underneath was different and is now fixed: the name
   was **clipped with no ellipsis marker** where the reference draws
   `…` (`LibraryPage.qml:186`, `:195`, `:265`, `:272`,
   `elide: Text.ElideRight`). The port had recorded that as
   unreachable — "this iced has no ellipsis" — which was **false**:
   `Text::ellipsize` is real and honoured under `Wrap::None`. Also
   withdrawn: the archived screenshot this row was argued against
   (`docs/images/t19-long-title-tile.png`) **does not exist**; there is
   no `docs/images/` directory. See `../audit/BASELINE.md` item 4.
5. ~~`scripts/verify.sh` carries uncommitted `--stage-timeout` work.~~
   **Resolved under this audit** — committed on `audit-hardening`, and
   `scripts/verify.sh` has since been run to completion.
6. **WITHDRAWN under this audit.** This report claimed "the Add Game
   form takes no keyboard focus. `Tab` cycles the top nav bar only
   (measured Tab-focus boxes at y 32..69), so there is no keyboard
   route into the form's fields, and with the form open `type zz`
   changes **0 of 1024000 pixels**." **The strong claim is false.**
   The y 32..69 boxes are 2 stops out of a 20-stop Tab ring; stops
   12–20 are inside the form body. The measurement that settles it is
   **growth with input length**, not a pixel count, because an empty
   focused field also returns ~50 px (a caret blink) and that confound
   nearly produced a false negative: 8 characters gave ~92 px and 19
   characters ~229 px against that ~50 px caret; Tab stops 12/14/15
   gave 496/512/530 px for a 12-character string; and a form was
   completed **entirely by keyboard** — `Ctrl+N`, `Tab`×12, `type`,
   `scroll` — the scrolled frame showing the typed text with a caret
   after it. What survives is narrower and is not this report's
   finding: the form does not take focus *when it opens*, Tab is not a
   *practical* route (20 stops), and the form's dropdowns and togglers
   are not keyboard-operable because libcosmic's `Dropdown` has
   `operate`/`a11y_nodes` commented out and iced's toggler has no
   `operate` — `../audit/COSMIC-UX.md` UX-01/02/03. See
   `../audit/BASELINE.md` for the full withdrawal.
7. **New under this audit:** with the game form open, `Ctrl+F` and
   `Ctrl+,` are still answered and change `state.page` underneath it —
   `Ctrl+F` via `App::on_search` → `Shell::focus_library_search` →
   `show_page(Page::Library)` (`main.rs:3775`, `:1303`), `Ctrl+,` via
   `shortcuts.rs:149`. Neither clears `state.game_form`, and
   `view_with_overlays` (`main.rs:1466`) returns the form *before*
   consulting the page, so the body keeps drawing the form while the
   sidebar highlight moves. `Ctrl+,`'s rail move was measured (a
   sidebar-region-only capture delta). The port's own comment there
   records the intended model — "each QML layer closes the other" — and
   that mutual clear is implemented only for the two confirm dialogs,
   not for navigation. Source-confirmed; **not** reproduced live
   through to a close. Filed as `../audit/BUGS.md` BUG-46.
