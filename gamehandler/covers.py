"""Cover art and Steam metadata helpers.

Looks up a title on the public Steam store search API, scores the best
match, then downloads a portrait library cover (with header/capsule
fallbacks). No SteamGridDB key is required.

Store launchers and ordinary Windows apps are not Steam store products, so
that search finds nothing for them. Those fall back to the icon the
executable already carries, which needs no network and is always the right
artwork for the thing it was taken from.
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import urllib.error
import urllib.parse
from dataclasses import dataclass
from pathlib import Path
from urllib.request import Request, urlopen

from . import config
from .exe_icons import extract_icon

USER_AGENT = "GameHandler"
STORE_SEARCH_URL = "https://store.steampowered.com/api/storesearch/"
APPDETAILS_URL = "https://store.steampowered.com/api/appdetails"
CDN_ROOTS = (
    "https://cdn.akamai.steamstatic.com/steam/apps",
    "https://cdn.cloudflare.steamstatic.com/steam/apps",
    "https://steamcdn-a.akamaihd.net/steam/apps",
)
COVER_ASSETS = (
    "library_600x900.jpg",
    "library_600x900_2x.jpg",
    "library_capsule.jpg",
    "portrait.png",
    "header.jpg",
    "capsule_616x353.jpg",
)

# Steam genres mapped onto GameHandler's category list.
GENRE_MAP = {
    "action": "Action",
    "adventure": "Adventure",
    "rpg": "RPG",
    "role-playing": "RPG",
    "strategy": "Strategy",
    "simulation": "Simulation",
    "racing": "Racing",
    "sports": "Sports",
    "casual": "Puzzle",
    "indie": "Indie",
    "free to play": "Indie",
    "early access": "Indie",
    "massively multiplayer": "Action",
    "animation & modeling": "Utility",
    "utilities": "Utility",
    "design & illustration": "Utility",
    "video production": "Utility",
    "audio production": "Utility",
    "education": "Utility",
    "web publishing": "Utility",
    "software training": "Utility",
    "photo editing": "Utility",
    "game development": "Utility",
    "violent": "Action",
    "gore": "Action",
    "nudity": "Adventure",
}

DEFAULT_CATEGORIES = (
    "Uncategorized",
    "Action",
    "Adventure",
    "RPG",
    "Strategy",
    "Shooter",
    "Racing",
    "Simulation",
    "Sports",
    "Puzzle",
    "Indie",
    "Utility",
    "Emulation",
)


# Tile art falls back to initials on a coloured plate when a game has no cover,
# which also keeps the library readable when an icon theme lacks our symbolics.
COVER_ACCENTS = 8


def initials(name: str) -> str:
    """Up to two uppercase initials for placeholder cover art."""
    words = [word for word in re.split(r"[^0-9A-Za-z]+", name or "") if word]
    if not words:
        return "?"
    if len(words) == 1:
        return words[0][:2].upper()
    return (words[0][0] + words[1][0]).upper()


def accent_index(seed: str, buckets: int = COVER_ACCENTS) -> int:
    """A stable colour bucket for *seed*, so a game's tile never changes shade."""
    buckets = max(1, buckets)
    digest = hashlib.sha256((seed or "").encode("utf-8")).digest()
    return digest[0] % buckets


def normalize_title(value: str) -> str:
    """Lowercase a title and strip punctuation / trademark noise."""
    cleaned = value.lower()
    cleaned = cleaned.replace("&", " and ")
    cleaned = re.sub(r"[™®©]", "", cleaned)
    cleaned = re.sub(r"[^a-z0-9]+", " ", cleaned)
    return " ".join(cleaned.split())


def score_title(query: str, candidate: str) -> float:
    """Return a 0–1 similarity score used to pick the Steam match."""
    left = normalize_title(query)
    right = normalize_title(candidate)
    if not left or not right:
        return 0.0
    if left == right:
        return 1.0
    left_parts = set(left.split())
    right_parts = set(right.split())
    shared = left_parts & right_parts
    if not left_parts or not shared:
        return 0.0
    coverage = len(shared) / len(left_parts)
    precision = len(shared) / len(right_parts)
    if right.startswith(left + " ") or left.startswith(right + " "):
        # An edition or sequel suffix ("Half-Life 2") is still the same series.
        coverage = max(coverage, 0.92)
    # Words the query never mentioned mark a different product far more often
    # than a longer spelling of the same one — "Battle.net" is not "Mega Man
    # Battle Network", and "EA App" is not "Easy Game Builder App". Charging
    # for those extra words is what keeps someone else's art off the tile.
    return round(coverage * (0.6 + 0.4 * precision), 6)


def map_steam_genre(genres: list[str]) -> str:
    """Map Steam genre names onto a GameHandler category."""
    for genre in genres:
        mapped = GENRE_MAP.get(genre.strip().lower())
        if mapped:
            return mapped
    return "Uncategorized"


def parse_store_search(payload: dict) -> list[dict]:
    """Normalize a Steam storesearch payload into match dicts."""
    items = []
    for item in payload.get("items") or []:
        appid = item.get("id")
        name = item.get("name") or ""
        if not appid or not name:
            continue
        items.append(
            {
                "appid": int(appid),
                "name": name,
                "tiny_image": item.get("tiny_image") or "",
                "type": item.get("type") or "",
            }
        )
    return items


# A single-word title is too generic to accept a partial match on: Steam's
# search answers "Steam" with "DCS World Steam Edition" and "Discord" with
# "Bot Maker For Discord". Hanging that art on the tile is worse than leaving
# the tile blank, so one-word queries have to match a title outright.
GENERIC_QUERY_MINIMUM = 0.9


def pick_best_match(query: str, items: list[dict], minimum: float = 0.45) -> dict | None:
    """Choose the Steam search hit that best matches *query*."""
    threshold = minimum
    if len(normalize_title(query).split()) < 2:
        threshold = max(minimum, GENERIC_QUERY_MINIMUM)
    ranked = []
    for item in items:
        if item.get("type") and item["type"] not in {"app", "game", ""}:
            continue
        name = item.get("name") or ""
        score = score_title(query, name)
        if score >= threshold:
            # Equal scores go to the tighter title, not to whichever hit the
            # store happened to rank first.
            ranked.append((score, -len(normalize_title(name).split()), item))
    ranked.sort(key=lambda entry: (entry[0], entry[1]), reverse=True)
    return ranked[0][2] if ranked else None


def cover_urls_for_app(appid: int, tiny_image: str = "") -> list[str]:
    urls = [
        f"{root}/{appid}/{asset}"
        for asset in COVER_ASSETS
        for root in CDN_ROOTS
    ]
    if tiny_image:
        urls.append(tiny_image)
    return urls


# Covers are small; refuse to buffer a CDN that streams without end.
MAX_RESPONSE_BYTES = 12 * 1024 * 1024


def _request(url: str, timeout: int = 20, limit: int = MAX_RESPONSE_BYTES) -> bytes:
    req = Request(url, headers={"User-Agent": USER_AGENT, "Accept": "*/*"})
    with urlopen(req, timeout=timeout) as resp:  # noqa: S310
        if getattr(resp, "status", 200) >= 400:
            raise urllib.error.HTTPError(url, resp.status, "HTTP error", resp.headers, None)
        data = resp.read(limit + 1)
    if len(data) > limit:
        raise RuntimeError(f"Response from {url} exceeded {limit} bytes")
    return data


def search_steam(query: str, timeout: int = 20) -> list[dict]:
    params = urllib.parse.urlencode({"term": query, "l": "english", "cc": "US"})
    raw = _request(f"{STORE_SEARCH_URL}?{params}", timeout=timeout)
    payload = json.loads(raw.decode("utf-8"))
    if not isinstance(payload, dict):
        return []
    return parse_store_search(payload)


def fetch_app_details(appid: int, timeout: int = 20) -> dict:
    raw = _request(f"{APPDETAILS_URL}?appids={appid}&l=english", timeout=timeout)
    payload = json.loads(raw.decode("utf-8"))
    entry = (payload or {}).get(str(appid)) or {}
    if not entry.get("success"):
        return {}
    data = entry.get("data") or {}
    genres = [item.get("description", "") for item in data.get("genres") or [] if item.get("description")]
    return {
        "name": data.get("name") or "",
        "header_image": data.get("header_image") or "",
        "genres": genres,
        "category": map_steam_genre(genres),
    }


STEAM_SOURCE = "steam"
ICON_SOURCE = "icon"


@dataclass
class CoverHit:
    appid: int
    name: str
    category: str
    cover_path: str
    source_url: str
    source: str = STEAM_SOURCE

    @property
    def origin_label(self) -> str:
        """Where the artwork came from, for the toast that announces it."""
        return "the app icon" if self.source == ICON_SOURCE else "Steam"


def download_image(url: str, destination: Path, timeout: int = 30) -> Path:
    data = _request(url, timeout=timeout)
    if len(data) < 1024:
        raise RuntimeError("Cover download was empty")
    destination.parent.mkdir(parents=True, exist_ok=True)
    tmp = destination.with_suffix(destination.suffix + ".tmp")
    tmp.write_bytes(data)
    tmp.replace(destination)
    return destination


def save_cover_from_urls(urls: list[str], game_id: str) -> tuple[Path, str]:
    destination = config.covers_dir() / f"{game_id}.jpg"
    last_error = "No cover URLs"
    for url in urls:
        try:
            return download_image(url, destination), url
        except (OSError, urllib.error.URLError, urllib.error.HTTPError, RuntimeError) as exc:
            last_error = str(exc)
            continue
    raise RuntimeError(f"Could not download a cover: {last_error}")


def copy_custom_cover(source: str | Path, game_id: str) -> Path:
    """Copy a user-selected image into the covers directory."""
    src = Path(source)
    if not src.is_file():
        raise FileNotFoundError(str(src))
    suffix = src.suffix.lower() if src.suffix.lower() in {".png", ".jpg", ".jpeg", ".webp"} else ".jpg"
    destination = config.covers_dir() / f"{game_id}{suffix}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, destination)
    return destination


def save_exe_icon(exe_path: str | Path, game_id: str) -> Path:
    """Write the icon embedded in a Windows executable into the covers dir."""
    icon = extract_icon(exe_path)
    if not icon:
        raise RuntimeError(f"{Path(exe_path).name} carries no icon to use as a cover")
    destination = config.covers_dir() / f"{game_id}.ico"
    destination.parent.mkdir(parents=True, exist_ok=True)
    tmp = destination.with_suffix(destination.suffix + ".tmp")
    tmp.write_bytes(icon)
    tmp.replace(destination)
    return destination


def icon_cover(name: str, exe_path: str | Path, game_id: str) -> CoverHit:
    """Build a :class:`CoverHit` from a Windows executable's own icon."""
    path = save_exe_icon(exe_path, game_id)
    return CoverHit(
        appid=0,
        name=name or Path(exe_path).stem,
        category="Uncategorized",
        cover_path=str(path),
        source_url=str(exe_path),
        source=ICON_SOURCE,
    )


def steam_cover(query: str, game_id: str, timeout: int = 20) -> CoverHit:
    """Search Steam for *query* and download the best matching cover."""
    items = search_steam(query, timeout=timeout)
    match = pick_best_match(query, items)
    if not match:
        raise RuntimeError(f"No Steam cover found for “{query}”")
    details = {}
    try:
        details = fetch_app_details(match["appid"], timeout=timeout)
    except (OSError, urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError):
        details = {}
    urls = cover_urls_for_app(match["appid"], match.get("tiny_image") or "")
    header = details.get("header_image")
    if header:
        urls.insert(0, header)
    path, source = save_cover_from_urls(urls, game_id)
    return CoverHit(
        appid=match["appid"],
        name=details.get("name") or match["name"],
        category=details.get("category") or "Uncategorized",
        cover_path=str(path),
        source_url=source,
        source=STEAM_SOURCE,
    )


def fetch_cover(query: str, game_id: str, timeout: int = 20, exe_path: str | Path = "") -> CoverHit:
    """Find artwork for *query*, preferring Steam and falling back to the exe.

    Steam has real portrait library art for the games it sells, so it goes
    first. It has nothing at all for store launchers and ordinary Windows
    apps, which is why *exe_path* is the second source: the executable's own
    icon is offline, unambiguous, and belongs to the thing being launched.
    """
    try:
        return steam_cover(query, game_id, timeout=timeout)
    except (
        OSError,
        RuntimeError,
        json.JSONDecodeError,
        urllib.error.URLError,
        urllib.error.HTTPError,
    ) as exc:
        steam_error = str(exc)
    if exe_path and Path(exe_path).is_file():
        try:
            return icon_cover(query, exe_path, game_id)
        except (OSError, RuntimeError):
            pass
    raise RuntimeError(steam_error)


__all__ = [
    "COVER_ACCENTS",
    "DEFAULT_CATEGORIES",
    "GENERIC_QUERY_MINIMUM",
    "ICON_SOURCE",
    "STEAM_SOURCE",
    "accent_index",
    "initials",
    "CoverHit",
    "normalize_title",
    "score_title",
    "map_steam_genre",
    "parse_store_search",
    "pick_best_match",
    "cover_urls_for_app",
    "copy_custom_cover",
    "fetch_cover",
    "icon_cover",
    "save_exe_icon",
    "search_steam",
    "steam_cover",
]
