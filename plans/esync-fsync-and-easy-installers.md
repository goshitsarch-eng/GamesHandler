# Esync/Fsync toggles and easy installers

## Context

GameHandler already has Lutris/Faugus-style per-game launch helpers (`mangohud`, `gamemode`, `prefer_sdl`, `wayland`, `hdr`) stored on `Game`, defaulted in `Settings`, shown as `Adw.SwitchRow`s, and applied in `apply_launch_options()`. It does **not** expose Esync/Fsync, and adding a Windows app still means picking an `.exe` by hand.

The request is two related UX gaps:

1. **Lutris-style Esync / Fsync toggles** so users can turn Wine eventfd/futex sync on or off without editing env vars.
2. **Faugus / Bottles-style easy installers** so store launchers and a few common apps can be set up in one click (download official installer, run it in a prefix, add the resulting library entry).

This stays in the existing GTK-free core + libadwaita page pattern. No Bottles YAML engine and no Lutris script database — a small first-party catalog, same way runners are a first-party family list.

## Approach

### 1. Esync / Fsync (Lutris-style switches)

Add two booleans on `Game` and matching defaults on `Settings`, both **on by default** (Proton already enables them when the kernel allows it; existing library JSON will pick up the dataclass defaults).

`apply_launch_options()` sets both Wine and Proton knobs so the same switch works for System Wine, Kron4ek Wine, and Proton/UMU:

| Toggle | On | Off |
| --- | --- | --- |
| Esync | `WINEESYNC=1` | `WINEESYNC=0` and `PROTON_NO_ESYNC=1` |
| Fsync | `WINEFSYNC=1` | `WINEFSYNC=0` and `PROTON_NO_FSYNC=1` |

Both off-flags are required: Proton historically lets `WINEESYNC`/`WINEFSYNC` override `PROTON_NO_*`.

UI:

- Add/Edit Game → Launch options: `Adw.SwitchRow`s next to Wayland/HDR. Disabled for Linux-native titles (same as Wayland/HDR).
- Settings → New games: default Esync/Fsync switches next to MangoHud/GameMode/SDL.
- Subtitles: Esync = eventfd sync (disable if you hit `EMFILE` / fd-limit crashes). Fsync = futex sync, preferred when the kernel supports it.

### 2. Easy installers (Faugus catalog + Bottles store feel)

Add a first-party **Installers** page (sidebar, between Library and Runners) that looks like the existing Runners/Plugins pages: search, category filter, one **Install** button per recipe.

**Catalog (shipped in code, official vendor URLs only):**

Store launchers (Faugus-style): Battle.net, Epic Games, EA App, Ubisoft Connect, GOG Galaxy, Amazon Games, Rockstar Launcher, Steam (Windows).

Common apps (Bottles-style): Discord. More can be added later without a schema change.

Each recipe is a frozen dataclass, not YAML:

- `id`, `name`, `description`, `category` (`Launchers` / `Apps`)
- official `download_url` + `filename`
- `kind`: `exe` or `msi`
- `expected_exe`: one or more `drive_c`-relative paths (first hit wins)
- optional installer `arguments`, `notes`
- recommended launch flags (`esync`, `fsync`, …)

**Install flow** (GTK-free engine + progress dialog):

1. Pick recipe + runner (default = Settings default runner). Refuse Windows recipes if no Wine/Proton is available.
2. Create an isolated prefix under `prefixes_dir() / <game-id>`.
3. Download the vendor installer to `downloads_dir()` (new XDG path), reusing the Proton download progress callback style.
4. Launch it with the selected runner (`wine setup.exe`, or `wine msiexec /i setup.msi` for Epic). Wait for the process. The vendor wizard stays interactive — silent flags are unreliable under Wine.
5. Scan the prefix for `expected_exe` (case-insensitive, a few well-known fallbacks such as `Battle.net.exe` vs `Battle.net Launcher.exe`).
6. On success: add a `Game` (name, exe, prefix, runner, category, recommended sync flags), toast, refresh library, optionally Steam-cover fetch by launcher name.
7. On missing exe: keep the prefix, toast, and offer “Browse for executable…” so a finished-but-relocated install is not thrown away.

Do **not** redistribute game binaries, do **not** scrape unofficial mirrors, and do **not** implement Bottles `Dependencies` / winetricks auto-steps in this pass. Winetricks stays on the existing Prefix tools menu. Optional `winetricks` tuples can be reserved on the dataclass for a later pass.

## Files to modify

- `gamehandler/models.py` — `esync` / `fsync` on `Game`
- `gamehandler/settings.py` — `default_esync` / `default_fsync`
- `gamehandler/runners.py` — apply sync env in `apply_launch_options()`
- `gamehandler/add_game_dialog.py` — SwitchRows + persist + kind sensitivity
- `gamehandler/settings_page.py` — default SwitchRows
- `gamehandler/window.py` — Installers nav item, empty-state CTA, page wiring
- `gamehandler/main.py` — `app.installers` action
- `gamehandler/config.py` — `downloads_dir()` + `ensure_dirs()`
- `gamehandler/meson.build` — install new modules
- `README.md`, `data/com.goshapps.GameHandler.metainfo.xml` — features + 0.4.0 notes
- `gamehandler/__init__.py` — version `0.4.0` (keep in lockstep with meson/metainfo)
- `meson.build` — project version `0.4.0`

New:

- `gamehandler/installers.py` — catalog, download, installer argv, prefix exe lookup (no GTK)
- `gamehandler/installers_page.py` — Installers page + progress dialog
- `tests/test_installers.py` — catalog, msiexec argv, exe discovery, env flags

## Reuse

- Launch-option pattern: `Game` field → Settings default → `AddGameDialog` SwitchRow → `apply_launch_options()` → unit test (same as Wayland/HDR).
- Download + progress: `ProtonManager.install()` / `RunnersPage` progress bar + worker thread + `GLib.idle_add`.
- Prefix launch: `Runner.build_command()` / `tool_command()` for `WINEPREFIX` + wine binary; MSI wraps `msiexec /i`.
- Navigation shell: `window.py` sidebar + `Gtk.Stack` pages.
- Cover autofetch after add: `_autofetch_cover()`.
- Public vendor URLs from Faugus (`download_launcher`) and Bottles `programs` (Steam, Battle.net) as references only — write our own catalog, do not vendor their files.

## Implementation checklist

- [ ] Add `Game.esync` / `Game.fsync` (default `True`) and keep `from_dict` tolerant of older JSON
- [ ] Add `Settings.default_esync` / `default_fsync` (default `True`)
- [ ] Apply Lutris-compatible env in `apply_launch_options()` for both Wine and Proton
- [ ] Add Esync/Fsync SwitchRows to Add/Edit Game; disable for Linux-native
- [ ] Add matching default SwitchRows on the Settings page
- [ ] Add `downloads_dir()` and include it in `ensure_dirs()`
- [ ] Implement `installers.py` catalog (8 launchers + Discord) with official URLs and expected `drive_c` paths
- [ ] Implement download + `exe`/`msi` launch + case-insensitive prefix exe discovery
- [ ] Add Installers page (search, Launchers/Apps filter, runner picker, progress, browse-fallback)
- [ ] Wire sidebar, app action, empty-library “Easy install” button
- [ ] Bump version to 0.4.0 and document the features
- [ ] Unit tests for sync env, catalog completeness, msiexec argv, and exe lookup

## Verification

- `python3 -m unittest discover -s tests`
- Manual: toggle Esync/Fsync on a Wine game and confirm `WINEESYNC`/`WINEFSYNC` / `PROTON_NO_*` in a throwaway debug print or by inspecting `apply_launch_options()` tests
- Manual: Installers page lists recipes, download progress works, Epic uses `msiexec /i`, a completed install becomes a library card
- Manual: cancel / missing exe keeps the prefix and offers browse
- Existing add/edit/launch paths still work for games saved before this change
