# GameHandler

GameHandler is a game manager for Linux. It runs Windows games through Wine or
Proton, runs Linux-native games directly, downloads compatibility runners,
manages isolated prefixes, and installs the big store launchers for you.

Current release: **0.8.0** — a Rust + libcosmic (COSMIC toolkit) application,
shipped as a Flatpak. The earlier Python + Qt 6/Kirigami implementation remains
in `gamehandler/` as the parity reference the Rust code is tested against; it
is not what the Flatpak ships.

## AI-assisted development

I use AI tools to speed up development, but I work architecture-first. I define
the architecture, build and review the implementation, refactor it, and repeat
the process as the project evolves.

I treat AI as a junior developer: useful for implementation and exploration,
but not the final authority. I remain responsible for the architecture,
technical decisions, and quality of the code.

I'm including this notice so you can make an informed choice about whether
AI-assisted software is something you're comfortable using.

## What it is

GameHandler is a **front-end, not a compatibility layer**. Every Windows game
it launches runs on [Wine](https://www.winehq.org), usually through a
[Proton](https://github.com/ValveSoftware/Proton) build maintained by someone
else, with Direct3D translated by [DXVK](https://github.com/doitsujin/dxvk) and
[VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton). GameHandler
downloads those builds from their maintainers' own release pages, sets up
isolated prefixes, and stays out of their way. Its design owes a lot to
[Lutris](https://lutris.net),
[Faugus Launcher](https://github.com/Faugus/faugus-launcher), and
[Bottles](https://usebottles.com). Full acknowledgements are
[below](#thanks-to-the-projects-gamehandler-stands-on) and on the app's own
About & Credits page.

## Features

- Library with grid and list views, search, category filters, sorting
  (name, recently played, recently added), and per-game edit
- Cover art: Steam lookup by name, custom images, the icon inside the
  executable, and generated art when nothing else is found
- Windows `.exe` and Linux-native games; pick a runner per game and change
  it later
- Download and remove runners from inside the app: Proton-GE, Proton-GE RTSP,
  Proton-CachyOS, Proton-EM, and the Kron4ek Wine-Vanilla, Wine-Staging,
  Wine-Staging-Tkg and Wine-Proton builds — each with a short guide to when
  you'd want it
- Isolated Wine prefixes per game, with Winecfg, Winetricks, and an
  "open prefix folder" action on each entry
- Launch options per game and as defaults: MangoHud, GameMode, Prefer SDL,
  Wine Wayland, HDR, Esync, Fsync, DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye,
  Easy Anti-Cheat, Gamescope, a Wine virtual desktop, and custom environment
  variables
- One-click installers for Battle.net, Epic Games, EA App, Ubisoft Connect,
  GOG Galaxy, Amazon Games, Rockstar, Steam, and Discord. Each downloads the
  vendor's own installer, verifies its Authenticode signature, waits for the
  wizard, then adds the result to your library
- Plugins page that detects MangoHud, GameMode, Winetricks, UMU, and
  Gamescope, and offers install commands where the platform allows it
- Desktop shortcuts that launch a game straight from your app menu
- Games and covers on network shares work: `smb://`-style locations resolve
  through their mounted GVFS path so Wine can run them

## Install

The shipped package is a Flatpak bundle you build locally:

```bash
./build-aux/flatpak/build.sh
flatpak --user install dist/gamehandler-0.8.0.flatpak
flatpak run com.goshapps.GameHandler
```

The build needs `flatpak`, `flatpak-builder`, and `flatpak-repo`, and downloads
the Freedesktop 25.08 runtime/SDK, the `rust-stable` SDK extension, and the
Wine `stable-25.08` BaseApp from Flathub on first run. Once those are
installed, crate fetching is offline — every crate is vendored in
`build-aux/flatpak/cargo-sources.json`, generated from `Cargo.lock`. Set
`GAMEHANDLER_BUILD_OFFLINE=1` to skip the remaining Flathub dependency
resolution too.

The bundle name follows the workspace version, so it changes on each release.
The manifest bundles osslsigncode (used to verify the Easy Installers'
Authenticode signatures), the Microsoft Authenticode trust root, and the DXVK
runtime used by Wine runners that ship without their own.

### Releases

Prebuilt artifacts for Linux x86_64 and aarch64 — a `.tar.gz` and a `.flatpak`
per architecture plus `SHA256SUMS` — are built and published by GitHub Actions
whenever a `vX.Y.Z` tag is pushed. See [docs/RELEASING.md](docs/RELEASING.md)
for the maintainer release process and
[docs/release/](docs/release/) for how the pipeline works.

### Permissions

A game launcher has to execute games and tools from wherever your library
lives and pass controllers through, so the sandbox is wider than a document
app's — but it is deliberately narrower than it used to be:

- `--filesystem=home:ro` — read-only access to your home directory, because
  games execute from it
- `--filesystem=~/.local/share/applications:create` — the one writable
  carve-out, so "create desktop shortcut" can write launcher files
- `--device=dri`, `--device=input`, `--device=usb` — GPU, controllers, and
  game hardware; deliberately **not** `--device=all`, which also exposed
  `/dev/mem`, `/dev/kvm`, and raw disk nodes no game needs
- `--filesystem=xdg-run/gvfs` — so games on GVFS-mounted network shares launch
  from inside the sandbox
- `--allow=multiarch` — required for 32-bit Windows games and downloaded
  Wine/Proton builds

The Plugins page never runs `apt`, `dnf`, `pacman`, `zypper`, `sudo`, or
`pkexec` inside the Flatpak — host package installs are refused outright, and
helpers installed outside the sandbox are not visible to it. MangoHud and
Gamescope have Flatpak extension installs instead. To use Gamescope inside
the Flatpak, install the matching runtime extension:

```bash
flatpak install flathub org.freedesktop.Platform.VulkanLayer.gamescope//25.08
```

## Use

The window has six pages: **Library**, **Installers**, **Runners**,
**Plugins**, **About & Credits**, and **Settings**.

Add a game from the Library page (or `Ctrl+N`): point it at a Windows `.exe`
or a Linux executable, pick a runner, and set any launch options. The game
menu on each entry handles cover art, Winecfg, Winetricks, the prefix folder,
desktop shortcuts, and removal. Settings holds the theme, the library layout,
and the default values new games inherit.

Games and settings live in the usual XDG locations:

- `~/.config/gamehandler/` — `games.json`, `settings.json`
- `~/.local/share/gamehandler/` — downloaded runners, prefixes, covers,
  downloads

`GAMEHANDLER_CONFIG_HOME` and `GAMEHANDLER_DATA_HOME` override those two
roots (and take precedence over `XDG_CONFIG_HOME` / `XDG_DATA_HOME`).

### Command line

```bash
gamehandler --list              # print library ids and names
gamehandler --launch <GAME_ID>  # launch an entry without opening the GUI
gamehandler --version
gamehandler --help
```

`--launch` is what generated desktop shortcuts call, so it works without a
display. With no display at all, the GUI exits with a diagnostic naming the
missing `WAYLAND_DISPLAY`/`DISPLAY` variables rather than crashing.

### Keyboard shortcuts

| Keys | Action |
|---|---|
| `Ctrl+N` | Add a game |
| `Ctrl+F` | Search the library |
| `Ctrl+,` | Settings |
| `Ctrl+Q` | Quit |

## Known limitations

- **Linux on x86_64 only.** The Wine base app and every downloadable runner
  are x86 builds.
- **Proton runners need `umu-run` on PATH.** Without it a Proton build
  launches as plain Wine, and the Proton-only options (NVAPI/DLSS, FSR, Wine
  Wayland, HDR) do nothing. `umu-run` is not bundled in the Flatpak and has
  no Flatpak extension, so this fallback is what Flatpak users get today.
  From a source install, `umu-launcher` from your distribution enables the
  full Proton path.
- **Flatpak helpers:** helpers installed on the host are not visible inside
  the sandbox. MangoHud and Gamescope are available as Flatpak extensions;
  GameMode, Winetricks, and UMU currently are not.
- **The library file is JSON, not a database.** It is meant to be read and
  backed up by hand; there is no import from other launchers.
- Anti-cheat runtimes are configured, not guaranteed — games whose anti-cheat
  refuses Wine still will not run, as with any launcher.

## Development

```bash
cargo build                   # build the workspace
cargo run                     # run the GUI
cargo test                    # the test suite — headless, no display or GPU needed
cargo clippy --all-targets -- -D warnings
```

The workspace is `crates/core` (all game logic, no GUI dependency — it stays
headless and testable) and `crates/app` (the libcosmic interface and CLI).
Rust 1.93 or newer is required; `rust-toolchain.toml` pins 1.98.1 for
development, the same version the Flatpak's rust-stable SDK extension
provides.

`scripts/verify.sh` is the full verification pipeline — build, fmt, clippy,
docs, tests, CLI checks, the Python parity suite, the packaging checks, a
Flatpak build and smoke test, and more:

```bash
bash scripts/verify.sh --help            # lists every stage and flag
bash scripts/verify.sh --skip-flatpak    # fast local run
```

The Python reference tree in `gamehandler/` also runs from source and has its
own suite:

```bash
python3 -m gamehandler                            # run the reference app
python3 -m unittest discover -s tests -t .        # its tests
```

Contributing: see [CONTRIBUTING.md](CONTRIBUTING.md). The port's working
record — plans, decisions, parity inventory, audit findings — lives under
`docs/migration/` and `docs/audit/`; those are historical engineering
documents, not user documentation.

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

- **[Rust](https://www.rust-lang.org)** — The Rust Project developers _(Apache-2.0 OR MIT)_  
  The language the shipped application is written in.
- **[libcosmic](https://github.com/pop-os/libcosmic)** — System76 and the COSMIC contributors _(MPL-2.0)_  
  The COSMIC toolkit the shipped interface is built with.
- **[Qt](https://www.qt.io)** — The Qt Company and the Qt Project _(LGPL-3.0-only)_  
  The Qt 6 framework the Python parity implementation runs on.
- **[Kirigami](https://develop.kde.org/frameworks/kirigami/)** — The KDE community _(LGPL-2.0-or-later)_  
  KDE's QML framework behind the parity implementation's adaptive pages, drawer navigation, and light and dark themes.
- **[PySide6](https://doc.qt.io/qtforpython-6/)** — The Qt Company _(LGPL-3.0-only)_  
  The Python bindings the parity implementation uses to drive Qt.
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

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
