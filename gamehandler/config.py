"""XDG-compliant filesystem locations used by GameHandler.

All paths are resolved lazily so tests can redirect them by setting the
standard ``XDG_*`` or the ``GAMEHANDLER_*`` override environment variables
before importing anything that touches the filesystem.
"""

from __future__ import annotations

import os
from pathlib import Path

from . import APP_ID


def _xdg(env_var: str, default: Path) -> Path:
    value = os.environ.get(env_var)
    return Path(value) if value else default


def data_home() -> Path:
    """Base directory for persistent application data (games, runners, prefixes)."""
    override = os.environ.get("GAMEHANDLER_DATA_HOME")
    if override:
        return Path(override)
    base = _xdg("XDG_DATA_HOME", Path.home() / ".local" / "share")
    return base / "gamehandler"


def config_home() -> Path:
    """Base directory for configuration files."""
    override = os.environ.get("GAMEHANDLER_CONFIG_HOME")
    if override:
        return Path(override)
    base = _xdg("XDG_CONFIG_HOME", Path.home() / ".config")
    return base / "gamehandler"


def games_file() -> Path:
    return config_home() / "games.json"


def settings_file() -> Path:
    return config_home() / "settings.json"


def runners_dir() -> Path:
    """Where downloaded Proton/Wine builds are extracted."""
    return data_home() / "runners"


def prefixes_dir() -> Path:
    """Default parent directory for per-game Wine prefixes."""
    return data_home() / "prefixes"


def covers_dir() -> Path:
    """Where downloaded and imported cover images are stored."""
    return data_home() / "covers"


def ensure_dirs() -> None:
    for path in (config_home(), data_home(), runners_dir(), prefixes_dir(), covers_dir()):
        path.mkdir(parents=True, exist_ok=True)


__all__ = [
    "APP_ID",
    "data_home",
    "config_home",
    "games_file",
    "settings_file",
    "runners_dir",
    "prefixes_dir",
    "covers_dir",
    "ensure_dirs",
]
