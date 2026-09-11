# Packaging plan: Python/Qt → Rust/libcosmic (Flatpak)

Scope: PHASE 1 documentation only. No source changes. Branch: `cosmic-migration`.

Status legend used below: **[verified]** = confirmed on this host or by a
fetched upstream source during this spike; **[spike]** = verified empirically
by the lead's packaging spike on this host; **[to-verify]** = must be confirmed
when manifests/scripts are written in a later phase.

Existing behavior is cited as `file:line`.

---

## 1. New Flatpak design

### 1.1 App id — unchanged

`com.goshapps.GameHandler` stays. The id is pinned by `tests/test_packaging.py:14-24`
(`test_permanent_identity_is_consistent` asserts the desktop file `Icon=`,
the metainfo `<id>`, and the `<launchable>` all match), by the desktop filename
`data/com.goshapps.GameHandler.desktop`, and by existing user installs.
Renaming would orphan `~/.var/app/com.goshapps.GameHandler` data (library,
prefixes, downloaded runners). **Do not rename.**

### 1.2 Runtime + SDK **[verified]**

| Slot | Old (Python/Qt) | New (Rust/libcosmic) |
| --- | --- | --- |
| `runtime` | `org.kde.Platform` (`build-aux/flatpak/com.goshapps.GameHandler.json:4`) | `org.freedesktop.Platform` |
| `runtime-version` | `6.10` (manifest `:5`) | `25.08` |
| `sdk` | `org.kde.Sdk` (manifest `:6`) | `org.freedesktop.Sdk` |
| `sdk-extensions` | `org.freedesktop.Sdk.Extension.llvm21` (manifest `:7-9`, needed only to build PySide6) | `org.freedesktop.Sdk.Extension.rust-stable` |
| `base` / `base-version` | `org.winehq.Wine` / `stable-25.08` (manifest `:10-11`) | **unchanged** — see 1.4 |

Why Freedesktop 25.08:

- libcosmic is a pure Rust GUI stack (winit + wgpu + iced). It needs no KDE
  or GNOME platform libraries, so the Freedesktop runtime is the correct,
  smallest base. (The `flatpak-cargo-generator` reference manifest itself
  targets `org.freedesktop.Platform`; see section 2.)
- 25.08 is current and installed on this host **[verified]**:
  `org.freedesktop.Platform freedesktop-sdk-25.08.16` is present in
  `flatpak list --runtime`, alongside `GL.default`/`GL32.default` 26.1.6 and
  `Compat.i386` 25.08.
- `org.freedesktop.Sdk//25.08` exists **[verified]** (`flatpak remote-info`
  lists `org.freedesktop.Sdk/x86_64/25.08`).

### 1.3 Rust SDK extension **[verified]**

`org.freedesktop.Sdk.Extension.rust-stable` exists for 25.08 at **version
1.98.1** **[verified]** via `flatpak search`:

```
org.freedesktop.Sdk.Extension.rust-stable  1.98.1  25.08  flathub
org.freedesktop.Sdk.Extension.rust-stable  1.98.0  26.08  flathub
org.freedesktop.Sdk.Extension.rust-stable  1.89.0  24.08  flathub
```

- libcosmic 1.0.0 declares `rust-version = "1.93"`; 1.98.1 satisfies it.
- Host `cargo 1.98.x` matches the extension's 1.98.1, so lockfile-compatible
  builds locally and in the sandbox resolve identically. (Host is
  `rustc/cargo 1.98.1` per the spike; `rustc 1.98.0` was observed earlier —
  either way, same stable train as the extension.)
- Manifest wiring (standard pattern from the `flatpak-cargo-generator` README):

```json
"sdk-extensions": ["org.freedesktop.Sdk.Extension.rust-stable"]
```

with the module's `build-options.append-path` set to
`/usr/lib/sdk/rust-stable/bin` and `CARGO_HOME` pointed inside the build dir
(see the full module sketch in section 2).

### 1.4 Wine BaseApp — KEEP **[verified]**

`base: org.winehq.Wine`, `base-version: stable-25.08` (manifest `:10-11`)
**can and must stay**. Losing it would be a parity break: the app's whole
purpose is launching Windows games under Wine (README.md:3-4, 10-19).

Verification **[verified]**: the Wine BaseApp manifest at
`flathub/org.winehq.Wine`, `branch/stable-25.08`, `org.winehq.Wine.yml`
declares:

```yaml
runtime: org.freedesktop.Platform
runtime-version: '25.08'
```

i.e. the current `stable-25.08` BaseApp is **already built on the exact
Freedesktop 25.08 runtime we are moving to**. No runtime/base mismatch, no
re-packaging of Wine, no change to how downloaded Proton/Wine builds,
`--allow=multiarch`, or the `Compat.i386`/`GL32` inherit-extensions behave.
The old manifest's multilib story (manifest `:28-31`,
`inherit-extensions: GL32 + Compat.i386`, README.md:125-128) carries over
unchanged.

Notes:

- A `wow64-25.08` BaseApp branch also exists **[verified]** (`flatpak
  remote-info` branch list). It is an option for later (single-arch WoW64
  builds), but the default stays `stable-25.08` for parity.
- `stable-26.08` BaseApp and `org.freedesktop.Sdk//26.08` branches exist;
  do not chase them yet — 25.08 is what is installed and tested here.

### 1.5 Modules that disappear

- `pyside6` module (manifest `:57-93`, the Qt-source build with llvm21) —
  deleted entirely, with the `llvm21` sdk-extension.
- `python3-pyside-requirements.json` (manifest `:57`; the pip wheel bundle)
  — deleted entirely, including the `PYTHONPATH` env (section 3).
- New modules: `cargo-sources.json` (generated, section 2) + the app module
  itself (cargo build + install of binary, desktop file, metainfo, icon).
- Kept as-is: `osslsigncode` (manifest `:94-111`, Easy Installer signature
  verification, README.md:131-137) and `dxvk-runtime` (manifest `:112-133`).
  Both are toolchain-independent (`cmake-ninja` / `simple`).

---

## 2. Vendored offline build (cargo + git deps)

### 2.1 What the trap was, and was not **[spike]**

The brief warned that libcosmic's `iced` submodule (`path = "./iced"` in
libcosmic's `Cargo.toml`, `.gitmodules` pointing at
`https://github.com/pop-os/iced.git` branch master, reference clone at
`/tmp/cosmos-spike/libcosmic`) would defeat `cargo vendor` and need a
hand-rolled `git`-source hack in flatpak-builder.

**The spike disproved this.** Cargo initializes submodules for git sources
itself: after `cargo build`, the `iced` content is present in cargo's own
checkout at `~/.cargo/git/checkouts/libcosmic-41009aea1d72760b/a401af8/iced`.
No separate flatpak-builder `git` source for `iced` is needed, and no `path`
override is needed.

What the spike proved instead:

1. **Git-dependency consumption works.** This spec builds clean (EXIT 0,
   ~40s) in a standalone crate, compiling `iced_*`, `cosmic-config`,
   `cosmic-theme`, `iced_wgpu`, `iced_tiny_skia` — all resolved from
   libcosmic's own git repo:

   ```toml
   libcosmic = { git = "https://github.com/pop-os/libcosmic.git", rev = "a401af8b1c54a8abd393b8c5b7c8809402f83850", default-features = false, features = ["winit", "tokio", "wayland", "a11y", "xdg-portal"] }
   ```

   Pin by `rev`, not branch: libcosmic's `iced` submodule tracks
   `heads/master` (moving target). The pinned rev `a401af8` is the tested one;
   bumping it is a deliberate, re-tested act (regenerate lockfile +
   `cargo-sources.json`, re-run `scripts/verify.sh`).

2. **`cargo vendor` succeeds: 630 vendored crates**, including `iced`,
   `iced_wgpu`, `iced_widget`, `cosmic-config`, `cosmic-config-derive`,
   `cosmic-text`, `cosmic-protocols`, `cosmic-client-toolkit`,
   `cosmic-settings-daemon`, `cosmic-freedesktop-icons`.

3. **The real vendoring trap is transitive git deps.** `Cargo.lock` pulls git
   sources beyond libcosmic itself. The spike confirmed these must be covered
   by `cargo-sources.json` (all observed **[spike]**; the `cctk` /
   `freedesktop-icons` / `dbus-settings-bindings` lines below additionally
   confirmed in libcosmic's own `Cargo.toml` on disk):

   | Git source | Pinned as |
   | --- | --- |
   | `https://github.com/pop-os/libcosmic.git` | `rev = "a401af8…"` (top-level dep) |
   | `https://github.com/pop-os/dbus-settings-bindings` | via `cosmic-settings-daemon` dep |
   | `https://github.com/wash2/accesskit` | `tag = "cosmic-0.14"` |
   | `https://github.com/iced-rs/cryoglyph.git` | `rev = "e429a025"` |
   | `https://github.com/pop-os/cosmic-protocols` (`cosmic-client-toolkit`) | `rev = "32283d7"` |
   | `https://github.com/pop-os/freedesktop-icons` (`cosmic-freedesktop-icons`) | git (unpinned rev — inherits via lockfile) |

### 2.2 The mechanism: generated `cargo-sources.json`, nothing hand-rolled

Offline Flatpak path: run `flatpak-cargo-generator.py` (from
`flatpak/flatpak-builder-tools`, `cargo/` — existence **[verified]** via
GitHub API; usage below **[verified]** from its README) against our
`Cargo.lock`:

```bash
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/flatpak/cargo-sources.json
```

The README-confirmed workflow (flatpak-builder ≥ 1.2; ours is 1.4.10)
produces a JSON list of `type: file` (crates.io) and `type: git` (git deps)
sources that is spliced into the app module's `sources`. The app module then
builds fully offline (`CARGO_NET_OFFLINE=true`, `cargo --offline`).

Concrete module sketch (field names per the generator README):

```json
{
    "name": "gamehandler",
    "buildsystem": "simple",
    "build-options": {
        "append-path": "/usr/lib/sdk/rust-stable/bin",
        "env": {
            "CARGO_HOME": "/run/build/gamehandler/cargo",
            "CARGO_NET_OFFLINE": "true"
        }
    },
    "build-commands": [
        "cargo --offline fetch --manifest-path Cargo.toml --verbose",
        "cargo --offline build --release",
        "install -Dm0755 target/release/gamehandler ${FLATPAK_DEST}/bin/gamehandler",
        "install -Dm0644 data/com.goshapps.GameHandler.desktop ${FLATPAK_DEST}/share/applications/com.goshapps.GameHandler.desktop",
        "install -Dm0644 data/com.goshapps.GameHandler.metainfo.xml ${FLATPAK_DEST}/share/metainfo/com.goshapps.GameHandler.metainfo.xml",
        "install -Dm0644 data/icons/hicolor/scalable/apps/com.goshapps.GameHandler.svg ${FLATPAK_DEST}/share/icons/hicolor/scalable/apps/com.goshapps.GameHandler.svg"
    ],
    "sources": [
        { "type": "dir", "path": "../.." },
        "cargo-sources.json"
    ]
}
```

Rules for the code phase:

- `cargo-sources.json` is **generated, committed, and regenerated on every
  `Cargo.lock` change**. CI (`scripts/verify.sh`, section 6) must fail if the
  checked-in file is stale (regenerate to a temp file, `diff`).
- After generation, **assert the git entries exist**: one `type: git` source
  per git URL in `Cargo.lock` (the six rows in 2.1). If any is missing, the
  offline build will fail at `cargo --offline fetch` — this check is the
  regression test for the transitive-git-dep trap.
- The `iced` submodule must **not** appear as its own source: it arrives
  inside libcosmic's git checkout (`./iced` path dep resolves within the
  cargo git cache). If a future libcosmic rev breaks that invariant, the
  stale-check + offline-fetch failure surfaces it.
- Keep the `rev =` pin for libcosmic in `Cargo.toml` in lockstep with the
  generated sources (same commit hash in both).

### 2.3 libcosmic features we enable

Per the verified spec: `winit`, `tokio`, `wayland`, `a11y`, `xdg-portal`
(`default-features = false`). Meaning for packaging:

- `xdg-portal` = `ashpd` (libcosmic `Cargo.toml`: `xdg-portal = ["ashpd"]`).
  This is what makes file dialogs and notifications go through
  `xdg-desktop-portal` instead of direct filesystem access (section 4).
  `wayland` pulls `ashpd?/wayland` integration automatically.
- `x11` is **not** enabled: the app is Wayland-first; X11 users get
  `socket=fallback-x11` + XWayland from the compositor side (unchanged from
  today, manifest `:16`). Revisit only if X11-native testing demands it.
- Provisional **[to-verify]** in the code phase: whether our crate must also
  enable libcosmic's `rfd` feature (`dep:rfd`, rfd 0.16 with `xdg-portal`)
  for portal file dialogs, or whether libcosmic's own dialog helpers already
  cover it. Decide when the first file dialog is ported; either way no
  packaging change beyond the feature flag.

---

## 3. finish-args: every arg justified

Current list (`build-aux/flatpak/com.goshapps.GameHandler.json:13-27`).
Disposition for the Rust build:

| finish-arg | Verdict | Reason |
| --- | --- | --- |
| `--share=network` | **KEEP** | Runner/installer downloads, Steam artwork lookup, release-page fetches are core function (README.md:12-16). |
| `--share=ipc` | **KEEP** | Required for GPU buffers / SHM with the compositor; standard for any GUI Flatpak, wgpu included. |
| `--socket=fallback-x11` | **KEEP** | XWayland fallback on X11 sessions. libcosmic/winit talks X11 via XWayland; unchanged need. |
| `--socket=wayland` | **KEEP** | Primary display path for winit. |
| `--socket=pulseaudio` | **KEEP** | Game audio (Wine/PulseAudio socket passthrough). Unrelated to toolkit. |
| `--allow=multiarch` | **KEEP** | 32-bit Windows games and downloaded Wine/Proton builds (README.md:127-128). Non-negotiable for a Wine launcher. |
| `--device=all` | **KEEP, flagged** | Reviewed launcher exception (README.md:139-145): controllers and game hardware passthrough to launched games, which inherit the sandbox. Minimization candidate is `--device=dri` (all wgpu needs is render nodes), but gamepads classically need the full device set. Plan: ship `--device=dri` first, test controller hotplug through a launched game, and restore `--device=all` with a test note if controllers regress. Do not silently keep `all` without that test. |
| `--filesystem=home` | **KEEP** | Reviewed exception (README.md:141-145): libraries live in arbitrary user locations. Portal file *choosers* (section 4) do not replace this — the app must *execute* games from those locations afterwards. |
| `--filesystem=xdg-run/gvfs` | **KEEP** | Network-share games resolve via mounted GVFS paths (README.md:129-130, `gamehandler/netpaths.py`). Unrelated to toolkit. |
| `--filesystem=~/.var/app/com.valvesoftware.Steam/data/Steam:ro` | **KEEP** | Read-only Steam library/artwork access. Unrelated to toolkit. |
| `--env=PATH=…gamescope…` | **KEEP, conditionally** | Only while Gamescope integration is retained. Path prefix must be re-checked against the Freedesktop runtime layout (current value targets the KDE-runtime gamescope extension path; the Freedesktop gamescope Vulkan-layer extension is `org.freedesktop.Platform.VulkanLayer.gamescope//25.08`, README.md:46-50). Drop if Gamescope support is deferred. |
| `--env=PYTHONPATH=…` | **DELETE** | Python is gone. |
| `--talk-name=org.freedesktop.Notifications` | **DELETE, replaced by portal** | Direct notification-bus access is unnecessary once notifications go through the `org.freedesktop.portal.Notification` (ashpd) API, which needs no explicit bus permission (portals are brokered by `xdg-desktop-portal` — see section 4). Code-phase dependency: keep this line until the notification call site is ported to ashpd, then remove. |
| *(new)* `--device=dri` | **ADD** | wgpu renders via Vulkan/GL on render nodes. Without DRI access the app either fails to create a surface or falls back to CPU rasterization. (If the `--device=all` minimization test above restores `all`, `dri` is subsumed and listed only as documentation.) |
| *(new)* `--socket=pulseaudio` etc. | — | Already present; no new sockets needed. Portal access (`OpenFile`, `Notification`, `Settings` for dark-mode) requires **no** finish-args — that is the point of portals. Do not add `--talk-name=org.freedesktop.portal.*`. |

`inherit-extensions` (`GL32`, `Compat.i386`, manifest `:28-31`) is **unchanged**:
32-bit GL for 32-bit games and the i386 compat layer are Wine needs, not Qt needs.

`cleanup` (manifest `:32-55`): delete all `python*`/`PySide`/`shiboken`/`numpy`/
`OpenGL` entries; keep `/include`, `/lib/cmake`, `/lib/pkgconfig`,
`/share/doc`, and the LLVM/clang entries only if the Rust build still needs
them (it should not — no C++ binding generation remains; verify at manifest
time and drop).

---

## 4. Portals and desktop integration

### 4.1 File dialogs via portal, not direct filesystem dialogs

Rule: every open/save dialog goes through `xdg-desktop-portal`
(`xdg-portal` feature + `ashpd`, section 2.3), never through a direct
filesystem dialog. Consequences:

- The sandbox keeps no new filesystem permissions for choosing files; the
  portal grants per-file access to what the user picks.
- `--filesystem=home` stays regardless (section 3): picking a game and
  *running* it later are different operations.
- Test hook: settings persistence round-trips (section 5) must cover
  portal-granted paths being remembered and re-opened.

### 4.2 Desktop file (`data/com.goshapps.GameHandler.desktop`)

Current content (desktop `:1-11`): `Exec=gamehandler`, `Icon=` app id,
`Categories=Qt;KDE;Game;`, `Terminal=false`, `StartupNotify=true`.

Changes:

- `Categories=Qt;KDE;Game;` → drop the toolkit tags. New value:
  `Categories=Game;` (keep `Game`; toolkit categories were never load-bearing
  for search — `Keywords=Wine;Proton;…` (desktop `:10`) stays as the search
  surface).
- `Exec`, `Icon`, `Name`, `Comment`, `StartupNotify`, `Terminal` unchanged.
- Validation unchanged: `meson test validate-desktop` via
  `desktop-file-validate` (`data/meson.build:15-18`) becomes `cargo`-phase
  `desktop-file-validate` in `verify.sh` (section 6).

### 4.3 AppStream metainfo (`data/com.goshapps.GameHandler.metainfo.xml`)

- **Unchanged**: `<id>`, `<launchable>`, `<project_license>GPL-3.0-or-later</project_license>`,
  `<developer><name>Gosh</name>`, homepage/bugtracker URLs, `<content_rating/>`
  (all asserted by `test_packaging.py:14-24,76-88` — the Gosh-identity test
  must keep passing; "Made by Gosh", no personal names).
- **Description**: replace the Qt 6/Kirigami sentences (metainfo `:9-14`)
  with the COSMIC/libcosmic equivalents. Keep the front-end-not-compat-layer
  paragraph (`:15-22`) and the feature list shape; update entries that name
  Qt-only behaviors.
- **Releases**: prepend a `<release version="0.8.0" date="…">` entry.
  Version-bump strategy: the three-way lockstep test
  (`test_packaging.py:117-136`) currently ties `meson.build` version ↔
  `__init__.__version__` ↔ newest metainfo release ↔ README bundle name.
  Its Rust successor must tie `Cargo.toml` version ↔ newest metainfo release
  ↔ README bundle name (section 5). The 0.8.0 entry describes the toolkit
  migration the way 0.7.0 documented the Kirigami rewrite (metainfo `:61-81`).
- **No `<requires>` runtime entry is added**: the runtime binding lives in the
  Flatpak manifest, not the metainfo. (There is no `Runtime`/`Tags`
  requirement in the current metainfo to migrate — note for reviewers: if a
  future lint demands `<recommends>`/`<tags>`, add then.)

### 4.4 Icon (`data/icons/.../com.goshapps.GameHandler.svg`)

The SVG gamepad mark installs unchanged (`data/meson.build:36-40`). Two notes:

- The only headless-spike warning observed was
  `xdg_toplevel_icon_manager_v1 is not supported` **[spike]** — benign (no
  Wayland icon manager on the test compositor), but confirm the taskbar icon
  resolves on GNOME/KDE/COSMIC during manual QA.
- Dark/light tray or headerbar variants, if ever added, follow the
  `Name-symbolic` convention; not needed for 0.8.0.

---

## 5. Test suite plan (Rust)

Mirror the intent of the 15 Python test modules, restructured for The Elm
Architecture (state + `Message` + `update()` + view). No pixel matching
anywhere — all UI coverage is message+state driven, the direct analogue of
`tests/test_qml_smoke.py`'s "instantiate every page against the real backend
and fail on binding errors" approach (test_qml_smoke.py:1-7, 67-133).

### 5.1 Unit tests: every `Message` variant `update()` handles

One test per `Message` variant, asserting the resulting state (not the
rendering). Required coverage, mapped from current modules:

- Library CRUD messages (add/edit/remove game) — cf. `test_models.py`,
  `test_library_view.py`.
- Runner selection/download-progress/completion messages —
  cf. `test_runners.py` (release parsing, install-id sanitization).
- Settings messages (theme, view mode, defaults toggles) —
  cf. `test_settings.py`, plus `theme.py` behavior.
- Installer catalog/progress messages — cf. `test_installers.py`.
- Plugin detection-result messages — cf. `test_plugins.py`.
- Cover-art lookup result messages (success/miss/reject-non-Steam-match) —
  cf. `test_covers.py`.
- EXE icon extraction result messages — cf. `test_exe_icons.py`.
- Credits content test (upstream list intact, Gosh identity, no personal
  names) — cf. `test_credits.py` and `test_packaging.py:76-88`.

Exhaustiveness is enforced Reflexively: a test that matches on `Message`
must fail to compile when a new variant is added (e.g. a
`match` with no wildcard arm in the test harness, or a
`strum`-style variant-count assertion) — so "every variant" stays true as
the enum grows.

### 5.2 Business-logic tests (no UI)

Port, do not drop:

- Archive extraction confinement (traversal/absolute/symlink, member and
  byte limits, private staging, atomic rename) — `test_security.py:97-190`
  (`ArchiveExtractionTests`) and `266-419` (`RunnerInstallationTests`).
- Filename/install-id sanitization and collision-free id derivation —
  `test_security.py:192-263`.
- Desktop shortcut escaping/injection resistance —
  `test_security.py:422-451`.
- GVFS/network-share path resolution — `test_netpaths.py`.
- Runner command building (env toggles: Esync/Fsync/DXVK/VKD3D/NVAPI/FSR/
  GameMode/MangoHud/Gamescope/virtual-desktop) — `test_runners.py`.
- Download size bounds — `test_security.py:454-457`.

### 5.3 Settings persistence round-trips

Serialize → write → read → deserialize for every settings field, including
default-vs-explicit values and forward tolerance (unknown keys ignored, so
0.7.x configs do not break 0.8.0). Include portal-granted paths (section 4.1).

### 5.4 Integration tests: user flows through messages+state

Each major flow is a scripted `Vec<Message>` applied to an initial state,
asserting intermediate and final states:

1. Add Windows `.exe` game → pick runner → launch command materializes.
2. Download a Proton family build → progress → installed → set as game's runner.
3. Easy installer: catalog entry → vendor wizard completion → prefix scan finds exe → Play button state.
4. Prefix tools: winecfg/winetricks with the game's own Wine build.
5. Cover art: missing cover → Steam lookup → accept/reject → fallback to exe icon.
6. Remove game with confirmation; sorting/filtering/search of the library.
7. Game exits immediately → error reported, no stuck "Launching…" state
   (regression from metainfo 0.6.1 notes, metainfo `:82-105`).

### 5.5 Packaging/manifest tests (successor to `test_packaging.py`)

- Identity consistency (app id across desktop/metainfo/manifest) —
  `test_packaging.py:14-24` equivalent.
- Manifest asserts: runtime `org.freedesktop.Platform` 25.08, sdk
  `org.freedesktop.Sdk`, `rust-stable` extension, base `org.winehq.Wine`
  `stable-25.08`, `--allow=multiarch`, `--filesystem=home`,
  `--filesystem=xdg-run/gvfs`, `--device=dri` (or documented `all`),
  `Compat.i386`+`GL32` inherit-extensions, **no** `PYTHONPATH`, no
  `llvm21`, no `pyside6` module — the inverse of `test_packaging.py:26-74`.
- Version lockstep: `Cargo.toml` version == newest metainfo release ==
  README bundle name — successor to `test_packaging.py:117-136`.
- `cargo-sources.json` freshness + git-entry presence (section 2.2).
- Discovery-without-GUI analogue: unit/integration tests must run with no
  display and no GPU (`test_discovery.py`'s spirit — see section 7 for what
  genuinely needs a compositor).

### 5.6 Headless Flatpak smoke test

Section 7. Design here, implement in code phase: launch the installed
Flatpak binary under a headless compositor with software Vulkan, drive the
app through its pages (the iced equivalent of visiting every QML page,
test_qml_smoke.py:106-130), fail on renderer/panic errors, with the
`--version` CLI fallback (section 7).

---

## 6. `scripts/verify.sh` design

Single entry point. (Note: no `scripts/` directory exists at HEAD — greenfield.)

```
scripts/verify.sh [--offline] [--skip-flatpak]
```

Stages, in order. **Fail fast**: `set -euo pipefail`; each stage prints
`### <stage>` on entry and `ok <stage> (<time>)` on success; any failure
prints `FAIL <stage>` plus the failing command's tail and exits non-zero
immediately (no point running the smoke test if clippy failed).

1. `cargo build` (workspace root). Catches compile breaks first; keeps later
   stages' errors meaningful.
2. `cargo clippy --all-targets -- -D warnings`. **Must run at OUR workspace
   root only, never inside the libcosmic checkout [spike]**: libcosmic's own
   `cosmic-config-derive/src/lib.rs:3` (`use syn;`) trips
   `clippy::single_component_path_imports`, and path-dependency members do
   not get `--cap-lints allow`. As a git dependency of our workspace, cargo
   caps its lints and our clippy run is unaffected. Host prerequisite: the
   `clippy` component must be installed (on Fedora it was missing and needed
   the `clippy` package; in the Flatpak SDK it arrives via rust-stable).
3. `cargo test`. Unit + business-logic + integration + manifest tests
   (sections 5.1–5.5). Must pass with no display server and no GPU
   (unset `DISPLAY`/`WAYLAND_DISPLAY` in this stage to prove it).
4. `cargo-sources.json` freshness: regenerate from `Cargo.lock` to a temp
   file, `diff` against the committed copy, and assert all `Cargo.lock` git
   URLs have `type: git` entries (section 2.2). Fails with "run
   flatpak-cargo-generator.py and commit the result".
5. `flatpak-builder` (build; `--skip-flatpak` skips stages 5–7 for fast
   local iteration). Uses the same flags as `build-aux/flatpak/build.sh:10-18`
   (`--force-clean`, `--install-deps-from=flathub`, `--default-branch=stable`).
   `--offline` passes `--disable-network` after sources are fetched.
6. Headless smoke test (section 7): launch under headless compositor +
   software Vulkan; fail on panic/renderer errors.
7. `desktop-file-validate` + `appstreamcli validate --no-net` on the
   installed files (successor to `data/meson.build:15-34`'s
   `validate-desktop`/`validate-metainfo` tests).

Output contract: one line per stage to stdout, machine-greppable
(`ok|FAIL|SKIP <stage>`); full tool output goes to `target/verify-logs/`
(per-stage files) and only the tail is echoed on failure.

---

## 7. Headless smoke test verdict (wgpu, not Qt)

### 7.1 What the spike proved **[spike]**

The prebuilt libcosmic `application` example **launched and stayed alive**
on this host (killed by timeout; exit 124 = still running, no crash),
rendering via **wgpu** with no Qt involved. Environment: `DISPLAY=:1`,
`WAYLAND_DISPLAY=wayland-1`, lavapipe software-Vulkan ICD present
(`/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`). Sole warning: the benign
`xdg_toplevel_icon_manager_v1 is not supported` (no icon manager on that
compositor; cf. section 4.4).

**Verdict: a headless smoke test is viable — it was demonstrated working.**
This is not a Qt-offscreen situation (`QT_QPA_PLATFORM=offscreen`,
test_qml_smoke.py:47) and must not be built as one. The correct stack:

- **Compositor**: headless Wayland compositor holding `WAYLAND_DISPLAY`
  (e.g. `weston --backend=headless` **[to-verify]** availability in CI;
  alternatively the host's existing compositor socket). winit connects over
  Wayland exactly as on a real desktop.
- **GPU**: software Vulkan via lavapipe (`VK_ICD_FILENAMES` pointed at the
  lavapipe ICD, `LIBGL_ALWAYS_SOFTWARE`-style CPU rasterization for any GL
  fallback). wgpu needs a Vulkan (or GL) device; lavapipe provides one with
  no hardware.
- **Caveat [spike]**: the host's ICD JSONs exist but `vulkaninfo` is absent
  and the `libvulkan` loader presence was not conclusively confirmed
  (`ldconfig` shows Mesa driver libs like `libvulkan_radeon`, but the loader
  itself must be checked). The smoke test must therefore **assert loader +
  ICD explicitly** (`test -n "$(ldconfig -p | grep libvulkan.so)"`-style
  probe, or a tiny `vkEnumerateInstanceVersion` check) and fail with "no
  Vulkan loader" rather than a cryptic wgpu panic. Inside the Flatpak,
  Vulkan comes from the `GL.default` extension — present on this host
  (26.1.6, section 1.2).

### 7.2 What to do if wgpu cannot start in some CI environment

Ranked fallbacks (strongest first); the smoke stage tries each in order and
reports which level passed:

1. Headless compositor + lavapipe, full launch, page walk (target state).
2. `Xvfb` + lavapipe over winit's X11 backend (libcosmic keeps an `x11`
   feature we do not enable — this fallback would need it; weigh before
   paying that cost).
3. CLI-only assertions: `gamehandler --version`, `--list`, and a
   build-and-launch assertion (process starts, creates its window object,
   exits 0 on `--quit-after-init`-style flag if we add one). Honest floor:
   this proves packaging + startup linkage, not rendering.

If even level 3 cannot run (no display server at all can be provided), say so
in the CI log and gate releases on a maintainer-run level-1 pass. Do not
fake a green smoke test.

### 7.3 Why this is still a top risk

wgpu initialization depends on loader + ICD + compositor socket agreeing —
three moving parts outside our code, multiplied by Flatpak's GL extension
layer. The spike de-risks the toolkit itself (it runs); what remains is
pinning the CI recipe so it runs *repeatably*. Budget for one round of
CI-environment wrangling in the code phase.

---

## 8. Top packaging risks (for the code phase)

1. **Transitive git deps drifting** (section 2.1): six git sources must stay
   in `cargo-sources.json`; a libcosmic rev bump can add a seventh silently.
   Mitigation: the freshness/git-presence check in `verify.sh` stage 4.
2. **Headless-GPU CI flakiness** (section 7.3): loader/ICD/compositor recipe
   must be pinned; fallbacks ranked and reported, never faked.
3. **`--device` minimization vs controllers** (section 3): `dri`-only is the
   goal, `all` is the fallback; needs a real gamepad-through-launched-game
   test to close.
4. **Wine BaseApp / runtime skew over time**: today `stable-25.08` sits on
   Freedesktop 25.08 (verified); when either side moves (26.08 branches
   already exist), manifest, extension, and lockfile must move together.
   Mitigation: manifest asserts in the test suite (section 5.5).
5. **Notification/file-dialog portal behavior across desktops**: ashpd is
   only as good as the host's `xdg-desktop-portal` backends (GNOME/KDE/
   COSMIC/wlroots all differ). Manual QA matrix needed; the
   `xdg_toplevel_icon_manager_v1` warning is a foretaste.

## 9. Sources consulted (upstream facts)

- `org.winehq.Wine` `branch/stable-25.08` `org.winehq.Wine.yml` header
  (runtime `org.freedesktop.Platform` 25.08) — fetched from
  `flathub/org.winehq.Wine` via GitHub API/raw.
- `flatpak-cargo-generator` README (`flatpak/flatpak-builder-tools`,
  `cargo/`) — module wiring (`append-path`, `CARGO_HOME`,
  `CARGO_NET_OFFLINE`, `cargo --offline`) — fetched via raw GitHub.
- `flatpak search org.freedesktop.Sdk.Extension.rust-stable` — 1.98.1/25.08
  row (this host's Flathub remote).
- Local: `flatpak list --runtime` (25.08 platform/SDK deps present),
  `/usr/share/vulkan/icd.d/lvp_icd.x86_64.json` (lavapipe ICD),
  `/tmp/cosmos-spike/libcosmic` (`Cargo.toml` portal/git deps,
  `.gitmodules`, submodule SHAs), toolchain versions
  (`rustc/cargo 1.98.x`, `flatpak-builder 1.4.10`, `meson 1.11.2`).
- Lead's empirical spike (sections marked **[spike]**): git-dep build,
  submodule-fetch behavior, 630-crate vendor, transitive git list, example
  launch under headless Wayland, clippy workspace gotcha.
