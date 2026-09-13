# GameHandler

GameHandler is a modern game manager for Linux, focused on running **Windows games**
via **Wine** and **Proton**.

Current release: **0.8.0**. This release ports the interface from Python +
Qt 6/Kirigami to Rust + libcosmic (the COSMIC desktop toolkit). The previous
release, 0.7.2, fixed package-aware test discovery on hosts without PySide6:
the test-only Kirigami stub now loads Qt types lazily, so the core tests run
and the optional QML smoke test skips as intended.

> **This branch is porting the interface to Rust.** GameHandler is being
> rewritten from Python 3 + PySide6/QML (Qt 6, styled with Kirigami) to **Rust**
> with **libcosmic**, the COSMIC desktop toolkit. Both trees are present here
> and both are documented below: the Python application is the parity reference
> and still runs exactly as described, and the Rust workspace under `crates/` is
> what 0.8.0 ships. See [The Rust application](#the-rust-application) for what
> builds today and `docs/migration/PLAN.md` for the task list.

It is a **front-end, not a compatibility layer**. Every Windows game it launches runs
on [Wine](https://www.winehq.org), usually through a [Proton](https://github.com/ValveSoftware/Proton)
build maintained by someone else, with Direct3D translated by [DXVK](https://github.com/doitsujin/dxvk)
and [VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton). GameHandler
downloads those builds from their maintainers' own release pages — the same upstream
sources [ProtonPlus](https://github.com/Vysp3r/ProtonPlus) uses — sets up isolated
prefixes, and stays out of their way. Its design owes a lot to
[Lutris](https://lutris.net), [Faugus Launcher](https://github.com/Faugus/faugus-launcher),
and [Bottles](https://usebottles.com). Full acknowledgements are
[below](#thanks-to-the-projects-gamehandler-stands-on) and on the app's own About & Credits page.

## Features

- Dark mode by default, plus system and light themes — both first-class
- Grid and list library views, search, categories, sorting, and per-game edit
- Generated cover art for titles without artwork, so the grid never looks empty
- Custom cover art, automatic Steam lookup by game name, and — for anything
  Steam has never sold, such as a store launcher — the icon the Windows
  executable already carries
- Add Windows `.exe` titles or Linux-native games
- Choose a Proton or Wine runner when adding a game, and change it later
- Download and remove compatibility tools from inside the app:
  - Proton-GE, Proton-GE RTSP, Proton-CachyOS, Proton-EM
  - Kron4ek Wine-Vanilla, Wine-Staging, Wine-Staging-Tkg, Wine-Proton
- Guide describing when to use each Proton or Wine family
- Isolated Wine prefixes, Winecfg, Winetricks, and prefix folder access
- Launch helpers: MangoHud, Feral GameMode, Prefer SDL, Wine Wayland, HDR, Esync, Fsync, DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye, EAC, Gamescope, virtual desktop, and custom environment variables
- Easy installers for Battle.net, Epic, EA App, Ubisoft Connect, GOG Galaxy, Amazon Games, Rockstar, Steam, and Discord —
  each waits for the vendor's own wizard to finish, then adds the result with a Play button
- Plugins page that detects MangoHud, GameMode, Winetricks, UMU, and Gamescope
- About & Credits page naming every upstream project, with links and licenses
- Desktop shortcuts that launch a library entry with `gamehandler --launch`
- Games and covers on network shares work: `smb://`-style locations are
  resolved through their mounted GVFS path so Wine can actually run them

Flatpak users who enable Gamescope also need the matching Freedesktop 25.08 extension:

```bash
flatpak install flathub org.freedesktop.Platform.VulkanLayer.gamescope//25.08
```

## Tech stack

| Area | Rust application (0.8.0, `crates/`) | Python application (0.7.2, parity reference) |
| --- | --- | --- |
| Language | Rust (edition 2024, floor 1.93) | Python 3 |
| UI toolkit | libcosmic (COSMIC desktop toolkit), software renderer | Qt 6 + Kirigami (`PySide6` + QML) |
| Build system | Cargo workspace | Meson |
| Packaging | Flatpak (Freedesktop 25.08 + rust-stable) | not packaged on this branch — run from source |
| Runners | System Wine plus downloaded Proton/Wine builds | same |

## Requirements

To build and run the Rust application:

```
rustup / cargo        # 1.93 or newer; the Flatpak uses the rust-stable SDK extension
flatpak flatpak-builder  # only to build the Flatpak
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable  # only for the Flatpak
wine                  # to actually launch Windows games
osslsigncode          # version 2.14+ verifies Easy Installer Authenticode signatures
```

To run the Python application (Debian/Ubuntu names):

```
python3-pyside6.qtcore python3-pyside6.qtgui python3-pyside6.qtwidgets python3-pyside6.qtqml python3-pyside6.qtquick
qml6-module-org-kde-kirigami qml6-module-qtquick-dialogs qml6-module-qtquick-layouts qqc2-desktop-style
meson ninja-build gettext desktop-file-utils appstream
wine        # to actually launch Windows games
osslsigncode # version 2.14+ verifies Easy Installer Authenticode signatures
```

On Arch: `pyside6 kirigami qqc2-desktop-style`. On Fedora:
`python3-pyside6 kf6-kirigami qqc2-desktop-style`. PySide6 from pip also works
(`pip install PySide6`), as long as the distribution provides the Kirigami QML
modules and `qqc2-desktop-style`.

Optional helpers (both applications): `winetricks`, `mangohud`, `gamemode`,
`umu-run`.

## The Rust application

The Rust workspace is `crates/core` (the ported logic, with no GUI dependency at
all, so it is testable headless) and `crates/app` (the libcosmic interface plus
the `--list` / `--launch` / `--version` command line).

**What runs today:** the workspace builds, `cargo test` passes, and the CLI
entry point parses and dispatches before anything GUI-shaped is touched, so
`--launch` never needs a display. The library pages, the pages in
`docs/migration/PLAN.md` §4 and the ported logic land task by task — check
`docs/migration/PLAN.md` §6 before relying on a feature here.

```bash
cargo build                  # build the workspace
cargo run -- --version       # print the app name and the Cargo.toml version
cargo run -- --list          # print the library's ids and names
cargo run                    # the interface
```

Run the test suite — headless, no display and no GPU required:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

`scripts/verify.sh` is the single verification entry point for the Rust side. It
runs the build, clippy, the tests, the compatibility-oracle freshness check, the
Python suite, the vendored `cargo-sources.json` check, the Flatpak build, a
headless smoke test and the desktop/metainfo validation, and reports one line
per stage:

```bash
bash scripts/verify.sh --keep-going   # add --skip-flatpak for a fast local run
```

## Running the Python application from source

No install step is required for development — the package runs directly:

```bash
python3 -m gamehandler
```

List library entries and their ids:

```bash
python3 -m gamehandler --list
```

Launch a saved game from a shortcut:

```bash
python3 -m gamehandler --launch GAME_ID
```

Print the version:

```bash
python3 -m gamehandler --version
```

## Building / installing the Python application with Meson

```bash
meson setup build --prefix=/usr
meson compile -C build
meson test -C build          # validates the desktop entry and AppStream metainfo
sudo meson install -C build  # installs the `gamehandler` launcher, desktop file and icon
```

## Building the Flatpak

The Flatpak on this branch builds the **Rust** application. The Python
application is not packaged here any more; run it from source as above.

```bash
./build-aux/flatpak/build.sh
flatpak --user install --reinstall dist/gamehandler-0.8.0.flatpak
flatpak run com.goshapps.GameHandler
```

The bundle name follows the version in the Cargo workspace, so it changes with
the version bump. The manifest uses the Freedesktop 25.08 runtime and SDK with
the `rust-stable` extension on the Wine `stable-25.08` BaseApp, and inherits the
Freedesktop `Compat.i386` and `GL32` extensions. The build is offline: every
crate it needs is vendored in `build-aux/flatpak/cargo-sources.json`, generated
from `Cargo.lock`. `--allow=multiarch` is required for 32-bit Windows games and
downloaded Wine/Proton builds, and `--filesystem=xdg-run/gvfs` lets games on
mounted network shares launch from inside the sandbox.

The package bundles a checksum-pinned osslsigncode build for authenticated
Easy Installer downloads. The Microsoft Identity Verification Root CA used for
Ubisoft's Azure Trusted Signing chain comes from Microsoft's official PKI
repository at
`https://www.microsoft.com/pkiops/certs/microsoft%20identity%20verification%20root%20certificate%20authority%202020.crt`;
its DER SHA-256 is
`5367f20c7ade0e2bca790915056d086b720c33c1fa2a2661acf787e3292e1270`.

### Reviewed game-launcher permissions

Unlike a document-oriented app, a game launcher must execute user-selected
games and compatibility tools from arbitrary library locations and pass through
controllers and other game hardware. The Flatpak therefore deliberately keeps
`--filesystem=home`, and grants exactly the three device classes a launched
game needs — `--device=dri`, `--device=input`, `--device=usb`. These are
reviewed functionality exceptions, not permissions for package management or
unrelated host changes.

The device grant is deliberately **not** `--device=all`. That was the previous
value, and measuring what it actually put in the sandbox showed it exposed the
raw disk nodes, `/dev/mem`, `/dev/kvm`, the virtualisation and vhost nodes,
the watchdog, NVRAM and the raw serial ports to every game the launcher starts
— none of which any code here opens, and none of which a game needs. The three
narrow classes cover the GPU and the controller path; see
`docs/migration/packaging.md` section 3 for the before-and-after listing.

The Plugins page does **not** run `apt`, `dnf`, `pacman`, `zypper`, `sudo`, or
`pkexec` inside Flatpak. Host packages are not visible merely because they were
installed outside the sandbox, so missing optional helpers are shown as
unavailable. Bundling or runtime-extension integration for MangoHud, GameMode,
Winetricks, and UMU remains future packaging work. Source installs retain their
existing package-manager helper.

## Thanks to the projects GameHandler stands on

GameHandler is a front-end. It does not implement Windows compatibility, Direct3D translation, or a Proton build of its own — it configures the work of the projects below and stays out of their way. All of them are independent; none endorse GameHandler.

### Compatibility layers

The projects that actually run Windows games. GameHandler does not reimplement any of this — it configures it and gets out of the way.

- **[Wine](https://www.winehq.org)** — WineHQ and hundreds of contributors since 1993 _(LGPL-2.1-or-later)_  
  The Windows compatibility layer underneath every Windows title GameHandler launches, including every Proton build. Without Wine there is no GameHandler.
- **[Proton](https://github.com/ValveSoftware/Proton)** — Valve Software and CodeWeavers _(BSD-3-Clause, plus each bundled component's own license)_  
  Valve's Wine distribution with gaming patches and the Steam Linux Runtime. Every Proton family GameHandler downloads is a fork of it.
- **[DXVK](https://github.com/doitsujin/dxvk)** — Philip Rebohle and contributors _(zlib)_  
  Direct3D 8/9/10/11 over Vulkan — the DXVK toggle, and the runtime GameHandler bundles for raw Wine runners that ship without it.
- **[VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton)** — Hans-Kristian Arntzen, Philip Rebohle, and contributors _(LGPL-2.1-or-later)_  
  Direct3D 12 over Vulkan, behind the VKD3D toggle.
- **[DXVK-NVAPI](https://github.com/jp7677/dxvk-nvapi)** — Jens Peters and contributors _(MIT)_  
  NVIDIA NVAPI and DLSS support, behind the NVAPI/DLSS toggle.

### Proton and Wine builds

GameHandler ships none of these. It downloads the maintainers' own official release archives, from their own release pages, on request.

- **[Proton-GE](https://github.com/GloriousEggroll/proton-ge-custom)** — GloriousEggroll  
  Community Proton with codecs, protonfixes, and broad game fixes.
- **[proton-rtsp](https://github.com/SpookySkeletons/proton-rtsp)** — SpookySkeletons  
  Proton with RTSP and in-game media playback patches.
- **[Proton-CachyOS](https://github.com/CachyOS/proton-cachyos)** — The CachyOS project  
  Performance-oriented Proton with additional Wayland work.
- **[Proton-EM](https://github.com/Etaash-mathamsetty/Proton)** — Etaash Mathamsetty  
  Proton with Wine Wayland, HDR, and FSR additions.
- **[Wine-Builds](https://github.com/Kron4ek/Wine-Builds)** — Kron4ek  
  The standalone Wine, Wine-Staging, Staging-TkG, and Wine-Proton builds behind GameHandler's four Wine families.

### Launchers that shaped GameHandler

Prior art we studied and learned from. No code was copied from any of them; what GameHandler borrowed is their design thinking.

- **[Lutris](https://lutris.net)** — Mathieu Comandon and contributors _(GPL-3.0-or-later)_  
  The per-game runner and environment model, and the shape of the Esync/Fsync/DXVK/VKD3D/FSR compatibility toggles.
- **[Faugus Launcher](https://github.com/Faugus/faugus-launcher)** — Faugus _(MIT)_  
  The case for a small, focused launcher, and the idea of one-click store-launcher installs.
- **[Bottles](https://usebottles.com)** — The Bottles project _(GPL-3.0-or-later)_  
  Isolated per-title prefixes and the installable-programs catalog idea.
- **[ProtonPlus](https://github.com/Vysp3r/ProtonPlus)** — Vysp3r and contributors _(GPL-3.0-or-later)_  
  The compatibility-tool family list and the upstream release sources GameHandler fetches Proton and Wine builds from.
- **[Heroic Games Launcher](https://heroicgameslauncher.com)** — The Heroic Games Launcher team _(GPL-3.0-or-later)_  
  Prior art for treating store launchers as first-class library entries.

### Runtime helpers

Optional tools GameHandler detects and wraps launches with. They are installed from your distribution, never vendored here.

- **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)** — Open Wine Components _(GPL-3.0-or-later)_  
  Runs Proton builds with the Steam Linux Runtime outside Steam — how GameHandler launches Proton runners at all.
- **[MangoHud](https://github.com/flightlessmango/MangoHud)** — FlightlessMango and contributors _(MIT)_  
  The performance overlay behind the per-game MangoHud switch.
- **[Feral GameMode](https://github.com/FeralInteractive/gamemode)** — Feral Interactive _(BSD-3-Clause)_  
  Temporary system tuning while a game runs.
- **[Gamescope](https://github.com/ValveSoftware/gamescope)** — Valve Software _(BSD-2-Clause)_  
  The nested compositor behind scaling, a stable session, and HDR.
- **[Winetricks](https://github.com/Winetricks/winetricks)** — The Winetricks maintainers _(LGPL-2.1-or-later)_  
  Installs Windows runtimes and fonts from a game's Prefix tools menu.

### Platform

What GameHandler itself is built and shipped with.

- **[Qt](https://www.qt.io)** — The Qt Company and the Qt Project _(LGPL-3.0-only)_  
  The Qt 6 application framework the whole interface runs on.
- **[Kirigami](https://develop.kde.org/frameworks/kirigami/)** — The KDE community _(LGPL-2.0-or-later)_  
  KDE's QML framework behind the adaptive pages, drawer navigation, and the light and dark themes.
- **[PySide6](https://doc.qt.io/qtforpython-6/)** — The Qt Company _(LGPL-3.0-only)_  
  The official Python bindings that let GameHandler drive Qt.
- **[Meson and Flatpak](https://flatpak.org)** — The Meson and Flatpak projects  
  How GameHandler is built and packaged.
- **[Steam store web API](https://store.steampowered.com)** — Valve Software  
  Public artwork and genre lookup for automatic cover art.

## Why one app instead of assembling the stack yourself

### The stack was always the hard part, not the games

Running a Windows game on Linux takes a compatibility layer, a Proton or Wine build, Vulkan translation for Direct3D, a clean prefix, and the right environment variables. Each of those is a mature, excellent project. Assembling them is the part that stops people.

### One app instead of five

Without GameHandler the usual route is ProtonPlus (or a hand-extracted tarball) for runners, Lutris or Bottles for prefixes, winetricks by hand for runtimes, a separate wiki tab to learn which Proton fork a game needs, and a vendor installer run manually for each store launcher. GameHandler does those five jobs in one window.

### Bundling the workflow, not the projects

GameHandler vendors none of the runners it offers. It downloads the maintainers' own official release archives from their own release pages, on request, and keeps them in your data directory. Optional helpers like MangoHud and GameMode come from your distribution. Upgrading is still the upstream project's business, not ours.

### Defaults that match what the projects recommend

Esync, Fsync, DXVK, VKD3D, and the anti-cheat runtimes are on by default because that is what Proton does. Every one of them is a switch you can turn off per game, because the projects that wrote them documented when you should.

### The credit stays visible

Every runner family names its maintainer in the app, every compatibility toggle names the project that implements it, and this list ships inside the application rather than only in a file on a repository page.

## Running the tests

### Python suite — the behavioural reference

The Python suite is the contract the Rust port is written against: it pins how
the library, the runner command lines, the download handling and the security
checks behave, and it stays green while both trees are on this branch. The core
logic (library persistence, runner command building, multi-family Proton/Wine
release parsing) is covered by headless unit tests:

```bash
python3 -m unittest discover -s tests -t .
```

No PySide6 installation is needed for the core tests. The offscreen QML smoke
test runs when PySide6 is installed and is reported as skipped otherwise.
Discovery itself never imports the stub’s Qt types.

### Rust suite

```bash
cargo test
```

It needs no display server and no GPU, and that is a property of the code rather
than of the ambient environment: `crates/core` has no GUI dependency at all, so
the logic tests cannot reach a display even when `DISPLAY` and `WAYLAND_DISPLAY`
are set. (An earlier version of this paragraph said the suite "runs with" those
variables unset and implied plain `cargo test` was what unset them — it is not.
`bash scripts/verify.sh` is what unsets them, at `scripts/verify.sh:851`, and it
does so in the pipeline rather than in `cargo test`. The suite passes either way;
measured with both variables set, the whole workspace is green. `ARCH-08`.)

The Rust equivalents assert *behaviour* against the same cases the Python suite
covers (archive-extraction confinement, desktop-file escaping, runner
environment construction, JSON round-trips) rather than matching pixels.

`bash scripts/verify.sh` runs both suites together with the rest of the pipeline,
including a check that fails when `build-aux/flatpak/cargo-sources.json` is stale
against `Cargo.lock`, and a check that fails when the frozen Python-compatibility
fixtures in `docs/migration/oracle/` no longer match the Python implementation.

## Project layout

```
crates/                 # Rust workspace — the shipped application (0.8.0)
  core/                 # gamehandler-core: models, settings, paths, runners,
                        # installers, covers. No GUI dependency at all (enforced)
  app/                  # gamehandler: the libcosmic interface and the CLI
gamehandler/            # Python package — the parity reference (0.7.2)
  main.py               # CLI entry point + Qt application bootstrap
  bridge.py             # the QML-facing backend (library, runners, installers…)
  theme.py              # light/dark/system color schemes
  qml/                  # the Kirigami interface
    Main.qml            # application window, navigation, notifications
    LibraryPage.qml     # grid/list library with search, sort, categories
    GameFormPage.qml    # add / edit game form
    InstallersPage.qml  # one-click store-launcher installs
    RunnersPage.qml     # Proton/Wine downloads and guide
    PluginsPage.qml     # optional helper detection
    CreditsPage.qml     # About, maker identity, and upstream acknowledgements
    SettingsPage.qml    # appearance, defaults, behavior
    CoverArt.qml        # cover tiles and generated placeholder art
  installers.py         # easy-installer catalog and prefix helpers
  settings.py           # persisted preferences
  models.py             # Game model + JSON-backed Library
  credits.py            # upstream acknowledgements (source of truth)
  runners.py            # runner families, downloads, launch helpers
  netpaths.py           # network-share (GVFS) path resolution
  config.py             # XDG paths
bin/gamehandler.in      # installed launcher template (Python, Meson)
data/                   # desktop entry, AppStream metainfo, icon, trust root
build-aux/flatpak/      # Flatpak manifest + vendored cargo sources
scripts/verify.sh       # the single verification entry point
docs/migration/         # the migration plan, decisions and parity inventory
tests/                  # Python headless unit tests (incl. an offscreen QML smoke test)
```

`data/` is shared by both applications: the Flatpak manifest installs the
desktop entry, the AppStream metainfo and the icon from there
(`data/meson.build` installs the same three files, plus the project LICENSE,
for a Meson/source install). The Microsoft Authenticode trust root also lives
in `data/`, and the Python tree's Meson build installs that one from
`gamehandler/meson.build` — it is what `installers.py` verifies Easy Installer
signatures against.
