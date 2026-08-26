# GameHandler

GameHandler is a modern game manager for Linux, focused on running **Windows games**
via **Wine** and **Proton**, built with a clean **GTK4 + libadwaita** interface.

It is a **front-end, not a compatibility layer**. Every Windows game it launches runs
on [Wine](https://www.winehq.org), usually through a [Proton](https://github.com/ValveSoftware/Proton)
build maintained by someone else, with Direct3D translated by [DXVK](https://github.com/doitsujin/dxvk)
and [VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton). GameHandler
downloads those builds from their maintainers' own release pages — the same upstream
sources [ProtonPlus](https://github.com/Vysp3r/ProtonPlus) uses — sets up isolated
prefixes, and stays out of their way. Its design owes a lot to
[Lutris](https://lutris.net), [Faugus Launcher](https://github.com/Faugus/faugus-launcher),
and [Bottles](https://usebottles.com). Full acknowledgements are
[below](#thanks-to-the-projects-gamehandler-stands-on) and on the app's own Credits page.

## Features

- Dark mode by default, plus system and light themes
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
- Credits page naming every upstream project, with links and licenses
- Desktop shortcuts that launch a library entry with `gamehandler --launch`

Flatpak users who enable Gamescope also need the matching Freedesktop 25.08 extension:

```bash
flatpak install flathub org.freedesktop.Platform.VulkanLayer.gamescope//25.08
```

## Tech stack

| Area | Choice |
| --- | --- |
| Language | Python 3 |
| UI toolkit | GTK 4 + libadwaita (`PyGObject`) |
| Build system | Meson |
| Packaging | Flatpak (GNOME runtime) |
| Runners | System Wine plus downloaded Proton/Wine builds |

## Requirements

System packages (Debian/Ubuntu names):

```
python3-gi python3-gi-cairo gir1.2-gtk-4.0 gir1.2-adw-1 libadwaita-1-0
meson ninja-build gettext desktop-file-utils appstream
wine        # to actually launch Windows games
osslsigncode # version 2.14+ verifies Easy Installer Authenticode signatures
```

Optional helpers: `winetricks`, `mangohud`, `gamemode`, `umu-run`.

## Running from source

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

## Building / installing with Meson

```bash
meson setup build --prefix=/usr
meson compile -C build
meson test -C build          # validates the desktop entry and AppStream metainfo
sudo meson install -C build  # installs the `gamehandler` launcher, desktop file and icon
```

## Building the Flatpak

```bash
./build-aux/flatpak/build.sh
flatpak --user install --reinstall dist/gamehandler-0.6.1.flatpak
flatpak run com.goshapps.GameHandler
```

The Flatpak uses GNOME 50 on the Wine `stable-25.08` BaseApp and inherits the
Freedesktop `Compat.i386` and `GL32` extensions. `--allow=multiarch` is required
for 32-bit Windows games and downloaded Wine/Proton builds.
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
`--filesystem=home` and `--device=all`. These are reviewed functionality
exceptions, not permissions for package management or unrelated host changes.

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

- **[GTK](https://www.gtk.org)** — The GNOME Project _(LGPL-2.1-or-later)_  
  The toolkit the whole interface is built on.
- **[libadwaita](https://gitlab.gnome.org/GNOME/libadwaita)** — The GNOME Project _(LGPL-2.1-or-later)_  
  Adaptive widgets, the dark theme, and the GNOME look.
- **[PyGObject](https://pygobject.gnome.org)** — The PyGObject maintainers _(LGPL-2.1-or-later)_  
  The Python bindings that let GameHandler drive GTK.
- **[Meson and Flatpak](https://flatpak.org)** — The Meson and Flatpak projects  
  How GameHandler is built and packaged.
- **[Steam store web API](https://store.steampowered.com)** — Valve Software  
  Public artwork and genre lookup for automatic cover art.

## Why one app instead of assembling the stack yourself

### The stack was always the hard part, not the games

Running a Windows game on Linux takes a compatibility layer, a Proton or Wine build, Vulkan translation for Direct3D, a clean prefix, and the right environment variables. Each of those is a mature, excellent project. Assembling them is the part that stops people.

### One app instead of five

Without GameHandler the usual route is ProtonPlus (or a hand-extracted tarball) for runners, Lutris or Bottles for prefixes, winetricks by hand for runtimes, a separate wiki tab to learn which Proton fork a game needs, and a vendor installer run manually for each store launcher. GameHandler does those five jobs in one GTK4 window.

### Bundling the workflow, not the projects

GameHandler vendors none of the runners it offers. It downloads the maintainers' own official release archives from their own release pages, on request, and keeps them in your data directory. Optional helpers like MangoHud and GameMode come from your distribution. Upgrading is still the upstream project's business, not ours.

### Defaults that match what the projects recommend

Esync, Fsync, DXVK, VKD3D, and the anti-cheat runtimes are on by default because that is what Proton does. Every one of them is a switch you can turn off per game, because the projects that wrote them documented when you should.

### The credit stays visible

Every runner family names its maintainer in the app, every compatibility toggle names the project that implements it, and this list ships inside the application rather than only in a file on a repository page.

## Running the tests

The core logic (library persistence, runner command building, multi-family
Proton/Wine release parsing) is covered by headless unit tests:

```bash
python3 -m unittest discover -s tests
```

## Project layout

```
gamehandler/            # Python package (application code)
  main.py               # Adw.Application + entry point
  window.py             # library, runners, and settings shell
  add_game_dialog.py    # add / edit game dialog
  installers.py         # easy-installer catalog and prefix helpers
  installers_page.py    # Installers store page
  runners_dialog.py     # Proton/Wine download page
  settings.py           # persisted preferences
  models.py             # Game model + JSON-backed Library
  credits.py            # upstream acknowledgements (source of truth)
  credits_page.py       # in-app Credits page
  runners.py            # runner families, downloads, launch helpers
  config.py             # XDG paths
bin/gamehandler.in      # installed launcher template
data/                   # desktop entry, AppStream metainfo, icon
build-aux/flatpak/      # Flatpak manifest
tests/                  # headless unit tests
```
