"""Read the icon a Windows executable carries in its PE resource section.

Store launchers and most Windows apps are not Steam store products, so a
Steam cover lookup finds nothing for them — or, worse, something else with a
similar name. The executable the vendor's own installer just wrote is a
source that is always present, always the right artwork, needs no network,
and redistributes nothing: the icon comes out of the user's own install.

Pure stdlib and UI-free, so the parser can be unit tested headlessly.
"""

from __future__ import annotations

import mmap
import struct
from pathlib import Path

RT_ICON = 3
RT_GROUP_ICON = 14

# A resource tree is data we did not write, and a truncated or hostile one
# must cost a moment rather than a gigabyte. Every walk is bounded.
MAX_EXECUTABLE_BYTES = 512 * 1024 * 1024
MAX_SECTIONS = 96
MAX_DIRECTORY_ENTRIES = 4096
MAX_ICON_IMAGES = 64
MAX_ICON_BYTES = 8 * 1024 * 1024

_PE32_MAGIC = 0x10B
_PE32PLUS_MAGIC = 0x20B
_RESOURCE_DIRECTORY_INDEX = 2
_SUBDIRECTORY_FLAG = 0x80000000
_NAME_FLAG = 0x80000000


def _sections_and_resource_root(view) -> tuple[list[tuple[int, int, int, int]], int, int] | None:
    """Parse the PE headers into ``(sections, resource_rva, resource_size)``."""
    size = len(view)
    if size < 64 or view[:2] != b"MZ":
        return None
    (lfanew,) = struct.unpack_from("<I", view, 0x3C)
    if lfanew <= 0 or lfanew + 24 > size or view[lfanew : lfanew + 4] != b"PE\x00\x00":
        return None

    (section_count,) = struct.unpack_from("<H", view, lfanew + 6)
    (optional_size,) = struct.unpack_from("<H", view, lfanew + 20)
    optional = lfanew + 24
    if optional_size < 2 or optional + optional_size > size:
        return None
    (magic,) = struct.unpack_from("<H", view, optional)
    if magic == _PE32_MAGIC:
        count_offset, directory_offset = 92, 96
    elif magic == _PE32PLUS_MAGIC:
        count_offset, directory_offset = 108, 112
    else:
        return None
    if optional + directory_offset + 8 * (_RESOURCE_DIRECTORY_INDEX + 1) > size:
        return None
    (directory_count,) = struct.unpack_from("<I", view, optional + count_offset)
    if directory_count <= _RESOURCE_DIRECTORY_INDEX:
        return None
    resource_rva, resource_size = struct.unpack_from(
        "<II", view, optional + directory_offset + 8 * _RESOURCE_DIRECTORY_INDEX
    )
    if not resource_rva or not resource_size:
        return None

    table = optional + optional_size
    sections: list[tuple[int, int, int, int]] = []
    for index in range(min(section_count, MAX_SECTIONS)):
        position = table + index * 40
        if position + 40 > size:
            break
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from(
            "<IIII", view, position + 8
        )
        # A section's mapped span can exceed its on-disk bytes (BSS-style
        # padding) and vice versa; containment uses the larger of the two,
        # reads are bounded by what is actually on disk.
        sections.append((virtual_address, max(virtual_size, raw_size), raw_pointer, raw_size))
    if not sections:
        return None
    return sections, resource_rva, resource_size


def _file_offset(sections, rva: int, length: int) -> int | None:
    """Translate a relative virtual address into a readable file offset."""
    for virtual_address, span, raw_pointer, raw_size in sections:
        if virtual_address <= rva < virtual_address + span:
            delta = rva - virtual_address
            if delta + length > raw_size:
                return None
            return raw_pointer + delta
    return None


def _directory_entries(view, base: int, offset: int) -> list[tuple[int, int, bool, bool]]:
    """``(id, target, is_directory, is_named)`` for one resource directory."""
    start = base + offset
    if start < 0 or start + 16 > len(view):
        return []
    named, numbered = struct.unpack_from("<HH", view, start + 12)
    total = min(named + numbered, MAX_DIRECTORY_ENTRIES)
    entries = []
    for index in range(total):
        position = start + 16 + index * 8
        if position + 8 > len(view):
            break
        name, target = struct.unpack_from("<II", view, position)
        entries.append(
            (
                name & ~_NAME_FLAG,
                target & ~_SUBDIRECTORY_FLAG,
                bool(target & _SUBDIRECTORY_FLAG),
                bool(name & _NAME_FLAG),
            )
        )
    return entries


def _collect_resources(view, base: int, offset: int) -> dict[int, tuple[int, int]]:
    """``{resource id: (data rva, size)}`` under one resource *type* directory."""
    found: dict[int, tuple[int, int]] = {}
    for resource_id, target, is_directory, is_named in _directory_entries(view, base, offset):
        if is_named or not is_directory:
            continue
        for _language, leaf, leaf_is_directory, _leaf_named in _directory_entries(
            view, base, target
        ):
            if leaf_is_directory:
                continue
            position = base + leaf
            if position + 16 > len(view):
                break
            data_rva, data_size = struct.unpack_from("<II", view, position)
            if data_size:
                # Only the first language of an icon is needed; they are the
                # same artwork with different locale metadata.
                found[resource_id] = (data_rva, data_size)
            break
    return found


def _build_ico(group: bytes, images: dict[int, bytes]) -> bytes | None:
    """Reassemble a real ``.ico`` file out of a GROUP_ICON and its images."""
    if len(group) < 6:
        return None
    reserved, kind, count = struct.unpack_from("<HHH", group, 0)
    if reserved != 0 or kind != 1 or not 1 <= count <= MAX_ICON_IMAGES:
        return None
    entries = []
    payload_bytes = 0
    for index in range(count):
        position = 6 + index * 14
        if position + 14 > len(group):
            break
        width, height, colours, pad, planes, bits, _size, identifier = struct.unpack_from(
            "<BBBBHHIH", group, position
        )
        payload = images.get(identifier)
        if not payload:
            continue
        payload_bytes += len(payload)
        if payload_bytes > MAX_ICON_BYTES:
            break
        entries.append((width, height, colours, pad, planes, bits, payload))
    if not entries:
        return None

    directory = b""
    blob = b""
    offset = 6 + 16 * len(entries)
    for width, height, colours, pad, planes, bits, payload in entries:
        directory += struct.pack(
            "<BBBBHHII", width, height, colours, pad, planes, bits, len(payload), offset
        )
        offset += len(payload)
        blob += payload
    return struct.pack("<HHH", 0, 1, len(entries)) + directory + blob


def icon_bytes(data) -> bytes | None:
    """Return the primary icon of an in-memory PE image as ``.ico`` bytes."""
    parsed = _sections_and_resource_root(data)
    if parsed is None:
        return None
    sections, resource_rva, _resource_size = parsed
    base = _file_offset(sections, resource_rva, 16)
    if base is None:
        return None

    types = {
        resource_id: target
        for resource_id, target, is_directory, is_named in _directory_entries(data, base, 0)
        if is_directory and not is_named
    }
    if RT_GROUP_ICON not in types or RT_ICON not in types:
        return None

    groups = _collect_resources(data, base, types[RT_GROUP_ICON])
    icons = _collect_resources(data, base, types[RT_ICON])
    if not groups or not icons:
        return None

    def read(entry: tuple[int, int]) -> bytes | None:
        rva, size = entry
        if size <= 0 or size > MAX_ICON_BYTES:
            return None
        offset = _file_offset(sections, rva, size)
        if offset is None:
            return None
        return bytes(data[offset : offset + size])

    images = {}
    total = 0
    for identifier, entry in icons.items():
        payload = read(entry)
        if payload is None:
            continue
        total += len(payload)
        if total > MAX_ICON_BYTES:
            break
        images[identifier] = payload
    if not images:
        return None

    # Windows shows the lowest-numbered icon group as the application icon.
    for identifier in sorted(groups):
        group = read(groups[identifier])
        if group is None:
            continue
        ico = _build_ico(group, images)
        if ico:
            return ico
    return None


def extract_icon(exe_path: str | Path) -> bytes | None:
    """Return *exe_path*'s embedded icon as ``.ico`` bytes, or ``None``."""
    path = Path(exe_path)
    try:
        size = path.stat().st_size
    except OSError:
        return None
    if size < 64 or size > MAX_EXECUTABLE_BYTES:
        return None
    try:
        with path.open("rb") as stream:
            # Executables run to hundreds of megabytes; map the file instead
            # of reading it so a cover lookup never buffers a whole game.
            with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as view:
                return icon_bytes(view)
    except (OSError, ValueError, struct.error):
        return None


__all__ = ["RT_GROUP_ICON", "RT_ICON", "extract_icon", "icon_bytes"]
