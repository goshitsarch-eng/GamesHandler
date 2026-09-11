# GameHandler migration plan: Qt 6/Kirigami → libcosmic (Rust)

**Status: Phase 1 complete. Phase 2 in progress.**
Branch `cosmic-migration`. Baseline: GameHandler 0.7.2 at `cbaf7e6`, 241 tests
passing (1 skipped).

This document consolidates the three Phase 1 teammate docs and the adversarial
review. It is the authoritative task list and acceptance standard.

Source documents:

| Doc | Owner | Contents |
|---|---|---|
| `ux.md` | UX | 84-row UI inventory (several rows cover ID ranges), per-item libcosmic mapping, COSMIC conventions, a11y/i18n |
| `architecture.md` | Architecture | Module mapping, workspace layout, `Message`/`State`/`update`, async, persistence, CLI |
| `packaging.md` | Packaging/QA | Flatpak design, vendoring, finish-args, portals, test plan, `verify.sh` |
| `review-phase1.md` | Devil's advocate | Premise audit, independent 78-item checklist, risk register, kill criteria |
| `DECISIONS.md` | Lead | The 12 decisions made so far, with alternatives and reasons |

---

## 1. Premise correction (recorded first, because everything depends on it)

**The migration brief was wrong.** It described GameHandler as "a desktop app
currently built with GTK4 in [LANGUAGE]", with "GObject subclasses, signals,
callbacks", and shipped with unfilled `[APP NAME]` / `[LANGUAGE]`
placeholders.

At HEAD the app is **Python 3 + PySide6 + QML styled with Kirigami (Qt 6)**,
built with Meson, packaged on the **KDE** Flatpak runtime. The GTK4/libadwaita
stack was deleted in `3c4b735` ("Rewrite the interface on Qt 6 and Kirigami,
replacing GTK/libadwaita entirely", 2026-08-28); five commits and two releases
sit on top of it. `tests/test_packaging.py:90` actively fails the build if GTK
idioms reappear.

**Consequences that shaped this plan:**
- There are no GObject subclasses. The entire UI contract is **one `QObject`**,
  `Backend` in `gamehandler/bridge.py` (1,072 lines): 23 slots, 33 properties,
  16 signals.
- The hard part is **not** UI glue. It is `bridge.py`'s threaded dispatch model
  and `runners.py`'s Wine environment construction — neither of which is
  GTK-related. A plan written against "GTK4" would have budgeted the wrong work.
- The parity target is **GameHandler 0.7.2 as it exists at HEAD**. Confirmed by
  the maintainer (DECISIONS D-01).

---

## 2. Feasibility — what was proven before committing to a plan

Each of these was settled empirically rather than assumed. All were
kill-criteria in `review-phase1.md` §5.

| # | Question | Result | Evidence |
|---|---|---|---|
| F-1 | Does libcosmic build at all here? | **Yes.** 34 crates, 0 errors. | `cargo build` of `examples/application`, exit 0 |
| F-2 | Is the `iced` git submodule a vendoring trap? | **No.** Cargo fetches submodules for git sources. `cargo vendor` → **630 crates**, incl. `iced*`. | `~/.cargo/git/checkouts/libcosmic-*/a401af8/iced` |
| F-3 | Does it run headlessly? | **Yes**, and needs **no GPU**: software rendering via `iced_tiny_skia`. | Spike app ran to timeout (exit 124) with no display server |
| F-4 | Does clippy pass? | **Only at our workspace root.** libcosmic's own workspace fails on `cosmic-config-derive/src/lib.rs:3`. Dependencies get `--cap-lints allow`. | DECISIONS D-08 |
| F-5 | Is there a Rust SDK extension? | **Yes**: `rust-stable//25.08` ships **Rust 1.98.1** (floor is 1.93). | `flatpak remote-ls flathub` |
| F-6 | **Does the full stack build inside the Flatpak SDK cold?** | **Yes.** Rust 1.98.1 in-sandbox, full libcosmic/iced/accesskit build, **45s**. | `flatpak run --devel org.freedesktop.Sdk//25.08` |
| F-7 | Can the Wine BaseApp be kept? | **Yes** — `stable-25.08` already sits on Freedesktop **25.08**, the runtime we are moving to. No mismatch. | `org.winehq.Wine.yml` |
| F-8 | What happens with no display? | **Gap found.** iced **panics** (exit 101, `Create event loop: NotSupported` at `iced/winit/src/lib.rs:92`). The Qt app prints a friendly hint. **Fix required** — parity + testability. | Spike run B |

**Verdict: the migration is feasible, and the four kill-criteria gates in
`review-phase1.md` §5.3 are cleared** — brief reissued (D-01), core test
strategy defined (§7), cold-cache SDK build proven (F-6), no-GPU path
demonstrated (F-3).

### 2.1 The consequence of F-3 for the whole design

libcosmic's `wgpu` feature is **opt-in** and is *not* in its default set. The
default resolves to `iced_tiny_skia` — **CPU rasterization**. This is a major
de-risking lever and it reverses the packaging doc's assumption that the app
renders "via wgpu":

- The headless smoke test does not need lavapipe, a Vulkan loader, or a GPU.
- A user with no working GPU does not get a blank window or a panic.
- The Flatpak needs no `--device=dri` *for rendering*.

**Decision D-11 (renderer): ship the software path as the default.** Keep
`wgpu` off unless a concrete need appears. Rationale by the project priority
order: it protects parity and sandbox correctness, and it is the simplest thing
that works. Revisit only with evidence that CPU rasterization is too slow.

**Precision (verified in-tree after T-01).** The *renderer* is software, but the
**`iced_wgpu` crate is still compiled**. libcosmic's `wayland` feature enables
`iced_wgpu/wayland` without the `?` that would make it conditional, so the crate
enters the dependency graph regardless. Checked with
`cargo tree -e features`:

```
iced_renderer feature "tiny-skia"     -> present
iced_renderer feature "wgpu"          -> ABSENT (count 0)
libcosmic feature "iced_wgpu"         -> present      (via `wayland`)
```

So D-11 holds at **runtime** — nothing initialises a GPU device — and the
earlier phrasing "the wgpu tree is not in the build" was wrong. The distinction
matters for exactly one thing: build time. It has **no** packaging consequence,
since `cargo-sources.json` must cover the crate either way (D-09 anticipated
this). Recorded so nobody later mistakes a compiled-but-unused crate for an
active GPU path.

---

## 3. Deliverable shape

```text
Cargo.toml                      # workspace
crates/
  core/                         # gamehandler-core: pure logic, NO GUI deps
    src/{models,settings,paths,runners/*,installers,covers,exe_icons,
         netpaths,plugins,credits}.rs
    tests/                      # integration tests + Python-written fixtures
  app/                          # gamehandler: libcosmic UI + CLI
    src/{main,state,view/*,tasks}.rs
flatpak/                        # new manifest + cargo-sources.json
scripts/verify.sh               # single entry point
docs/migration/                 # this plan and its sources
```

**Hard rule:** `crates/core` must not depend on `libcosmic`, `iced`, or any GUI
crate. It is what keeps the port's logic testable on a headless machine, and it
is enforced mechanically (D-03).

---

## 4. Feature parity checklist

Combines the UX inventory (`ux.md`) with the advocate's independent list
(`review-phase1.md`, P-01…P-78) and the packaging plan. The advocate's
numbering is kept as the acceptance standard. `ux.md` item IDs are cited where
they add detail.

Legend: **☐** not started · **~** in progress · **☑** done+verified

### A. Library
| ID | Item | Source | Verify |
|---|---|---|---|
| P-01 | Grid view with cover tiles | ux L8/L9 | 3 games render as tiles |
| P-02 | List view: thumbnail, subtitle, last-played | ux L10 | toggle shows "Category · Runner · Played X" |
| P-03 | Grid/list toggle persisted | ux L5, T2 | toggle, restart, retained |
| P-04 | Search matches name **or** category | ux L2 | search a category term finds the game |
| P-05 | Category filter ("All" + categories) | ux L3 | filter narrows list |
| P-06 | Sort: name / recently played / added | ux L4 | orders match Python exactly |
| P-07 | Blank category → "Uncategorized", sorted last | ux L3 | empty-category game lands last |
| P-08 | Empty-library placeholder, 3 onboarding actions | ux L6 | fresh profile |
| P-09 | No-results placeholder + clear filters | ux L7 | search gibberish |
| P-10 | Double-click plays | ux L8/L10 | launches |
| P-11 | Right-click context menu (9 items) | ux L11 | all items present |
| P-12 | Play / Edit / Find cover art | ux L11 | each works |
| P-13 | Winecfg / Winetricks / Open prefix (Windows only, disabled for Linux) | ux L11 | disabled state on Linux game |
| P-14 | Create desktop shortcut | ux L11 | `.desktop` appears and launches |
| P-15 | Remove with confirm; prefix left on disk | ux L12 | prefix survives removal |
| P-16 | "Last played" labels incl. future-timestamp clamp | ux L10 | clock-skewed timestamp |
| P-17 | Runner label per row | ux L10 | correct per runner |

### B. Add/Edit game form
| ID | Item | Source | Verify |
|---|---|---|---|
| P-18 | Add (Ctrl+N) / Edit; Save gated on non-blank name | ux F1 | blank name refused |
| P-19 | Windows vs Linux-native; Linux disables runner/prefix/Wine toggles | ux F3/F28 | switch type |
| P-20 | Executable browse (`*.exe`), auto-fill name from basename | ux F4 | browse populates name |
| P-21 | `smb://` / `file://` URLs resolved to local paths on save **and** browse | ux F4 | paste share URL → GVFS path stored |
| P-22 | Launch args, working directory | ux F5/F6 | quoted args survive |
| P-23 | Editable category combo (preset + custom) | ux F7 | new category appears in filter |
| P-24 | Cover preview + Find cover + custom import (png/jpg/jpeg/webp) | ux F8 | lookup + import |
| P-25 | Steam genre auto-categorises when Uncategorized | ux F8 | lookup sets category |
| P-26 | Runner picker + optional prefix (empty = per-game isolated) | ux F9/F10 | prefix auto-created |
| P-27 | All 15 per-game toggles | ux F11-F25 | each affects launch env |
| P-28 | Virtual-desktop size, validated, default 1920x1080 | ux F19-F25 | garbage input falls back |
| P-29 | Additional app launched alongside | ux F26 | both processes start |
| P-30 | Custom `KEY=value` env, overrides toggles, quoting/`;` | ux F27 | `FOO="a;b"` intact |
| P-31 | Auto cover fetch on save when empty | ux F29 | toast confirms |

### C. Runners
| ID | Item | Source | Verify |
|---|---|---|---|
| P-32 | Installed list: System Wine (or "Not installed") + downloaded builds | ux R3 | rows correct |
| P-33 | 8 families + System Wine guide row, maintainers, homepages | ux R5/R8 | 9 guide cards |
| P-34 | Per-family release list (≤12): tag, asset, MB, Installed chip | ux R7 | select family |
| P-35 | Download with progress, busy guard, completion toast | ux R2/R7 | bar moves, toast |
| P-36 | Release asset filtering per family | — | CachyOS excludes `v3`/`znver4` |
| P-37 | Removal with confirm; games fall back to System Wine | ux R9 | remove in-use runner |
| P-38 | Secure extraction: traversal/symlink confinement, caps, no-replace rename | — | port `test_security.py` |
| P-39 | Tags sanitised to collision-resistant install ids | — | `release/v1` → hash form |

### D. Launch semantics
| ID | Item | Source | Verify |
|---|---|---|---|
| P-40 | Proton via `umu-run` (+`PROTONPATH`/`GAMEID`/`STORE`); plain-wine fallback | — | with/without umu |
| P-41 | Proton-only gates: NVAPI/FSR/Wayland raise on plain Wine | — | error before spawn |
| P-42 | Esync/Fsync/DXVK-off/VKD3D-off env mapping + dll overrides | — | inspect child env |
| P-43 | MangoHud/GameMode/Gamescope wrapping; gamescope-missing error | — | names Flathub extension |
| P-44 | Anti-cheat runtimes auto-located (incl. Flatpak Steam paths) | — | env set |
| P-45 | Bundled DXVK installed into raw-Wine prefixes once per version | — | dll + marker appear |
| P-46 | **Immediate-failure detection**: ~6s grace, stderr tail, toast + window restore | — | bad exe with close-on-launch |
| P-47 | `mark_played` + "Launching…" toast | — | timestamp updates |
| P-48 | Winecfg/Winetricks use the game's own WINE/WINESERVER | — | GE prefix opens under its own wine |
| P-49 | Open prefix folder (Proton `pfx` aware) | — | opens `…/pfx/drive_c` |
| P-50 | Linux-native launch (no Wine env) | — | no `WINEPREFIX` |

### E. Easy installers
| ID | Item | Source | Verify |
|---|---|---|---|
| P-51 | Catalog of 9 store launchers | ux I7 | 9 cards |
| P-52 | Search + Launchers/Apps filter | ux I1 | filter narrows |
| P-53 | Per-install runner choice, isolated prefix | ux I4 | prefix per uuid |
| P-54 | Origin allowlist + magic bytes + Authenticode before execution | — | tampered binary refused |
| P-55 | msi via `msiexec`, exe directly | — | Epic uses msiexec |
| P-56 | Wizard wait: poll, wineserver slicing, 6h ceiling, profile/case fallbacks | — | port `test_installers.py` |
| P-57 | **Fallback "Locate exe" dialog**; cancel keeps prefix | ux S15 | close wizard early → dialog |
| P-58 | Finished install → library entry with exe-icon cover + Play toast | ux S14 | toast offers Play |
| P-59 | Concurrent-install guard | ux I3 | second refused |

### F. Covers
| ID | Item | Source | Verify |
|---|---|---|---|
| P-60 | Steam search → scored match → portrait cover, CDN fallbacks | ux V1 | "Steam" must not match store art |
| P-61 | Offline exe-icon fallback; launchers get vendor icon | ux V1 | airplane mode |
| P-62 | Initials plate, 8 stable gradients, letterboxed icons | ux V1/V2 | stable across restarts |
| P-63 | Download size caps, empty-download rejection, atomic writes | — | oversized refused |

### G. Shell, settings, plugins, credits, CLI
| ID | Item | Source | Verify |
|---|---|---|---|
| P-64 | Plugins: 5 helpers, 3 states; Flatpak never offers host install | ux P1/P2 | unavailable in Flatpak |
| P-65 | About & Credits: 5 sections, licenses, links, Gosh identity | ux C1-C6 | README regenerable |
| P-66 | Settings: theme, grid/list, default runner, 13 toggles, close-on-launch | ux T1-T5 | persisted |
| P-67 | Theming; **light/dark/system** | ux V3, T1 | see D-13 below |
| P-68 | Shortcuts Ctrl+N / Ctrl+F / Ctrl+, / Ctrl+Q | ux §10 | each works |
| P-69 | Toasts for every mutation; install toast has Play action | ux S13/S14 | Play button |
| P-70 | `--list` / `--launch ID` / `--version` work headless | ux S18 | no display, no GPU |
| P-71 | Desktop shortcuts `gamehandler --launch <id>` | ux S18 | file valid, escapes `%`/newline |
| P-72 | Network-share games via GVFS; unmounted → instructional error | ux S15 | unmount and launch |
| P-73 | Corrupt `games.json`/`settings.json` tolerance + atomic saves | — | garbage file, no crash |
| P-74 | Timestamp normalization (bool/string/negative/NaN/Inf) | — | inject `"added": "soon"` |
| P-75 | Unknown JSON keys ignored (forward compat) | — | add `legacy_field` |
| P-76 | Categories list with Uncategorized last | — | order check |
| P-77 | Duplicate game ids: last write wins | — | duplicate entries |
| P-78 | Flatpak: Gamescope extension PATH, Steam read-only path, Wine BaseApp | pkg §3 | documented install works |
### Non-regression: bugs the Python app has, which the port must NOT copy
| ID | Item | Source | How to verify |
|---|---|---|---|
| **B-01** | A wrong-typed scalar in one entry does not break the library (D-18) | F-I | `{"name": 123}` in `games.json`; app opens, all three sorts work |
| **B-02** | `added: null` does not make `recent`/`added` sorts throw (D-14) | F-B | hand-edit `"added": null`; no crash in any sort |
| **B-03** | `settings.json` with an invalid byte does not prevent startup (D-20) | F-J | inject `0xff`; app opens with defaults |
| **B-04** | A BOM'd or deeply-nested `games.json` loads instead of being emptied (D-21) | F-K/F-L | BOM'd file whose games survive the next save |
| **B-05** | Timestamps survive a save unchanged at the bit level (D-19) | F-H | save a library the Rust app loaded; bytes match Python's |
| **B-06** | A pathologically deep `games.json` cannot abort the process (D-21) | F-L | deep nesting must be a parse *error at worst*, never SIGABRT |
| **B-07** | A failed launch reports the runner's actual error text, never the generic status line (F-O) | F-O | launch a game whose runner exits non-zero within the grace period; the message names the cause, on every attempt |
| **B-08** | Whether a cover is an *icon* is decided by its bytes, never by its filename suffix (F-P) | F-P | not walkable in the UI: the chooser (`GameFormPage.qml:352`) has no "All files (*)", so no UI path stores an `.ico` under another name. Evidence is meant to be the `view/cover.rs` unit test (`png-in-jpg`, `ico-honest` vs `ico-mislabelled`) — **but that module is currently dead code (task #21), so this item is NOT verified yet**. See D-30's correction |

A separate category from the parity checklist above, and not to be confused with
section H below. Every P-item says *"behave like the Qt app"*; every B-item says
**"do not"** — each corresponds to a defect verified by running the real Python
implementation (`oracle/FINDINGS.md` §1 and §4).

They are checklist items rather than footnotes because *"the port does not
reproduce this bug"* is a claim worth nothing unless it is demonstrated in the
running Flatpak. Asserting it in a document, or showing it passes a unit test,
does not establish it — T-19 walks each one by hand-editing the file named in
the verification column and confirming the real app survives.

B-01, B-02 and B-03 are the ones a user can hit **without doing anything
unusual**: no setting needs changing and no exotic input is required, just a
`games.json` or `settings.json` that something other than this app wrote.
B-05 is the one that is silent — it produces no crash at all, only bytes that
slowly drift. See R-12.

**B-06 is listed separately because it is not a Python bug the port declined to
copy — it was introduced by this migration's own first attempt at D-21.** The
port briefly used `serde_json`'s recursion-limit escape hatch, which overflows
the stack and aborts on input Python reads fine, and the abort was invisible to
`cargo test -p gamehandler-core` while killing a workspace-wide `cargo test`.
It is a checklist item so that Phase 3 verifies the fix against a real file, not
only against the test that was written alongside it.

**B-07 is the first B-item found *during* the port rather than in Phase 1** (by
the UX teammate, though it belongs to `core::runners`), and the first that is
*load-sensitive* rather than input-sensitive. `LaunchedGame.failure()`
(`runners.py:1358-1374`) calls `process.wait()` and then reads a stderr buffer
filled by a **separate daemon thread** (`_ErrorTail._drain`, `:1322-1345`).
`wait()` returning says the process was reaped, not that the pipe was drained,
so the read can beat the drainer and the real Wine/Proton error is silently
replaced by `"the runner exited with status {code}"`.

Both the UX teammate (12/300) and the lead, independently, (90/300) reproduced
it; the rate is load-dependent, which is why it appeared as an intermittent
`python-tests` failure while a `flatpak-builder` ran alongside. The user-visible
cost is a failed launch that says nothing useful, and the *flakiness* is the
tell: `tests/test_runners.py:761` fails only when the race is lost. A Rust port
that spawns a reader task and awaits the child inherits the same unordered
pairing, so the port must drain **before** reading — `child.wait_with_output()`
does this, reading a collected buffer after `wait()` without joining the reader
does not. The port's output *format* stays identical; only the reliability
changes, which is why a Rust test here must assert the error text is present and
must not accept the fallback string, or it will pass while the race is present —
exactly how the Python suite hid this.

### H. New items (not in the Qt app — added by this migration)
| ID | Item | Why |
|---|---|---|
| N-01 | **No-display startup diagnostic** | iced panics with a raw traceback (F-8); the Qt app printed a helpful hint. Parity + the `--launch` shortcut path must never need a display. |
| N-02 | Explicit non-zero exit + message when the GUI cannot open | Same; scriptability. |

---

## 5. Risk register (consolidated, ranked)

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| **R-1** | `bridge.py` — 1,072 lines of threaded dispatch with **one** test touching it (skipped without PySide6) must be re-expressed in Elm. Highest-cost, lowest-test-coverage item. | **Critical** | Port `test_installers.py`-style injected-clock tests first; add headless state-model tests for P-46/P-56/P-57/P-59 **before** the UI. |
| **R-2** | Moving foundation: git-pinned libcosmic atop `iced@master`, a personal `accesskit` fork, Rust 1.93 floor. | **High** | Pin by rev (D-04); `cargo-sources.json` freshness + git-entry checks in `verify.sh`; MITIGATED by F-6 (SDK build proven). |
| **R-3** | Wine env construction (`runners.py`) is subtle and pinned by **90 tests** (60 `test_runners.py` + 30 of `test_security.py`'s 32 — two of the 32 test other modules: `test_download_filenames_are_reduced_to_a_bare_filename` is `installers.py`'s `safe_download_name`, and `test_cover_downloads_are_bounded` is `covers.py`'s `MAX_RESPONSE_BYTES`): quote-aware env parsing, `pfx` indirection, the **user env block applied twice** (once at `runners.py:1396` so prefix setup can read `WINEARCH`, again inside `apply_launch_options` at `:1258` so it wins over the toggles — *not* `apply_launch_options` itself, which is called once at `:1412`), bounded stderr tail, `RENAME_NOREPLACE`, and bounded tar extraction (**Rust's `tar` crate has no `data_filter`** — must be hand-rolled). | **High** | Port module-by-module against the existing test vectors; `test_security.py` cases are the gate. **Build `archive.rs` first, test-first** — it is the one place a defect is a vulnerability rather than a bug. |
| **R-4** | Sandbox: keep Wine BaseApp, multiarch, GVFS, game hardware. | **Medium** | F-7 (BaseApp compatible). `--device=dri` vs `--device=all` needs a real gamepad test; keep `all` until proven otherwise. |
| **R-5** | Accessibility: the Qt app got a free accessible tree; iced a11y is **per-widget opt-in**. A naive port is *less* accessible than the baseline. | **High** | Every interactive element gets an explicit label; a11y walkthrough is an acceptance gate. |
| **R-6** | i18n: no translations exist today, so nothing to regress — but hardcoded English would mortgage the one clear improvement available. | **Medium** | Route user strings through Fluent `fl!` from the start. |
| **R-7** | Untested paths a rewrite silently breaks: the whole CLI contract, `launch()` integration, `resolve_game_paths` failure text, QML-only logic (exe auto-name, category reset, runner-gone fallback). | **High** | Enumerate in `review-phase1.md` §R-7; each becomes a Rust test. |
| **R-8** | Rust ≠ Python divergences: **`serde_json` rejects `NaN`/`Infinity` that Python accepts** (D-06); byte-compatible JSON output; no locale collation on either side (keep it that way); `f64` precision; URL percent-decoding and SMB mount-name ordering in `as_local_path`. | **High** | Lenient parse + normalization table; round-trip fixtures written by the Python build. |
| **R-9** | Feature-loss candidates: network-share file dialogs (portal returns mounted paths), the Locate-exe fallback dialog (P-57), toast actions, `.ico` letterboxing, global shortcuts, right-click menus, Breeze icon-name mapping onto COSMIC icons. | **Medium** | Each has a P-item; none may be silently dropped. |
| **R-10** | Build time regresses sharply (iced+libcosmic from source). | **Low** | Accept; document. |
| **R-11** | **`.webp` covers will not decode.** Verified at the pinned rev: libcosmic depends on `image` with `features = ["ico", "jpeg", "png"]` only. But `covers.py:300` stores custom covers as `.png`/`.jpg`/`.jpeg`/**`.webp`**. Cosmos's own `animated-image` feature turns on `webp`; if we do not, a user's imported `.webp` cover silently fails to render. | **Medium** | Enable `webp` at T-14, or transcode `.webp` to PNG on import in `core::covers`. Either is fine; silently dropping support is not. |
| **R-12** | **Silent compatibility regressions.** F-H is the proof that this class is real and was invisible to the tests written for it: a green suite passed a change that would have corrupted a fifth of users' timestamps. Any dependency edit that drops `float_roundtrip`, or any future fixture set drawn from round numbers, silently reintroduces it. | **High** | D-19 pins it with bit-level fixtures built from dense random values; the reason is written next to the dependency as well as in DECISIONS.md; `verify.sh` stage 4 fails if the oracle drifts. Treat "the fixtures all pass" as necessary, never sufficient. |

---

## 6. Ordered task list

**Invariant: the workspace builds and `cargo test` passes after every task.**
Each task is one commit. Ownership is by file, so no two teammates edit the
same file (D-05).

| # | Task | Owner | Notes |
|---|---|---|---|
| T-01 | Workspace scaffold: `Cargo.toml`, `crates/core`, `crates/app`, minimal libcosmic app that compiles and runs | Arch + UX | **DONE** (`f76cedd`). Proves F-1/F-3 in-tree. Core has **no** GUI deps — verified with `cargo tree -p gamehandler-core`. |
| T-01a | Compatibility oracle: run the Python impl, freeze its JSON behaviour | Lead | **DONE** (`docs/migration/oracle/`). Blocking input for T-02. |
| T-02 | `core::paths` + `core::models` + `core::settings` | Arch | **DONE** (`7628f23`) — 91 core tests, 242 Python tests, clippy clean at the workspace root. Fixture-verified including D-14…D-22. Was: must satisfy the oracle fixtures — see §6a. **Struct field order must match the Python dataclasses** (31 `Game` fields, 18 `Settings` fields) or byte equality fails. **Gate: D-19** (`float_roundtrip` — without it the round-trip test passes on fixtures and fails on real data), **D-18** (typed scalars), **D-20** (`Settings` UTF-8), **D-21** (BOM, nesting). |
| T-03 | `core::runners` — families, archive extraction, env/launch, desktop shortcuts | Arch | Largest port. Security tests are the gate. |
| T-04 | `core::installers` | Arch | Wizard/poll state machine with injectable clock. |
| T-05 | `core::covers` + `exe_icons` + `netpaths` | Arch | PE parser + GVFS mapping. Cover paths are user-controlled strings — apply the D-18 typed-scalar rule to `cover_path` too. |
| T-06 | `core::plugins` + `core::credits` | Arch | Small. |
| T-07 | `app`: `State` + `Message` + `update()` + CLI (`--list`/`--launch`/`--version`) | Arch | **CLI works headless here** (D-12, N-01). State-model tests land with it. |
| T-08 | `app`: shell — nav bar, page routing, toaster, no-display diagnostic | UX | N-01/N-02. |
| T-09 | `app`: Library page (grid/list, search, sort, filter, context menu, dialogs) | UX | P-01…P-17. Grid virtualization is the risky part (R-9). |
| T-10 | `app`: Game form | UX | P-18…P-31. 15 toggles, 6 sections. |
| T-11 | `app`: Runners page | UX | P-32…P-39. |
| T-12 | `app`: Installers page | UX | P-51…P-59. Depends on the file chooser (P-57). |
| T-13 | `app`: Settings + Plugins + Credits pages | UX | P-64…P-68. |
| T-14 | `app`: cover tiles, pills, async images, theming | UX | P-60…P-63, P-67. **Enable libcosmic's `image`/`webp` support first — see R-11.** |
| T-15 | File chooser via portal; wire P-21/P-24/P-57 | UX + Pkg | **No new dependency** (D-23) — `cosmic::dialog::file_chooser`, already available. Returns a **`url()`**, so handle the URI→path case explicitly for GVFS (R-9). |
| T-16 | Flatpak: new manifest, `cargo-sources.json`, `build.sh` | Pkg | Removes PySide6/llvm21/PYTHONPATH. Keeps Wine base, osslsigncode, DXVK. |
| T-17 | `scripts/verify.sh` + headless smoke test | Pkg | 7 stages per `packaging.md` §6. |
| T-18 | Metadata: desktop file, metainfo, README, version bump to 0.8.0 | Pkg + UX | Version-lockstep test. |
| T-19 | Phase 3 verification pass | Advocate | Walk every P-item against the running Flatpak. Walk **B-01…B-08** — hand-edit the file named for each and confirm the real app survives it. "The port does not copy the bug" is a claim that needs demonstrating, not asserting. **B-08 is the exception to the hand-edit method**: its input is unreachable through the UI (see the B-08 row), so it is evidenced by the unit test, and T-19 should confirm the test exists and fails when `classify` is made suffix-aware rather than attempting a manual reproduction. |
| T-20 | `docs/migration/REPORT.md` | Lead | Final deliverable. |
| T-21 | `image`/`webp`: enable libcosmic's `animated-image`, regenerate `cargo-sources.json` | Pkg | **DONE** (`db9c7c3`, corrected `a449837`). D-29. |
| T-22 | Verify a `.webp` cover actually **renders** through iced end-to-end | UX | **Owed at T-14.** D-29 proves the decoder is *reachable*; it explicitly does not prove a render succeeds, and says neither the crate-level probe nor `cargo tree` is evidence for it. Until this runs, "webp cover render" is unverified. |
| T-23 | Install the app's GPL text and the Microsoft trust root; assert the Flatpak carries them | Pkg | **DONE** (`cad22b2`, corrected `4ddda6c`). Stage 10 (`flatpak-contents`). |

Ordering rationale: logic (`T-02`–`T-06`) lands before UI, so the UI is built
on tested foundations and R-1's control-flow redesign is validated by tests
before pixels depend on it. T-07 proves the headless CLI contract early, which
is what desktop shortcuts rely on.

---

## 6a. Compatibility oracle (complete — the contract for T-02)

`docs/migration/oracle/` holds the Python app's JSON behaviour as executable
fixtures, generated by **running the real Python code** (`gen_oracle.py`) rather
than by reasoning about it. Regenerating is one command, so the contract cannot
drift from the app.

Running it overturned four assumptions the team had written down and found two
real bugs. **Adversarial review of those results then found six more**, including
one that every fixture had passed while a naive port would still have been wrong
on a fifth of real user data. Full detail in `oracle/FINDINGS.md` (§4–5 are the
review findings); the load-bearing consequences:

| # | Finding | Requirement on the port |
|---|---|---|
| F-A | Python saves with `ensure_ascii=True`, escaping all non-ASCII | Match it (D-15) so the round-trip test can be a byte comparison |
| F-B | `"added": null` survives as `None` and makes `sort="recent"/"added"` raise `TypeError` — reachable from the shipped app via `Backend._get_games` → `Library.search(sort=settings.sort_mode)` | **Do not replicate.** Normalize `null` like every other invalid value (D-14) |
| F-C | `42` normalizes to `42.0` | Parse all JSON numbers as `f64` |
| F-D | Unknown keys are dropped on load, not errored on | No `deny_unknown_fields` |
| F-E | Validation fallbacks are fixed literals, and `color_scheme`'s is the hardcoded `"dark"` | Use the same literals |
| F-F | Python renders floats with `%g` exponent rules; `serde_json` uses Ryu (`1e-07` vs `1e-7`) | Byte-equality is bounded; assert numeric equality after reparse where only the exponent spelling differs (D-15) |
| F-G | `serde_json` **rejects** `1e400`; Python parses it to `inf` and normalizes — so a naive port discards the **whole library** | **Lenient number parsing**, saturating outside `f64` range (D-16) |
| **F-H** | **`serde_json`'s default `f64` reader is not correctly rounded** — 4070/20000 realistic timestamps change on a parse→serialize round-trip. Every earlier fixture passed anyway | **`serde_json = { version = "1", features = ["float_roundtrip"] }`** — 0/20000 with it (D-19). Pinned by `floats_roundtrip`, comparing **bits** |
| **F-I** | A wrong-typed `name` (`123`, `["x"]`) is stored as-is, then breaks **all three** sorts including the default; `name: null` drops the game entirely. Wider reach than F-B and needs no non-default setting | **Coerce scalars into their declared types** (D-18). Same class as F-B |
| **F-J** | `Library.load` catches `UnicodeDecodeError`; `Settings.load` does not — so a bad byte in `settings.json` kills the app at startup | **Tolerate and fall back to defaults** (D-20), like `Library` already does |
| **F-K** | A UTF-8 BOM → empty library → the next `save()` writes `[]` **over the user's file** | **Strip the BOM** (D-21). Being more tolerant than Python is deliberate here |
| **F-L** | Python parses 1000-deep nesting; `serde_json`'s limit is 128 and would reject the file | **Skip discarded values without recursing** (D-21) — they are thrown away anyway per F-D |
| **F-M** | `save()` **replaces** a symlinked `games.json` instead of writing through it — silent data loss for a synced library | **No divergence**: keep the atomic temp-and-replace, document the limitation (D-22) |

F-G is the most serious of the *parity* findings: it is precisely the failure
mode D-06 exists to prevent, reached by an input the original analysis missed.
It is a hard requirement, not a polish item.

**F-H is the most serious finding of the whole exercise, for a different
reason.** It is not a parity gap — it is a *silent* one. The fixtures that exist
to catch it all passed, because they used small round values that survive a
1-ULP error; the error needs the dense, arbitrary values the app actually
writes. A green test suite would have shipped a port that drifted every user's
timestamps in the low bits on the first save. The lesson recorded in D-19 is
that a fixture set is only as good as the inputs it dares to use, and that
"the tests pass" was, in this case, evidence of nothing.

F-B, F-I, F-J and F-K are all **bugs in the Python app that the port
deliberately does not copy**, and all four are worth reporting upstream
independently of this migration. D-14, D-18, D-20 and D-21 record the reasoning
for each; F-M is the one inherited defect, listed in REPORT.md instead.

**T-02 is not complete until every fixture in `oracle/fixtures/` passes**, and
`verify.sh` fails if the checked-in oracle is stale relative to the Python app.

---

## 7. Testing strategy

- **Unit tests (inline, in `crates/core`):** every `Message` variant's
  `update()` transition; models/settings persistence incl. corrupt-file arms;
  runner command construction; installer catalog; scoring; `netpaths`;
  credits. Exhaustiveness enforced by a `match` with no wildcard arm so a new
  variant fails to compile until it is handled.
- **Integration tests (`crates/core/tests/`):** each major user flow as a
  scripted `Vec<Message>` applied to initial state, asserting intermediate and
  final state — never pixel matching.
- **Round-trip compatibility:** fixtures written by the *Python* build, read by
  Rust, and rewritten, asserting both directions (D-06, R-8).
- **Security:** every `test_security.py` case gets a Rust equivalent
  (traversal, symlink escape, no-replace atomicity, Authenticode gating,
  desktop-file injection). This is a release blocker.
- **`scripts/verify.sh`:** one entry point, fail-fast, 7 stages:
  build → clippy (workspace root only) → test (with display unset) →
  `cargo-sources.json` freshness → flatpak-builder → headless smoke test →
  desktop/metainfo validation.

---

## 8. Open questions

| # | Question | Resolved by |
|---|---|---|
| Q-1 | Renderer: software vs wgpu | **Resolved (D-11): software default.** |
| Q-2 | `--device=dri` vs `--device=all` | Real gamepad-through-launched-game test (R-4). Keep `all` until proven. |
| Q-3 | Does our crate need libcosmic's `rfd` feature for file dialogs? | **Resolved (D-23): no.** libcosmic ships a portal-backed chooser behind the `xdg-portal` feature we already enable; `rfd` is the non-Linux alternate. No dependency change for T-15. |
| Q-4 | Headless compositor recipe for CI | T-17. Software rendering (F-3) removes the GPU dependency, so this is now about a compositor socket only. |
| Q-5 | Game form: modal dialog vs page | T-10, per `ux.md` R6. |

---

## 9. Definition of done

**Per task:** `cargo build` + `cargo clippy --all-targets -- -D warnings` +
`cargo test` pass · flatpak-builder succeeds and the smoke test passes · the
relevant P-items are ticked with a note on how each was verified · the
advocate has reviewed the diff and signed off · committed with a descriptive
message.

**Project:** every P-item ticked and verified in the **running Flatpak** ·
`scripts/verify.sh` passes from a clean checkout · Phase 3 verification found
no remaining failures · `docs/migration/REPORT.md` written.

### D-13 (open, needs a decision at T-13)
`ux.md` R4 flags a genuine conflict: the README advertises **dark by default**,
but COSMIC apps follow the system theme and forcing dark fights the toolkit.
The proposal is to default to the system theme while keeping explicit
light/dark overrides (the app already has a "Match system" option). This is
user-visible, so it will be decided at T-13 and recorded here, not silently.
