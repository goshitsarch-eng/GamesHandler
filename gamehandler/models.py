"""Data model and JSON-backed persistence for the game library."""

from __future__ import annotations

import json
import time
import uuid
from dataclasses import asdict, dataclass, field, fields
from pathlib import Path
from typing import Iterable

from . import config

UNCATEGORIZED = "Uncategorized"
SORT_MODES = ("name", "recent", "added")


@dataclass
class Game:
    """A single library entry managed by GameHandler."""

    name: str
    exe_path: str = ""
    runner: str = "wine-system"
    prefix_path: str = ""
    arguments: str = ""
    cover_path: str = ""
    category: str = "Uncategorized"
    steam_appid: int = 0
    kind: str = "windows"  # windows | linux
    working_directory: str = ""
    additional_app: str = ""
    mangohud: bool = False
    gamemode: bool = False
    prefer_sdl: bool = False
    wayland: bool = False
    hdr: bool = False
    esync: bool = True
    fsync: bool = True
    dxvk: bool = True
    vkd3d: bool = True
    nvapi: bool = False
    fsr: bool = False
    battleye: bool = True
    eac: bool = True
    gamescope: bool = False
    virtual_desktop: bool = False
    virtual_desktop_size: str = "1920x1080"
    environment: str = ""
    id: str = field(default_factory=lambda: uuid.uuid4().hex)
    added: float = field(default_factory=time.time)
    last_played: float = 0.0

    @property
    def is_linux(self) -> bool:
        return self.kind == "linux"

    @property
    def display_category(self) -> str:
        """The category shown and filtered on; blanks fold into Uncategorized."""
        return (self.category or "").strip() or UNCATEGORIZED

    @classmethod
    def from_dict(cls, data: dict) -> "Game":
        known = {f.name for f in fields(cls)}
        return cls(**{k: v for k, v in data.items() if k in known})

    def to_dict(self) -> dict:
        return asdict(self)


def format_last_played(timestamp: float, now: float | None = None) -> str:
    """A short, human-readable 'last played' label for library rows."""
    if not timestamp:
        return "Never played"
    current = time.time() if now is None else now
    seconds = max(0.0, current - timestamp)
    minutes = seconds / 60
    if minutes < 2:
        return "Played just now"
    if minutes < 60:
        return f"Played {int(minutes)} min ago"
    hours = minutes / 60
    if hours < 24:
        count = int(hours)
        return f"Played {count} hour ago" if count == 1 else f"Played {count} hours ago"
    days = int(hours / 24)
    if days == 1:
        return "Played yesterday"
    if days < 30:
        return f"Played {days} days ago"
    months = days // 30
    if months < 12:
        return f"Played {months} month ago" if months == 1 else f"Played {months} months ago"
    years = months // 12
    return f"Played {years} year ago" if years == 1 else f"Played {years} years ago"


class Library:
    """Loads, mutates and persists a collection of :class:`Game` objects."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = Path(path) if path is not None else config.games_file()
        self._games: dict[str, Game] = {}
        self.load()

    def load(self) -> None:
        self._games = {}
        if not self.path.exists():
            return
        try:
            raw = json.loads(self.path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError, UnicodeDecodeError):
            raw = []
        if not isinstance(raw, list):
            # A corrupt or hand-edited file must not take the library down.
            raw = []
        for item in raw:
            if not isinstance(item, dict):
                continue
            try:
                game = Game.from_dict(item)
            except (TypeError, ValueError):
                continue
            if game.name:
                self._games[game.id] = game

    def save(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = [g.to_dict() for g in self.all()]
        tmp = self.path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        tmp.replace(self.path)

    def all(self, sort: str = "name") -> list[Game]:
        games = list(self._games.values())
        if sort == "recent":
            # Never-played titles sink to the bottom, then alphabetical.
            return sorted(games, key=lambda g: (-g.last_played, g.name.lower()))
        if sort == "added":
            return sorted(games, key=lambda g: (-g.added, g.name.lower()))
        return sorted(games, key=lambda g: g.name.lower())

    def get(self, game_id: str) -> Game | None:
        return self._games.get(game_id)

    def add(self, game: Game) -> Game:
        self._games[game.id] = game
        self.save()
        return game

    def remove(self, game_id: str) -> None:
        if game_id in self._games:
            del self._games[game_id]
            self.save()

    def update(self, game: Game) -> None:
        self._games[game.id] = game
        self.save()

    def mark_played(self, game_id: str) -> None:
        game = self._games.get(game_id)
        if game:
            game.last_played = time.time()
            self.save()

    def search(self, query: str, category: str = "", sort: str = "name") -> list[Game]:
        query = query.strip().lower()
        games = self.all(sort=sort)
        if category and category != "All":
            games = [g for g in games if g.display_category == category]
        if not query:
            return games
        return [
            g
            for g in games
            if query in g.name.lower() or query in g.display_category.lower()
        ]

    def categories(self) -> list[str]:
        found = {g.display_category for g in self._games.values()}
        return sorted(found, key=lambda name: (name == UNCATEGORIZED, name.lower()))

    def __len__(self) -> int:
        return len(self._games)

    def __iter__(self) -> Iterable[Game]:
        return iter(self.all())


__all__ = ["SORT_MODES", "UNCATEGORIZED", "Game", "Library", "format_last_played"]
