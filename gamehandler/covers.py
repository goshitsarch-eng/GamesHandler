"""Cover art and Steam metadata helpers.

Looks up a title on the public Steam store search API, scores the best
match, then downloads a portrait library cover (with header/capsule
fallbacks). No SteamGridDB key is required.
"""

from __future__ import annotations

import json
import re
import shutil
import urllib.error
import urllib.parse
from dataclasses import dataclass
from pathlib import Path
from urllib.request import Request, urlopen

from . import config

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
    if right.startswith(left) or left.startswith(right):
        return 0.92
    left_parts = set(left.split())
    right_parts = set(right.split())
    if not left_parts:
        return 0.0
    overlap = len(left_parts & right_parts) / len(left_parts)
    if left in right or right in left:
        overlap = max(overlap, 0.8)
    return overlap


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


def pick_best_match(query: str, items: list[dict], minimum: float = 0.45) -> dict | None:
    """Choose the Steam search hit that best matches *query*."""
    ranked = []
    for item in items:
        if item.get("type") and item["type"] not in {"app", "game", ""}:
            continue
        score = score_title(query, item.get("name") or "")
        if score >= minimum:
            ranked.append((score, item))
    ranked.sort(key=lambda pair: pair[0], reverse=True)
    return ranked[0][1] if ranked else None


def cover_urls_for_app(appid: int, tiny_image: str = "") -> list[str]:
    urls = [
        f"{root}/{appid}/{asset}"
        for asset in COVER_ASSETS
        for root in CDN_ROOTS
    ]
    if tiny_image:
        urls.append(tiny_image)
    return urls


def _request(url: str, timeout: int = 20) -> bytes:
    req = Request(url, headers={"User-Agent": USER_AGENT, "Accept": "*/*"})
    with urlopen(req, timeout=timeout) as resp:  # noqa: S310
        if getattr(resp, "status", 200) >= 400:
            raise urllib.error.HTTPError(url, resp.status, "HTTP error", resp.headers, None)
        return resp.read()


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


@dataclass
class CoverHit:
    appid: int
    name: str
    category: str
    cover_path: str
    source_url: str


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


def fetch_cover(query: str, game_id: str, timeout: int = 20) -> CoverHit:
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
    )


__all__ = [
    "DEFAULT_CATEGORIES",
    "CoverHit",
    "normalize_title",
    "score_title",
    "map_steam_genre",
    "parse_store_search",
    "pick_best_match",
    "cover_urls_for_app",
    "copy_custom_cover",
    "fetch_cover",
    "search_steam",
]
