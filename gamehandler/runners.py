"""Wine and Proton runner management.

This module is intentionally free of any UI dependency so launching and
compatibility-tool logic can be unit tested headlessly.

GameHandler downloads Proton and Wine builds itself from the same upstream
sources used by ProtonPlus (GitHub releases), then lets each game pick one.
"""

from __future__ import annotations

import bz2
import ctypes
import errno
import gzip
import hashlib
import io
import json
import lzma
import os
import posixpath
import re
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import threading
from contextlib import contextmanager
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Callable, Iterable
from urllib.request import Request, urlopen

from . import config
from .models import Game
from .netpaths import as_local_path, is_remote_url, unreachable_share_message

SYSTEM_WINE = "wine-system"
METADATA_NAME = ".gamehandler.json"
USER_AGENT = "GameHandler"
# umu-run hands WINEPREFIX to Proton as STEAM_COMPAT_DATA_PATH, and Proton
# builds its Wine prefix in a "pfx" subdirectory of that. So a title installed
# through a Proton runner lives in <prefix>/pfx/drive_c, while the same
# directory used by raw Wine holds drive_c directly.
PROTON_PREFIX_SUBDIR = "pfx"
DXVK_VERSION = "3.0.2"
DXVK_ROOT = Path("/app/share/gamehandler/dxvk")

# Runner releases are large, but all supported upstream archives fit well below
# 2 GiB compressed. The expanded ceiling leaves room for Proton's runtime while
# preventing small compressed tar bombs from consuming the whole disk.
MAX_RUNNER_ARCHIVE_BYTES = 2 * 1024 * 1024 * 1024
MAX_ARCHIVE_UNCOMPRESSED_BYTES = 20 * 1024 * 1024 * 1024
# Includes tar headers, PAX/GNU metadata, padding, and file payloads.
MAX_ARCHIVE_DECOMPRESSED_BYTES = 24 * 1024 * 1024 * 1024
MAX_ARCHIVE_MEMBERS = 250_000

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
    maintainer: str = ""
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
        maintainer="GloriousEggroll",
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
        maintainer="SpookySkeletons",
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
        maintainer="The CachyOS project",
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
        maintainer="Etaash Mathamsetty",
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
        maintainer="Kron4ek",
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
        maintainer="Kron4ek",
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
        maintainer="Kron4ek",
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
        maintainer="Kron4ek",
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


@dataclass(frozen=True)
class RunnerGuide:
    """A "which runner should I use?" row, with credit to its maintainer."""

    title: str
    kind: str
    advice: str
    maintainer: str = ""
    homepage: str = ""


def runner_guide_details() -> list[RunnerGuide]:
    """Guide rows including who maintains each build and where it lives."""
    rows = [
        RunnerGuide(
            title="System Wine",
            kind="wine",
            advice=SYSTEM_WINE_GUIDE,
            maintainer="WineHQ",
            homepage="https://www.winehq.org",
        )
    ]
    for family in RUNNER_FAMILIES:
        rows.append(
            RunnerGuide(
                title=family.name,
                kind=family.kind,
                advice=family.when_to_use,
                maintainer=family.maintainer,
                homepage=family.homepage,
            )
        )
    return rows


def runner_guides() -> list[tuple[str, str, str]]:
    """``(title, kind, advice)`` rows for the Runners guide."""
    return [(row.title, row.kind, row.advice) for row in runner_guide_details()]


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
        safe_tag = sanitise_release_tag(self.tag)
        if self.family_id == "proton-ge":
            return safe_tag
        safe_family = sanitise_release_tag(self.family_id)
        combined = f"{safe_family}~f{safe_tag}"
        if len(combined) <= 180:
            return safe_install_id(combined)
        identity = f"{len(self.family_id)}:{self.family_id}{self.tag}"
        digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:12]
        return safe_install_id(f"{combined[:166]}~i{digest}")

    @property
    def family(self) -> RunnerFamily:
        return family_by_id(self.family_id)


def prefix_drive_cs(prefix: str | Path) -> list[Path]:
    """Every ``drive_c`` that exists under *prefix*, Proton's layout first."""
    root = Path(prefix)
    candidates = (root / PROTON_PREFIX_SUBDIR / "drive_c", root / "drive_c")
    return [candidate for candidate in candidates if candidate.is_dir()]


def prefix_drive_c(prefix: str | Path) -> Path | None:
    """The ``drive_c`` a prefix actually installed into, if it has one."""
    found = prefix_drive_cs(prefix)
    return found[0] if found else None


def wine_prefix_root(prefix: str | Path) -> str:
    """Where ``WINEPREFIX`` must point for raw Wine to see *prefix*'s files.

    A prefix that Proton created keeps its registry and drive_c one level
    down. Pointing plain Wine at the parent makes it build a second, empty
    prefix beside the real one, which looks exactly like a game that installed
    fine and then refuses to start.
    """
    root = Path(prefix)
    if (root / PROTON_PREFIX_SUBDIR / "drive_c").is_dir():
        return str(root / PROTON_PREFIX_SUBDIR)
    return str(root)


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
        # Raw Wine has no Proton indirection, so it needs the directory that
        # actually holds drive_c — including when Proton created it.
        env["WINEPREFIX"] = wine_prefix_root(prefix)
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
            # Proton appends "pfx" itself, so umu keeps the parent directory.
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
        # Without umu this build runs as plain Wine, which cannot see through
        # Proton's "pfx" indirection on its own.
        env["WINEPREFIX"] = wine_prefix_root(prefix)
        argv = [wine]
        if game.exe_path:
            argv.append(game.exe_path)
        if game.arguments:
            argv.extend(shlex.split(game.arguments))
        return argv, env


def safe_archive_name(name: str, fallback: str = "runner.tar.gz") -> str:
    """Reduce a remote asset name to a bare, safe filename.

    GitHub asset names are attacker-controllable if an upstream account is
    compromised, so never join them onto a directory unfiltered.
    """
    candidate = Path(str(name or "").replace("\\", "/")).name.strip()
    if not candidate or candidate in {".", ".."} or "/" in candidate:
        return fallback
    return candidate


def safe_install_id(install_id: str) -> str:
    """Reject runner directory names that could escape the runners directory."""
    candidate = str(install_id or "").strip()
    if not candidate or candidate in {".", ".."}:
        raise ValueError(f"Unsafe runner id: {install_id!r}")
    if "/" in candidate or "\\" in candidate or candidate.startswith("."):
        raise ValueError(f"Unsafe runner id: {install_id!r}")
    return candidate


def sanitise_release_tag(tag: str) -> str:
    """Turn an untrusted release tag into one safe, collision-resistant component.

    Common upstream tags remain unchanged. If filtering or truncation changes a
    tag, bind the resulting component to the complete raw value with a short
    hash so distinct releases cannot alias the same install directory.
    """
    raw = str(tag or "").strip()
    candidate = re.sub(r"[^A-Za-z0-9._+-]+", "-", raw)
    candidate = candidate.strip(" ._-")
    if not candidate:
        raise ValueError(f"Unsafe runner tag: {tag!r}")
    if candidate == raw and len(candidate) <= 180:
        return safe_install_id(candidate)
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]
    prefix = candidate[:166].rstrip(" ._-") or "runner"
    # ``~`` is outside the accepted raw-tag alphabet, so transformed IDs live
    # in a namespace that an unchanged raw tag can never occupy.
    return safe_install_id(f"{prefix}~h{digest}")


class _BoundedReader(io.RawIOBase):
    """Count every decompressed tar byte before tarfile parses metadata."""

    def __init__(self, source, limit: int):
        self.source = source
        self.limit = limit
        self.consumed = 0

    def read(self, size: int = -1) -> bytes:
        remaining = self.limit - self.consumed
        request = remaining + 1 if size < 0 or size > remaining + 1 else size
        chunk = self.source.read(request)
        self.consumed += len(chunk)
        if self.consumed > self.limit:
            raise RuntimeError("Runner archive exceeds the decompressed stream limit")
        return chunk

    def readable(self) -> bool:
        return True


@contextmanager
def _bounded_decompressed_stream(archive: Path):
    """Yield a size-limited uncompressed tar stream for supported formats."""

    raw = archive.open("rb")
    stream = raw
    try:
        magic = raw.read(6)
        raw.seek(0)
        if magic.startswith(b"\x1f\x8b"):
            stream = gzip.GzipFile(fileobj=raw, mode="rb")
        elif magic.startswith(b"\xfd7zXZ\x00"):
            stream = lzma.LZMAFile(raw, mode="rb")
        elif magic.startswith(b"BZh"):
            stream = bz2.BZ2File(raw, mode="rb")
        yield _BoundedReader(stream, MAX_ARCHIVE_DECOMPRESSED_BYTES)
    finally:
        if stream is not raw:
            stream.close()
        raw.close()


def extract_archive(archive: Path, destination: Path) -> None:
    """Safely stream a bounded Proton/Wine tarball into private staging.

    The caller always supplies a fresh staging directory that is discarded on
    failure, so members can be validated and extracted in one bounded pass. The
    decompressed-byte limit includes PAX/GNU metadata before tarfile parses it.
    """
    data_filter = getattr(tarfile, "data_filter", None)
    if data_filter is None:  # pragma: no cover - supported production Pythons
        raise RuntimeError("Secure tar extraction requires Python tarfile.data_filter")

    destination.mkdir(parents=True, exist_ok=True)
    member_count = 0
    payload_bytes = 0
    with _bounded_decompressed_stream(archive) as stream:
        with tarfile.open(fileobj=stream, mode="r|") as tar:
            for member in tar:
                member_count += 1
                if member_count > MAX_ARCHIVE_MEMBERS:
                    raise RuntimeError("Runner archive contains too many members")
                if member.size < 0:
                    raise RuntimeError(
                        f"Runner archive has an invalid member size: {member.name}"
                    )
                payload_bytes += member.size
                if payload_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES:
                    raise RuntimeError(
                        "Runner archive exceeds the uncompressed payload size limit"
                    )
                data_filter(member, str(destination))
                tar.extract(member, destination, filter="data")


def _validate_staged_runner(candidate: Path, extraction_root: Path) -> None:
    """Ensure a selected staged tree is self-contained and still runnable."""
    root = extraction_root.resolve()
    resolved = candidate.resolve()
    if candidate.is_symlink() or (resolved != root and root not in resolved.parents):
        raise RuntimeError("Runner archive resolved outside its private staging directory")
    if (candidate / METADATA_NAME).exists() or (candidate / METADATA_NAME).is_symlink():
        raise RuntimeError(f"Runner archive contains reserved file {METADATA_NAME}")

    # A link that is safe relative to the extraction root can become unsafe
    # after a top-level runner is moved beside existing installations. Re-check
    # links against the exact tree that will be renamed into place.
    for current, directories, files in os.walk(candidate, followlinks=False):
        for name in [*directories, *files]:
            path = Path(current) / name
            if not path.is_symlink():
                continue
            link_target = os.readlink(path)
            if Path(link_target).is_absolute():
                raise RuntimeError(f"Refusing escaping link in runner: {path}")
            relative_parent = path.relative_to(candidate).parent.as_posix()
            lexical_target = posixpath.normpath(
                posixpath.join(relative_parent, link_target)
            )
            if lexical_target == ".." or lexical_target.startswith("../"):
                raise RuntimeError(f"Refusing escaping link in runner: {path}")

    if find_wine_binary(candidate) is None and not (candidate / "proton").is_file():
        raise RuntimeError("Extracted archive does not contain a usable runner")


def _rename_noreplace(source: Path, target: Path) -> None:
    """Atomically rename *source* to a target that must not already exist."""
    libc = ctypes.CDLL(None, use_errno=True)
    renameat2 = getattr(libc, "renameat2", None)
    if renameat2 is None:  # pragma: no cover - supported on Linux targets
        raise RuntimeError("Atomic no-replace runner installation is unavailable")
    renameat2.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    renameat2.restype = ctypes.c_int
    result = renameat2(
        -100,  # AT_FDCWD
        os.fsencode(source),
        -100,
        os.fsencode(target),
        1,  # RENAME_NOREPLACE
    )
    if result == 0:
        return
    error = ctypes.get_errno()
    if error == errno.EEXIST:
        raise FileExistsError(f"Runner '{target.name}' is already installed")
    raise OSError(error, os.strerror(error), str(target))


def _read_metadata(root: Path) -> dict:
    if root.is_symlink():
        return {}
    meta = root / METADATA_NAME
    if not meta.exists():
        return {}
    try:
        data = json.loads(meta.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {}
    return data if isinstance(data, dict) else {}


def _read_family_id(root: Path) -> str:
    return str(_read_metadata(root).get("family") or "")


def _write_metadata(root: Path, release: ReleaseInfo) -> None:
    payload = {
        "family": release.family_id,
        "tag": release.tag,
        "asset": release.name,
        "source": release.family.github,
    }
    with (root / METADATA_NAME).open("x", encoding="utf-8") as stream:
        json.dump(payload, stream, indent=2)
        stream.write("\n")


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
            target = self.runners_directory / sanitise_release_tag(tag)
        if find_wine_binary(target) is not None or (target / "proton").exists():
            return True
        if family_id and self.runners_directory.is_dir():
            # Preserve recognition of runners installed with the earlier,
            # non-namespaced directory scheme by their authoritative metadata.
            for child in self.runners_directory.iterdir():
                metadata = _read_metadata(child)
                if (
                    metadata.get("family") == family_id
                    and metadata.get("tag") == tag
                    and (
                        find_wine_binary(child) is not None
                        or (child / "proton").is_file()
                    )
                ):
                    return True
        return False

    def is_release_installed(self, release: ReleaseInfo) -> bool:
        return self.is_installed(release.tag, release.family_id)

    def install(
        self,
        release: ReleaseInfo,
        progress_cb: Callable[[float], None] | None = None,
        timeout: int = 60,
    ) -> Path:
        """Download, stage, validate, and atomically install a runner build."""
        self.runners_directory.mkdir(parents=True, exist_ok=True)
        install_id = safe_install_id(release.install_id)
        target = self.runners_directory / install_id
        if target.exists() or target.is_symlink():
            raise FileExistsError(f"Runner '{install_id}' is already installed")
        if release.size < 0 or release.size > MAX_RUNNER_ARCHIVE_BYTES:
            raise RuntimeError("Runner archive exceeds the download size limit")

        with tempfile.TemporaryDirectory(
            dir=self.runners_directory, prefix=".install-"
        ) as staging_name:
            staging = Path(staging_name)
            # The archive's remote name is irrelevant once inside the private
            # staging directory, so do not use it as a local path at all.
            archive = staging / "runner.archive"
            extraction_root = staging / "extracted"
            extraction_root.mkdir(mode=0o700)

            req = Request(release.download_url, headers={"User-Agent": USER_AGENT})
            with urlopen(req, timeout=timeout) as resp:  # noqa: S310
                try:
                    declared = int(resp.headers.get("Content-Length", 0) or 0)
                except (TypeError, ValueError) as exc:
                    raise RuntimeError("Runner download has an invalid Content-Length") from exc
                if declared < 0 or declared > MAX_RUNNER_ARCHIVE_BYTES:
                    raise RuntimeError("Runner archive exceeds the download size limit")
                total = declared or release.size
                downloaded = 0
                with archive.open("xb") as stream:
                    while True:
                        chunk = resp.read(1024 * 256)
                        if not chunk:
                            break
                        downloaded += len(chunk)
                        if downloaded > MAX_RUNNER_ARCHIVE_BYTES:
                            raise RuntimeError(
                                "Runner archive exceeds the download size limit"
                            )
                        stream.write(chunk)
                        if progress_cb and total:
                            progress_cb(min(downloaded / total, 1.0))

            extract_archive(archive, extraction_root)
            extracted = self._resolve_staged(extraction_root)
            _validate_staged_runner(extracted, extraction_root)
            _write_metadata(extracted, release)
            if progress_cb:
                progress_cb(1.0)
            _rename_noreplace(extracted, target)
        return target

    def uninstall(self, runner_id: str) -> None:
        if runner_id in {"", SYSTEM_WINE}:
            raise ValueError("System Wine cannot be uninstalled")
        target = self.runners_directory / safe_install_id(runner_id)
        if target.is_symlink() or not target.is_dir():
            return
        shutil.rmtree(target)

    @staticmethod
    def _resolve_staged(extraction_root: Path) -> Path:
        """Select a usable root from immediate, non-symlink staged entries."""
        entries = sorted(extraction_root.iterdir(), key=lambda path: path.name)
        # Some Wine archives are rootless (bin/wine); keep their complete
        # extracted tree together rather than guessing one child to move.
        if find_wine_binary(extraction_root) or (extraction_root / "proton").is_file():
            return extraction_root
        if any(entry.is_symlink() for entry in entries):
            raise RuntimeError("Runner archive contains an unsafe top-level link")

        candidates = []
        for entry in entries:
            if not entry.is_dir():
                continue
            safe_install_id(entry.name)
            if find_wine_binary(entry) or (entry / "proton").is_file():
                candidates.append(entry)
        if len(candidates) != 1:
            raise RuntimeError("Could not locate one usable runner in the staged archive")
        return candidates[0]


_DESKTOP_SIZE_RE = re.compile(r"^\d{2,5}x\d{2,5}$")
_ANTICHEAT_DIR_NAMES = {
    "battleye": (
        "battleye_runtime", "BattlEye_Runtime", "proton-battleye-runtime",
        "Proton BattlEye Runtime",
    ),
    "eac": (
        "easyanticheat_runtime", "eac_runtime", "EasyAntiCheatRuntime",
        "proton-eac-runtime", "Proton EasyAntiCheat Runtime",
    ),
}


def parse_env_block(text: str) -> dict[str, str]:
    """Parse ``KEY=value`` pairs from a free-form environment block."""
    result: dict[str, str] = {}
    if not text:
        return result

    raw_lines: list[str] = []
    current: list[str] = []
    quote = ""
    escaped = False
    for char in text.replace("\r", "\n"):
        if escaped:
            current.append(char)
            escaped = False
            continue
        if char == "\\" and quote:
            current.append(char)
            escaped = True
            continue
        if char in {"'", '"'}:
            if not quote:
                quote = char
            elif quote == char:
                quote = ""
            current.append(char)
            continue
        if not quote and char in {";", "\n"}:
            raw_lines.append("".join(current))
            current = []
            continue
        current.append(char)
    raw_lines.append("".join(current))

    for raw in raw_lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        try:
            lexer = shlex.shlex(line, posix=True)
            lexer.whitespace_split = True
            lexer.commenters = ""
            lexer.escape = ""
            parts = list(lexer)
        except ValueError:
            parts = [line]
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


def install_bundled_dxvk(
    env: dict[str, str],
    root: Path | None = None,
) -> None:
    """Install the bundled DXVK DLLs into a raw-Wine prefix once per version."""
    prefix_value = env.get("WINEPREFIX", "").strip()
    if not prefix_value:
        raise RuntimeError("DXVK requires a configured Wine prefix")
    source = Path(root) if root is not None else Path(
        os.environ.get("GAMEHANDLER_DXVK_ROOT", str(DXVK_ROOT))
    )
    required = [source / arch / "d3d11.dll" for arch in ("x32", "x64")]
    if not all(path.is_file() for path in required):
        raise RuntimeError("Bundled DXVK runtime is unavailable")

    prefix = Path(prefix_value)
    marker = prefix / ".gamehandler-dxvk-version"
    try:
        if marker.read_text(encoding="utf-8").strip() == DXVK_VERSION:
            merge_dll_overrides(env, "d3d8,d3d9,d3d10core,d3d11,dxgi=n,b")
            return
    except OSError:
        pass

    prefix.mkdir(parents=True, exist_ok=True)

    windows = prefix / "drive_c" / "windows"
    is_win32 = env.get("WINEARCH", "").strip().lower() == "win32"
    if not is_win32:
        try:
            is_win32 = "#arch=win32" in (prefix / "system.reg").read_text(
                encoding="utf-8", errors="ignore"
            )[:512]
        except OSError:
            pass
    if is_win32:
        targets = ((source / "x32", windows / "system32"),)
    else:
        targets = (
            (source / "x64", windows / "system32"),
            (source / "x32", windows / "syswow64"),
        )
    for source_dir, target_dir in targets:
        target_dir.mkdir(parents=True, exist_ok=True)
        for dll in source_dir.glob("*.dll"):
            shutil.copy2(dll, target_dir / dll.name)
    marker.write_text(DXVK_VERSION + "\n", encoding="utf-8")
    merge_dll_overrides(env, "d3d8,d3d9,d3d10core,d3d11,dxgi=n,b")


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
        Path.home() / ".var/app/com.valvesoftware.Steam/data/Steam/steamapps/common",
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


def uses_proton_runtime(runner: "Runner", argv: list[str]) -> bool:
    """Whether *argv* reaches a real Proton build through UMU.

    Proton-only environment variables are meaningless, and misleading in a bug
    report, when the command is plain Wine. Both ``launch()`` and the easy
    installers gate on this, so it lives in one place.
    """
    return (
        isinstance(runner, ProtonRunner)
        and runner.proton_script() is not None
        and bool(argv)
        and Path(argv[0]).name == "umu-run"
    )


def apply_launch_options(
    game: Game,
    argv: list[str],
    env: dict[str, str],
    *,
    proton_features: bool = True,
) -> tuple[list[str], dict[str, str]]:
    """Apply Lutris/Faugus-style launch helpers and compatibility toggles."""
    env = dict(env)
    wrapped = list(argv)

    if game.prefer_sdl:
        env["SDL_JOYSTICK_HIDAPI"] = "1"
        if proton_features:
            env["PROTON_ENABLE_HIDAPI"] = "1"
            env["PROTON_NO_HIDRAW"] = "1"
    if game.wayland and proton_features:
        env["PROTON_ENABLE_WAYLAND"] = "1"
        env["DISPLAY"] = ""
    if game.hdr:
        env["DXVK_HDR"] = "1"
        if proton_features:
            env["PROTON_ENABLE_HDR"] = "1"
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
            merge_dll_overrides(env, "dxgi,d3d11,d3d10core,d3d9=b")
        if not game.vkd3d:
            merge_dll_overrides(env, "d3d12,d3d12core=b")
        if game.nvapi and proton_features:
            env["PROTON_ENABLE_NVAPI"] = "1"
            env["DXVK_ENABLE_NVAPI"] = "1"
            env["DXVK_NVAPIHACK"] = "0"
        if game.fsr and proton_features:
            env["WINE_FULLSCREEN_FSR"] = "1"
            env.setdefault("WINE_FULLSCREEN_FSR_STRENGTH", "2")
        if proton_features:
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
        else:
            raise RuntimeError(
                "Gamescope is enabled but unavailable. Flatpak users must install "
                "org.freedesktop.Platform.VulkanLayer.gamescope//25.08 from Flathub."
            )

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


def resolve_game_paths(game: Game) -> Game:
    """Return *game* with picker URLs and share locations turned into paths.

    A library entry that lives on a network share is stored however the file
    dialog handed it over. Resolving through the GVFS FUSE mount here means a
    share-hosted title launches like a local one instead of failing with a
    URL Wine cannot execute. A share that is not mounted any more fails with
    an instruction, not a mystery.
    """
    exe = as_local_path(game.exe_path)
    cwd = as_local_path(game.working_directory)
    extra = as_local_path(game.additional_app)
    if is_remote_url(exe):
        raise RuntimeError(unreachable_share_message(game.exe_path))
    if (exe, cwd, extra) == (game.exe_path, game.working_directory, game.additional_app):
        return game
    return replace(game, exe_path=exe, working_directory=cwd, additional_app=extra)


# How long a launched title gets to stay alive before we stop watching it. A
# real game is still running after this; one that mis-configured its prefix or
# never found its executable is long gone.
LAUNCH_GRACE_SECONDS = 6.0
# Wine is chatty even at WINEDEBUG=-all; only the tail is worth showing.
_ERROR_TAIL_LINES = 4
_ERROR_MESSAGE_CHARS = 240
_ERROR_BUFFER_BYTES = 64 * 1024
_NOISE_PREFIXES = ("fixme:", "warn:", "trace:", "info:")


def _readable_error(text: str) -> str:
    """Reduce a runner's output to the part worth putting in a toast."""
    lines = [line.strip() for line in text.replace("\r", "\n").splitlines()]
    useful = [
        line
        for line in lines
        if line and not line.lower().startswith(_NOISE_PREFIXES)
    ]
    tail = (useful or [line for line in lines if line])[-_ERROR_TAIL_LINES:]
    return " ".join(tail)[:_ERROR_MESSAGE_CHARS]


class _ErrorTail:
    """Drain a child's stderr on a thread, keeping only its last few KB.

    Draining is what makes capturing safe: an undrained pipe fills and stalls
    the game, and an unbounded buffer would grow for as long as the title runs.
    """

    def __init__(self, stream, limit: int = _ERROR_BUFFER_BYTES) -> None:
        self._limit = limit
        self._chunks: list[bytes] = []
        self._size = 0
        self._lock = threading.Lock()
        self._thread = threading.Thread(target=self._drain, args=(stream,), daemon=True)
        self._thread.start()

    def _drain(self, stream) -> None:
        try:
            with stream:
                for chunk in iter(lambda: stream.read(4096), b""):
                    if not isinstance(chunk, (bytes, bytearray)):
                        break
                    with self._lock:
                        self._chunks.append(chunk)
                        self._size += len(chunk)
                        while self._size > self._limit and len(self._chunks) > 1:
                            self._size -= len(self._chunks.pop(0))
        except (OSError, ValueError):  # pragma: no cover - the child closed first
            pass

    def text(self) -> str:
        with self._lock:
            return b"".join(self._chunks).decode("utf-8", "replace")


@dataclass
class LaunchedGame:
    """A started title, plus the means to notice it died on the doorstep."""

    process: subprocess.Popen
    errors: _ErrorTail | None = None

    @property
    def pid(self) -> int:
        return self.process.pid

    def failure(self, timeout: float = LAUNCH_GRACE_SECONDS) -> str | None:
        """Why the title stopped, if it stopped badly within *timeout*.

        ``Popen`` succeeding only proves the runner binary exists. Wine exiting
        two seconds later because it could not find the executable looked
        exactly like a successful launch, which is the whole reason this exists.
        """
        try:
            code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            return None
        if code == 0:
            return None
        detail = _readable_error(self.errors.text()) if self.errors is not None else ""
        return detail or f"the runner exited with status {code}"


def launch(game: Game, manager: RunnerManager | None = None) -> LaunchedGame:
    """Launch *game* with its configured runner.

    Returns a :class:`LaunchedGame` whose ``failure()`` reports a title that
    exited immediately, so a broken launch surfaces instead of looking like a
    successful one.
    """
    manager = manager or RunnerManager()
    game = resolve_game_paths(game)
    uses_proton = False
    if game.is_linux:
        argv, env = build_linux_command(game)
        runner_executable = ""
    else:
        runner = manager.get(game.runner)
        argv, env = runner.build_command(game)
        # Apply the user's environment block early as well as last: prefix setup
        # and the bundled-DXVK installer below read WINEARCH from this env.
        # apply_launch_options() re-applies it so it still wins over the toggles.
        env.update(parse_env_block(game.environment))
        runner_executable = argv[0] if argv else ""
        prefix = env.get("WINEPREFIX")
        if prefix:
            Path(prefix).mkdir(parents=True, exist_ok=True)
        uses_proton = uses_proton_runtime(runner, argv)
        if game.nvapi and not uses_proton:
            raise RuntimeError("NVAPI/DLSS requires a Proton runner through UMU")
        if game.fsr and not uses_proton:
            raise RuntimeError("FSR requires a compatible Proton runner through UMU")
        if game.wayland and not uses_proton:
            raise RuntimeError("Wayland mode requires a Proton runner through UMU")
        dxvk_root = Path(os.environ.get("GAMEHANDLER_DXVK_ROOT", str(DXVK_ROOT)))
        if game.dxvk and not uses_proton and dxvk_root.is_dir():
            install_bundled_dxvk(env, dxvk_root)

    argv, env = apply_launch_options(game, argv, env, proton_features=uses_proton)

    extra = game.additional_app.strip()
    if extra:
        extra_argv = [runner_executable, extra] if runner_executable else [extra]
        subprocess.Popen(extra_argv, env=env)

    cwd = game.working_directory or None
    if not cwd and game.exe_path and Path(game.exe_path).exists():
        cwd = str(Path(game.exe_path).parent)
    # stdout stays inherited so running from a terminal still shows the game's
    # own output; only stderr is captured, and only to explain a fast exit.
    process = subprocess.Popen(argv, env=env, cwd=cwd, stderr=subprocess.PIPE)
    errors = _ErrorTail(process.stderr) if process.stderr is not None else None
    return LaunchedGame(process, errors)


def escape_desktop_value(value: str) -> str:
    """Escape a string for a Desktop Entry value.

    Newlines are the dangerous case: an unescaped one in a game name would let
    the title inject extra keys (including its own ``Exec=``) into the file.
    """
    return (
        str(value)
        .replace("\\", "\\\\")
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )


def desktop_exec(command: str) -> str:
    """Escape a command for ``Exec=``, which reserves ``%`` for field codes."""
    return escape_desktop_value(command).replace("%", "%%")


def create_desktop_shortcut(game: Game, command: str, directory: Path | None = None) -> Path:
    """Write a ``.desktop`` launcher for *game* and return its path."""
    apps = directory or (Path.home() / ".local" / "share" / "applications")
    apps.mkdir(parents=True, exist_ok=True)
    slug = "".join(ch if ch.isalnum() else "-" for ch in game.name.lower()).strip("-")
    path = apps / f"gamehandler-{game.id[:8]}-{slug or 'game'}.desktop"
    icon = game.cover_path if game.cover_path else "applications-games"
    name = escape_desktop_value(game.name) or "Game"
    body = "\n".join(
        [
            "[Desktop Entry]",
            "Type=Application",
            f"Name={name}",
            f"Comment=Launch {name} with GameHandler",
            f"Exec={desktop_exec(command)}",
            f"Icon={escape_desktop_value(icon)}",
            "Terminal=false",
            "StartupNotify=true",
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
    prefix = wine_prefix_root(game.prefix_path or str(config.prefixes_dir() / game.id))
    env = dict(os.environ)
    env["WINEPREFIX"] = prefix
    # Point the tool at the same Wine the game runs on. Winetricks otherwise
    # falls back to the system wine, which refuses (or silently migrates) a
    # prefix that a newer Proton/Wine build created.
    env["WINE"] = wine
    wineserver = Path(wine).with_name("wineserver")
    if wineserver.is_file() and os.access(wineserver, os.X_OK):
        env["WINESERVER"] = str(wineserver)
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
    "PROTON_PREFIX_SUBDIR",
    "LAUNCH_GRACE_SECONDS",
    "LaunchedGame",
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
    "runner_guide_details",
    "RunnerGuide",
    "SYSTEM_WINE_GUIDE",
    "find_wine_binary",
    "prefix_drive_c",
    "prefix_drive_cs",
    "wine_prefix_root",
    "parse_env_block",
    "merge_dll_overrides",
    "install_bundled_dxvk",
    "normalize_desktop_size",
    "virtual_desktop_argv",
    "find_anticheat_runtime",
    "resolve_game_paths",
    "apply_launch_options",
    "uses_proton_runtime",
    "build_linux_command",
    "create_desktop_shortcut",
    "escape_desktop_value",
    "desktop_exec",
    "extract_archive",
    "safe_archive_name",
    "safe_install_id",
    "tool_command",
    "launch",
]
