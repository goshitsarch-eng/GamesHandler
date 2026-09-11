# Phase 1 review — Devil's advocate audit of the libcosmic migration

**Reviewer role:** adversarial. This document is an independent audit built from
the code at HEAD (`cbaf7e6`, branch `cosmic-migration`), the test suite
(241 tests, 1 skipped — verified by running `python3 -m unittest discover -s
tests -t .`), the Flatpak manifest, and the libcosmic reference clone at
`/tmp/cosmos-spike/libcosmic` (submodules initialized). It was written without
reference to the UX / Architecture / Packaging teammates' drafts.

**One-sentence verdict:** the migration premise in the team brief is wrong
(this is a Qt 6/Kirigami app, not GTK4), the most expensive code to port
(`bridge.py`, which has ~zero unit tests) is exactly the code a rewrite is
most likely to silently break, and the libcosmic dependency stack (git-pinned
`iced@master`, a forked `accesskit`, `rust-version = "1.93"`) is the least
stable foundation this app has ever stood on. Details and kill criteria below.

---

## 1. Premise audit — the brief describes an app that no longer exists

### 1.1 What the brief claims vs. what HEAD contains

The migration brief given to the team describes the app as **GTK4** with
"GObject subclasses, signals, callbacks, async usage", and contains unfilled
placeholders (`[APP NAME]`, `[LANGUAGE]`). That description matches the app
*before* commit `3c4b735`. It does not match HEAD.

Evidence:

- `git show 3c4b735 --stat` (commit message: *"Rewrite the interface on Qt 6
  and Kirigami, replacing GTK/libadwaita entirely"*). The stat shows the GTK
  layer deleted outright:
  `gamehandler/add_game_dialog.py | 583 ------` (deleted),
  `gamehandler/runners_dialog.py | 343 ------` (deleted),
  `gamehandler/settings_page.py | 227 -----` (deleted),
  `gamehandler/style.css | 127 ---` (deleted),
  `gamehandler/window.py | 925 -------------------` (deleted),
  replaced by `gamehandler/bridge.py | 1072 ++++` (new) plus 9 QML files.
- `git log --oneline`: `3c4b735` landed 2026-08-28; five further commits sit on
  top of it (up to `cbaf7e6`, "Fix unittest discovery without PySide6 for
  0.7.2"). The GTK code is two minor releases in the past, not the present.
- A repo-wide grep for `import gi`, `gi.repository`, `Adwaita`,
  `libadwaita`, `Gtk.` across `*.py`/`*.qml` returns **zero hits in
  application code** — only self-referential mentions in
  `data/com.goshapps.GameHandler.metainfo.xml:65` ("nothing of the old GTK
  and libadwaita stack…") and the regression test that enforces their absence
  (`tests/test_packaging.py:90-105`,
  `test_no_gtk_or_adwaita_remains_anywhere`).
- `README.md:52-60` tech-stack table: "Language: Python 3. UI toolkit: Qt 6 +
  Kirigami (PySide6 + QML). Build system: Meson. Packaging: Flatpak (KDE
  runtime)."
- `gamehandler/main.py:1-7,74-115`: the GUI entry point imports
  `PySide6.QtGui/Qml/Widgets`, sets `QT_QUICK_CONTROLS_STYLE` to
  `org.kde.desktop`, and loads `gamehandler/qml/Main.qml` through a
  `QQmlApplicationEngine`. There is no Gtk import anywhere on this path.
- Flatpak manifest
  (`build-aux/flatpak/com.goshapps.GameHandler.json:31-35`): runtime
  `org.kde.Platform` 6.10, SDK `org.kde.Sdk`, base `org.winehq.Wine`
  `stable-25.08`. No GNOME runtime, no libadwaita module.

### 1.2 What the correct parity target therefore is

The parity target is **GameHandler 0.7.2 as a Python 3 + PySide6 + QML
(Kirigami) application**:

- Backend logic in `gamehandler/*.py` (~5,200 lines incl. `runners.py` at
  1,547 lines; counts from `wc`).
- All user-visible behavior mediated by exactly one object: `Backend` in
  `gamehandler/bridge.py` (1,072 lines) — every QML page talks only to the
  `backend` context property (`main.py:109`).
- UI in `gamehandler/qml/*.qml` (9 files, ~1,760 lines).
- Behavior contract in `tests/` (15 files, ~2,300 lines, 241 tests).
- Feature list in `README.md:21-44` — treated as the checklist authority in §2.

### 1.3 Consequences of the wrong premise (flagged loudly, as instructed)

Any migration plan written against "GTK4 → libcosmic" will:

1. **Budget the wrong work.** It assumes throwaway UI glue over GObject, when
   the real cost center is `bridge.py`'s threaded dispatch model (§3.1) and
   `runners.py`'s Wine environment construction (§3.2) — neither of which has
   anything to do with GTK.
2. **Misidentify the toolkit delta.** Qt already gives this app: automatic
   accessible object tree for all Controls, `FileDialog` with network-share
   URLs, `QDesktopServices.openUrl`, passive notifications with action
   buttons, `QIcon` theme fallback to Breeze (`main.py:92-95`), and an
   offscreen QML smoke test (`tests/test_qml_smoke.py`). A plan that credits
   these to "the old GTK app" will fail to require them of the Rust port.
3. **Misread the tests.** `test_packaging.py:90` *fails the build* if GTK
   idioms reappear; the QML smoke test instantiates every page against the
   real backend. These are Qt-era assets a GTK-based plan doesn't know exist.
4. Ship placeholders: the brief's `[APP NAME]` / `[LANGUAGE]` were never
   filled in, which suggests no one re-read it after the Kirigami rewrite.
   **Recommendation: the brief must be reissued against the Qt/Kirigami
   baseline before any implementation plan is approved.**

---

## 2. Independent parity checklist

Built from `README.md:21-44` cross-checked against the code. Each item names
its implementation (`file:line`) and how to verify it. Numbered **P-01…P-78**;
this numbering is the proposed acceptance standard for the finished app.

### Library page (`gamehandler/qml/LibraryPage.qml`, backend in `bridge.py:275-455`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-01 | Grid view of games with cover tiles | `LibraryPage.qml:132-219`, `CoverArt.qml` | Add 3 games with covers; tiles render 200×300 |
| P-02 | List view with cover thumbnail + subtitle + last-played | `LibraryPage.qml:223-287`, `bridge.py:304-322` (`_game_row`) | Toggle view; row shows "Category · Runner · Played X" |
| P-03 | Grid/list toggle persisted | `LibraryPage.qml:67-74`, `settings.py:14`, `bridge.py:207-216` | Toggle, restart, still list |
| P-04 | Search matches name *or* category, case-insensitive | `LibraryPage.qml:30-36`, `models.py:188-199` | Search "shoot" finds Doom via category (cf. `test_library_view.py:41`) |
| P-05 | Category filter dropdown ("All" + categories) | `LibraryPage.qml:38-54`, `models.py:201-203` | Filter Strategy shows only Civ |
| P-06 | Sort: Name / Recently played / Recently added | `LibraryPage.qml:56-65`, `bridge.py:217-230`, `models.py:156-163` | Add games, play one, check orders |
| P-07 | Blank category folds into "Uncategorized", sorted last | `models.py:60-62,201-203` | Add game with empty category |
| P-08 | Empty-library placeholder with 3 onboarding buttons | `LibraryPage.qml:82-110` | Fresh profile shows Add / Easy install / Download runner |
| P-09 | No-results placeholder with Clear filters | `LibraryPage.qml:112-128` | Search gibberish, clear |
| P-10 | Double-click (grid) / double-click row (list) plays | `LibraryPage.qml:159-162,239` | Double-click launches |
| P-11 | Right-click context menu on card and row | `LibraryPage.qml:163-166,240-243,298-343` | Right-click shows 9-item menu |
| P-12 | Play / Edit / Find cover art menu items | `LibraryPage.qml:301-315`, `bridge.py:457-486,537-559` | Each works from menu |
| P-13 | Winecfg / Winetricks / Open prefix folder (Windows only, disabled for Linux games) | `LibraryPage.qml:317-332`, `bridge.py:487-519` | Disabled state visible on Linux-native entry |
| P-14 | Create desktop shortcut | `LibraryPage.qml:334-337`, `bridge.py:521-532`, `runners.py:1449-1473` | `.desktop` appears, launches game |
| P-15 | Remove with confirm dialog; prefix + files left on disk | `LibraryPage.qml:345-360`, `bridge.py:447-454` | Remove; prefix dir still exists |
| P-16 | "Last played" human labels incl. future-timestamp clamp | `models.py:93-117` | Set clock-skewed timestamp; label never negative (cf. `test_library_view.py:161`) |
| P-17 | Runner label per row ("Linux native" or "Name · Family") | `bridge.py:299-302,757-766` | Row subtitle correct for GE vs system wine |

### Add/Edit game form (`GameFormPage.qml`, `bridge.py:356-445`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-18 | Add (Ctrl+N) and Edit paths, Add/Save gated on non-blank name | `Main.qml:121-125,47-54`, `GameFormPage.qml:32-36`, `bridge.py:410-413` | Blank name refuses with "A game needs a name" |
| P-19 | Windows vs Linux-native type switch; Linux disables runner/prefix/Wine toggles | `GameFormPage.qml:87-92,172-191,216-312`, `bridge.py:419-420` | Switch to Linux; Wine section greys out |
| P-20 | Executable browse with `*.exe` filter; auto-fills name from basename | `GameFormPage.qml:334-347` | Browse, name populates |
| P-21 | Share URLs (`smb://`, `file://`) resolved to local paths on save *and* on browse | `bridge.py:414-418,601-603`, `netpaths.py:144-167` | Paste `smb://server/share/g.exe` with mounted share; stored path is `/run/user/…/gvfs/…` |
| P-22 | Launch arguments, working directory, free-text fields | `GameFormPage.qml:109-119`, `bridge.py:415-417` | Args with quotes pass through (`runners.py:406-407` uses `shlex.split`) |
| P-23 | Editable category combobox (preset list + custom) | `GameFormPage.qml:126-132`, `bridge.py:344-350` (`formCategories`) | Type new category; appears in filter |
| P-24 | Cover preview + Find cover (Steam → exe-icon fallback) + custom image import (png/jpg/jpeg/webp) | `GameFormPage.qml:134-165,349-361`, `bridge.py:561-609`, `covers.py:359-382,295-304` | Find cover on "Half-Life 2"; import a `.webp` |
| P-25 | Steam genre auto-categorises game when Uncategorized | `bridge.py:553-554`, `covers.py:237-250,46-73` | Lookup sets category e.g. Action |
| P-26 | Runner picker + optional per-game prefix (empty = isolated `<prefixes>/<id>`) | `GameFormPage.qml:172-191`, `bridge.py:420-421`, `runners.py:394,479` | Leave empty; prefix created per game id |
| P-27 | All 15 per-game toggles: MangoHud, GameMode, Prefer SDL, Wayland, HDR, Esync, Fsync, Gamescope, DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye, EAC, virtual desktop | `GameFormPage.qml:198-305`, `bridge.py:74-78`, `models.py:34-48` | Toggle each; relaunch shows env effect |
| P-28 | Virtual-desktop size field, validated, default 1920x1080 | `GameFormPage.qml:306-312`, `runners.py:1088-1092,1095-1101` | Enter garbage; launches with 1920x1080 |
| P-29 | Additional app launched alongside in same prefix | `GameFormPage.qml:319-324`, `runners.py:1414-1417` | Set helper exe; both processes start |
| P-30 | Custom `KEY=value` env block, overrides toggles, supports quoting/`;` | `GameFormPage.qml:325-330`, `runners.py:1025-1026,1258-1259,966-1023` | `FOO="a;b"` survives as one value |
| P-31 | Saving without cover triggers auto-fetch | `bridge.py:444-445` | Save coverless game; toast "Cover set from …" |

### Runners (`RunnersPage.qml`, `runners.py`, `bridge.py:611-781`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-32 | Installed list: System Wine (version or "Not installed") + downloaded builds with family label | `RunnersPage.qml:51-88`, `bridge.py:621-646` | Row shows e.g. "GE-Proton9-5 · Proton-GE" |
| P-33 | Download families: Proton-GE, RTSP, CachyOS, EM, Wine-Vanilla/Staging/Staging-Tkg/Proton (8) + System Wine guide row | `runners.py:93-200,232-253`, `bridge.py:647-673` | 9 guide cards each with maintainer + homepage |
| P-34 | Per-family release list (≤12), tag + asset + MB + Installed chip | `RunnersPage.qml:168-205`, `bridge.py:675-717`, `runners.py:815-834` | Select family; 12 rows appear |
| P-35 | Download with progress bar, busy-guard, completion toast | `RunnersPage.qml:33-38`, `bridge.py:737-769`, `runners.py:864-920` | Install; bar moves; "Installed …" toast |
| P-36 | Release asset filtering (require/exclude/prefer tokens per family) | `runners.py:266-300` | CachyOS list has no `v3`/`znver4` assets |
| P-37 | Runner removal with confirm; games fall back to System Wine | `RunnersPage.qml:256-271`, `bridge.py:771-781`, `runners.py:754-756` | Remove in-use runner; game still launches |
| P-38 | Secure extraction: traversal/symlink confinement, size/member caps, no-replace atomic rename, staging cleanup | `runners.py:599-690,922-928` | Covered by `test_security.py:97-190,266-420` — re-run equivalents |
| P-39 | Runner tags sanitised to collision-resistant install ids | `runners.py:535-553,326-343` | `release/v1` → `release-v1~h<hash>` |

### Launch semantics (`runners.py:966-1426`, `bridge.py:457-486`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-40 | Proton via `umu-run` with `PROTONPATH`/`GAMEID`/`STORE`; plain-wine fallback when umu absent | `runners.py:485-510` | Launch with/without umu-run on PATH |
| P-41 | Proton-only gates: NVAPI, FSR, Wayland raise without umu+proton | `runners.py:1402-1407` | Enable NVAPI on system wine → error toast, no launch |
| P-42 | Esync/Fsync/DXVK-off/VKD3D-off env mapping incl. `PROTON_NO_*` and dll overrides | `runners.py:1195-1230,1026-1035` | Inspect env of launched process |
| P-43 | MangoHud prepend-or-`MANGOHUD=1`, GameMode prepend, Gamescope wrap (+`--hdr-enabled`), error if gamescope missing | `runners.py:1234-1256` | Enable Gamescope without binary → actionable error naming the Flathub extension |
| P-44 | Anti-cheat runtimes auto-located (Flatpak Steam paths included), empty-string disable | `runners.py:1218-1230,1113-1154` | Enable EAC with runtime present → `PROTON_EAC_RUNTIME` set |
| P-45 | Bundled DXVK installed into raw-Wine prefixes once per version | `runners.py:1038-1085,1408-1410` | Launch on raw wine; `drive_c/windows/system32/d3d11.dll` appears + marker file |
| P-46 | Immediate-failure detection: 6 s grace, stderr tail, toast + window restore | `runners.py:1315-1375`, `bridge.py:473-485`, `Main.qml:170-179` | Launch bad exe with close-on-launch; window returns with reason |
| P-47 | `mark_played` timestamp + "Launching …" toast | `bridge.py:467-469`, `models.py:182-186` | lastPlayed updates |
| P-48 | Winecfg/Winetricks run with the game's own WINE+WINESERVER | `runners.py:1476-1500` | Prefix created by GE opens under its own wine |
| P-49 | Open prefix folder (Proton `pfx` layout aware) | `bridge.py:503-519`, `runners.py:352-355` | Opens `…/pfx/drive_c` for Proton games |
| P-50 | Linux-native launch (no Wine env at all) | `runners.py:1264-1270,1387-1389` | Native game launches; no `WINEPREFIX` in env |

### Easy installers (`InstallersPage.qml`, `installers.py`, `bridge.py:783-948`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-51 | Catalog of 9: Battle.net, Epic, EA App, Ubisoft, GOG, Amazon, Rockstar, Steam (+Discord under Apps) | `installers.py:63-225` | 9 cards; Discord chip reads "Apps" |
| P-52 | Search + Launchers/Apps filter | `InstallersPage.qml:12-27`, `installers.py:241-256` | Search "epic"; filter Apps → Discord only |
| P-53 | Per-install runner choice, isolated prefix per install | `InstallersPage.qml:45-73`, `bridge.py:840-846,649-653` | Install creates `<prefixes>/<uuid>` |
| P-54 | Download (≤1 GiB) with origin-allowlist + magic-byte + Authenticode (osslsigncode, approved publisher, MS root for Ubisoft) checks before execution | `installers.py:551-646` | Tampered binary → "invalid Authenticode signature", nothing executed |
| P-55 | msi via `msiexec /i`, exe directly; no Proton-only vars under raw Wine | `installers.py:258-291` | Epic uses msiexec path |
| P-56 | Wizard wait: poll expected exe throughout; wineserver-idle slicing; 6 h ceiling; per-user profile + case-insensitive + basename fallback search | `installers.py:360-530` | Covered by `test_installers.py:487-560` |
| P-57 | Fallback "Locate …" dialog when exe not found; cancel keeps prefix with explanatory toast | `Main.qml:163-168,182-188`, `bridge.py:886-948` | Close wizard without installing → Locate dialog → Cancel → "Kept the … prefix" |
| P-58 | Finished install becomes library entry with exe-icon cover and Play-action toast | `bridge.py:905-918`, `Main.qml:154-161`, `installers.py:655-676` | Toast offers Play; cover is the vendor icon |
| P-59 | Concurrent-install guard ("Another install is already running") | `bridge.py:832-835` | Start two installs; second refused |

### Covers (`covers.py`, `CoverArt.qml`)

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-60 | Steam search → scored match (threshold 0.45; 0.9 for single-word queries) → portrait cover download with CDN fallbacks | `covers.py:183-210,333-356,202-210,36-43` | "Steam"/"Discord" must NOT match store pages (cf. `test_covers.py:65-83`) |
| P-61 | Offline exe-icon fallback; store launchers get vendor icon, never wrong Steam art | `covers.py:359-382`, `exe_icons.py` (PE parser) | Airplane mode + Battle.net exe → icon cover |
| P-62 | Generated initials plate, 8 stable gradients, letterboxed icons | `CoverArt.qml`, `theme.py:29-38`, `covers.py:97-111` | Coverless game shows 2-letter plate, stable across restarts |
| P-63 | Download size caps (12 MiB), empty-download rejection, atomic writes | `covers.py:214-226,272-280` | Oversized CDN response refused |

### Plugins / Credits / Settings / Shell / CLI

| ID | Feature | Implements | Verify |
|----|---------|-----------|--------|
| P-64 | Plugins page: 5 helpers with installed/missing/unavailable states; Flatpak never offers host install; source shows `pkexec/sudo` command | `PluginsPage.qml`, `bridge.py:949-1031`, `plugins.py:30-175` | In Flatpak: all missing → "unavailable" + sandbox explanation |
| P-65 | About & Credits: 5 sections, 25 entries, licenses, Visit buttons, "Made by Gosh", version, why-all-in-one | `CreditsPage.qml`, `credits.py:45-289`, `bridge.py:1033-1069` | README section regenerable from `credits.markdown()` (`test_credits.py`) |
| P-66 | Settings: system/light/dark (dark default), grid/list, default runner, 13 default toggles, close-on-launch | `SettingsPage.qml`, `settings.py:17-51`, `bridge.py:188-270` | Change each; restart; persisted in `settings.json` |
| P-67 | Breeze-flavoured palettes; system scheme restores platform palette; Breeze icon fallback | `theme.py:18-89`, `main.py:92-95` | Force light on GNOME; no invisible text |
| P-68 | Shortcuts Ctrl+N / Ctrl+F / Ctrl+, / Ctrl+Q, listed in Settings | `Main.qml:121-143`, `SettingsPage.qml:132-135` | Each works; list matches behavior |
| P-69 | Toasts for every mutation; game-installed toast has Play action; hide/restore on launch | `Main.qml:147-180`, `bridge.py` (39 `notify.emit` sites) | Install → toast with Play button |
| P-70 | `--list`, `--launch ID`, `--version` work headless (no Qt import) | `main.py:28-57,118-131` | Run with `QT_QPA_PLATFORM=offscreen` uninstalled… actually without PySide6 installed at all |
| P-71 | Desktop shortcuts `gamehandler --launch <id>` | `runners.py:1449-1473`, `bridge.py:526` | Shortcut file valid single-group entry, executable bit, `%`/`\n` escaped |
| P-72 | Network-share games launch via GVFS FUSE; unmounted share → instructional error, not silence | `netpaths.py`, `runners.py:1273-1289` | Unmount share; launch → "open … in your file manager" message |
| P-73 | `games.json`/`settings.json` corruption tolerance + atomic saves | `models.py:128-154,149-154`, `settings.py:56-74` | Write `{ garbage`; app starts with empty library, no crash |
| P-74 | Timestamp normalization (bool/string/negative/NaN/Inf → sane) | `models.py:68-87` | Inject `"added": "soon"`; loads, normalized |
| P-75 | Unknown JSON keys ignored (forward compat) | `models.py:66-67`, `settings.py:42-43` | Add `legacy_field`; loads fine |
| P-76 | Categories list with Uncategorized last | `models.py:201-203` | Order check |
| P-77 | Duplicate game ids: last write wins (dict keyed by id) | `models.py:140-147` | Duplicate id entries → one game |
| P-78 | KDE Flatpak specifics: Gamescope extension PATH + required VulkanLayer note; Steam Flatpak anti-cheat roots; `~/.var/…Steam…:ro` | manifest `:24,28`, `runners.py:1118-1126`, `README.md:46-50` | Documented install command works |

Checklist size: **78 items**.

---

## 3. Risk register (ranked)

### R-1 (CRITICAL). `bridge.py` — 1,072 lines of untested threading glue must be re-expressed in a foreign concurrency model

- **Evidence.** `Backend` is the *only* path between UI and logic
  (`main.py:109`). It owns: a hand-rolled thread→UI dispatch
  (`bridge.py:145-166`: `threading.Thread(daemon=True)` + `_dispatch`
  queued signal), 39 `notify.emit` call sites, progress reporting from worker
  threads (`_progress_cb`, `bridge.py:733-735`), a 6-hour blocking wizard wait
  running on a daemon thread (`installEasy`, `bridge.py:857-872`), token-based
  pending-install state (`_pending_installs`, `bridge.py:142,886-948`), and
  launch-watch callbacks (`playGame`, `bridge.py:473-485`).
- **Test gap.** `grep Backend tests/` shows exactly **one** test touching it:
  `test_qml_smoke.py:71-120`, which merely instantiates pages and is
  **skipped when PySide6 is absent** (the CI baseline ran 241 tests with 1
  skip — this one). There are zero unit tests for `saveGame` validation,
  `playGame` error paths, `installEasy`/`completeEasyInstall`/
  `cancelEasyInstall` token flow, `fetchReleases`/`installRelease`/
  `uninstallRunner`, plugin install, or any `notify` text.
- **Why it is the most expensive port item.** iced's Elm architecture
  (`Message` + `Task`/`Subscription`) has no moral equivalent of "fire a
  daemon thread that blocks for 6 hours and pokes queued lambdas at the UI".
  Each flow must be redesigned as cancellable tasks with explicit state
  machines (especially P-56/P-57: the wizard wait + fallback dialog + cancel
  path). Daemon-thread abandonment (e.g. window closed mid-download) becomes
  explicit task-cancellation design. This is a rewrite of the app's nervous
  system with no test net.
- **Mitigation demanded:** port `tests/test_installers.py`-style injected-clock
  tests *first*, and add Backend-level tests (save/play/install flows with a
  headless `Backend` double) *before* writing Rust UI. If the Rust port cannot
  demonstrate the P-57 fallback path in a headless test, it is not ready.

### R-2 (CRITICAL). Fragile dependency pins: git `iced@master` + forked `accesskit` + `rust-version = "1.93"` + submodules

- **Evidence** (all verified in `/tmp/cosmos-spike/libcosmic`):
  - `Cargo.toml:3-5`: `version = "1.0.0"`, `rust-version = "1.93"` — a very
    new toolchain floor.
  - `.gitmodules`: `iced` → `https://github.com/pop-os/iced.git`,
    **branch = master**; `cosmic-icons` → pop-os/cosmic-icons. Submodule
    status pins `iced` at `ffe1f1d` (a moving head, not a tag) and
    `cosmic-icons` at `epoch-1.8.0`.
  - `iced/accessibility/Cargo.toml:13-16`: `accesskit` from
    `https://github.com/wash2/accesskit`, **tag `cosmic-0.14`** — a personal
    fork, not upstream accesskit, fetched by git at build time.
  - `iced/Cargo.toml:209`: workspace `rust-version = "1.92"`.
- **Why it matters.** (a) Any consumer pins libcosmic by git rev; every
  `cargo update`-equivalent pulls a new `iced@master` with no SemVer promise —
  the app can break without touching its own code. (b) The Flatpak build
  needs a Rust 1.93-capable toolchain inside `org.kde.Sdk` 6.10; SDK Rust
  lags stable routinely — Packaging must prove the toolchain exists or vendor
  one (huge build-time cost: iced+libcosmic from source is a multi-GB,
  30-plus-minute compile). (c) Offline/flatpak-builder source pinning must
  cover *git* submodules-of-submodules (accesskit inside iced's tree) —
  `flatpak-builder` git sources do not recurse submodules by default.
- **Mitigation demanded:** vendor the full source tree (libcosmic + iced +
  cosmic-icons + accesskit fork) as checksummed tarballs; record exact revs;
  CI builds from a cold cache weekly to catch upstream drift; do not track
  `master`.

### R-3 (HIGH). Wine env construction (`runners.py:966-1426`) is subtle, security-sensitive, and behavior-pinned by 60 tests

Hard parts, each with dedicated tests that must be re-proven in Rust:

- `parse_env_block` (`runners.py:966-1023`): quote-aware `;`/newline
  splitting, `shlex` fallback — 1:1 port required, tests are portable.
- `wine_prefix_root` / Proton `pfx` indirection (`runners.py:358-370`): point
  raw Wine at the wrong level and the game "installs fine then refuses to
  start". `tool_command` pins WINE+WINESERVER beside the game's own wine
  (`runners.py:1476-1500`).
- `uses_proton_runtime` gating (`runners.py:1157-1169`): Proton-only vars are
  *meaningless and misleading* on plain Wine; NVAPI/FSR/Wayland hard-raise.
- `_ErrorTail` + `LaunchedGame.failure` (`runners.py:1315-1375`): bounded
  64 KiB stderr drain on a thread; 6 s grace; fixme/warn/trace/info filtering;
  4-line/240-char cap. A naive `Command::output()` port either deadlocks the
  game (full pipe) or loses the error.
- `_rename_noreplace` via `renameat2(RENAME_NOREPLACE)` through ctypes
  (`runners.py:663-689`): TOCTOU-safe install; Rust port needs `libc::
  renameat2` directly — available, but must be a deliberate choice, not
  `fs::rename`.
- `extract_archive` bounded streaming + `data_filter` equivalent
  (`runners.py:599-629`): **Rust's `tar` crate has no `data_filter`**; the
  traversal/symlink/pax-bomb validation must be hand-rolled and re-proven
  against every case in `test_security.py:97-190`.
- `wait_for_installer` polling state machine (`installers.py:360-408`) with
  injected `sleep`/`clock` — portable design, but the Rust port must keep the
  injectable clock or those 8 tests (`test_installers.py:487-560`) die.

### R-4 (HIGH). Sandbox: a libcosmic app needs everything the Qt app has, plus a GPU story

Current sandbox facts (`build-aux/flatpak/com.goshapps.GameHandler.json`):
`--filesystem=home`, `--device=all`, `--allow=multiarch`,
`--filesystem=xdg-run/gvfs`, Wine BaseApp `stable-25.08`,
`Compat.i386`+`GL32` inherit-extensions, Gamescope VulkanLayer extension on
PATH (`:24`), bundled osslsigncode + DXVK + pinned PySide6.

- `--device=all` already covers GPU device nodes, so wgpu *access* needs no
  new permission — **but** the renderer choice needs a proven fallback: iced
  offers `wgpu` (GPU) and `tiny-skia` (software) backends
  (`iced/Cargo.toml:25-33`). The libcosmic default feature set does not
  obviously force wgpu (its `default` lists `winit,tokio,a11y,dbus-config,
  x11,wayland,multi-window`; `wgpu` is a separate feature,
  `libcosmic/Cargo.toml:101`). **Uncertainty (marked):** I did not fully
  resolve which renderer a default libcosmic `Application` uses or how it
  degrades on a no-GPU / llvmpipe-only / headless box. The adversarial test
  plan (§4) includes no-GPU runs precisely because an iced app that assumes
  wgpu will fail to open *at all* where today's Qt app falls back to
  software GL.
- The Wine BaseApp + multiarch + i386 extensions are renderer-independent and
  must be carried over verbatim; a "clean room" manifest rewrite risks
  dropping `GL32` (game rendering) or `--allow=multiarch` (32-bit games,
  downloaded Wine builds).
- New risk the Qt app doesn't have: compiling iced/libcosmic inside the KDE
  SDK (see R-2) — the current manifest's hardest module is already PySide6
  from source (`test_packaging.py:46-74` pins its sha256 and build flags); a
  Rust toolchain + vendored registry will be bigger.

### R-5 (HIGH). Accessibility: no regression *only* because the baseline is already thin — and Qt's free baseline disappears

- **Current state, verified by grep:** zero `Accessible.*` properties and zero
  `accessibleName`/`description` in all 9 QML files and all Python. The app
  relies entirely on **Qt's automatic accessible tree** (every Quick Control
  gets a role/name from its `text`), exposed over AT-SPI on Linux for free.
- **libcosmic side, verified:** `a11y` is a default feature
  (`libcosmic/Cargo.toml:12-22`), plumbed as
  `iced/a11y → iced_accessibility → accesskit`, with per-widget
  `description: Option<Description>` fields (e.g.
  `iced/widget/src/button.rs:91`). The backing is `accesskit` from a **fork**
  (`wash2/accesskit`, tag `cosmic-0.14`), and the winit platform backend
  (`accesskit_winit`) is optional/gated
  (`iced/accessibility/Cargo.toml:8-16`). **Uncertainty (marked):** I did not
  verify that a Flatpak'd libcosmic app actually emits AT-SPI events on a
  stock GNOME/KDE session (that requires the platform backend + running
  at-spi bus, and the fork's winit integration is young).
- **Risk:** per-widget a11y in iced is *opt-in per call site* (`describe()` /
  `description`). A faithful widget-for-widget port with no explicit
  annotations will produce an app that is **less** accessible than today's
  zero-effort Qt baseline (nameless custom widgets like the cover grid,
  toasts without live-region equivalents). Every interactive element needs an
  explicit label task; Orca walkthrough of Library → form → install must be an
  acceptance gate (P-items to be annotated when the final app exists).

### R-6 (MEDIUM). i18n: nothing to regress, but the port must not cement English-only

- **Verified:** no `qsTr`/`qsTranslate` in any QML, no `gettext`, no `.po`/
  `.pot` anywhere. Every user string (including 39 toast texts and all form
  labels) is a hardcoded English literal. So there is **no i18n regression
  risk** — there is nothing to lose.
- The opportunity/risk: libcosmic ships a real i18n story (`i18n-embed` +
  `i18n-embed-fl`, `i18n.toml` with `assets_dir = "i18n"`, dozens of locales
  under `i18n/`). If the Rust port hardcodes `format!()` English strings the
  way the Python app does, it mortgages the one part of the migration that
  could have been a strict improvement. Require `fl!()` from day one.

### R-7 (MEDIUM). Untested code paths — the exact list a rewrite will silently break

Read every test file; the following shipped behaviors have **no test today**:

1. **All of `bridge.py` except construction** — saveGame validation/notify
   texts, playGame error/close-on-launch paths, runPrefixTool/openPrefix/
   createShortcut, fetchCover/fetchCoverForForm/importCustomCover,
   fetchReleases/installRelease/uninstallRunner busy+progress transitions,
   installEasy/complete/cancel token lifecycle, installPlugin flows,
   sortMode/viewMode/colorScheme setters (§R-1).
2. **`main.py` CLI entirely** — `_launch_from_cli` (incl. the "stopped right
   away" stderr contract), `_list_games`, `--version`, Kirigami-missing
   error path. A Rust port that regresses `--launch` breaks every desktop
   shortcut users already created (P-71).
3. **`launch()` integration** — unit-tested env builders, but no test that
   `launch()` wires resolve→build→gate→dxvk→wrap→spawn together, creates the
   prefix dir, defaults cwd to the exe's parent, or spawns `additional_app`.
4. **`resolve_game_paths` failure text** for unmounted shares (P-72) — the
   message exists (`netpaths.py:170-177`) but no test pins it; easy to drop.
5. **`format_last_played` boundaries** — partially covered
   (`test_library_view.py:149-161`); singular/plural edges ("1 hours"?) rely
   on eyeballing.
6. **`theme.py` palette application** — no test that dark/light palettes apply
   or that "system" restores the platform palette.
7. **QML-only logic** — exe-dialog auto-name (`GameFormPage.qml:341-344`),
   category-reset on `categoriesChanged` (`LibraryPage.qml:44-53`), sort-box
   init (`LibraryPage.qml:63`), runner-box fallback to index 0 when the saved
   runner is gone (`GameFormPage.qml:179-182`, `SettingsPage.qml:83-86,
   InstallersPage.qml:54-64`) — the "runner removed from disk while selected"
   path (§4) lives here.
8. **`find_anticheat_runtime` search order** and Flatpak Steam roots
   (`runners.py:1113-1154`) — lightly covered; order matters.
9. **`download_installer` end-to-end** (redirect→magic→osslsigncode→atomic
   replace) is mocked at each step; no test runs the real `osslsigncode`
   binary path selection incl. `-CAfile` injection for Ubisoft.

### R-8 (MEDIUM). Behavioral divergences: Rust ≠ Python in five concrete places

1. **JSON `NaN`/`Infinity`.** Python's `json.loads` *accepts* `NaN`,
   `Infinity`, `-Infinity` by default; `models.py:68-87` then normalizes them
   to sane timestamps. `serde_json` **rejects** them at parse time. A library
   hand-edited or written by an older build containing `NaN` loads (empty
   timestamps) in Python but fails the whole file in naive Rust. The port must
   parse leniently (or pre-scan) and apply the same normalization table,
   including the bool-rejection rule (`isinstance(value, bool)` check,
   `models.py:73-76` — JSON `true` must not become timestamp `1.0`).
2. **Atomic-save + fsync semantics.** `Library.save`/`Settings.save` write
   temp + `os.replace` (`models.py:149-154`, `settings.py:69-74`). Rust must do
   temp + `rename` (and ideally fsync) — and, critically, **write byte-
   compatible files**: `json.dumps(..., indent=2)`, same keys, so a Rust-
   written `games.json` still loads in the Python build (downgrade path) and
   `git diff` of the file stays clean.
3. **Sorting.** Both sides lowercase-then-codepoint-sort
   (`models.py:156-163` uses `str.lower()`), and UTF-8 byte order == codepoint
   order, so `to_lowercase()` sorting is *equivalent* — I checked rather than
   assumed. The real gap: **neither side does locale collation**, so
   keep-it-that-way is the requirement; do not "improve" with ICU collation or
   sort orders change for existing users.
4. **Float timestamps.** Python writes `repr` shortest-round-trip floats;
   `serde_json` likewise. `f64` precision must be preserved exactly — `last_
   played`/`added` feed "recent" sort keys (`models.py:160-162` negates them;
   float error could reorder ties; tie-break is `name.lower()`).
5. **Path handling.** `as_local_path` (`netpaths.py:144-167`) does URL
   unquoting (`%20` → space), `file://` unwrapping, GVFS candidate probing
   with `exists()` checks, and returns the *original* on failure. A Rust port
   using `url`+`PathBuf` must reproduce: percent-decoding, the
   most-specific-first SMB mount-name order (`netpaths.py:48-59`, incl. the
   legacy-lowercased-share variant), the directory-scan fallback
   (`netpaths.py:104-141`), and the return-original-on-unresolvable contract
   callers depend on.

### R-9 (MEDIUM). Feature-loss risks: cheap in Qt, costly or absent in iced/libcosmic

Ranked by likelihood × user impact (all checked against the
`/tmp/cosmos-spike/libcosmic` tree):

1. **File dialogs over network shares (P-21/P-72).** Qt's `FileDialog`
   returns `smb://` URLs the app resolves itself. libcosmic's chooser is the
   ashpd portal (`src/dialog/file_chooser/`: `ashpd … SaveFileRequest` /
   `rfd` fallback). Portals return mounted paths when they show shares at
   all — hand-typed `smb://` URLs and the "not mounted yet, open in file
   manager" guidance (`netpaths.py:170-177`) have no portal equivalent and
   must be rebuilt as custom UI. Likely outcome: the beta "supports files,
   not shares" — a advertised README feature (P-72) silently lost.
2. **The easy-install fallback dialog (P-57).** `easyInstallNeedsExe →
   FileDialog → complete/cancel` is a three-hop async token flow across the
   thread boundary with zero tests. In Elm terms it is a task that suspends
   for *user input* mid-wizard-wait — the single most awkward control flow in
   the app. Highest single-feature loss probability.
3. **Toasts with actions (P-58/P-69).** Qt `showPassiveNotification(msg,
   "long", "Play", fn)`; libcosmic has a `toaster` widget with action support
   (`src/widget/toaster/mod.rs:33-38` shows `toast.action`), so the path
   exists — but duration, queueing, and the close-on-launch hide/show dance
   (`Main.qml:170-179`) must be deliberately rebuilt.
4. **`.ico` cover display (P-62).** The `image` crate with `ico` feature is in
   libcosmic's tree (`Cargo.toml:139-143`: `ico, jpeg, png`), so decoding is
   available — but the letterboxed-plate presentation (`CoverArt.qml:53-66`)
   and `.ico`-detection plumbing (`bridge.py:313`) are custom work that reads
   as "polish" and gets cut first.
5. **Keyboard shortcuts + focus (P-68, Ctrl+F focus-search).** libcosmic has
   `keyboard_nav`, but global shortcuts and programmatic focus are framework
   friction in iced; expect these to arrive late or never.
6. **Right-click context menus (P-11).** `widget/context_menu.rs` exists, so
   feasible — flagged only because every menu item's enabled-state logic
   (P-13) must be re-derived from message state.
7. **Breeze icon-theme fallback (P-67).** `cosmic-icons` submodule replaces the
   Freedesktop/Breeze lookup (`main.py:92-95`); icon *names* used across QML
   (`applications-games`, `media-playback-start`, `view-more-symbolic`…)
   must be re-mapped onto the COSMIC icon set or Elliott-era symbolic names
   render as missing-image boxes.

### R-10 (LOW, but noted). Non-functional deltas

- **Startup/resource use** will likely *improve* (no PySide6 import, no QML
  engine warm-up) — not a risk, and not a justification either.
- **Build time** will severely regress (R-2): iced + libcosmic from source
  dwarfs the current PySide6 module build. Contributor onboarding suffers.
- **Crash behavior**: Python tracebacks on stdout vs Rust panics; the CLI
  `--launch` contract (exit codes + stderr text, `main.py:28-46`) must be
  preserved byte-for-thought, since desktop files depend on it.

---

## 4. Adversarial test plan — how I will try to break the finished app

Each attack names the current behavior (the oracle) and the breakage I expect
from a naive port. All are executable without network except where noted.

**Persistence & compat (the downgrade contract)**

- T-01. `games.json` written by the Python build (all 15 toggles, custom env,
  unicode names, `steam_appid`, future `last_played`) opens in the Rust app
  with every field intact — and a Rust-written file re-opens in the Python
  build. Diff field-by-field; any dropped key fails.
- T-02. Corrupt `games.json` (`{ not valid json`), JSON object instead of
  list, non-dict entries, `{"name": "x", "added": "soon", "last_played":
  true, "added": NaN}` — app starts, library empty-or-normalized, no crash,
  no data loss of *other* entries (oracle: `models.py:128-147`,
  `test_library_view.py:93-132`). **Naive-`serde_json` ports fail T-02 on
  `NaN` alone** (R-8.1).
- T-03. Kill the app mid-save (SIGKILL during runner install + library
  update); restart. Atomic temp+rename must leave either old or new file,
  never a truncation.
- T-04. Duplicate game ids in one file → exactly one entry survives
  (oracle: `models.py:147` last-wins).

**Launch & runners**

- T-05. No Wine on PATH, no downloaded runners: every Windows game shows a
  clear error; nothing spawns; exit paths don't wedge the UI busy flag.
- T-06. Runner directory deleted from disk while selected (game, default
  runner, installer runner box): launch falls back to System Wine
  (`runners.py:749-756`); dropdowns reset to index 0
  (`GameFormPage.qml:179-182`); no panic on missing dir.
- T-07. Prefix that does not exist / is a file / is unreadable: clean error,
  prefix auto-created only on the launch path (`runners.py:1399-1400`).
- T-08. Game whose exe prints 100 MB of `fixme:` noise then exits 1 within
  2 s: toast shows ≤240 chars of *useful* tail (oracle:
  `runners.py:1303-1312`); game process itself must not stall on a full pipe
  (unbounded-capture ports hang here).
- T-09. Headless (`HEADLESS=1`/no Wayland/X) and no-GPU (llvmpipe only,
  `/dev/dri` hidden): CLI `--list`/`--launch` work; GUI either starts on
  software rendering or fails with a *diagnostic*, never a wgpu panic
  traceback (R-4).
- T-10. NVAPI/FSR/Wayland toggles on System Wine without umu: launch refused
  *before* spawn with the exact gate message (oracle: `runners.py:1402-1407`).
- T-11. Gamescope enabled, binary absent in Flatpak: error names the
  `org.freedesktop.Platform.VulkanLayer.gamescope//25.08` extension
  (oracle: `runners.py:1253-1256`).
- T-12. `additional_app` set + main exe missing: helper must not launch alone
  into a void; error names the missing exe.

**Installers**

- T-13. Concurrent downloads (runner + installer, or two installers): second
  refused with busy message; progress bars don't cross-talk (shared
  `_progress` in `bridge.py:724-735` is a known smell — the port must do
  *better*, per-task progress).
- T-14. Installer download that redirects off-allowlist, serves non-MZ bytes,
  or carries a wrong-publisher signature: rejected *before* execution, temp
  file removed (oracle: `test_installers.py:101-158`).
- T-15. Wizard that never writes the exe: give up at deadline, offer Locate;
  cancel → "Kept the … prefix" and busy flag cleared (oracle: P-57). Then:
  close the app mid-wizard-wait — no orphaned `wineserver -w`, no wedged
  busy state on restart.
- T-16. Ubisoft installer without the bundled MS root CA present: verification
  fails closed with a comprehensible message, not an `osslsigncode` stderr
  dump.

**Paths, names, locales**

- T-17. Unicode game names (CJK, emoji, RTL, `İ`/`ß` edge cases): sort order
  matches Python build exactly (R-8.3); initials plate shows sane glyphs
  (oracle: `covers.py:97-104` splits on `[^0-9A-Za-z]` — non-Latin names all
  render "?" today; the port must reproduce *or* deliberately improve, not
  accidentally mojibake).
- T-18. `smb://server/share/Game/g.exe` with share unmounted: instructional
  error naming the host (oracle: `netpaths.py:170-177`); mount it mid-session
  via file manager; retry works without re-adding the game.
- T-19. `file://` URL with `%20`, exe path with trailing whitespace/newline,
  working dir that is a URL: all normalize on save (oracle:
  `netpaths.py:144-167`, `bridge.py:414-418`).
- T-20. Game named `Doom\nExec=/bin/sh …` (desktop-injection): shortcut file
  has exactly one `Name=`/`Exec=`, `%` doubled (oracle:
  `test_security.py:422-451`).
- T-21. 40 000-file prefix + missing exe: prefix scan stays bounded
  (`installers.py:454-456` caps); UI never freezes (port must keep search off
  the UI thread).

**Covers & net**

- T-22. Offline: add "Battle.net" with its exe present → icon cover, zero
  network; add gibberish title offline → surfaced Steam error, no hang past
  timeout (oracle: `covers.py:359-382`).
- T-23. Adversarial Steam answers ("Steam" → "DCS World Steam Edition"):
  single-word queries must not hang чужое art (oracle:
  `test_covers.py:65-83`, threshold 0.9). Re-run those tests against the Rust
  scorer with identical fixtures.
- T-24. CDN serving infinite stream / 1 KiB "image": refused by size caps
  (oracle: `covers.py:214-226,272-280`).

**State & a11y**

- T-25. Remove the game currently shown in the Edit form / while its cover
  fetch is in flight: no use-after-remove, no toast for a deleted game
  (oracle: `bridge.py:548-549` re-checks `library.get`).
- T-26. Orca/screen-reader walkthrough of Library → Add Game → Installers
  with all 15 toggles: every control announced (R-5). Compare against Qt
  baseline recording.
- T-27. Rapid toggle storm (flip 15 toggles + save in <1 s) and double-Play
  clicks: exactly one save, exactly one launch-watcher.

---

## 5. Kill criteria — when not to ship, and the case for not doing it

### 5.1 Ship-blockers (any one kills the release)

1. **Round-trip breakage.** A Rust-written `games.json`/`settings.json` that
   the Python 0.7.2 build cannot read (or vice versa) — users must be able to
   downgrade. Tested by T-01/T-02.
2. **Silent launch failures.** If `LaunchedGame.failure` semantics (P-46) are
   not reproduced — quick exits reported with cause, window restored on
   close-on-launch — the port is worse than every released version since the
   failures were fixed (#20, #21) and must not ship.
3. **Sandbox regression.** If the Flatpak loses any of: Wine BaseApp,
   multiarch/32-bit game support, `--filesystem=home` game access, GVFS share
   launch, GPU rendering of games, or Gamescope extension wiring — not shippable.
4. **Security-validation regression.** If any `test_security.py` case
   (traversal, symlink escape, no-replace atomicity, Authenticode gating,
   desktop injection) has no passing Rust equivalent, the port is a CVE
   factory and must not ship.
5. **A11y worse than baseline.** If the Orca walkthrough (T-26) shows fewer
   announced controls than the Qt build's free accessible tree, hold the
   release.
6. **No-headless CLI.** If `--launch` requires a display/GPU (iced init on the
   CLI path), every user desktop shortcut breaks (P-70/P-71). Kill.

### 5.2 The strongest case for NOT doing this migration

The honest adversarial position:

- **The app just finished its rewrite.** The GTK→Kirigami migration
  (`3c4b735`, Aug 2026) is eight weeks old. Its 241 tests, GVFS support,
  Authenticode verification, secure extraction, and prefix-layout fixes were
  all paid for *recently*. A second full rewrite now discards that investment
  while the paint is still wet.
- **The logic is already toolkit-free.** `runners.py`, `installers.py`,
  `covers.py`, `exe_icons.py`, `netpaths.py`, `plugins.py`, `credits.py` are
  Qt-free by design ("intentionally free of any UI dependency",
  `runners.py:3-4`) and fully headless-tested. Only `bridge.py` + QML are
  Qt-coupled — roughly a quarter of the codebase. The migration's stated
  prize (COSMIC integration) requires touching 100% of it.
- **The target is a moving foundation.** Qt 6 promises source compatibility
  across major versions; today's app builds against a stable KDE 6.10
  runtime. libcosmic 1.0.0 sits atop `iced@master` (no tags, no SemVer), a
  personal accesskit fork, and a Rust 1.93 floor — every one of which can
  break the build without the app changing. Trading a boring-stable stack for
  a moving one, for a launcher whose job is to *stay out of the way*, is a
  bad risk trade on its face.
- **The concurrency model fight is real.** The app's hardest flows (6-hour
  wizard waits, mid-flow user dialogs, cross-thread progress) were shaped
  around threads + queued signals. iced will force a redesign of exactly the
  least-tested code (R-1, R-7.1). History says rewrites break untested code;
  here the untested code *is* the app's control plane.
- **What I'd do instead:** (a) keep the Python core and re-skin only the
  shell — COSMIC already runs Qt apps; add `XDG_CURRENT_DESKTOP` polish,
  a COSMIC-style icon, and portal file-choosing; (b) if Rust is non-
  negotiable, port *bottom-up*: pure-logic crates (`netpaths`, `exe_icons`,
  scoring, env parser) with the existing fixtures as acceptance tests, ship
  them as libraries first, and only then discuss the UI; (c) at minimum,
  reissue the brief against the real Qt baseline and require the Architecture
  plan to cost `bridge.py`'s state machine, not "GObject signals".

### 5.3 Kill-criteria verdict

**Do not approve implementation until:** the brief is reissued against the
Qt/Kirigami baseline (§1.4); Backend-level headless tests exist for P-46,
P-56/P-57, and P-59 flows; the Rust toolchain + vendored libcosmic/iced/
accesskit sources are proven to build inside the KDE Flatpak SDK from a cold
cache; and the renderer fallback on no-GPU is demonstrated, not asserted. If
any of those four cannot be met, **kill the migration** and take option (a)
above. The app works; the burden of proof is on the rewrite.
