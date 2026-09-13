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

- libcosmic is a pure Rust GUI stack (winit + iced; wgpu is opt-in and we do
  not enable it — D-11). It needs no KDE
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

As shipped in `crates/app/Cargo.toml` (`default-features = false`):
`winit`, `tokio`, `wayland`, **`x11`**, `a11y`, `xdg-portal`. Meaning for
packaging:

- `xdg-portal` = `ashpd` (libcosmic `Cargo.toml`: `xdg-portal = ["ashpd"]`).
  This is what makes file dialogs and notifications go through
  `xdg-desktop-portal` instead of direct filesystem access (section 4).
  `wayland` pulls `ashpd?/wayland` integration automatically.
- **`x11` IS enabled** (corrected during T-16; this section previously said
  the opposite). The app is Wayland-first, but enabling the winit X11 backend
  costs nothing in the sandbox — `--socket=fallback-x11` is kept regardless —
  and buys two concrete things: the app runs natively on X11 sessions rather
  than only via XWayland, and it makes **smoke-test fallback #2 (Xvfb) in
  section 6 available**, which the old text had ruled out on the grounds that
  we did not enable `x11`. A prior worry that this pulls GPU/X11 baggage does
  not apply: `--device=all` already covers it and rendering is software (D-11).
- `wgpu` is deliberately **absent**: D-11 ships `iced_tiny_skia`.
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
| `--share=ipc` | **KEEP** | Shared memory for the compositor; standard for any GUI Flatpak. (The old rationale cited GPU buffers via wgpu; that no longer applies under D-11, but the flag is still wanted for SHM.) |
| `--socket=fallback-x11` | **KEEP** | XWayland fallback on X11 sessions, and now also the native path for X11 sessions since `x11` is enabled (§2.3). |
| `--socket=wayland` | **KEEP** | Primary display path for winit. |
| `--socket=pulseaudio` | **KEEP** | Game audio (Wine/PulseAudio socket passthrough). Unrelated to toolkit. |
| `--allow=multiarch` | **KEEP** | 32-bit Windows games and downloaded Wine/Proton builds (README.md:127-128). Non-negotiable for a Wine launcher. |
| `--device=dri` + `--device=input` + `--device=usb` | **NARROWED (SEC-01)** | The three classes a launched game needs and nothing else: `dri` for the GPU, `input` for controllers and the event devices SDL reads, `usb` so `/dev/bus/usb` exists for enumeration. This closes PLAN.md Q-2 by measurement rather than by the gamepad test Q-2 asked for. `--device=all` was **not** required for gamepads: `--device=input` is what exposes `/dev/input`, and opening an event node needs its own unix permissions either way. **Measured in the sandbox** (`flatpak run --command=/bin/sh … -c 'ls /dev'`, before and after): the narrow grant removes `/dev/mem`, `/dev/kvm`, `/dev/nvme0n1` and its partitions, `/dev/vfio`, `/dev/vhost-net`, `/dev/watchdog`, `/dev/watchdog0`, `/dev/nvram`, `/dev/ttyS0-3`, `/dev/ppp`, `/dev/rfkill`, `/dev/hwrng`, `/dev/mtd*`, `/dev/gpiochip0` and `/dev/udmabuf` — every one of which `all` had put inside the sandbox that third-party game binaries run in, and none of which any code in `crates/` opens. `flatpak-metadata(5)` is explicit that a device grant exposes the nodes and grants nothing the user does not already have, so this is about the sandbox's blast radius rather than a privilege boundary. `tests/test_packaging.py` asserts the narrow set **and** asserts `--device=all` is absent, because the positive list alone is satisfied by a manifest that carries both. |
| `--filesystem=home` | **KEEP** | Reviewed exception (README.md:141-145): libraries live in arbitrary user locations. Portal file *choosers* (section 4) do not replace this — the app must *execute* games from those locations afterwards. |
| `--filesystem=xdg-run/gvfs` | **KEEP** | Network-share games resolve via mounted GVFS paths (README.md:129-130, `gamehandler/netpaths.py`). Unrelated to toolkit. |
| `--filesystem=~/.var/app/com.valvesoftware.Steam/data/Steam:ro` | **KEEP** | Read-only Steam library/artwork access. Unrelated to toolkit. |
| `--env=PATH=…gamescope…` | **KEEP, conditionally** | Only while Gamescope integration is retained. Path prefix must be re-checked against the Freedesktop runtime layout (current value targets the KDE-runtime gamescope extension path; the Freedesktop gamescope Vulkan-layer extension is `org.freedesktop.Platform.VulkanLayer.gamescope//25.08`, README.md:46-50). Drop if Gamescope support is deferred. |
| `--env=PYTHONPATH=…` | **DELETE** | Python is gone. |
| `--talk-name=org.freedesktop.Notifications` | **DELETE, replaced by portal** | Direct notification-bus access is unnecessary once notifications go through the `org.freedesktop.portal.Notification` (ashpd) API, which needs no explicit bus permission (portals are brokered by `xdg-desktop-portal` — see section 4). Code-phase dependency: keep this line until the notification call site is ported to ashpd, then remove. |
| `--device=all` | **REMOVED (SEC-01)** | Removed by the narrowing in the row above. The T-16 reasoning recorded here was correct about the *app* — under D-11 it renders in software and needs no device node at all — and it is that paragraph which made the wide grant's survival indefensible once nobody could name what the extra nodes were for. Recorded as a removal so the next reader does not re-add it: `all` was never a superset anyone chose, it was the default nobody had narrowed. |
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

**Implemented at T-17.** Single entry point. (This section was written before
`scripts/` existed; the notes below record where the implementation departed
from the design and why.)

```
scripts/verify.sh [--skip-flatpak] [--skip-smoke] [--keep-going] [--offline] [--hold SECONDS]
```

Stages, in order. **Fail fast** by default: each stage prints `### <stage>` on
entry and `ok <stage> (<time>)` on success; a failure prints `FAIL <stage>`
plus the tail of that stage's log and exits non-zero immediately (no point
running the smoke test if clippy failed). `--keep-going` runs every stage and
prints the full table instead, for a diagnostic sweep.

1. `build` — `cargo build` (workspace root). Catches compile breaks first;
   keeps later stages' errors meaningful.
2. `clippy` — `cargo clippy --all-targets -- -D warnings`. **Runs at OUR
   workspace root only, never inside the libcosmic checkout [spike]**:
   libcosmic's own `cosmic-config-derive/src/lib.rs:3` (`use syn;`) trips
   `clippy::single_component_path_imports`, and path-dependency members do not
   get `--cap-lints allow`. As a git dependency of our workspace, cargo caps
   its lints and our clippy run is unaffected. Host prerequisite: the `clippy`
   component must be installed (on Fedora it was missing and needed the
   `clippy` package; in the Flatpak SDK it arrives via rust-stable).
3. `test` — `cargo test`. Unit + business-logic + integration + manifest tests
   (sections 5.1–5.5). Runs with `DISPLAY`/`WAYLAND_DISPLAY`/`WAYLAND_SOCKET`
   unset, so it must pass with no display server and no GPU. **Two invocations
   since task #27**, because there are two configurations and they can disagree:
   the workspace run (features unified across every member) and
   `cargo test -p gamehandler-core` (that crate's own graph, where
   `serde_json/preserve_order` is off — `cosmic-theme` turns it on and cargo
   unifies it into everything that depends on libcosmic). D-33 has the rule this
   follows: a verdict without its build scope is not evidence. The narrow run is
   given its own `CARGO_TARGET_DIR` under `target/verify-narrow`, so alternating
   between configurations does not rebuild core's dependency graph twice a run.
4. `cli` — the headless CLI (`--list` / `--launch` / `--version`), driven
   against a library the stage owns, with `HOME` and all three XDG bases pointed
   into one temp directory. It exists because the smoke test checked the *empty*
   case of `--list` (exit code only) and **a check that only exercises the empty
   case passes on a program that never reads the file** — which is what happened
   (task #31: `--list` printed the empty-library line over a real two-game
   library and exited 0). The assertions are deliberately of two different kinds:
   `--list` on a known-non-empty library is compared **byte for byte** against
   the expected rows (in *lowercased*-name order, so the fixture's `apple` /
   `Banana` is also a sorting trap), while `--launch <unknown id>` is asserted on
   its **exit code** with the reason required on stderr — the stub printed the
   right words and returned the wrong status, so a text-only check would pass it.
   A *known* id is never launched: that would start a game through Wine, which is
   not something a verification gate should do. It uses the debug binary
   `cargo build` produced, not the Flatpak's release binary, so it needs no build
   tree and no lock.
5. `oracle-freshness` — regenerate `docs/migration/oracle/fixtures/` from the
   Python implementation and fail if the checked-in copy differs. `gen_oracle.py`
   resolves its repo root from its own path (`parents[3]`) and writes its
   fixtures beside itself, so the stage stages a throwaway copy of the
   `gamehandler` package plus the generator, at the same depth, in a temp dir.
   (The generator imports `gamehandler.models` and `gamehandler.settings` and
   nothing else, which is what makes the copy sufficient.) The fixtures on disk
   are SHA-256 hashed before and after and asserted unchanged, so the "verify
   never dirties the tree" rule is checked rather than assumed.
6. `python-tests` — the existing Python suite (`python3 -m unittest discover
   -s tests -t .`, per README.md:261 — **not** pytest, which the repo does not
   use and this host does not have) must stay green. DECISIONS D-17.
7. `cargo-sources` — `cargo-sources.json` freshness: regenerate from
   `Cargo.lock` to a temp file, compare as a *set* of canonicalised entries
   (the generator's entry order is an implementation detail; the entry set is
   the contract), and assert every `Cargo.lock` git URL has a `type: git`
   entry (section 2.2). This is the one stage that needs something a clean
   checkout does not contain — the generator lives in `flatpak/flatpak-builder-
   tools` and imports `aiohttp` — so it **SKIPs loudly** with install
   instructions rather than failing obscurely. It looks in `$FLATPAK_CARGO_
   GENERATOR`, on `PATH`, and in the usual cache paths, and prefers a
   `venv/bin/python` beside the script when one exists.
8. `flatpak-build` — `flatpak-builder`, with the flags from
   `build-aux/flatpak/build.sh` (`--force-clean`, `--install-deps-from=flathub`,
   `--default-branch=stable`) plus `--state-dir=.flatpak-builder` and
   `--repo=flatpak-repo`. **Correction:** `--offline` passes
   `--disable-download`; flatpak-builder 1.4.10 has no `--disable-network`
   flag, which the original text of this section named.
9. `smoke-test` — delegates to `scripts/smoke-test.sh` (section 7.2). Its
   four sub-checks are echoed indented under the stage line, so a pass is
   legible without opening the log. Exit 77 from the script (no compositor
   available) is reported as `SKIP`, never as `ok`.
10. `desktop-metainfo` — `desktop-file-validate` + `appstreamcli validate
   --no-net`, on the copies inside `build-flatpak/files/share/` when they exist
   and on `data/` otherwise. Successor to `data/meson.build:15-34`'s
   `validate-desktop`/`validate-metainfo` tests. An invalid file fails the
   stage; a *missing* validator reports `SKIP` with install instructions rather
   than `ok`, because the stage did not actually run — silence there would be a
   fake pass.
11. `flatpak-contents` — added by T-23, later than the rest of this list. Stages
   1–9 all ask whether a file is *well-formed*; none asked whether the Flatpak
   *contains* anything, and that gap was real: T-16 moved the build from meson
   (which ran `data/meson.build`'s `install_data`) to cargo, dropped the
   application's own licence install with it, and every stage stayed green while
   the Flatpak shipped a GPL-3 binary with no licence text. This stage holds a
   short list of (repository file → path under `/app`) pairs — the licence and
   the Authenticode trust root, neither of which is an input to any validator —
   and checks it in two halves: that the manifest *declares* an install of that
   source to that destination (reads the JSON; needs no build tree, so it still
   runs when `flatpak-builder` cannot, which is the half that would have caught
   the original defect), and that the installed bytes are byte-identical to the
   repository's own copy (only when `build-flatpak/files/bin/gamehandler`
   exists — flatpak-builder normalises timestamps, so mtime cannot date the
   tree). No completed tree reports `SKIP`, never `ok`, and prints which half
   did run.

    The list is not short by *count*: it holds every file the `gamehandler`
    module installs except the binary, and half 1 additionally reads the
    manifest's own install commands and fails for any destination the list does
    not cover — so a new install cannot be added to the manifest without being
    covered here. **Correction (task #23).** This paragraph originally read "a
    stage that asserted every installed path would duplicate the manifest's own
    three metadata installs, which stage 9 already covers by content", and that
    was wrong twice over. Stage 9 loads **two** of the three metadata files (the
    desktop entry and the metainfo; never the icon), and it validates their
    *content*, not that the Flatpak installs them or where. The installed
    destinations of all three were checked by nothing: renaming one in the
    manifest, or deleting the icon's install line, left every stage green while
    the shipped desktop file's `Icon=com.goshapps.GameHandler` resolved to
    nothing.

Output contract: one machine-greppable line per stage to stdout — `### <stage>`,
then `ok|FAIL|SKIP <stage>` — with sub-check lines indented so they never
collide with it. Full tool output goes to `target/verify-logs/<stage>.log`;
that file is written from the start of each stage so it can be tailed while a
long stage runs, and only its tail is echoed on failure.

**Read-only with respect to the repository.** Every stage writes only to
gitignored paths (`target/`, `.flatpak-builder/`, `build-flatpak/`,
`flatpak-repo/`). The script records `git status --porcelain` before and after
and prints a warning if the two differ, because a verify script that leaves the
tree dirty cannot be run in CI or before a commit.

**SKIPs are not passes.** The summary names the skipped stages and says so
explicitly. Since D-31 the exit status distinguishes the two causes: a skip the
caller asked for (`--skip-flatpak` / `--skip-smoke`) is a legitimate 0, and a
skip forced by a missing prerequisite exits 3, because that run did not verify
what this section claims it verifies.

**Nor is a *failure* a report of everything that ran.** Fail-fast is the default,
so a stage that fails stops the run — and until D-34 the summary named only
`passed`/`failed`/`skipped`, which meant the stages that were never attempted
appeared nowhere at all. A run that died at `oracle-freshness` printed
`failed: oracle-freshness` and nothing about the six stages it never reached,
which reads as "only the oracle is broken". The summary now prints those
explicitly, as `did not run (<reason>): ...`, with the reason supplied by
whatever stopped the run. The stage list itself is one array (`STAGES`) from
which the usage text is generated and which `begin()` checks every stage
against, so an edit to the run order cannot leave the documentation behind:
a stage announced out of order, or a `run_stage` naming a function that does not
exist, is a usage error (2) rather than a silent omission.

---

## 7. Headless smoke test verdict (software rendering, not Qt)

### 7.1 What the spike proved **[spike]**

The prebuilt libcosmic `application` example **launched and stayed alive**
on this host (killed by timeout; exit 124 = still running, no crash) with no
Qt involved. Environment: `DISPLAY=:1`, `WAYLAND_DISPLAY=wayland-1`. Sole
warning: the benign `xdg_toplevel_icon_manager_v1 is not supported` (no icon
manager on that compositor; cf. section 4.4).

**Correction at T-16.** This section originally attributed the successful
launch to **wgpu** over lavapipe. That was wrong on two counts, both settled
afterwards: the spike was run with libcosmic's *default* feature set, which
does **not** include `wgpu` (it is opt-in), so the renderer in that run was
`iced_tiny_skia` — and D-11 then decided the port ships exactly that. The
lavapipe/Vulkan loader discussion below was therefore scaffolding for a
requirement we do not have.

Concretely, verified with `cargo tree -e features` on our own crate:
`iced_renderer feature "tiny-skia"` present, `iced_renderer feature "wgpu"`
**absent**. (`libcosmic feature "iced_wgpu"` *is* present, because libcosmic's
`wayland` feature enables `iced_wgpu/wayland` unconditionally — that compiles
the crate but never initialises a device. See PLAN.md §2.1.)

**Verdict: a headless smoke test is viable — it was demonstrated working.**
This is not a Qt-offscreen situation (`QT_QPA_PLATFORM=offscreen`,
test_qml_smoke.py:47) and must not be built as one. What the stack needs:

- **Compositor**: headless Wayland compositor holding `WAYLAND_DISPLAY`
  (e.g. `weston --backend=headless` **[to-verify]** availability in CI;
  alternatively the host's existing compositor socket, or `Xvfb` via the
  enabled `x11` backend — §7.2). winit connects over Wayland exactly as on a
  real desktop.
- **GPU: nothing.** No Vulkan loader, no ICD, no lavapipe, no `/dev/dri`.
  The app rasterizes on the CPU. Removing this requirement is the single
  biggest simplification D-11 bought, and it is why the flatpak smoke test
  can run on an ordinary CI box.
- **No `vulkaninfo`/loader probes are needed.** The old text required the
  smoke test to assert loader and ICD presence and fail with "no Vulkan
  loader" rather than a cryptic wgpu panic. That whole failure mode is gone.
  Inside the Flatpak, Vulkan still arrives via `GL.default` (26.1.6, §1.2) —
  it just is not used for rendering, only available to launched games.

### 7.2 What the smoke test needs (revised after D-11)

**This section previously opened with "what to do if wgpu cannot start". That
premise is gone** (D-11): the app ships `iced_tiny_skia`, so it rasterizes on
the CPU and never initialises a GPU device. Lavapipe, the Vulkan loader, and
`/dev/dri` are therefore **not** smoke-test requirements — the spike proved the
binary starts and stays running with no GPU at all.

What is still required is a **display server**: winit needs a
`WAYLAND_DISPLAY` or `DISPLAY` to create its event loop, and with neither it
panics (N-01). Note this is a *display* dependency, not a *rendering* one, so
the cheap options get much cheaper:

1. **Headless Wayland compositor** (target state) — e.g. `cosmic-comp`,
   `weston --backend=headless`, or `sway` under `WLR_BACKENDS=headless`. Full
   launch and page walk; no GPU or lavapipe needed.
2. **`Xvfb`** over winit's X11 backend. Available because `x11` *is* enabled in
   `crates/app/Cargo.toml` (§2.3) — the old text ruled this out on the belief
   that we had not enabled it. Xvfb needs no GPU either.
3. **CLI-only assertions:** `gamehandler --version`, `--list`, `--launch <id>`,
   plus a build-and-launch assertion (process starts and stays alive for a
   fixed interval, then exits cleanly). Honest floor: proves packaging and
   startup linkage, not rendering.

If even level 3 cannot run, say so in the CI log and gate releases on a
maintainer-run level-1 pass. Do not fake a green smoke test.

**One thing to port from the Qt app rather than drop:** `main.py:run_gui()`
prints an actionable hint when no display is available. iced panics instead
(N-01/N-02, task T-08), which would make level 3 fail by crashing rather than
by reporting. Fix N-01 before relying on level 3.

### 7.2a The implemented recipe (`scripts/smoke-test.sh`, T-17)

Four sub-checks, each reported as `ok|FAIL|SKIP`:

| Check | What it asserts | Level |
| --- | --- | --- |
| `cli-version` | `--version` exits 0 with `GameHandler X.Y.Z`, with no display | 3 |
| `cli-list` | `--list` exits 0 with no display (D-12) | 3 |
| `no-display-diagnostic` | the **GUI** path with neither display variable set prints a message and exits non-zero, with no panic (D-12a / N-01, N-02) | 3 |
| `gui-stays-up` | under a display server, the GUI is still running after `--hold` seconds, printed no panic, and terminates on SIGTERM rather than hanging | 1 |

Exit status: `0` all runnable checks passed · `1` at least one failed · `77`
only the CLI checks ran because no display was available. The `77` case prints
"NOT a pass: the GUI has not been exercised", and `verify.sh` reports it as
`SKIP`, not `ok`. A `SKIP` can therefore never be mistaken for a green smoke
test.

**Measured facts about the sandbox and the host** (T-17, this machine):

- **Neither `weston` nor `Xvfb` is present** — not on the host, and not in
  `org.freedesktop.Sdk//25.08` (checked with `flatpak run --command=sh --devel
  … -c 'command -v weston'`). They are not Flatpak apps on Flathub either. So
  the "headless compositor on PATH" route needs an out-of-band install and is
  *not* assumed. The script auto-detects both when they do exist, in that
  order, and teardown is scoped to the process it started.
- **The ambient session works and was used**: with `WAYLAND_DISPLAY=wayland-1`
  the app stayed alive for the whole interval and terminated on SIGTERM. This
  is the level-1 pass, on a real compositor.
- **`flatpak build` does not inherit the manifest's sockets.** It assembles its
  sandbox from explicit flags (`flatpak-build(1)`), so the two the app needs —
  `--socket=wayland` and `--socket=fallback-x11` — are passed by the script.
  Without them winit sees no socket even with `WAYLAND_DISPLAY` set.
- **`--env` is needed too.** A socket alone is not enough: the display
  variable must be forwarded explicitly with `--env=WAYLAND_DISPLAY=…`, because
  the ambient value is not inherited.
- **`--die-with-parent` matters.** In `build-dir` mode a killed `flatpak build`
  wrapper otherwise leaves the sandboxed app running.
- The script prefers `flatpak run com.goshapps.GameHandler` when the app is
  installed (real sandbox, manifest finish-args) and falls back to
  `flatpak build <dir>` so it also works on a build tree that was never
  installed.

**CI recipe.** Install `weston` (or `Xvfb`) in the CI image and run
`scripts/smoke-test.sh`; no GPU, no Vulkan, no `/dev/dri`. Without one, run it
with `--compositor 'CMD'` pointed at whatever display server the runner has.
The `--app-cmd` flag exists so the failure path can be exercised on purpose:
`--app-cmd /app/bin/7z` must fail with "exited … before the hold interval",
and a fixture that stays alive while printing `panicked at` must fail with
"alive but printed a panic". Both were verified.

### 7.3 Why this is now a much smaller risk

This was written expecting "loader + ICD + compositor socket agreeing — three
moving parts outside our code, multiplied by Flatpak's GL extension layer".
Under D-11 there is **one** moving part: a compositor socket. No loader, no
ICD, no GL extension layer in the rendering path, and no GPU. The spike
de-risked the toolkit itself (it runs); what remains is pinning the CI recipe
so it runs *repeatably* — realistically a `weston --backend=headless` (or
`Xvfb`) invocation. Budget a short round of CI-environment wrangling, not a
long one.

The risk that *does* stay live is unrelated to rendering: **N-01**, the
no-display panic. A smoke test asserts "starts and stays running without
errors", and a panic is neither — so N-01 must be fixed (T-08) before the
smoke test can be trusted at any level.

---

## 8. Top packaging risks (for the code phase)

1. **Transitive git deps drifting** (section 2.1): git sources must stay in
   `cargo-sources.json`; a libcosmic rev bump can add one silently. Mitigation:
   the freshness/git-presence check in `verify.sh` stage 6. **Updated at T-17:**
   `Cargo.lock` now carries **11** distinct git URLs, not the six §2.1
   predicted — `pop-os/cosmic-protocols`, `pop-os/freedesktop-icons`,
   `pop-os/dbus-settings-bindings`, `wash2/accesskit`, `iced-rs/cryoglyph` and
   `pop-os/libcosmic` (the six), plus `jackpot51/rust-atomicwrites`,
   `pop-os/smithay-clipboard`, `pop-os/softbuffer`, `pop-os/window_clipboard`
   and `pop-os/winit`. The check enumerates them from `Cargo.lock` rather than
   from a hardcoded list, which is why it stays correct as the set grows.
2. **Headless-GPU CI flakiness** (section 7.3): the loader/ICD question is
   settled (there is none — software rendering, D-11); what remains is a
   compositor. **Updated at T-17:** the recipe is pinned in §7.2a and works on
   this host over the ambient session, but **no headless compositor is
   available here or in the SDK**, so a CI box must install `weston` or
   `Xvfb`. Without one the check SKIPs (exit 77) and says so — never a fake
   green.
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
