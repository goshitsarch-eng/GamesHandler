"""Persisted application preferences."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, fields
from pathlib import Path

from . import config
from .runners import SYSTEM_WINE

COLOR_SCHEMES = ("system", "light", "dark")
VIEW_MODES = ("grid", "list")


@dataclass
class Settings:
    """User preferences stored under the XDG config directory."""

    color_scheme: str = "dark"
    view_mode: str = "grid"
    default_runner: str = SYSTEM_WINE
    default_mangohud: bool = False
    default_gamemode: bool = False
    default_prefer_sdl: bool = False
    close_on_launch: bool = False

    @classmethod
    def from_dict(cls, data: dict) -> "Settings":
        known = {f.name for f in fields(cls)}
        cleaned = {k: v for k, v in data.items() if k in known}
        settings = cls(**cleaned)
        if settings.color_scheme not in COLOR_SCHEMES:
            settings.color_scheme = "dark"
        if settings.view_mode not in VIEW_MODES:
            settings.view_mode = "grid"
        return settings

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def load(cls, path: Path | None = None) -> "Settings":
        target = Path(path) if path is not None else config.settings_file()
        if not target.exists():
            return cls()
        try:
            raw = json.loads(target.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            return cls()
        if not isinstance(raw, dict):
            return cls()
        return cls.from_dict(raw)

    def save(self, path: Path | None = None) -> None:
        target = Path(path) if path is not None else config.settings_file()
        target.parent.mkdir(parents=True, exist_ok=True)
        tmp = target.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")
        tmp.replace(target)


__all__ = ["COLOR_SCHEMES", "VIEW_MODES", "Settings"]
