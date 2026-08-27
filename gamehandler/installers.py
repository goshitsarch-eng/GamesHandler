"""First-party easy-installer catalog and Wine prefix helpers.

This module is GTK-free so catalog lookup, download, msiexec argv, and
prefix exe discovery can be unit tested headlessly. Official vendor
download URLs only — no redistributed game binaries.
"""

from __future__ import annotations

import os
import shlex
import shutil
import subprocess
import tempfile
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from . import config
from .models import Game
from .runners import (
    USER_AGENT,
    Runner,
    apply_launch_options,
    prefix_drive_c,
    prefix_drive_cs,
    uses_proton_runtime,
    wine_prefix_root,
)

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
    allowed_hosts: tuple[str, ...]
    publishers: tuple[str, ...]
    arguments: str = ""
    launch_arguments: str = ""
    notes: str = ""
    esync: bool = True
    fsync: bool = True
    library_category: str = LAUNCHERS
    microsoft_trust_root: bool = False


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
        allowed_hosts=("downloader.battle.net",),
        publishers=("Blizzard Entertainment",),
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
        allowed_hosts=(
            "launcher-public-service-prod06.ol.epicgames.com",
            "epicgames-download1.akamaized.net",
        ),
        publishers=("Epic Games Inc.",),
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
        allowed_hosts=("origin-a.akamaihd.net",),
        publishers=("Electronic Arts",),
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
        allowed_hosts=("static3.cdn.ubi.com",),
        publishers=("UBISOFT ENTERTAINMENT",),
        microsoft_trust_root=True,
    ),
    Installer(
        id="gog",
        name="GOG Galaxy",
        description="GOG's DRM-free store client and optional game overlay.",
        category=LAUNCHERS,
        download_url=(
            "https://content-system.gog.com/open_link/download"
            "?path=/open/galaxy/client/setup_galaxy_2.1.8.30.exe"
        ),
        filename="setup_galaxy_2.1.8.30.exe",
        kind="exe",
        expected_exe=(
            "Program Files (x86)/GOG Galaxy/GalaxyClient.exe",
            "Program Files/GOG Galaxy/GalaxyClient.exe",
        ),
        allowed_hosts=("content-system.gog.com", "gog-cdn-fastly.gog.com"),
        publishers=("CN=GOG  sp. z o.o,O=GOG  sp. z o.o",),
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
        allowed_hosts=("download.amazongames.com",),
        publishers=("Amazon.com Services LLC",),
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
        allowed_hosts=("gamedownloads.rockstargames.com",),
        publishers=("Rockstar Games",),
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
        allowed_hosts=("cdn.akamai.steamstatic.com",),
        publishers=("Valve Corp.",),
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
        allowed_hosts=("discord.com", "stable.dl2.discordapp.net"),
        publishers=("Discord Inc.",),
        launch_arguments="--processStart Discord.exe",
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
    # A vendor installer run under raw Wine must not be handed Proton-only
    # variables, for the same reason launch() gates them.
    return apply_launch_options(
        placeholder, wrapped, env, proton_features=uses_proton_runtime(runner, argv)
    )


# Vendor installers are almost all bootstrappers: the downloaded exe unpacks a
# payload, starts the real wizard as a separate process, and exits within
# seconds. Waiting on that first process alone means scanning the prefix while
# the user is still on the wizard's first page — which is why an install that
# plainly succeeded used to end in "could not find the executable".
INSTALL_SETTLE_TIMEOUT = 6 * 60 * 60


def wineserver_binary(runner: Runner) -> str | None:
    """The ``wineserver`` that goes with *runner*, if one can be found."""
    wine = runner.wine_binary()
    if wine:
        sibling = Path(wine).with_name("wineserver")
        if sibling.is_file() and os.access(sibling, os.X_OK):
            return str(sibling)
    return shutil.which("wineserver")


def wait_for_prefix_idle(
    runner: Runner,
    env: dict[str, str],
    timeout: int = INSTALL_SETTLE_TIMEOUT,
) -> bool:
    """Block until nothing is running in the prefix any more.

    ``wineserver -w`` returns once the server owning ``WINEPREFIX`` has no
    processes left, which is the only reliable "the wizard is closed" signal
    when the installer we started has already handed off and exited.
    """
    server = wineserver_binary(runner)
    prefix = env.get("WINEPREFIX")
    if not server or not prefix:
        return False
    # A Proton install put its registry under "pfx"; that is the prefix whose
    # server is worth waiting on.
    waiting_env = dict(env)
    waiting_env["WINEPREFIX"] = wine_prefix_root(prefix)
    try:
        subprocess.run(
            [server, "-w"],
            env=waiting_env,
            check=False,
            timeout=timeout,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return True


# Long enough for a bootstrapper to unpack and start the real wizard.
INSTALL_HANDOFF_SECONDS = 3.0
INSTALL_POLL_SECONDS = 2.0
# Waiting on a wineserver only tells us something if it actually waited.
# Returning at once means there was no server to attach to — which is what
# happens for a Proton install, whose wineserver runs inside umu's container
# where a host wineserver cannot see it.
INSTALL_WAIT_EVIDENCE_SECONDS = 5.0
# Once the prefix is known idle, only the wizard's last writes are still in
# flight. Without that, the wizard itself is what we are waiting on, so the
# window has to cover a person reading and clicking through it.
INSTALL_FLUSH_SECONDS = 20.0
INSTALL_WIZARD_SECONDS = 10 * 60.0


def wait_for_installer(
    runner: Runner,
    env: dict[str, str],
    prefix: str | Path,
    expected: Iterable[str],
    *,
    sleep=time.sleep,
    clock=time.monotonic,
) -> Path | None:
    """Wait out a vendor wizard and return the executable it installed.

    The process GameHandler started is usually just the bootstrapper, so its
    exit says nothing about whether the install finished. Poll for the
    expected executable the whole time: store clients often keep ``wineserver``
    busy after the launcher is already on disk, so waiting for idle first can
    stall for hours. Slice the wineserver wait so a leftover process cannot
    hide a finished install.
    """
    expected = list(expected)
    sleep(INSTALL_HANDOFF_SECONDS)
    started = clock()
    busy_wait = 0.0
    deadline = started + INSTALL_SETTLE_TIMEOUT
    while True:
        found = find_prefix_exe(prefix, expected)
        if found is not None:
            return found
        now = clock()
        if now >= deadline:
            return None
        remaining = deadline - now
        slice_timeout = max(1, int(min(INSTALL_POLL_SECONDS, remaining)))
        wait_started = clock()
        idle = wait_for_prefix_idle(runner, env, timeout=slice_timeout)
        waited = clock() - wait_started
        if idle:
            confirmed = (
                waited >= INSTALL_WAIT_EVIDENCE_SECONDS
                or busy_wait >= INSTALL_WAIT_EVIDENCE_SECONDS
            )
            extra = INSTALL_FLUSH_SECONDS if confirmed else INSTALL_WIZARD_SECONDS
            deadline = min(deadline, clock() + extra)
        else:
            busy_wait += waited
        leftover = INSTALL_POLL_SECONDS - waited
        if leftover > 0:
            now = clock()
            if now < deadline:
                sleep(min(leftover, deadline - now))


def safe_download_name(filename: str, fallback: str = "installer") -> str:
    """Reduce a catalog filename to a bare name before joining it onto a path."""
    candidate = Path(str(filename or "").replace("\\", "/")).name.strip()
    if not candidate or candidate in {".", ".."}:
        return f"{fallback}.exe"
    return candidate


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


# A finished prefix holds tens of thousands of files. Bound the fallback scan so
# a missing executable costs a moment, not a frozen window.
MAX_SCAN_DEPTH = 6
MAX_SCAN_ENTRIES = 40000
_SKIP_DIRS = {"windows", "syswow64", "system32", "winsxs", "temp", "tmp", "cache"}


def _search_known_roots(drive_c: Path, filenames: Iterable[str]) -> Path | None:
    """Find the first of *filenames* under typical install locations.

    Walks each root once for the whole set of names — scanning per name used to
    re-read Program Files from scratch for every candidate — and stops at a
    bounded depth and entry count so an unlucky prefix cannot hang the UI.
    """
    targets = {name.lower() for name in filenames if name}
    if not targets:
        return None
    roots = [
        drive_c / "Program Files",
        drive_c / "Program Files (x86)",
        *_user_profile_candidates(drive_c),
    ]
    budget = MAX_SCAN_ENTRIES
    for root in roots:
        if not root.is_dir():
            continue
        stack: list[tuple[Path, int]] = [(root, 0)]
        while stack and budget > 0:
            current, depth = stack.pop()
            for child in _iter_ci_children(current):
                budget -= 1
                if budget <= 0:
                    break
                try:
                    is_dir = child.is_dir()
                except OSError:
                    continue
                if is_dir:
                    if depth < MAX_SCAN_DEPTH and child.name.lower() not in _SKIP_DIRS:
                        stack.append((child, depth + 1))
                elif child.name.lower() in targets:
                    return child
    return None


def _find_in_drive_c(drive_c: Path, expected_list: list[str]) -> Path | None:
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

    names = [Path(rel).name for rel in expected_list if Path(rel).name]
    return _search_known_roots(drive_c, names)


def find_prefix_exe(prefix: str | Path, expected: Iterable[str]) -> Path | None:
    """Locate an installed executable under *prefix*.

    Tries each expected ``drive_c``-relative path case-insensitively, then
    substitutes every Wine user profile for ``users/<name>/...`` entries,
    then searches Program Files and user profiles for the basename. Both
    prefix layouts are searched: raw Wine puts ``drive_c`` directly under the
    prefix, Proton puts it under ``pfx``.
    """
    expected_list = [item for item in expected if item]
    if not expected_list:
        return None
    for drive_c in prefix_drive_cs(prefix):
        found = _find_in_drive_c(drive_c, expected_list)
        if found is not None:
            return found
    return None


MAX_INSTALLER_BYTES = 1024 * 1024 * 1024
_AUTHENTICODE_ROOT_NAME = "microsoft-identity-verification-root-ca-2020.pem"


def _authenticode_root_path() -> Path:
    override = os.environ.get("GAMEHANDLER_AUTHENTICODE_ROOT", "").strip()
    candidates = (
        Path(override) if override else None,
        Path(__file__).with_name(_AUTHENTICODE_ROOT_NAME),
        Path(__file__).resolve().parent.parent / "data" / _AUTHENTICODE_ROOT_NAME,
        Path("/app/share/gamehandler") / _AUTHENTICODE_ROOT_NAME,
    )
    for candidate in candidates:
        if candidate is not None and candidate.is_file():
            return candidate
    raise RuntimeError("The Microsoft Authenticode trust root is unavailable")


def _validate_download_origin(installer: Installer, final_url: str) -> None:
    parsed = urlparse(final_url)
    host = (parsed.hostname or "").lower()
    allowed = {item.lower() for item in installer.allowed_hosts}
    if parsed.scheme.lower() != "https" or host not in allowed:
        raise RuntimeError(
            f"{installer.name} redirected to an untrusted download origin: {final_url}"
        )


def _validate_installer_magic(installer: Installer, path: Path) -> None:
    with path.open("rb") as stream:
        magic = stream.read(8)
    expected = b"MZ" if installer.kind == "exe" else bytes.fromhex("D0CF11E0A1B11AE1")
    if not magic.startswith(expected):
        raise RuntimeError(f"{installer.name} download is not a valid {installer.kind.upper()} file")


def verify_installer_authenticity(installer: Installer, path: Path) -> None:
    """Verify the Authenticode chain and expected publisher before execution."""
    verifier = shutil.which("osslsigncode")
    if verifier is None:
        raise RuntimeError("osslsigncode is required to verify downloaded installers")
    command = [verifier, "verify", "-in", str(path)]
    if installer.microsoft_trust_root:
        root = str(_authenticode_root_path())
        command[2:2] = ["-CAfile", root, "-TSA-CAfile", root]
    try:
        result = subprocess.run(
            command,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=90,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"Timed out verifying {installer.name}'s signature") from exc
    output = result.stdout or ""
    if result.returncode != 0 or "Signature verification: ok" not in output:
        tail = "\n".join(output.splitlines()[-8:])
        raise RuntimeError(f"{installer.name} has an invalid Authenticode signature\n{tail}")
    folded = output.casefold()
    if not any(publisher.casefold() in folded for publisher in installer.publishers):
        raise RuntimeError(f"{installer.name} is not signed by an approved publisher")


def download_installer(
    installer: Installer,
    dest_dir: Path | None = None,
    progress_cb: Callable[[float], None] | None = None,
    timeout: int = 60,
) -> Path:
    """Atomically download and authenticate a vendor installer."""
    dest = Path(dest_dir) if dest_dir is not None else config.downloads_dir()
    dest.mkdir(parents=True, exist_ok=True)
    # Reduce the catalog filename to a bare name before it reaches the
    # filesystem: it is joined onto a directory and used as a mkstemp prefix,
    # neither of which tolerates a separator.
    filename = safe_download_name(installer.filename, installer.id)
    target = dest / filename
    descriptor, temporary_name = tempfile.mkstemp(
        dir=dest, prefix=f".{filename}.", suffix=".part"
    )
    temporary = Path(temporary_name)
    try:
        req = Request(installer.download_url, headers={"User-Agent": USER_AGENT})
        with os.fdopen(descriptor, "wb") as stream, urlopen(req, timeout=timeout) as resp:  # noqa: S310
            _validate_download_origin(installer, resp.geturl())
            total = int(resp.headers.get("Content-Length", 0) or 0)
            if total > MAX_INSTALLER_BYTES:
                raise RuntimeError(f"{installer.name} installer exceeds the download size limit")
            downloaded = 0
            while True:
                chunk = resp.read(1024 * 256)
                if not chunk:
                    break
                downloaded += len(chunk)
                if downloaded > MAX_INSTALLER_BYTES:
                    raise RuntimeError(f"{installer.name} installer exceeds the download size limit")
                stream.write(chunk)
                if progress_cb and total:
                    progress_cb(min(downloaded / total, 1.0))
        _validate_installer_magic(installer, temporary)
        verify_installer_authenticity(installer, temporary)
        os.replace(temporary, target)
    except Exception:
        try:
            os.close(descriptor)
        except OSError:
            pass
        temporary.unlink(missing_ok=True)
        raise
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
        arguments=installer.launch_arguments,
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
    "INSTALL_WIZARD_SECONDS",
    "LAUNCHERS",
    "Installer",
    "build_installer_command",
    "download_installer",
    "find_prefix_exe",
    "game_from_install",
    "installer_argv",
    "installer_by_id",
    "installers",
    "prefix_drive_c",
    "prepare_prefix",
    "resolve_case_insensitive",
    "safe_download_name",
    "search_installers",
    "verify_installer_authenticity",
    "wait_for_installer",
    "wait_for_prefix_idle",
    "wineserver_binary",
]
