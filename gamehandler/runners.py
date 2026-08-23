"""Wine and Proton runner management.

This module is intentionally free of any GTK dependency so launching and
compatibility-tool logic can be unit tested headlessly.

GameHandler downloads Proton and Wine builds itself from the same upstream
sources used by ProtonPlus (GitHub releases), then lets each game pick one.
"""

from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import subprocess
import tarfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable
from urllib.request import Request, urlopen

from . import config
from .models import Game

SYSTEM_WINE = "wine-system"
METADATA_NAME = ".gamehandler.json"
USER_AGENT = "GameHandler"

# Layouts used by Proton tarballs and Kron4ek Wine-Builds.
_WINE_CANDIDATES = (
    ("files", "bin", "wine"),
    ("dist", "bin", "wine"),
    ("bin", "wine"),
)


@dataclass(frozen=True)
class RunnerFamily:
    """A downloadable Proton or Wine family, matching ProtonPlus coverage."""

    id: str
    name: str
    description: str
    github: str
    kind: str  # proton | wine
    require: tuple[str, ...] = ()
    exclude: tuple[str, ...] = ()
    prefer: tuple[str, ...] = ()
    when_to_use: str = ""

    @property
    def releases_url(self) -> str:
        return f"https://api.github.com/repos/{self.github}/releases"

    @property
    def homepage(self) -> str:
        return f"https://github.com/{self.github}"


# Families ProtonPlus exposes that ship public tarball releases we can install
# into a standalone launcher (wrappers like Luxtorpeda/Boxtron are Steam-only).
RUNNER_FAMILIES: tuple[RunnerFamily, ...] = (
    RunnerFamily(
        id="proton-ge",
        name="Proton-GE",
        description="GloriousEggroll's community Proton with codecs and game fixes.",
        github="GloriousEggroll/proton-ge-custom",
        kind="proton",
        when_to_use=(
            "Start here for most Windows games. GE includes codecs, protonfixes, "
            "and the broadest out-of-the-box compatibility."
        ),
    ),
    RunnerFamily(
        id="proton-ge-rtsp",
        name="Proton-GE RTSP",
        description="Proton build with RTSP and media playback patches (VRChat).",
        github="SpookySkeletons/proton-rtsp",
        kind="proton",
        when_to_use=(
            "Use for VRChat or titles that play in-game video over RTSP. "
            "Not a general replacement for Proton-GE."
        ),
    ),
    RunnerFamily(
        id="proton-cachyos",
        name="Proton-CachyOS",
        description="CachyOS Proton with extra performance and Wayland work.",
        github="CachyOS/proton-cachyos",
        kind="proton",
        prefer=("slr", "x86_64"),
        exclude=("v3", "znver4", "native"),
        when_to_use=(
            "A performance-oriented Proton. Try it when a game already runs "
            "but you want a bit more speed on recent hardware."
        ),
    ),
    RunnerFamily(
        id="proton-em",
        name="Proton-EM",
        description="Etaash Proton with Wine Wayland, HDR, and FSR additions.",
        github="Etaash-mathamsetty/Proton",
        kind="proton",
        when_to_use=(
            "Pick this for native Wine Wayland, HDR, or FSR extras. Needs a "
            "recent GPU stack; keep Proton-GE as the fallback."
        ),
    ),
    RunnerFamily(
        id="wine-vanilla",
        name="Wine-Vanilla",
        description="Kron4ek upstream Wine, without staging patches.",
        github="Kron4ek/Wine-Builds",
        kind="wine",
        require=("amd64",),
        exclude=("staging", "tkg", "proton", "wow64"),
        when_to_use=(
            "Use for older games or Windows apps that behave better on plain "
            "Wine than on Proton. No Steam runtime bundled."
        ),
    ),
    RunnerFamily(
        id="wine-staging",
        name="Wine-Staging",
        description="Kron4ek Wine with the Staging patchset.",
        github="Kron4ek/Wine-Builds",
        kind="wine",
        require=("staging", "amd64"),
        exclude=("tkg", "wow64"),
        when_to_use=(
            "Newer Wine features that have not landed upstream yet. Try this "
            "when Vanilla is too old for a specific title."
        ),
    ),
    RunnerFamily(
        id="wine-staging-tkg",
        name="Wine-Staging-Tkg",
        description="Kron4ek Wine Staging plus TkG gaming patches.",
        github="Kron4ek/Wine-Builds",
        kind="wine",
        require=("staging-tkg", "amd64"),
        exclude=("wow64",),
        when_to_use=(
            "Wine with extra gaming patches, without a full Proton tree. "
            "Good when you want Wine, not Steam's Proton layout."
        ),
    ),
    RunnerFamily(
        id="wine-proton",
        name="Wine-Proton",
        description="Kron4ek Wine built from Proton's Wine tree.",
        github="Kron4ek/Wine-Builds",
        kind="wine",
        require=("proton", "amd64"),
        exclude=("staging", "wow64"),
        when_to_use=(
            "Proton's Wine packaged as standalone Wine. A middle ground if "
            "full Proton is heavier than you need."
        ),
    ),
)

SYSTEM_WINE_GUIDE = (
    "The Wine already installed on this system. Fine for simple apps and "
    "older games. For modern titles, download Proton-GE from the list below."
)

_FAMILIES = {family.id: family for family in RUNNER_FAMILIES}


def family_by_id(family_id: str) -> RunnerFamily:
    try:
        return _FAMILIES[family_id]
    except KeyError as exc:
        raise KeyError(f"Unknown runner family: {family_id}") from exc


def families() -> tuple[RunnerFamily, ...]:
    return RUNNER_FAMILIES


def runner_guides() -> list[tuple[str, str, str]]:
    """``(title, kind, advice)`` rows for the Runners guide."""
    rows = [("System Wine", "wine", SYSTEM_WINE_GUIDE)]
    for family in RUNNER_FAMILIES:
        rows.append((family.name, family.kind, family.when_to_use))
    return rows


def _looks_like_archive(name: str) -> bool:
    lowered = name.lower()
    return lowered.endswith((".tar.gz", ".tar.xz", ".tgz", ".tar.bz2"))


def asset_matches(asset_name: str, family: RunnerFamily) -> bool:
    """Return whether a GitHub asset belongs to *family*."""
    if not _looks_like_archive(asset_name):
        return False
    lowered = asset_name.lower()
    for token in family.exclude:
        if token.lower() in lowered:
            return False
    for token in family.require:
        if token.lower() not in lowered:
            return False
    return True


def pick_asset(assets: Iterable[dict], family: RunnerFamily) -> dict | None:
    """Choose the best archive asset for *family* from a GitHub release."""
    matches = [
        asset
        for asset in assets
        if asset_matches(str(asset.get("name") or ""), family)
    ]
    if not matches:
        return None
    if family.prefer:
        preferred = [
            asset
            for asset in matches
            if all(
                token.lower() in str(asset.get("name") or "").lower()
                for token in family.prefer
            )
        ]
        if preferred:
            matches = preferred
    return matches[0]


def find_wine_binary(root: Path) -> Path | None:
    """Locate the Wine binary inside an extracted Proton or Wine build."""
    for parts in _WINE_CANDIDATES:
        candidate = root.joinpath(*parts)
        if candidate.exists():
            return candidate
    return None


@dataclass
class ReleaseInfo:
    """A downloadable Proton or Wine build published on GitHub."""

    tag: str
    name: str
    download_url: str
    size: int
    family_id: str = "proton-ge"

    @property
    def size_mb(self) -> float:
        return self.size / (1024 * 1024)

    @property
    def install_id(self) -> str:
        """Stable directory name used under the runners folder."""
        if self.family_id == "proton-ge":
            return self.tag
        safe_tag = self.tag.replace("/", "-")
        return f"{self.family_id}-{safe_tag}"

    @property
    def family(self) -> RunnerFamily:
        return family_by_id(self.family_id)


class Runner:
    """Base class for anything that can launch a Windows executable."""

    id: str = ""
    name: str = ""
    family_id: str = ""

    def is_available(self) -> bool:  # pragma: no cover - trivial
        raise NotImplementedError

    def version(self) -> str:  # pragma: no cover - trivial
        raise NotImplementedError

    def wine_binary(self) -> str | None:
        raise NotImplementedError

    def build_command(self, game: Game) -> tuple[list[str], dict[str, str]]:
        """Return the ``(argv, environment)`` used to launch *game*."""
        wine = self.wine_binary()
        if not wine:
            raise RuntimeError(f"Runner '{self.name}' is not available")

        prefix = game.prefix_path or str(config.prefixes_dir() / game.id)
        env = dict(os.environ)
        env["WINEPREFIX"] = prefix
        # Keep first-run quiet and non-interactive for Windows games.
        env.setdefault("WINEDLLOVERRIDES", "winemenubuilder.exe=d;mscoree,mshtml=")
        env.setdefault("WINEDEBUG", "-all")

        argv = [wine]
        if game.exe_path:
            argv.append(game.exe_path)
        if game.arguments:
            argv.extend(shlex.split(game.arguments))
        return argv, env


class WineRunner(Runner):
    """The Wine installation provided by the host system."""

    id = SYSTEM_WINE
    name = "System Wine"
    family_id = "system"

    _AUTODETECT = object()

    def __init__(self, binary=_AUTODETECT) -> None:
        if binary is WineRunner._AUTODETECT:
            self._binary = shutil.which("wine")
        else:
            self._binary = binary

    def wine_binary(self) -> str | None:
        return self._binary

    def is_available(self) -> bool:
        return bool(self._binary)

    def version(self) -> str:
        if not self._binary:
            return "not installed"
        try:
            out = subprocess.run(
                [self._binary, "--version"],
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            return out.stdout.strip() or out.stderr.strip() or "unknown"
        except (OSError, subprocess.SubprocessError):
            return "unknown"


class ProtonRunner(Runner):
    """A downloaded Proton or Wine build extracted under the runners directory."""

    def __init__(self, path: Path, family_id: str = "", display_name: str = "") -> None:
        self.path = Path(path)
        self.id = self.path.name
        self.family_id = family_id or _read_family_id(self.path)
        self.name = display_name or self.path.name

    def wine_binary(self) -> str | None:
        found = find_wine_binary(self.path)
        return str(found) if found else None

    def proton_script(self) -> Path | None:
        candidate = self.path / "proton"
        return candidate if candidate.exists() else None

    def is_available(self) -> bool:
        return self.wine_binary() is not None or self.proton_script() is not None

    def version(self) -> str:
        return self.path.name

    def family_label(self) -> str:
        if self.family_id and self.family_id in _FAMILIES:
            return family_by_id(self.family_id).name
        if self.family_id == "system":
            return "System Wine"
        return "Downloaded runner"

    def build_command(self, game: Game) -> tuple[list[str], dict[str, str]]:
        prefix = game.prefix_path or str(config.prefixes_dir() / game.id)
        env = dict(os.environ)
        env["WINEPREFIX"] = prefix
        env.setdefault("WINEDLLOVERRIDES", "winemenubuilder.exe=d;mscoree,mshtml=")
        env.setdefault("WINEDEBUG", "-all")

        umu = shutil.which("umu-run")
        proton = self.proton_script()
        if umu and proton is not None:
            env["PROTONPATH"] = str(self.path)
            env["GAMEID"] = f"gh-{game.id[:8]}"
            env["STORE"] = "none"
            argv = [umu]
            if game.exe_path:
                argv.append(game.exe_path)
            if game.arguments:
                argv.extend(shlex.split(game.arguments))
            return argv, env

        wine = self.wine_binary()
        if not wine:
            raise RuntimeError(f"Runner '{self.name}' is not available")
        argv = [wine]
        if game.exe_path:
            argv.append(game.exe_path)
        if game.arguments:
            argv.extend(shlex.split(game.arguments))
        return argv, env


def _read_family_id(root: Path) -> str:
    meta = root / METADATA_NAME
    if not meta.exists():
        return ""
    try:
        data = json.loads(meta.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return ""
    return str(data.get("family") or "")


def _write_metadata(root: Path, release: ReleaseInfo) -> None:
    payload = {
        "family": release.family_id,
        "tag": release.tag,
        "asset": release.name,
        "source": release.family.github,
    }
    (root / METADATA_NAME).write_text(json.dumps(payload, indent=2), encoding="utf-8")


class RunnerManager:
    """Discovers the available runners (system Wine + installed builds)."""

    def __init__(self, runners_directory: Path | None = None) -> None:
        self.runners_directory = (
            Path(runners_directory)
            if runners_directory is not None
            else config.runners_dir()
        )

    def system_wine(self) -> WineRunner:
        return WineRunner()

    def installed_protons(self) -> list[ProtonRunner]:
        if not self.runners_directory.exists():
            return []
        protons = []
        for child in sorted(self.runners_directory.iterdir()):
            if not child.is_dir():
                continue
            runner = ProtonRunner(child)
            if runner.is_available():
                protons.append(runner)
        return protons

    def all_runners(self) -> list[Runner]:
        return [self.system_wine(), *self.installed_protons()]

    def get(self, runner_id: str) -> Runner:
        if runner_id == SYSTEM_WINE or not runner_id:
            return self.system_wine()
        candidate = self.runners_directory / runner_id
        if candidate.exists():
            return ProtonRunner(candidate)
        # Fall back to system wine so a missing Proton build never blocks launch.
        return self.system_wine()

    def label(self, runner_id: str) -> str:
        if runner_id == SYSTEM_WINE or not runner_id:
            return "System Wine"
        runner = self.get(runner_id)
        if isinstance(runner, ProtonRunner):
            family = runner.family_label()
            if family != runner.name:
                return f"{runner.name} · {family}"
        return runner.name

    def choices(self) -> list[tuple[str, str]]:
        """``(id, label)`` pairs suitable for a dropdown."""
        items = [(SYSTEM_WINE, "System Wine")]
        for proton in self.installed_protons():
            items.append((proton.id, proton.name))
        return items


class ProtonManager:
    """Lists, installs, and removes Proton/Wine builds from GitHub."""

    def __init__(self, runners_directory: Path | None = None) -> None:
        self.runners_directory = (
            Path(runners_directory)
            if runners_directory is not None
            else config.runners_dir()
        )

    @staticmethod
    def parse_releases(
        data: list[dict], family: RunnerFamily | str | None = None
    ) -> list[ReleaseInfo]:
        """Turn a GitHub releases payload into :class:`ReleaseInfo` objects."""
        if isinstance(family, str):
            resolved = family_by_id(family)
        elif family is None:
            resolved = family_by_id("proton-ge")
        else:
            resolved = family

        releases: list[ReleaseInfo] = []
        for release in data:
            tag = release.get("tag_name") or release.get("name") or ""
            asset = pick_asset(release.get("assets") or [], resolved)
            if not asset:
                continue
            releases.append(
                ReleaseInfo(
                    tag=tag,
                    name=str(asset.get("name") or ""),
                    download_url=str(asset.get("browser_download_url") or ""),
                    size=int(asset.get("size", 0) or 0),
                    family_id=resolved.id,
                )
            )
        return releases

    def fetch_available(
        self,
        limit: int = 15,
        timeout: int = 30,
        family: RunnerFamily | str | None = None,
    ) -> list[ReleaseInfo]:
        resolved = (
            family_by_id(family)
            if isinstance(family, str)
            else family or family_by_id("proton-ge")
        )
        req = Request(
            resolved.releases_url,
            headers={"Accept": "application/vnd.github+json", "User-Agent": USER_AGENT},
        )
        with urlopen(req, timeout=timeout) as resp:  # noqa: S310
            data = json.loads(resp.read().decode("utf-8"))
        if not isinstance(data, list):
            raise RuntimeError("Unexpected GitHub releases response")
        return self.parse_releases(data, resolved)[:limit]

    def is_installed(self, tag: str, family_id: str | None = None) -> bool:
        if family_id:
            target = self.runners_directory / ReleaseInfo(
                tag=tag, name="", download_url="", size=0, family_id=family_id
            ).install_id
        else:
            target = self.runners_directory / tag
        return find_wine_binary(target) is not None or (target / "proton").exists()

    def is_release_installed(self, release: ReleaseInfo) -> bool:
        return self.is_installed(release.tag, release.family_id)

    def install(
        self,
        release: ReleaseInfo,
        progress_cb: Callable[[float], None] | None = None,
        timeout: int = 60,
    ) -> Path:
        """Download and extract a build, returning its install directory."""
        self.runners_directory.mkdir(parents=True, exist_ok=True)
        archive = self.runners_directory / release.name
        req = Request(release.download_url, headers={"User-Agent": USER_AGENT})
        with urlopen(req, timeout=timeout) as resp:  # noqa: S310
            total = int(resp.headers.get("Content-Length", release.size) or 0)
            downloaded = 0
            with open(archive, "wb") as fh:
                while True:
                    chunk = resp.read(1024 * 256)
                    if not chunk:
                        break
                    fh.write(chunk)
                    downloaded += len(chunk)
                    if progress_cb and total:
                        progress_cb(downloaded / total)

        before = {p.name for p in self.runners_directory.iterdir() if p.is_dir()}
        with tarfile.open(archive) as tar:
            tar.extractall(self.runners_directory)
        archive.unlink(missing_ok=True)

        extracted = self._resolve_extracted(release, before)
        target = self.runners_directory / release.install_id
        if extracted.resolve() != target.resolve():
            if target.exists():
                shutil.rmtree(target)
            extracted.rename(target)
        _write_metadata(target, release)
        return target

    def uninstall(self, runner_id: str) -> None:
        if runner_id in {"", SYSTEM_WINE}:
            raise ValueError("System Wine cannot be uninstalled")
        target = self.runners_directory / runner_id
        if target.exists():
            shutil.rmtree(target)

    def _resolve_extracted(self, release: ReleaseInfo, before: set[str]) -> Path:
        after = {p.name for p in self.runners_directory.iterdir() if p.is_dir()}
        created = [name for name in sorted(after - before) if name != release.install_id]
        tagged = self.runners_directory / release.tag
        if tagged.is_dir():
            return tagged
        if len(created) == 1:
            return self.runners_directory / created[0]
        for name in created:
            candidate = self.runners_directory / name
            if find_wine_binary(candidate) or (candidate / "proton").exists():
                return candidate
        raise RuntimeError(f"Could not locate extracted files for {release.tag}")


_DESKTOP_SIZE_RE = re.compile(r"^\d{2,5}x\d{2,5}$")
_ANTICHEAT_DIR_NAMES = {
    "battleye": ("battleye_runtime", "BattlEye_Runtime", "proton-battleye-runtime"),
    "eac": ("easyanticheat_runtime", "eac_runtime", "EasyAntiCheatRuntime", "proton-eac-runtime"),
}


def parse_env_block(text: str) -> dict[str, str]:
    """Parse ``KEY=value`` pairs from a free-form environment block."""
    result: dict[str, str] = {}
    if not text:
        return result
    for raw in text.replace("\r", "\n").replace(";", "\n").split("\n"):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        items = (
            parts
            if len(parts) > 1 and all("=" in part and not part.startswith("=") for part in parts)
            else [line]
        )
        for item in items:
            if "=" not in item:
                continue
            key, _, value = item.partition("=")
            key = key.strip()
            if key:
                result[key] = value.strip().strip('"').strip("'")
    return result


def merge_dll_overrides(env: dict[str, str], extra: str) -> None:
    """Append a WINEDLLOVERRIDES fragment onto *env*."""
    extra = extra.strip().strip(";")
    if not extra:
        return
    current = env.get("WINEDLLOVERRIDES", "").strip()
    if not current:
        env["WINEDLLOVERRIDES"] = extra
        return
    env["WINEDLLOVERRIDES"] = current.rstrip(";") + ";" + extra


def normalize_desktop_size(value: str) -> str:
    size = (value or "").strip().lower().replace(" ", "")
    if _DESKTOP_SIZE_RE.match(size):
        return size
    return "1920x1080"


def virtual_desktop_argv(argv: list[str], game: Game) -> list[str]:
    """Insert Wine's ``explorer /desktop=Name,WxH`` wrapper after the launcher."""
    if not argv:
        return argv
    slug = re.sub(r"[^A-Za-z0-9]+", "", game.name)[:16] or "Game"
    size = normalize_desktop_size(game.virtual_desktop_size)
    return [argv[0], "explorer", f"/desktop={slug},{size}", *argv[1:]]


def _runtime_dir_ok(path: Path) -> bool:
    if not path.is_dir():
        return False
    try:
        return any(path.iterdir())
    except OSError:
        return False


def find_anticheat_runtime(kind: str, extra_roots: Iterable[Path] | None = None) -> str:
    """Locate a BattlEye or Easy Anti-Cheat Proton runtime directory."""
    names = _ANTICHEAT_DIR_NAMES.get(kind)
    if not names:
        return ""
    roots = [
        Path.home() / ".local/share/umu",
        Path.home() / ".local/share/lutris/runtime",
        Path.home() / ".local/share/Steam/steamapps/common",
        Path("/usr/share/umu"),
        Path("/usr/share/steam/compatibilitytools.d"),
        config.runners_dir(),
    ]
    if extra_roots:
        roots = [*list(extra_roots), *roots]
    for root in roots:
        try:
            exists = root.exists()
        except OSError:
            continue
        if not exists:
            continue
        for name in names:
            candidate = root / name
            if _runtime_dir_ok(candidate):
                return str(candidate)
        try:
            children = list(root.iterdir())
        except OSError:
            continue
        for child in children:
            if not child.is_dir():
                continue
            for name in names:
                candidate = child / name
                if _runtime_dir_ok(candidate):
                    return str(candidate)
                nested = child / "files" / "share" / name
                if _runtime_dir_ok(nested):
                    return str(nested)
    return ""


def apply_launch_options(
    game: Game, argv: list[str], env: dict[str, str]
) -> tuple[list[str], dict[str, str]]:
    """Apply Lutris/Faugus-style launch helpers and compatibility toggles."""
    env = dict(env)
    wrapped = list(argv)

    if game.prefer_sdl:
        env["PROTON_ENABLE_HIDAPI"] = "1"
        env["SDL_JOYSTICK_HIDAPI"] = "1"
        env["PROTON_NO_HIDRAW"] = "1"
    if game.wayland:
        env["PROTON_ENABLE_WAYLAND"] = "1"
        env["DISPLAY"] = ""
    if game.hdr:
        env["PROTON_ENABLE_HDR"] = "1"
        env["DXVK_HDR"] = "1"
    if not game.is_linux:
        if game.esync:
            env["WINEESYNC"] = "1"
        else:
            env["WINEESYNC"] = "0"
            env["PROTON_NO_ESYNC"] = "1"
        if game.fsync:
            env["WINEFSYNC"] = "1"
        else:
            env["WINEFSYNC"] = "0"
            env["PROTON_NO_FSYNC"] = "1"
        if not game.dxvk:
            env["PROTON_USE_WINED3D"] = "1"
        if not game.vkd3d:
            merge_dll_overrides(env, "d3d12,d3d12core=b")
        if game.nvapi:
            env["PROTON_ENABLE_NVAPI"] = "1"
            env["DXVK_ENABLE_NVAPI"] = "1"
            env["DXVK_NVAPIHACK"] = "0"
        if game.fsr:
            env["WINE_FULLSCREEN_FSR"] = "1"
            env.setdefault("WINE_FULLSCREEN_FSR_STRENGTH", "2")
        if game.battleye:
            runtime = find_anticheat_runtime("battleye")
            if runtime:
                env["PROTON_BATTLEYE_RUNTIME"] = runtime
        else:
            env["PROTON_BATTLEYE_RUNTIME"] = ""
        if game.eac:
            runtime = find_anticheat_runtime("eac")
            if runtime:
                env["PROTON_EAC_RUNTIME"] = runtime
        else:
            env["PROTON_EAC_RUNTIME"] = ""
        if game.virtual_desktop:
            wrapped = virtual_desktop_argv(wrapped, game)

    if game.mangohud:
        mangohud = shutil.which("mangohud")
        if mangohud:
            wrapped = [mangohud, *wrapped]
        else:
            env["MANGOHUD"] = "1"
    if game.gamemode:
        gamemode = shutil.which("gamemoderun")
        if gamemode:
            wrapped = [gamemode, *wrapped]
    if game.gamescope:
        gamescope = shutil.which("gamescope")
        if gamescope:
            gs = [gamescope]
            if game.hdr:
                gs.append("--hdr-enabled")
            gs.append("--")
            wrapped = [*gs, *wrapped]

    for key, value in parse_env_block(game.environment).items():
        env[key] = value

    return wrapped, env


def build_linux_command(game: Game) -> tuple[list[str], dict[str, str]]:
    if not game.exe_path:
        raise RuntimeError("No executable is configured")
    argv = [game.exe_path]
    if game.arguments:
        argv.extend(shlex.split(game.arguments))
    return argv, dict(os.environ)


def launch(game: Game, manager: RunnerManager | None = None):
    """Launch *game* with its configured runner. Returns the ``Popen`` handle."""
    manager = manager or RunnerManager()
    if game.is_linux:
        argv, env = build_linux_command(game)
        runner_executable = ""
    else:
        runner = manager.get(game.runner)
        argv, env = runner.build_command(game)
        runner_executable = argv[0] if argv else ""
        prefix = env.get("WINEPREFIX")
        if prefix:
            Path(prefix).mkdir(parents=True, exist_ok=True)

    argv, env = apply_launch_options(game, argv, env)

    extra = game.additional_app.strip()
    if extra:
        extra_argv = [runner_executable, extra] if runner_executable else [extra]
        subprocess.Popen(extra_argv, env=env)

    cwd = game.working_directory or None
    if not cwd and game.exe_path and Path(game.exe_path).exists():
        cwd = str(Path(game.exe_path).parent)
    return subprocess.Popen(argv, env=env, cwd=cwd)


def create_desktop_shortcut(game: Game, command: str, directory: Path | None = None) -> Path:
    """Write a ``.desktop`` launcher for *game* and return its path."""
    apps = directory or (Path.home() / ".local" / "share" / "applications")
    apps.mkdir(parents=True, exist_ok=True)
    slug = "".join(ch if ch.isalnum() else "-" for ch in game.name.lower()).strip("-")
    path = apps / f"gamehandler-{game.id[:8]}-{slug or 'game'}.desktop"
    icon = game.cover_path or "applications-games"
    body = "\n".join(
        [
            "[Desktop Entry]",
            "Type=Application",
            f"Name={game.name}",
            f"Comment=Launch {game.name} with GameHandler",
            f"Exec={command}",
            f"Icon={icon}",
            "Terminal=false",
            "Categories=Game;",
            "",
        ]
    )
    path.write_text(body, encoding="utf-8")
    path.chmod(path.stat().st_mode | 0o111)
    return path


def tool_command(game: Game, manager: RunnerManager, tool: str) -> tuple[list[str], dict[str, str]]:
    """Build a command that opens winecfg or winetricks in the game prefix."""
    runner = manager.get(game.runner)
    wine = runner.wine_binary()
    if not wine:
        raise RuntimeError(f"Runner '{runner.name}' is not available")
    prefix = game.prefix_path or str(config.prefixes_dir() / game.id)
    env = dict(os.environ)
    env["WINEPREFIX"] = prefix
    Path(prefix).mkdir(parents=True, exist_ok=True)
    if tool == "winecfg":
        return [wine, "winecfg"], env
    if tool == "winetricks":
        winetricks = shutil.which("winetricks")
        if not winetricks:
            raise RuntimeError("winetricks is not installed")
        return [winetricks], env
    raise ValueError(f"Unknown tool: {tool}")


__all__ = [
    "SYSTEM_WINE",
    "METADATA_NAME",
    "RUNNER_FAMILIES",
    "ReleaseInfo",
    "RunnerFamily",
    "Runner",
    "WineRunner",
    "ProtonRunner",
    "RunnerManager",
    "ProtonManager",
    "asset_matches",
    "pick_asset",
    "family_by_id",
    "families",
    "runner_guides",
    "SYSTEM_WINE_GUIDE",
    "find_wine_binary",
    "parse_env_block",
    "merge_dll_overrides",
    "normalize_desktop_size",
    "virtual_desktop_argv",
    "find_anticheat_runtime",
    "apply_launch_options",
    "build_linux_command",
    "create_desktop_shortcut",
    "tool_command",
    "launch",
]
