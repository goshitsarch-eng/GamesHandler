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
| **B-08** | Whether a cover is an *icon* is decided by its bytes, never by its filename suffix (F-P) | F-P | not walkable in the UI: the chooser (`GameFormPage.qml:352`) has no "All files (*)", so no UI path stores an `.ico` under another name. **Evidence path is now sound as of `e75454f`** (was blocked by #21: the `view/` module was undeclared, so its tests ran nowhere — the note that stood here was true when written and is now stale). The test is `view/cover.rs::ico_bytes_are_an_icon_whatever_the_file_is_called` (`:300`), and it **calls `classify`**, so it is not an input-assertion. **Kill confirmed by construction, not by assertion:** its `mislabelled` fixture is `fixture("ico-mislabelled", "4.jpg", ICO_PREFIX)` — real ICO bytes under a `.jpg` name — so a suffix-based `classify` returns `Photo` exactly where the test asserts `Icon`. That single fixture is the discriminator, so the argument holds by construction — **and it was then run** (lead, `9b1ac50`): replacing `classify`'s `has_ico_magic(cover_path)` with a `cover_path.ends_with(".ico")` suffix test fails **3 tests** — `ico_bytes_are_an_icon_whatever_the_file_is_called`, `a_file_named_ico_that_is_not_an_icon_is_a_photo` (`left: Icon, right: Photo`, i.e. the V2 direction, exactly as predicted) and `a_file_shorter_than_the_magic_is_a_photo`. The run also confirms the module is live, not dead: `17 passed; 3 failed; 44 filtered out` = all 45 `view/` tests executing. **B-08 VERIFIED.** See D-30's correction |

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
| **R-13** | **The UI is one agent's queue and it is the critical path.** T-09…T-15 (seven tasks: the library grid, the 15-toggle game form, runners, installers, settings/plugins/credits, cover theming, the file chooser) all belong to UX, and as of `e52798c` only T-08 and T-14 have landed. Every other workstream can finish and the project still cannot ship, because the project DoD requires **every P-item verified in the running Flatpak** and the P-items live in these pages. The risk is not idleness — UX has been productive throughout — it is *serialisation*: a single queue for more than half the remaining work, with no second reader. | **High** | Planned rebalance, to be taken when T-03…T-06 closes: Architecture's core queue empties before UX's does, and T-11/T-12/T-13 (list-and-form pages over already-tested core APIs) are the natural handover — the same shape as T-07's CLI work, which is app-layer plumbing over landed core. **Within the four-person team; do not add a fifth member.** Sequence so that exactly one agent owns `main.rs` at a time: Architecture and UX were both editing it concurrently on 2026-09-11 (`main.rs` 232 insertions in flight against UX's `widgets.rs`), which is survivable only because the changes happen to be in different regions — that will not hold once T-09 starts editing `view()`. **Decided, as of `a04b841`: **UX owns `main.rs`; Architecture owns `crates/core`.** The concurrency that prompted this has already resolved itself — Architecture's in-flight changes are now entirely under `crates/core/src/runners/` and `oracle_tests.rs`, and `main.rs` is unmodified in the working tree, so T-09 can rewrite `view()` without a second editor. The rule stands for the rest of the migration: exactly one agent edits `main.rs` at a time, and the current holder is UX. **The rebalance is now the live question.** Architecture's remaining queue is #36 and #37, both now **closed** (`e16dd5f`), leaving its workstream empty while UX still holds T-09…T-15 and Packaging has taken #32. **DECIDED AND TAKEN, `d797676`: T-11 (Runners) and T-12 (Installers) are reassigned to Architecture, effective immediately.** My earlier trigger ("wait until T-09 settles the nav model") was based on a coupling assumption I checked and found wrong, so it is superseded rather than met:

- **`view_body` is a free function over `&State`** (`main.rs:944`), not a method on `App` — so a page's view is already decoupled and needs no access to the shell or the nav model.
- **The message vocabulary for both pages already exists**: `FetchReleases`, `ReleasesFetchFinished`, `InstallRunner`, `RunnerProgress`, `RunnerInstallFinished`, `UninstallRunner`, `SetInstallerSearch`, `SetInstallerCategory` are all declared (`main.rs:428-450`). T-11/T-12 add `view()` and `update()` logic, not `Message` variants, so #32/#33's restructuring of `Shell`/`update` does not touch them.
- **Architecture is the right owner on knowledge grounds**: both pages are thin views over `runners::proton`'s free functions (`install`, `uninstall`, `fetch_available`, `is_installed`, `resolve_staged`) — the API Architecture just built and the only agent that does not have to re-derive.

**The collision surface is bounded and named.** Each page is a new file (`view/runners.rs`, `view/installers.rs`) exposing `view(&State) -> Element<Message>` and an update entry point; **Architecture does not edit `main.rs`**, and hands UX the mechanical wiring diff (a `view_body` arm plus the page's `update` arms) to apply in `main.rs`, which UX owns and is mid-edit in. If a page turns out to need a new `State` field, Architecture requests it from UX rather than adding it. **The residual risk is visual consistency, not git:** Architecture must build on T-14's components (`cover.rs`, `meta.rs`, `metrics.rs`, `widgets.rs`) and follow `ux.md`, not invent widgets — a second visual idiom is the one outcome that would cost more than the serialisation it avoids. T-13 (Settings + Plugins + Credits) stays with UX: it is three pages over one state struct, and splitting it buys less than splitting the two list pages. |

---

## 6. Ordered task list

**Invariant: the workspace builds and `cargo test` passes after every task.**
Each task is one commit. Ownership is by file, so no two teammates edit the
same file (D-05).

| # | Task | Owner | Notes |
|---|---|---|---|
| T-01 | Workspace scaffold: `Cargo.toml`, `crates/core`, `crates/app`, minimal libcosmic app that compiles and runs | Arch + UX | **DONE** (`f76cedd`). Proves F-1/F-3 in-tree. Core has **no** GUI deps — verified with `cargo tree -p gamehandler-core`. |
| T-01a | Compatibility oracle: run the Python impl, freeze its JSON behaviour | Lead | **DONE** (`docs/migration/oracle/`). Blocking input for T-02. |
| T-02 | `core::paths` + `core::models` + `core::settings` | Arch | **DONE** (`7628f23`) — 91 core tests, 242 Python tests, clippy clean at the workspace root. Fixture-verified including D-14…D-22. Was: must satisfy the oracle fixtures — see §6a. **Struct field order must match the Python dataclasses** (31 `Game` fields, 18 `Settings` fields) or byte equality fails. **Gate: D-19** (`float_roundtrip` — without it the round-trip test passes on fixtures and fails on real data), **D-18** (typed scalars), **D-20** (`Settings` UTF-8), **D-21** (BOM, nesting). **Traceability (#66):** this task is the real owner of **P-73…P-77** (corrupt `games.json`/`settings.json` tolerance, timestamp normalisation, unknown-key tolerance, `Uncategorized`-last ordering, duplicate-id last-write-wins) — all five are implemented and tested here (`core::json` 847 lines/17 tests, `core::models` 1239/28; e.g. `unknown_keys_are_dropped`, `invalid_timestamps_regenerate_added_and_zero_last_played`, `a_boolean_is_never_a_timestamp`), and no task row named them until #66. |
| T-03 | `core::runners` — families, archive extraction, env/launch, desktop shortcuts | Arch | Largest port. Security tests are the gate. **Traceability (#66):** this task's `runners` port is the owner of **P-72**'s core half (GVFS network-share resolution — `launch_opts.rs`, test `a_share_url_is_mapped_onto_its_mount`, and the unmounted-share instructional error); the app-side rendering of that error is T-29's. |
| T-04 | `core::installers` | Arch | Wizard/poll state machine with injectable clock. **THIS IS THE CRITICAL PATH as of `1f8baea`, and it is the only one left.** Traced end to end: **T-12 cannot be wired without it** — `view/installers.rs` (734 lines) is complete as a *view* and taken by value through `InstallersView::catalog`, but its own doc says so in as many words (`:21`, `:32`): *"`installers()` (`installers.py:230`) is T-04, `core::installers`, and has not landed — `crates/core/src/` has no `installers.rs`. The nine cards are P-51's acceptance criterion."* Confirmed by the lead: **no `crates/core/src/installers.rs`**, and `main.rs:916` still routes `Page::Installers` through `pending_page(Page::Installers, "T-12")`, so it is the **last** `PENDING_PAGES` entry. The chain that gates the project is therefore **T-04 → T-12 wiring → `PENDING_PAGES` empty → T-19 (advocate Phase 3) → T-20 (`REPORT.md`)** — five steps, and every one of them is blocked on the first. The scope is `installers()` plus `search_installers` (`installers.py:241-255`) — a catalog and a filter, not a subprocess: the page already draws everything else. **Sequencing decision (lead, R-13's rule applied again): this goes ahead of the T-63 `.desktop` writer**, which is P-71 and one parity item against the nine P-51…P-59 that this unblocks. `PINNED_PENDING` cannot reach 0, and so T-19 cannot start, until T-04 lands and T-12 is wired. |
| T-05 | `core::covers` + `exe_icons` + `netpaths` | Arch | PE parser + GVFS mapping. Cover paths are user-controlled strings — apply the D-18 typed-scalar rule to `cover_path` too. |
| T-06 | `core::plugins` + `core::credits` | Arch | **NOT LANDED — BLOCKS TWO P-ITEMS AND TWO PAGES. This row used to read "Small.", and that one word is why it sat at the bottom of Architecture's queue while every other workstream emptied.** Against the reference it is not small: `plugins.py` **188 lines** (five helpers, three states, install commands) and `credits.py` **413** (five sections, 25 entries). Verified absent at `86337d9`: `crates/core/src/lib.rs` declares only covers, json, models, paths, runners, settings. **Consequences, all of them live:** (a) **P-64** (Plugins) and **P-65** (About & Credits) are unmet; (b) UX **correctly refused** to land the Plugins and Credits pages in T-13, because with no data layer the only options were a page invented from nothing or one drawing an empty list — both are the "passes without inspecting what it claims" defect with a UI; both dispatch arms carry the blocker in their TODO and both stay in `PENDING_PAGES`; (c) **T-19 cannot start**, since its acceptance criterion is an empty `PINNED_PENDING` (currently 4) and it cannot reach 0 without this. **REASSIGNED (`d97e01a`, lead decision): the two modules go to UX; Architecture keeps `lib.rs`.** Rationale, per the tie-break rules: UX was **idle with no unblocked work** (T-24/T-25 both need `main.rs`, which Architecture holds; Plugins/Credits need this task) while Architecture holds three items — the `mod http;` re-add, the T-11/T-12 wiring diff, and the fetch handlers. T-06's files are **new** (`crates/core/src/plugins.rs`, `crates/core/src/credits.rs`), so there is no collision with Architecture's in-flight `runners.rs`/`launch.rs`; the single shared point is two `pub mod` lines in `lib.rs`, which is a one-line handoff — **the same shape already used successfully for T-11/T-12, in the other direction.** It also keeps the core modules and the two pages that consume them in one agent's context, and it unblocks T-19. **Carve-out:** if any part of `plugins.py` *executes* a process rather than describing a command as data, UX routes that question back before porting it — process execution is Architecture's area and `core`'s no-side-effects discipline is theirs to keep. |
| T-07 | `app`: `State` + `Message` + `update()` + CLI (`--list`/`--launch`/`--version`) | Arch | **CLI works headless here** (D-12, N-01). State-model tests land with it. **NOT DONE: #31** — both CLI verbs are still stubs and both were falsified by the lead running the binary against a 2-game library: `--list` prints "the library is empty" and exits **0** without reading the file, and `--launch <known id>` reports the id unknown. Everything needed landed at T-02 (`Library::load`/`all`/`get`), so `--list` is ~10 lines; `all("name")` matches Python's default sort and both sorts are stable, so no byte-divergence. `verify.sh` matched `--list\|--launch\|--version` **zero times** — nothing in the DoD could see either stub, which is why this survived; a CLI stage is owed with #29. **Traceability (#66):** this task is the owner of **P-70** (`--list`/`--launch`/`--version` headless); it landed here and is gated (finding #31). |
| T-08 | `app`: shell — nav bar, page routing, toaster, no-display diagnostic | UX | **LANDED (`2694a5e`) BUT SIGN-OFF WITHHELD** on the advocate's two blockers: **#32** (an all-placeholder Flatpak passes `verify.sh` *and* `smoke-test.sh` completely — both scripts contain **zero** references to the placeholder pages, so the repository cannot tell scaffold from finished — **CLOSED (`50798f9`), in three complementary halves, and the record's account of who built what was wrong until now.** The mechanism is a single generator — `fn pending_page(page, task)` is the *only* producer of a placeholder body, called once per page — so guarding it is test/script work rather than page work. **Packaging built the whole of #32**: `crates/app/tests/pending_pages.rs`, which parses the page dispatch's arms out of `main.rs` and pins the set independently of any table, plus `verify.sh`'s artifact cross-check (source count vs the shipped ELF). **UX built a parallel source-side half** — the `PENDING_PAGES` table with `pending_task`, and a rendering test — and that half was **a tautology** until they refactored it (finding #43): it compared `pending_task`'s output against `PENDING_PAGES`, the very list `pending_task` reads. The lead's earlier note called `pending_pages.rs` "UX's"; it is Packaging's, and the reassignment was therefore never the duplication the note implied. Both directions are now verified by the lead: porting `Page::Library`'s arm while leaving it pinned FAILS UX's test naming the page; Packaging captured the artifact pair, where **every byte-identity check passes while the stage fails** — which is the point, since no installed file is `main.rs`. Packaging also answered the reachability question the lead raised: the artifact branch **cannot** fire under a full run (`--force-clean` erases the tree), but it is reachable via `--skip-flatpak`/`--skip-smoke`, which skip the build and still reach the stage against whatever `build-flatpak/` an earlier run left) and **#33** (`update()` has no test coverage at all: all five real handlers, `NavigateTo` included, can be replaced by no-ops with a green suite). Also filed: #34 (`go_to`'s invariant is convention, not structure — a T-09 design decision) and #35 (`Page::Credits` says "Credits" where `Main.qml:94` says **"About & Credits"** — a live P-65 parity divergence the label test cannot detect, because it compares `page.label()` against rows built from `page.label()`). **Verified and good:** the non-exhaustive-pattern guard works (`error[E0004]` on a new variant), the icon transcription reads `Main.qml` off disk and is CAUGHT, all four no-display-guard mutations are CAUGHT, and mutating the guard off reproduces exit 101 with the documented panic at `iced/winit/src/lib.rs:92:39` — so the guard's justification is unchanged. D-32's scoped allow survives untouched. **Both blockers are now closed — #32 at `50798f9`, #33 at `253498d` — so the sign-off is with the advocate for re-review as of the lead's request (2026-09-11). It is *with the advocate*, not granted: a withheld sign-off resolved by the lead saying the blockers closed would be the lead grading its own work.** |
| T-09 | `app`: Library page (grid/list, search, sort, filter, context menu, dialogs) | UX | P-01…P-17. Grid virtualization is the risky part (R-9). **This task carries three inherited requirements. They are listed here rather than left in the issue log, because a row that names only P-01…P-17 loses them the moment work starts.** (a) **#34 — decide the navigation's source of truth in this task, not later.** `go_to` writes the current page to *two* records — `State.page` and the nav model's selection, via `activate_page` — and nothing structural keeps them in step; `State.page` is also `pub`. Either derive the nav selection from `State.page` in `view()` (one record, the divergence becomes unrepresentable) or keep both and pin the invariant with a test. Record the choice in DECISIONS. (b) **#33 — `update()` has no coverage at all.** All five real handlers, `NavigateTo` included, can be replaced with no-ops under a green suite. Extract the arm bodies into pure functions over `(&mut State, …)` so each `Message` variant has a unit test; a `Message` nothing tests is a `Message` that can be deleted without anyone noticing. (c) **#35 — `Page::Credits` renders `"Credits"` where `Main.qml:94` says "About & Credits" (P-65).** The string is fixed in T-13, but the *test* is fixed here: the existing label assertion compares `page.label()` against rows that `build_nav_model` also sets from `page.label()`, so it is constant-against-the-same-constant and cannot detect any wrong label. Pin it against `Main.qml` read at test time, using the anti-rot pattern the icon transcription already uses. **STATUS `253498d`: all three landed.** (a) `#34` **decided — checked by structure, `page` left public**: `show_page` carries a `debug_assert!` that fires when `activate_page` answers false, i.e. when `build_nav_model` and `Page::ALL` have diverged; the doc comment states the one case it *cannot* see (a handler assigning `state.page` directly, because `show_page` writes both records before checking them) and names the test that covers it instead. (b) `#33` **landed**, and its obstacle was placement rather than the arms: `App` holds a `cosmic::Core`, `Core` is only ever constructed by the framework, so no test could build one and `update` was unreachable. The dispatcher moved to a `Shell { state, nav_model }` — a `State` with a sidebar and no window — with `App::update` reduced to `Quit` plus a delegation, and the no-wildcard arm preserved. (c) `#35`'s assertion **fixed**: the test now slices the drawer's `actions: [...]` out of `Main.qml` and compares label-by-label, with `KNOWN_LABEL_DIVERGENCE` recording `Credits` → `About & Credits` explicitly. **Note for T-13: fixing the string makes that assertion fail with "delete it (P-65)", which is the intended enforcement — the record cannot outlive the divergence.** **STATUS `2f017fd`: LANDED.** `view/library.rs` (446 lines) with `LibraryPage` taken **by value** so the element borrows the caller's `Library` — which is what dodges the E0515 that had killed the T-11/T-12 wiring attempt — and the two empty states as a pure `empty_state(total, shown)`; `Page::Library` dispatches to the real page (`main.rs:800`); `PENDING_PAGES` and `PINNED_PENDING` both lose `Library` (6→5), which per `pending_pages.rs`'s own header is the same act. `SetViewMode`/`SetSortMode` gained the allowed-set guard the reference has (`bridge.py:210-214`, `223-228`) alongside the load path's (`settings.rs:148-153`). **Verified by the lead, not taken on report:** touched `main.rs` + `core/src/lib.rs` to force a real check, observed `Compiling gamehandler-core` then `Compiling gamehandler`, then **164 + 3 + 2 + 358, 0 failed** (D-45's protocol). `empty_state` was checked specifically because a version keyed on the *filtered* list collapses NoGames into NoMatches and no single-state test would see it. **Process note, recorded because it was handled well:** UX edited `pending_pages.rs` — Packaging's file — because leaving it would have left HEAD's suite red for a reason unrelated to their change; they read it immediately before editing, and they deliberately did **not** wake Packaging over the one stale doc sentence (`~:73`), routing it through the lead instead. One agent per file, routed centrally, is the working rule. **CORRECTION (`7de961b`, #57): this task shipped a real bug that no test in the suite could see.** The category dropdown's `on_selected` is `impl Fn(usize) -> Message`, so `library.rs:253-256`'s `\|category\| Message::SetCategoryFilter(category.to_string())` stored **`"1"`** where the model wants `"Puzzle"` — a filter matching nothing, with every check green. UX found it while building T-13's selectors and reported it against their own committed work, which is the right way to handle it. **The reason it was uncatchable is the part that generalises: the closure lived inside a builder, and a builder needs a renderer** — so the index-to-name mapping has to be a *function* (`library::category_selection`), because a function is the only form an assertion can reach. Third instance of that wall (#46, #51's D, #57). | **REMAINDER, per D-52 (#68b) — this task is LANDED but the word does not mean its scope is met.** `T-09`'s declared scope is **P-01…P-17**, and four groups inside it are not covered: (a) **P-02 / P-16 (last played) are HALF MET** — `core::models:458/507` define and test `format_last_played`/`format_last_played_now`, but `git grep 'last_played' -- crates/app/src/` is **empty**: `list_body` (`view/library.rs:340`) passes only `resolved_runner_label`, so the row renders no timestamp. Owner **T-30**, unblocked. (b) **P-10…P-16 (the context menu and the row actions) DO NOT EXIST**: double-click play, the nine-item menu, Play/Edit/Find-cover, Winecfg/Winetricks/Open-prefix, create-shortcut and remove-with-confirm — `view/widgets.rs` constructs **zero** `Message`s and `card`/`row` take only `(game, label)`. The *handlers* are blocked on **T-10** (the form) and **T-29** (launch), but the *widgets* are themselves P-items and the reference draws them, so 'blocked' describes the handler and not the card. (c) **P-17's dialog** is behind the same two. (d) **P-03's Add-first-game button opens nothing** — that is **#65**, `main.rs:1046` is `{}` at HEAD (`:1059` and no longer empty in the working tree, UX implementing it as this is written), owned by T-10 and still the one live entry in `dispatch_coverage.rs`'s `KNOWN_DEAD` deferral. **The line number was `:1043` in earlier drafts of this row and in `#65`'s own text; it is verified here against `git show HEAD:crates/app/src/main.rs` because the tree moves under every citation and this file has recorded that class four times.** **This paragraph exists because #66, #55 and #65 all came within one audit of this row and none of them saw it:** every guard here asks whether a page renders a real body, and none asks whether its P-items are met. See **D-52**.
| T-10 | `app`: Game form | UX | P-18…P-31. 15 toggles, 6 sections. |
| T-11 | `app`: Runners page | **Architecture** | P-32…P-39. **Reassigned from UX (R-13).** Build as `crates/app/src/view/runners.rs`, a self-contained module over `&State` — the `Message` variants already exist (`main.rs:428-450`), and `view_body` is a free function, so nothing here needs the nav model or `main.rs`. **RE-POINTED after `253498d`: the premise holds and the wiring point moved.** `State` is unchanged (`pub page: Page` still `state.rs:336`, same field set), so a page module over `&State` is unaffected. What changed is that `fn view_body(state: &State)` is now `Shell::view_body(&self)` (`main.rs:788`) and the page's `update` arms go to `Shell::update` (`&mut self`, `self.state` in reach) rather than `App::update`. Same two hunks, re-pointed at `Shell`'s functions. The move was **forced, not stylistic**: `crates/app/tests/pending_pages.rs` anchors on the literal `match self.state.page`, so the free function (`match state.page`) failed that test's own anti-vacuity guard — and renaming the binding to keep a parser happy would have been a lie told to a parser. Deliver the mechanical wiring diff for UX to apply, since UX owns and is mid-edit in `main.rs`. Reuse T-14's components; do not introduce a second visual idiom. **STATUS `24da5f7`: the module has landed and the page is NOT reachable.** `view/runners.rs` (1155 lines **at `24da5f7`** — a property of that commit, like every count in this project; **1522 at `ed914b8`**) and the shared `view/badge.rs` (118 **at `24da5f7`**, 159 at `ed914b8`) are committed, with 42 fixture-free page tests in the suite. **Tests: 144 at `f4ffdcd`, 147 from `6823ff5` onward** (`cargo test -p gamehandler`; the +3 is Architecture's second round). Recorded with the commit named rather than as a bare number, because a bare number is exactly how this row was wrong twice: the lead wrote 147 (a *working-tree* measure with an uncommitted round in it), then wrote 144 as "the committed count" in the commit *before* the round landed — and the row was stale within the hour. A count is a property of a commit, not of the repository. `Page::Runners` still dispatches to `pending_page`, so the user-visible surface is unchanged — which is an honest intermediate *only* because `PINNED_PENDING` still names `Runners`, and becomes defect #32 the moment the task is called done without the wiring. **SEE THE WIRING CONDITION BELOW BEFORE WIRING.** Third-party claims in the commit — the `UninstallRunner` removal's faithfulness to `bridge.py:697,708,714` and `772-782`, and the assertion that three mutation survivors were *closed* rather than moved — are with the advocate. **ADVOCATE VERDICT (2026-09-11): sign-off on the CODE; sign-off REFUSED on the claim as worded.** (a) The `Idle`-removal is **confirmed by execution, not by reading** — the advocate extracted `Backend.uninstallRunner` with `ast` and ran it against stubs seeded `_releases_status = "ready"`: neither the success nor the failure path writes the field, `grep` shows writes only at 136/697/708/714, and `fetchReleases` sets `_releases = []` (`:698`) before `_releases_status = "ready"` (`:708`), so `Idle`-with-a-non-empty-list is unreachable in Python. The Rust defect was real: `status_line` gave `Idle` and empty-`Ready` the same sentence, rendered directly above the builds. **Claim 1 survives the challenge.** (b) **Claim 2 is half wrong, and the correction governs what we may quote onward.** Of the three survivors, only the system row's `runner_id` was genuinely *closed* (mutation C caught). `uninstall_line` and `install_press` were **moved, not closed**: their *function bodies* are now covered (control mutations E and F caught, so the work is not vacuous), but their *call sites* still survive (A, B). A fourth site survives one step downstream — the row's press message (D, `"system".to_string()` for `row.runner_id`) — so the field has an unobservable consumer at `runners.rs:403` even though the field itself is now pinned. The honest wording is **"the line and the guard are now readable"**, not "closed". Closing A/B/D properly needs a reader for `Toasts` and for a button's pressed message, neither of which libcosmic exposes — the same unreadable-observable wall `DismissToast` runs into. **The commit message cannot be amended without rewriting history, so the correction lives here**, and both modules' own doc comments (`runners.rs:1113-1120`, `installers.rs:485-492`) already state the bound honestly, which is the part that holds up. This is the #30 shape once more: a fix that is real, described more strongly than it was verified — and this time the person who wrote the row is the one who quoted it. **D WAS THEN ATTEMPTED, AND THE ATTEMPT WAS MEASURED INSUFFICIENT (2026-09-11).** The lead instructed Architecture to close D by asserting the press message equals `row.runner_id` — "pins the *site* rather than the field" — and the advocate implemented exactly that shape in a sandbox, which reached compile: baseline with the extraction wired in **SURVIVED** (148 passed, 0 failed); mutating the helper body to `"system"` **CAUGHT**; mutating the **site** to `Message::UninstallRunner("system")` **SURVIVED**. The extraction closes the *helper*, not the *site*: the test calls the helper directly, so routing the call site around it survives — the identical moved-not-closed pattern as A and B. **D therefore joins A and B as the third documented gap, and the lead's earlier "two" was wrong.** The wall is structural and was established by reading the vendored sources rather than recalled: `Operation` has seven arms and none carries a message (`iced/core/src/widget/operation.rs:21-68`); cosmic's `Button::operate` reports only `container` and `focusable` (`src/widget/button/widget.rs:338-357`); `Button.on_press` is a private field holding an opaque `Box<dyn Fn(..) -> Message>` (`widget.rs:48`), not a comparable value; the button `State` exposes only `is_focused()`/`is_hovered()`; `Toasts` exposes `new`/`push`/`remove` with no reader. So there is no reader and none is reachable. **Architecture's `remove_press` extraction is kept** — helper-level coverage is real and the doc comment records its own residual honestly — but the *test name* must not claim the button: `the_remove_button_names_the_row_it_belongs_to` asserts on a direct helper call, and the button is precisely what the test cannot see. Renamed to a helper-level name. The press call site carries a comment saying the site is **unobservable**, not that the message is pinned. **CLOSED as a finding (`c17c7da`, #51) — the helper is now wired, `runners.rs:430`: `.on_press(remove_press(row))`.** That is the repair the lead ruled for and it is the *smaller* diff than deletion: it makes the M1 catch non-vacuous rather than vacuous, turns the deletion mutant into `error[E0425]` (Architecture re-measured: both targets), and makes all three prose claims true at once. **D itself remains open** — the site's mutation survives the wiring, as the call-site comment correctly says. **BOTH BLOCKERS ARE NOW SETTLED — SEE `D-48`.** (1) The **HTTP** question was already answered by `D-26`: the seam exists and is fully built (`core::runners::proton::HttpClient`, `proton.rs:151`; `fetch_available`/`install` at `proton.rs:524`/`:936`; four `#[cfg(test)]` impls; no production one). Ruling: a **blocking** client (`ureq` + `rustls` + the **system** root store) as a direct dependency of `crates/app` **only**, implementing that trait — its `(url, headers, timeout, on_head, sink)` shape is blocking and needs no async adaptation, and `core` stays dependency-free by construction so D-26 holds structurally rather than by discipline. `cargo-sources.json` is **Packaging's**; Packaging regenerates it. (2) The **page-entry life cycle** does **not** need a new concept: `Shell::show_page` (`main.rs:759`) is already the single funnel and sole writer of `state.page` (`:760`), reached by the sidebar (`:1271`) and every programmatic route (`NavigateTo`, `:891`). What it lacks is an **emission** — it returns `()`. It must compare the incoming page to `state.page` **before** assigning and return the page-entry `Task` when they differ; compare-first is load-bearing because Python's `Component.onCompleted` fires once per page *instance*, so re-navigating to the page you are already on must not refetch. (3) **The executor was measured, and the lead's own working assumption was wrong** — `cargo tree -e features` shows `iced_futures` enables both `tokio` and `thread-pool`, and the cascade in `backend/default.rs` is `tokio → smol → thread-pool → null`, so the backend is **tokio** (`Runtime::new()`, multi-thread, spawning onto a worker thread). A blocking call in `Task::perform` is therefore off the UI thread and **no blocking hop is needed**. Worth knowing that the last rung is `null::Executor`, whose `spawn` **drops the future** and whose `block_on` is `unimplemented!()`. (4) **A parity item nearly missed:** `fetchReleases` (`bridge.py:695`) carries `if family_id != self._releases_family: return` in **both** callbacks, and `state.rs:354` already has the field — the guard must be ported with it, or a slow response landing after a family change writes releases for the family the user abandoned while the UI shows the new one. **SEQUENCE: fetch/update handlers first, then wire** — the pages are unreachable until the dispatch arm lands, and wiring before the handler exists is the exact failure this task was created for. **`main.rs` coordination:** UX holds it during T-13, so Architecture sends the `show_page` + `FetchReleases` diff for UX to apply, or waits for T-13 to land. |

**WIRING IS NOT THE NEXT STEP, and the lead's first instruction to wire was wrong.** UX built the wiring in a scratch copy and measured what the pages would *say*; the lead then checked every link against the tree and they hold: `status_line` (`view/runners.rs:204`) puts `ReleasesStatus::Ready` and `::Idle` in the **same match arm**, both returning `"No builds found for this family."`; `state.rs:408` defaults `releases_status` to `Idle`; `main.rs:959` is `Message::FetchReleases { family: _family } => {}`. So wiring today renders, on first run, a page whose entire subject is the builds list, **announcing that no builds were found, having fetched nothing and started no fetch** — a negative result the app never obtained, stated as a finding. It is worse than the placeholder it replaces, because #32's guard reads the dispatch arm: `pending_page` would be gone, `PINNED_PENDING` would drop both names, and every guard built this week would agree the page was finished. **The placeholder would have moved out of a string that says "not ported" and into a string that says "nothing found" — the one direction none of our checks look.** So: **the fetch/update handlers are part of this task, not a follow-up.** `FetchReleases` must set `releases_family`, set `releases_status = Loading` and return the task; `ReleasesFetchFinished` must drop a result whose family has been superseded. Until that exists the page cannot be wired truthfully.

**Interface shape — an arm cannot return a borrow of its own locals.** UX measured 7 × E0515 (`page`, `catalog`, `categories`, `runners`, `releases`, `installed`): `view()` takes `&'a [..]` slices, so an arm that builds the view from locals cannot return an `Element` outliving the match. Three shapes were proposed; **the lead ruled (a) — owned `Vec`s in the view structs — OUT for Runners, on correctness rather than taste:** `installed_rows` needs the manager, the manager runs `wine --version`, and owning the bundle at build time puts a subprocess on the render path on every frame. **No process may be spawned from `view_body`** — that is the constraint, not a preference. The live options are **(b)** `State` carries the recomputed bundle (consistent with `runners.rs`'s own module note that `installed_rows` needs the manager) and **(c)** splitting `view_body` so the bundle is built in `App::view`. Architecture owns the choice; (b) is the lead's inclination for Runners.

**The landing check is not "`view` returned an `Element`".** `view()` is called by **nothing** in the tree — `grep` finds only the definition — and `mod view;` is `allow(dead_code)`, so the 49 tests behind those modules never execute the function they are named for. The check that matters drives `Shell::view_body` for Runners/Installers and asserts the drawn strings are **neither the pending text nor an empty state** — the same "asserts the input, not the builder" trap as #46, which was satisfied by an undecodable fixture. |
| T-12 | `app`: Installers page | **Architecture** | P-51…P-59. **Reassigned from UX (R-13)**, same shape and same rules as T-11, including the `253498d` re-point to `Shell::view_body`/`Shell::update`. Depends on the file chooser (P-57), which is T-15 and stays with UX — build the page and leave the chooser call behind the existing `Message` variant. **STATUS `24da5f7`: the module has landed (`view/installers.rs`, 701 lines) and the page is NOT reachable** — same unwired intermediate as T-11 above, and **the same wiring condition applies, read it before wiring.** UX's measurement for this page is a different untruth from Runners': an empty `catalog` renders *"Run your first install to see it here."*, which is **indistinguishable from what a correctly wired page with an empty catalog would say**, and nothing surfaces `easy_busy` — so wiring it now yields a placeholder that reads as a working page, which is #32 exactly with the placeholder moved out of the body and into the string. The interface question is easier here (`InstallersView` has no manager-dependent input, so owned `Vec`s do not imply a subprocess), but it is Architecture's module and their call. `PINNED_PENDING` must keep naming `Installers` until the dispatch arm lands. |
| T-13 | `app`: Settings + Plugins + Credits pages | UX | P-64…P-68. **Carries #35:** `Page::Credits` (`state.rs:89`) reads `"Credits"` where `Main.qml:94` reads **"About & Credits"** — a live P-65 parity divergence. The string is corrected here; the *assertion* that would have caught it is corrected in T-09, then re-pointed at this page. |
| T-14 | `app`: cover tiles, pills, async images, theming | UX | P-60…P-63, P-67. **Enable libcosmic's `image`/`webp` support first — see R-11.** **STATUS: landed (`2e4dca4`, `6350336`, `e75454f`) but SIGN-OFF WITHHELD** on the audit's three blockers — all three now filed: **#28** (`widgets.rs`: no test calls any builder, so 9 mutations survive, including the initials/words swap its own test was written to prevent), **#29** (the verify.sh summary omits stages that never ran), **#30** (Windows rows render no runner label until the manager reaches the view). Sign-off is *withheld*, not overruled: the port is faithful where it was measurable — `COVER_GRADIENTS` 8/8 in order against `CoverArt.qml:19-24`, `initials` 0 disagreements across 3,060 cases against CPython, `accent_of` 5,060 cases with all seven buckets matching — but "headlessly-testable" is true of `cover`/`meta`/`metrics` and **not of `widgets`**. That distinction goes into REPORT.md as the audit's verdict. **All three blockers now closed — #28 at `ee18527`, #29 at `50798f9`, #30 tracked — so the sign-off the row withholds turns on the last clause above: whether `widgets` is still unverified now that its builders are rendered and traversed. With the advocate for re-review (2026-09-11). Again: with the advocate, not granted.** |
| T-15 | File chooser via portal; wire P-21/P-24/P-57 | UX + Pkg | **No new dependency** (D-23) — `cosmic::dialog::file_chooser`, already available. Returns a **`url()`**, so handle the URI→path case explicitly for GVFS (R-9). **REQUIREMENT, stated rather than assumed: the chooser must not be on `view_body`'s path.** A page switch must not wait on a dialog. **Today this holds by *shape*, not by assertion** — `view_body(&self) -> Element` is synchronous, reads only `&self`, and has no task plumbing, so nothing can await inside it. UX reports, correctly, that this was noticed rather than designed, and that **no test asserts it**: adding an `awaiting_exe_pick` flag and branching `view_body` on it would leave the whole suite green. So the falsifiable form belongs in this task rather than in a comment: **every page in `Page::ALL` must render its body from `&State` alone, with no task ever executed, and draw its own content rather than a spinner or "loading".** **The guard for that does not exist yet, and the obvious candidate must not be read as if it did.** `the_pending_pages_are_exactly_the_ones_whose_body_says_so` renders `view_body` per page without *needing* a task, but it asserts *which* pages are placeholders and **never observes that no task ran** — it is a reasonable place to *extend* (it already walks pages and renders them, so counting executed tasks is the same loop), and as written it is silent on the property. UX caught this in the first wording of this row, which presented the extension as the enforcement already in place; that is the same "the name reads as stronger than the check" defect as #39, so it is recorded rather than quietly reworded. Written down because a property that protects the design and is asserted nowhere is a property the next refactor is free to delete. |
| T-16 | Flatpak: new manifest, `cargo-sources.json`, `build.sh` | Pkg | Removes PySide6/llvm21/PYTHONPATH. Keeps Wine base, osslsigncode, DXVK. **Traceability (#66):** this task is the owner of **P-78** (Flatpak Gamescope extension PATH, Steam read-only path, Wine BaseApp) — present in `build-aux/flatpak/com.goshapps.GameHandler.json`. |
| T-17 | `scripts/verify.sh` + headless smoke test | Pkg | 7 stages per `packaging.md` §6. **The gate is SIGNED OFF by the advocate at `ed59974`**, scope stated per D-33: `scripts/verify.sh` *as a script* (the runner, `STAGES`, `begin`/`finish_*`, the summary, the lock and the exit tail exercised on the real script with a stubbed runner; the Rust-scope questions via `cargo test` on `ed59974` sources, not through `verify.sh`). **This is not a sign-off on the workspace, the Flatpak artefact, or any crate's parity.** Independently reproduced: the dual-configuration `stage_test` is **not** theater (`cargo tree` gives 0 `preserve_order` occurrences narrow vs 3 wide; a probe gives rc=101 narrow / rc=0 wide **with one shared target dir**, so the separate `CARGO_TARGET_DIR` really is cache hygiene only and the gate does not depend on it — D-41); the `cli` stage catches three mutants including a reproduction of the historical #31 regression; `banner_check` gates hard (9 structural mutations → rc=2); `begin` refuses unlisted, out-of-order, missing-`begin` and double-`begin`. **Two residuals filed, both non-blocking:** **#41** — a run that *finishes* with stages in `STAGES` never reached exits **0** while reporting that nothing was verified about them (the exit tail checks `FAILED` and `SKIPPED_UNREQUESTED` but not `unrun`, which `summary` computes and discards as a `local`); this is the residual of D-34 and the script's own comment at `:1362-1386` already argues for the fix. **#42** — `verify.sh:517` asserts the `cli` stage "is RED until #31 lands", and the line was **born false**: `e9ecf5e` (the #31 fix) is an ancestor of `398a7c0` (the commit that wrote it). A comment that pre-authorizes a failure, three lines above the stage that exists to catch one. |
| T-18 | Metadata: desktop file, metainfo, README, version bump to 0.8.0 | Pkg + UX | Version-lockstep test. |
| T-19 | Phase 3 verification pass | Advocate | Walk every P-item against the running Flatpak. Walk **B-01…B-08** — hand-edit the file named for each and confirm the real app survives it. "The port does not copy the bug" is a claim that needs demonstrating, not asserting. **B-08 is the exception to the hand-edit method**: its input is unreachable through the UI (see the B-08 row), so it is evidenced by the unit test, and T-19 should confirm the test exists and fails when `classify` is made suffix-aware rather than attempting a manual reproduction — **the lead has already run this** (a suffix-based `classify` fails 3 tests, including `left: Icon, right: Photo`), so Phase 3 re-confirms rather than re-derives. **T-19 must also confirm #32's pending-page list is empty, and do that FIRST**: until it is empty the six pages render placeholders, every P-item verdict walked against them is vacuous, and neither `verify.sh` nor `smoke-test.sh` can see the difference — so this is the one check that makes the rest of the pass meaningful. |
| T-20 | `docs/migration/REPORT.md` | Lead | Final deliverable. |
| T-21 | `image`/`webp`: enable libcosmic's `animated-image`, regenerate `cargo-sources.json` | Pkg | **DONE** (`db9c7c3`, corrected `a449837`). D-29. |
| T-22 | Verify a `.webp` cover actually **renders** through iced end-to-end | **Packaging** (reassigned from UX by the lead, 2026-09-11) | **Owed at T-14.** **Reassignment rationale:** this is a *verification* task, not page construction — it asks whether an artefact does a thing, which is Packaging's craft and this project's dominant defect class — and it is not on the critical path for the five unbuilt pages, so moving it off UX's queue costs the UI nothing. **It is also newly tractable:** this task's own note says it "must drive a real builder call (the same seam #28 introduces)", and `ee18527` has now introduced exactly that seam — the widget tests render builders and read a real `Tree`/`Operation` traversal under the software renderer. T-22 should reuse it rather than invent a second one.  D-29 proves the decoder is *reachable*; it explicitly does not prove a render succeeds, and says neither the crate-level probe nor `cargo tree` is evidence for it. Until this runs, "webp cover render" is unverified. **Interacts with #28:** the T-14 audit showed no test calls any `widgets.rs` builder, so a `.webp` assertion written the same way would prove only that the *decoder* was asked for bytes, not that a tile ever drew them — the same "asserts the input, not the builder" trap. T-22 must drive a real builder call (the same seam #28 introduces) or it repeats the defect it exists to catch. **STATUS: CLOSED (`7e8f2b3`), and it landed as the two halves D-43 split it into.** The evidence half — probing that the *decoder* works and that `leaves()` cannot witness it — is the lead's three-row table, which is what produced #46; the landing half is UX's `a_webp_cover_is_decoded` plus `assert_decodes` inside `widgets.rs`'s private `mod tests`, driving the real builder seam #28 introduced, exactly as the row required. **The row's own trap was avoided rather than restated:** the assertion is on `measure_image`, the one observable that moves with a decode, and *not* on `leaves`, which is satisfied by the `Size::ZERO` a failed decode produces. Both halves of the evidence were measured by the lead rather than taken on report — stripping `"animated-image"` reddens **exactly two** tests at the same `assert_decodes` line with 144 still green, and the 118-byte fixture decodes to `24x16` with 216 distinct RGB triples over 384 pixels under the **system** libwebp, a different implementation from the Rust `image` crate's VP8L path. Sign-off with the advocate. |
| T-23 | Install the app's GPL text and the Microsoft trust root; assert the Flatpak carries them | Pkg | **DONE** (`cad22b2`, corrected `4ddda6c`). Stage 10 (`flatpak-contents`). |
| T-24 | Apply the persisted colour scheme — **P-67** | UX | **OPEN, filed from UX's own T-13 finding (#55), verified by the lead against the reference.** The reference applies the scheme at **two** sites: `theme.py:85` `apply()` → `self._app.setPalette(build_palette(scheme))`, called from `main.py:106` **at startup** with the persisted value and from `bridge.py:201-202` **on change**. The port has **no `Shell::theme()` override** (verified: no `fn theme` in `main.rs`), and `settings.color_scheme` is written at `main.rs:975` and read by nothing but the Settings selector — so choosing light/dark/system today changes a stored string and nothing a user can see. **Acceptance:** (a) the persisted scheme is applied at startup; (b) changing it changes the app; (c) `"system"` follows the desktop; (d) the three-value `COLOR_SCHEMES` tuple and the persisted `colorScheme` key survive, so an existing user's saved choice is never lost; (e) **Phase 3 verifies P-67 in the running Flatpak.** Check first whether libcosmic's own theme mechanism (`ThemeMode` / `Application::theme`) subsumes this — **verify against the vendored source, do not assume.** Closes **D-13**, whose decision cannot be evaluated until this exists. |
| T-25 | Wire the four shortcuts — **P-68** | UX | **OPEN.** Verified: `main.rs` has no `subscription()`, no `on_key_press`, no keyboard handling of any kind, so the four rows `SettingsPage.qml` prints do nothing. UX recorded this rather than hiding it: `IMPLEMENTED_SHORTCUTS: [&str; 0] = []` (`view/settings.rs:143`) and `UNWIRED_SHORTCUTS: [&str; 4]` (`:154`), with a test requiring every printed row to be in exactly one of the two **and** cross-checking the empty-implemented claim against whether `main.rs` listens for keys — so the record cannot drift from the code. **Not page work:** it is a `Subscription` with real focus hazards (`Ctrl+F` must focus the search field; `Ctrl+N`/`Ctrl+F` must not fire while typing). **Sequencing:** edits `main.rs`, so it runs after T-11/T-12's wiring handoff clears that file. |
| T-26 | `core::plugins` + `core::credits` land, and the two pages consume them — **P-64, P-65** | UX | **This row was missing entirely until `#71`; the task was created, worked and landed with no row here.** **DONE (`41a6612`)** — `crates/core/src/plugins.rs` + `credits.rs` landed, `view/plugins.rs` (575 lines) consumes them, and `core/src/lib.rs:45/49` declares both modules (the declaration was missing at first, and is `#61`). This is **T-06's reassigned half** — T-06's own row carries the reassignment decision (`d97e01a`) and names P-64/P-65 as its ids, which is why the ids stayed traceable even with no T-26 row; that was luck of ordering rather than design, since T-06's row was written before the split. |
| T-27 | The thirteen toggle labels have no render-level check | UX | **OPEN, measured by UX, mechanism verified by the lead in the vendored source.** `Toggler::draw` (`libcosmic src/widget/toggler.rs:299`) paints its label with a direct `iced_widget::text::draw` (`:316`) rather than emitting a widget into the `Tree`, so the toggle labels and the close-on-launch explanation are drawn but **invisible to every tree-walking assertion** — the same blind spot as #46's decode, in a different widget. UX pinned the limit with a test so it cannot quietly stop being true; what remains is deciding whether the labels need a second, non-render assertion, and recording the bound in the findings table so a later reader does not re-derive a weaker check that appears to cover it. |
| T-28 | Verify the closure claim independently — UX retracted the "progress reporting" remainder | **Advocate** | **OPEN, and this row was also missing until `#71`.** Phase-3 work: the claim is that a task's closure is justified, and UX withdrew its "progress reporting" remainder before that could be tested — so what remains to check is whether anything else in that closure was asserted rather than verified. Runs under **T-19**. |
| T-29 | `app`: **the launch flow** — launch a game, watch it, toast a failure, mark it played | Architecture | **P-40…P-50, P-69, P-71.** **This task exists because of finding #66:** an audit of all 78 parity items against every task's scope found 21 named by no task at all, and these twelve are the ones with **no implementation anywhere** — the rest of the 21 turned out to be built and merely untraced. **The core half is already written and unreachable.** `core::runners::launch` is declared (`runners/mod.rs:33`) and carries the env mapping, the NVAPI/FSR/Wayland gate and the bundled-DXVK install; `main.rs:160` states the position plainly — *"the launch itself is not ported yet — `runners::launch`"*. Every missing piece is a **caller**: `git grep -n 'runners::launch' -- crates/app/src/` returns **two comments and no code**. The work is the five `main.rs` arms at `:1057-1066` (`mark_played` + the grace watch, `LaunchWatchFinished`, `OpenPrefixFolder` via `xdg-open`, `RunPrefixTool`, `CreateDesktopShortcut`) plus P-46's ~6 s grace-and-toast. **P-69's toaster exists** — `State.toasts` with `toasts.remove(id)` as its one handler (`main.rs:957-960`) — so per-mutation toasts are blocked on the mutations themselves, not on infrastructure. **P-71 has nothing at all**: the sole mention of desktop shortcuts in `core` is a doc comment line at `runners/mod.rs:28`. **MANDATORY RIDER — retag the TODOs.** `main.rs:1057-1066` assigns this work to **`TODO(T-10)`**, but `T-10` is scoped *"app: Game form \| P-18…P-31. 15 toggles, 6 sections."* An agent following the tree's own tags would do the launch work under the name of a task that does not contain it, leaving the matrix still showing P-40…P-50 unowned: **the tag and the plan would disagree, and neither is checked.** Retag every one of those sites `TODO(T-29)` as the first edit of this task, and add the reverse check — that no `TODO(T-nn)` in the tree names a task whose scope does not contain the work. **ORDERING:** after T-11/T-12 (which shrink `PINNED_PENDING` and unblock T-19), and it is **the largest remaining parity block** — eleven of the app's reason for existing — so it goes next, ahead of T-24/T-25. **VERIFICATION is Phase 3 work by construction:** P-46 is *"bad exe with close-on-launch"*, P-49 *"opens `…/pfx/drive_c`"*, P-43 *"names Flathub extension"* — none of which a unit test can assert, so T-19 must exercise them in the running Flatpak and this task is not done on a green suite |
| T-30 | `app`: wire the last-played subtitle into the Library row — **P-02 / P-16** | UX | **Unblocked and small, and it is the checkable half of #68.** `crates/core/src/models.rs:458/507` define `format_last_played` / `format_last_played_now` **with tests**, and **nothing calls them**: `git grep 'last_played' -- crates/app/src/` is empty. The reference renders `subtitle + " · " + lastPlayed` (`LibraryPage.qml:269`, supplied by `bridge.py:311`); the port's `list_body` (`view/library.rs:340`) passes only `resolved_runner_label`. This is a string in the row builder, not a data-layer job. Fold into T-10 or T-29 rather than opening a file. **Acceptance:** a game with a timestamp renders it; a game without one renders no stray separator. |
| T-31 | **Rule: a task row may not be marked LANDED while an id in its own declared scope is unmet** | Lead | **The structural half of #68, and the decision is the rule rather than a check.** #66 audits for P-items named by **no** task; T-09 **names** P-01…P-17 and is marked LANDED, so every id it names read as covered while P-02 was half met and P-10…P-16 did not exist. **Third instance of the #55/#65 shape.** The mechanism, stated once: every guard in this repository asks *"does this page render a real body?"* — `PENDING_PAGES` is keyed on the placeholder string, `dispatch_coverage` on the handler, the label tests on the text — and **none asks "are this page's P-items met?"** Parity is the only axis with no check. **Why a rule and not a per-page P-matrix:** a matrix would be a second hand-copied list, and this repo has already recorded three instances of a hand-copied fact nothing reads (#43, `PINNED_PENDING`'s own header, T-09's row). Deliverable: the rule in `DECISIONS.md`, plus T-09's row corrected to state its actual remainder the way its own row already does for #34/#33/#35. |
| T-32 | `view/meta.rs:52` — a correct guard justified by a defect that no longer exists | UX | **#69.** The paragraph reads *"the Rust view has no runner manager in its contract yet (T-07) and passes `\"\"`"*. **False at HEAD:** `view/widgets.rs:417-425` records that empty-string path as **#30, fixed**, and the view has taken a `&RunnerManager` since T-08. The guard is right and stays; the *reason* given for it is wrong, which is the #62 shape (a citation that does not land) in a second file — and a reader who trusts it would conclude the empty-label case is unreachable and delete the guard that handles it. Fix in place with the history recorded, not silently deleted. Small; ride with T-10 or T-29. |
| T-33 | `Message::OpenUrl` — the Credits page's 27 links are dead | UX | **#70.** `view/credits.rs` (827 lines) contains exactly **four** `Message::` occurrences and **all four are doc comments** — it constructs no message in production code. Root cause is structural: **`Message::OpenUrl` does not exist anywhere in `crates/`**, so `button::link` has no message to carry and the 26 "Visit" buttons plus the GitHub footer render **disabled**. The reference uses `Kirigami.UrlButton`, which opens URLs, so on this page the port is strictly less capable than what it replaces — on the page whose entire content is links. UX disclosed the gap in-file (`the_links_are_still_unwired` asserts both halves), which is the right way to land a known gap, but a disclosure in source is not an owner. **A named task and not a drive-by because a new `Message` variant also touches `message_variants!` and the two length pins (#38/#39).** After T-12. |

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
