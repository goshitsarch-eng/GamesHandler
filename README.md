# GameHandler

GameHandler is a modern game manager for Linux, focused on running **Windows games**
via **Wine** and **Proton**. It is inspired by [Faugus Launcher](https://github.com/Faugus/faugus-launcher)
and [Lutris](https://github.com/lutris/lutris), built with a clean **GTK4 + libadwaita**
interface, and downloads compatibility tools itself from the same upstream sources
used by [ProtonPlus](https://github.com/Vysp3r/ProtonPlus).

## Features

- Dark mode by default, plus system and light themes
- Grid and list library views, search, categories, and per-game edit
- Custom cover art, plus automatic Steam cover lookup by game name
- Add Windows `.exe` titles or Linux-native games
- Choose a Proton or Wine runner when adding a game, and change it later
- Download and remove compatibility tools from inside the app:
  - Proton-GE, Proton-GE RTSP, Proton-CachyOS, Proton-EM
  - Kron4ek Wine-Vanilla, Wine-Staging, Wine-Staging-Tkg, Wine-Proton
- Guide describing when to use each Proton or Wine family
- Isolated Wine prefixes, Winecfg, Winetricks, and prefix folder access
- Launch helpers: MangoHud, Feral GameMode, Prefer SDL, Wine Wayland, HDR, Esync, Fsync, DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye, EAC, Gamescope, virtual desktop, and custom environment variables
- Easy installers for Battle.net, Epic, EA App, Ubisoft Connect, GOG Galaxy, Amazon Games, Rockstar, Steam, and Discord
- Plugins page that detects MangoHud, GameMode, Winetricks, and UMU
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
```

Optional helpers: `winetricks`, `mangohud`, `gamemode`, `umu-run`.

## Running from source

No install step is required for development — the package runs directly:

```bash
python3 -m gamehandler
```

Launch a saved game from a shortcut:

```bash
python3 -m gamehandler --launch GAME_ID
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
flatpak --user install --reinstall dist/gamehandler-0.5.0.flatpak
flatpak run com.goshapps.GameHandler
```

The Flatpak uses GNOME 50 on the Wine `stable-25.08` BaseApp and inherits the
Freedesktop `Compat.i386` and `GL32` extensions. `--allow=multiarch` is required
for 32-bit Windows games and downloaded Wine/Proton builds.

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
  runners.py            # runner families, downloads, launch helpers
  config.py             # XDG paths
bin/gamehandler.in      # installed launcher template
data/                   # desktop entry, AppStream metainfo, icon
build-aux/flatpak/      # Flatpak manifest
tests/                  # headless unit tests
```
