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
- **F-ADD (new, P0): the form's Add/Save button closes the form
  without saving** — five GUI trials, harness error excluded three
  ways (pointer ±1 px, `WAYLAND_DEBUG` delivery proof, inert-area
  control). The view code, routing and unit tests all read correctly,
  so the suite is blind to it. P-18's Add half and P-31 stay
  unwalkable until the view owner fixes it.

## Gate

`bash scripts/verify.sh --keep-going`: 10/12 green at `a6166b8`; the
two reds were pre-existing HEAD defects from `b54fe70`, fixed here in
R1 (stale `launcher_command` rustdoc links; T-07c traceability
deferral — the row itself is the Lead's, D-05). Re-run after R1 is
12/12 green; see the commit messages for the per-stage summary.

## Residuals

1. F-ADD blocks all GUI form-save verification.
2. Close-on-launch restore (`minimize(false)` no-op + `gain_focus`)
   is unwitnessed on any compositor.
3. B-06/B-01-class silent coercions hide errors as empty libraries.
4. Long grid titles overflow into the neighbour tile (cosmetic).
5. `scripts/verify.sh` carries uncommitted `--stage-timeout` work from
   an earlier session; it ran as-is and is not part of these commits.
