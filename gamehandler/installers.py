"""First-party easy-installer catalog and Wine prefix helpers.

This module is GTK-free so catalog lookup, download, msiexec argv, and
prefix exe discovery can be unit tested headlessly. Official vendor
download URLs only — no redistributed game binaries.
"""

from __future__ import annotations

import os
import shlex
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable
from urllib.request import Request, urlopen

from . import config
from .models import Game
from .runners import USER_AGENT, Runner, apply_launch_options

LAUNCHERS = "Launchers"
APPS = "Apps"
INSTALLER_CATEGORIES = (LAUNCHERS, APPS)


@dataclass(frozen=True)
class Installer:
    """A curated one-click setup recipe for a Windows launcher or app."""

    id: str
    name: str
    description: str
    category: str
    download_url: str
    filename: str
    kind: str  # exe | msi
    expected_exe: tuple[str, ...]
    arguments: str = ""
    notes: str = ""
    esync: bool = True
    fsync: bool = True
    library_category: str = LAUNCHERS


INSTALLERS: tuple[Installer, ...] = (
    Installer(
        id="battlenet",
        name="Battle.net",
        description="Blizzard and Activision store client (Warcraft, Diablo, Overwatch, Call of Duty).",
        category=LAUNCHERS,
        download_url=(
            "https://downloader.battle.net/download/getInstaller"
            "?os=win&installer=Battle.net-Setup.exe"
        ),
        filename="Battle.net-Setup.exe",
        kind="exe",
        expected_exe=(
            "Program Files (x86)/Battle.net/Battle.net Launcher.exe",
            "Program Files (x86)/Battle.net/Battle.net.exe",
            "Program Files/Battle.net/Battle.net Launcher.exe",
            "Program Files/Battle.net/Battle.net.exe",
        ),
        notes="Complete the Battle.net wizard, then sign in once before launching games.",
    ),
    Installer(
        id="epic",
        name="Epic Games Launcher",
        description="Epic Games Store client for Fortnite and Epic exclusives.",
        category=LAUNCHERS,
        download_url=(
            "https://launcher-public-service-prod06.ol.epicgames.com/"
            "launcher/api/installer/download/EpicGamesLauncherInstaller.msi"
        ),
        filename="EpicGamesLauncherInstaller.msi",
        kind="msi",
        expected_exe=(
            "Program Files (x86)/Epic Games/Launcher/Portal/Binaries/Win32/EpicGamesLauncher.exe",
            "Program Files (x86)/Epic Games/Launcher/Portal/Binaries/Win64/EpicGamesLauncher.exe",
            "Program Files/Epic Games/Launcher/Portal/Binaries/Win64/EpicGamesLauncher.exe",
        ),
    ),
    Installer(
        id="ea-app",
        name="EA App",
        description="EA Desktop client (Apex, Battlefield, The Sims, Steam-unlisted EA titles).",
        category=LAUNCHERS,
        download_url=(
            "https://origin-a.akamaihd.net/EA-Desktop-Client-Download/"
            "installer-releases/EAappInstaller.exe"
        ),
        filename="EAappInstaller.exe",
        kind="exe",
        expected_exe=(
            "Program Files/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe",
            "Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe",
            "Program Files (x86)/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe",
        ),
    ),
    Installer(
        id="ubisoft",
        name="Ubisoft Connect",
        description="Ubisoft store and overlay (Assassin's Creed, Far Cry, Rainbow Six).",
        category=LAUNCHERS,
        download_url="https://static3.cdn.ubi.com/orbit/launcher_installer/UbisoftConnectInstaller.exe",
        filename="UbisoftConnectInstaller.exe",
        kind="exe",
        expected_exe=(
            "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe",
            "Program Files/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe",
            "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/upc.exe",
        ),
    ),
    Installer(
        id="gog",
        name="GOG Galaxy",
        description="GOG's DRM-free store client and optional game overlay.",
        category=LAUNCHERS,
        download_url=(
            "https://content-system.gog.com/open_link/download"
            "?path=/open/galaxy/client/setup_galaxy_2.0.exe"
        ),
        filename="setup_galaxy_2.0.exe",
        kind="exe",
        expected_exe=(
            "Program Files (x86)/GOG Galaxy/GalaxyClient.exe",
            "Program Files/GOG Galaxy/GalaxyClient.exe",
        ),
    ),
    Installer(
        id="amazon",
        name="Amazon Games",
        description="Amazon Games app for Prime Gaming claims and Amazon-published titles.",
        category=LAUNCHERS,
        download_url="https://download.amazongames.com/AmazonGamesSetup.exe",
        filename="AmazonGamesSetup.exe",
        kind="exe",
        expected_exe=(
            "users/steamuser/AppData/Local/Amazon Games/App/Amazon Games.exe",
            "Program Files/Amazon Games/App/Amazon Games.exe",
            "Program Files (x86)/Amazon Games/App/Amazon Games.exe",
        ),
    ),
    Installer(
        id="rockstar",
        name="Rockstar Games Launcher",
        description="Rockstar store client (GTA, Red Dead, older Rockstar titles).",
        category=LAUNCHERS,
        download_url="https://gamedownloads.rockstargames.com/public/installer/Rockstar-Games-Launcher.exe",
        filename="Rockstar-Games-Launcher.exe",
        kind="exe",
        expected_exe=(
            "Program Files/Rockstar Games/Launcher/Launcher.exe",
            "Program Files (x86)/Rockstar Games/Launcher/Launcher.exe",
        ),
    ),
    Installer(
        id="steam",
        name="Steam",
        description="Windows Steam client, useful for titles that need the official Steam overlay.",
        category=LAUNCHERS,
        download_url="https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe",
        filename="SteamSetup.exe",
        kind="exe",
        expected_exe=(
            "Program Files (x86)/Steam/steam.exe",
            "Program Files/Steam/steam.exe",
        ),
    ),
    Installer(
        id="discord",
        name="Discord",
        description="Discord desktop client for voice, chat, and overlays.",
        category=APPS,
        download_url="https://discord.com/api/download?platform=win",
        filename="DiscordSetup.exe",
        kind="exe",
        expected_exe=(
            "users/steamuser/AppData/Local/Discord/Update.exe",
            "users/steamuser/AppData/Local/Discord/Discord.exe",
        ),
        library_category="Utility",
        notes="Discord lives under Local AppData. Launch Update.exe if the app folder version changes.",
    ),
)

_INSTALLERS = {item.id: item for item in INSTALLERS}


def installers() -> tuple[Installer, ...]:
    return INSTALLERS


def installer_by_id(installer_id: str) -> Installer:
    try:
        return _INSTALLERS[installer_id]
    except KeyError as exc:
        raise KeyError(f"Unknown installer: {installer_id}") from exc


def search_installers(query: str = "", category: str = "") -> list[Installer]:
    """Filter the catalog by name/description and Launchers/Apps category."""
    needle = query.strip().lower()
    results = list(INSTALLERS)
    if category and category not in {"", "All"}:
        results = [item for item in results if item.category == category]
    if not needle:
        return results
    return [
        item
        for item in results
        if needle in item.name.lower()
        or needle in item.description.lower()
        or needle in item.id.lower()
    ]


def installer_argv(wine_or_umu: str, installer_path: str | Path, kind: str, arguments: str = "") -> list[str]:
    """Build the argv used to run a downloaded ``exe`` or ``msi`` installer."""
    path = str(installer_path)
    if kind == "msi":
        argv = [wine_or_umu, "msiexec", "/i", path]
    else:
        argv = [wine_or_umu, path]
    if arguments:
        argv.extend(shlex.split(arguments))
    return argv


def build_installer_command(
    runner: Runner,
    prefix: str | Path,
    installer: Installer,
    installer_path: str | Path,
) -> tuple[list[str], dict[str, str]]:
    """Return ``(argv, env)`` that launches *installer* inside *prefix*."""
    placeholder = Game(
        name=installer.name,
        prefix_path=str(prefix),
        esync=installer.esync,
        fsync=installer.fsync,
    )
    argv, env = runner.build_command(placeholder)
    if not argv:
        raise RuntimeError(f"Runner '{runner.name}' produced an empty command")
    wrapped = installer_argv(argv[0], installer_path, installer.kind, installer.arguments)
    return apply_launch_options(placeholder, wrapped, env)


def _iter_ci_children(directory: Path):
    try:
        yield from directory.iterdir()
    except OSError:
        return


def resolve_case_insensitive(root: Path, relative: str | Path) -> Path | None:
    """Walk *relative* under *root*, matching each path component case-insensitively."""
    current = root
    for part in Path(relative).parts:
        if part in {os.sep, "/", "."}:
            continue
        if not current.is_dir():
            return None
        match = None
        for child in _iter_ci_children(current):
            if child.name.lower() == part.lower():
                match = child
                break
        if match is None:
            return None
        current = match
    return current


def _user_profile_candidates(drive_c: Path) -> list[Path]:
    users = drive_c / "users"
    if not users.is_dir():
        return []
    return [child for child in _iter_ci_children(users) if child.is_dir()]


def _search_known_roots(drive_c: Path, filename: str) -> Path | None:
    """Look for *filename* under typical install locations without walking all of drive_c."""
    roots = [
        drive_c / "Program Files",
        drive_c / "Program Files (x86)",
        *_user_profile_candidates(drive_c),
    ]
    target = filename.lower()
    for root in roots:
        if not root.exists():
            continue
        try:
            for path in root.rglob("*"):
                if path.is_file() and path.name.lower() == target:
                    return path
        except OSError:
            continue
    return None


def find_prefix_exe(prefix: str | Path, expected: Iterable[str]) -> Path | None:
    """Locate an installed executable under *prefix*/drive_c.

    Tries each expected ``drive_c``-relative path case-insensitively, then
    substitutes every Wine user profile for ``users/<name>/...`` entries,
    then searches Program Files and user profiles for the basename.
    """
    drive_c = Path(prefix) / "drive_c"
    if not drive_c.exists():
        return None

    expected_list = [item for item in expected if item]
    for rel in expected_list:
        found = resolve_case_insensitive(drive_c, rel)
        if found is not None and found.is_file():
            return found
        parts = Path(rel).parts
        if len(parts) >= 2 and parts[0].lower() == "users":
            rest = Path(*parts[2:]) if len(parts) > 2 else Path()
            for profile in _user_profile_candidates(drive_c):
                found = resolve_case_insensitive(profile, rest)
                if found is not None and found.is_file():
                    return found

    seen: set[str] = set()
    for rel in expected_list:
        name = Path(rel).name
        key = name.lower()
        if not name or key in seen:
            continue
        seen.add(key)
        found = _search_known_roots(drive_c, name)
        if found is not None:
            return found
    return None


def download_installer(
    installer: Installer,
    dest_dir: Path | None = None,
    progress_cb: Callable[[float], None] | None = None,
    timeout: int = 60,
) -> Path:
    """Download the vendor installer and return the local file path."""
    dest = Path(dest_dir) if dest_dir is not None else config.downloads_dir()
    dest.mkdir(parents=True, exist_ok=True)
    target = dest / installer.filename
    req = Request(installer.download_url, headers={"User-Agent": USER_AGENT})
    with urlopen(req, timeout=timeout) as resp:  # noqa: S310
        total = int(resp.headers.get("Content-Length", 0) or 0)
        downloaded = 0
        with open(target, "wb") as fh:
            while True:
                chunk = resp.read(1024 * 256)
                if not chunk:
                    break
                fh.write(chunk)
                downloaded += len(chunk)
                if progress_cb and total:
                    progress_cb(min(downloaded / total, 1.0))
    if progress_cb:
        progress_cb(1.0)
    return target


def prepare_prefix(game_id: str) -> Path:
    prefix = config.prefixes_dir() / game_id
    prefix.mkdir(parents=True, exist_ok=True)
    return prefix


def game_from_install(
    installer: Installer,
    exe_path: str | Path,
    prefix: str | Path,
    runner_id: str,
    game_id: str | None = None,
) -> Game:
    """Build the library entry created after a successful easy install."""
    exe = Path(exe_path)
    return Game(
        id=game_id or uuid.uuid4().hex,
        name=installer.name,
        exe_path=str(exe),
        runner=runner_id,
        prefix_path=str(prefix),
        kind="windows",
        category=installer.library_category,
        esync=installer.esync,
        fsync=installer.fsync,
        working_directory=str(exe.parent),
    )


__all__ = [
    "APPS",
    "INSTALLERS",
    "INSTALLER_CATEGORIES",
    "LAUNCHERS",
    "Installer",
    "build_installer_command",
    "download_installer",
    "find_prefix_exe",
    "game_from_install",
    "installer_argv",
    "installer_by_id",
    "installers",
    "prepare_prefix",
    "resolve_case_insensitive",
    "search_installers",
]
