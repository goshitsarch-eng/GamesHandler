"""Resolve file-picker URLs and network-share locations to local paths.

A game library does not always live on the local disk. File choosers on Linux
happily list SMB/SFTP/WebDAV shares, but hand back ``smb://server/share/...``
style URLs — and Wine (or a Linux binary) can only execute a real filesystem
path. Earlier GameHandler releases simply gave up here and asked the desktop
to "open the location with another application", which is useless for a game
executable.

GVFS solves this properly: once a share is mounted, its contents are exposed
through a FUSE filesystem under ``$XDG_RUNTIME_DIR/gvfs``. This module maps a
share URL onto that FUSE path so a picked executable on a network share works
exactly like a local one. KDE's KIO-based dialogs usually hand over the FUSE
path themselves; this is the fallback for everything else, and for entries the
user pasted by hand.

Pure stdlib and Qt-free so it can be unit tested headlessly.
"""

from __future__ import annotations

import os
from pathlib import Path
from urllib.parse import unquote, urlsplit

# Schemes GVFS exposes through its FUSE daemon. Anything else (http, steam,
# ...) is not a browsable file location and is left untouched.
REMOTE_SCHEMES = ("smb", "sftp", "ssh", "ftp", "ftps", "dav", "davs", "nfs")


def is_remote_url(value: str) -> bool:
    """Whether *value* is a network-share URL rather than a local path."""
    parsed = urlsplit(str(value or "").strip())
    return parsed.scheme.lower() in REMOTE_SCHEMES and bool(parsed.netloc)


def gvfs_root() -> Path:
    """Where GVFS mounts network shares as regular directories."""
    override = os.environ.get("GAMEHANDLER_GVFS_ROOT")
    if override:
        return Path(override)
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return Path(runtime) / "gvfs"
    return Path(f"/run/user/{os.getuid()}/gvfs")


def _smb_mount_names(host: str, share: str, user: str) -> list[str]:
    """Directory names gvfsd-fuse uses for an SMB share, most specific first."""
    host = host.lower()
    names = []
    if user:
        names.append(f"smb-share:server={host},share={share},user={user}")
    names.append(f"smb-share:server={host},share={share}")
    # Older gvfs lowercases the share component as well.
    lowered = share.lower()
    if lowered != share:
        names.append(f"smb-share:server={host},share={lowered}")
    return names


def _generic_mount_names(scheme: str, host: str, user: str, port: int | None) -> list[str]:
    """gvfsd-fuse names for sftp/ftp/dav style mounts, most specific first."""
    if scheme == "ssh":
        scheme = "sftp"
    ssl = ""
    if scheme in {"davs", "ftps"}:
        scheme = {"davs": "dav", "ftps": "ftp"}[scheme]
        ssl = ",ssl=true"
    host = host.lower()
    details = []
    if user:
        details.append(f"user={user}")
    details.append(f"host={host}")
    if port:
        details.append(f"port={port}")
    names = [f"{scheme}:{','.join(details)}{ssl}"]
    if user or port:
        names.append(f"{scheme}:host={host}{ssl}")
    return names


def _mount_candidates(url) -> list[Path]:
    root = gvfs_root()
    scheme = url.scheme.lower()
    host = (url.hostname or "").strip()
    user = unquote(url.username or "")
    if not host:
        return []
    parts = [unquote(part) for part in url.path.split("/") if part]
    candidates: list[Path] = []
    if scheme == "smb":
        if not parts:
            return []
        share, rest = parts[0], parts[1:]
        for name in _smb_mount_names(host, share, user):
            candidates.append(root.joinpath(name, *rest))
    else:
        for name in _generic_mount_names(scheme, host, user, url.port):
            candidates.append(root.joinpath(name, *parts))
    return candidates


def _scan_gvfs_for(url) -> Path | None:
    """Fall back to scanning the GVFS root for a mount matching *url*'s host.

    Mount-name details (user, port, domain) vary between gvfs versions, so
    when the constructed names miss, any mounted entry naming the same scheme
    and host is tried with the URL's path appended.
    """
    root = gvfs_root()
    try:
        entries = list(root.iterdir())
    except OSError:
        return None
    scheme = url.scheme.lower()
    if scheme == "ssh":
        scheme = "sftp"
    elif scheme == "davs":
        scheme = "dav"
    elif scheme == "ftps":
        scheme = "ftp"
    host = (url.hostname or "").lower()
    parts = [unquote(part) for part in url.path.split("/") if part]
    for entry in entries:
        name = entry.name.lower()
        if not name.startswith((f"{scheme}-", f"{scheme}:")):
            continue
        if f"server={host}" not in name and f"host={host}" not in name:
            continue
        if scheme == "smb" and parts:
            share = parts[0].lower()
            if f"share={share}" in name:
                candidate = entry.joinpath(*parts[1:])
            else:
                candidate = entry.joinpath(*parts)
        else:
            candidate = entry.joinpath(*parts)
        if candidate.exists():
            return candidate
    return None


def as_local_path(value: str) -> str:
    """Turn a picker result or hand-typed location into a local path.

    ``file://`` URLs are unwrapped, network-share URLs are mapped onto their
    GVFS FUSE mount when one exists, and anything already a plain path is
    returned unchanged. When a share URL cannot be resolved the original
    value is returned so the caller can show an accurate error.
    """
    raw = str(value or "").strip()
    if not raw:
        return ""
    parsed = urlsplit(raw)
    scheme = parsed.scheme.lower()
    if scheme == "file":
        return unquote(parsed.path) or raw
    if scheme in REMOTE_SCHEMES and parsed.netloc:
        for candidate in _mount_candidates(parsed):
            if candidate.exists():
                return str(candidate)
        found = _scan_gvfs_for(parsed)
        if found is not None:
            return str(found)
        return raw
    return raw


def unreachable_share_message(value: str) -> str:
    """A user-facing explanation for a share URL that has no local mount."""
    parsed = urlsplit(str(value or ""))
    host = parsed.hostname or "the server"
    return (
        f"{value} is a network location that is not mounted yet. Open {host} "
        "in your file manager once so the share is mounted, then try again."
    )


__all__ = [
    "REMOTE_SCHEMES",
    "as_local_path",
    "gvfs_root",
    "is_remote_url",
    "unreachable_share_message",
]
