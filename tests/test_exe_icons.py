"""Tests for reading an icon out of a Windows executable's resource section.

The parser walks attacker-influenced offsets, so the cases that matter are the
malformed ones: a truncated file, a group pointing at an image that is not
there, and a resource tree that claims more entries than it holds. None of
them may raise — a cover lookup that cannot read an icon just has no icon.
"""

import struct
import tempfile
import unittest
from pathlib import Path

from gamehandler.exe_icons import RT_GROUP_ICON, RT_ICON, extract_icon, icon_bytes

RSRC_RVA = 0x1000
RSRC_FILE_OFFSET = 0x400
_SUBDIRECTORY = 0x80000000
_LANGUAGE = 0x409


def dib_icon(side: int, fill: int) -> bytes:
    """A real BITMAPINFOHEADER icon image of *side*×*side* 32-bit pixels."""
    header = struct.pack(
        "<IiiHHIIiiII", 40, side, side * 2, 1, 32, 0, side * side * 4, 0, 0, 0, 0
    )
    pixels = bytes([fill, fill, fill, 0xFF]) * (side * side)
    mask = b"\x00" * (((side + 31) // 32) * 4 * side)
    return header + pixels + mask


def group_icon(entries: list[tuple[int, int, int]]) -> bytes:
    """A GRPICONDIR over ``(side, byte count, resource id)`` entries."""
    blob = struct.pack("<HHH", 0, 1, len(entries))
    for side, size, identifier in entries:
        blob += struct.pack(
            "<BBBBHHIH", side % 256, side % 256, 0, 0, 1, 32, size, identifier
        )
    return blob


def _directory(entries: list[tuple[int, int, bool]]) -> bytes:
    blob = struct.pack("<IIHHHH", 0, 0, 0, 0, 0, len(entries))
    for identifier, offset, is_directory in entries:
        blob += struct.pack("<II", identifier, offset | (_SUBDIRECTORY if is_directory else 0))
    return blob


def resource_section(images: dict[int, bytes], group: bytes) -> bytes:
    """A .rsrc section holding RT_ICON images and one RT_GROUP_ICON."""
    icon_ids = sorted(images)
    cursor = 0

    def take(size: int) -> int:
        nonlocal cursor
        start = cursor
        cursor += size
        return start

    root = take(16 + 2 * 8)
    icon_type = take(16 + len(icon_ids) * 8)
    group_type = take(16 + 8)
    icon_langs = [take(16 + 8) for _ in icon_ids]
    group_lang = take(16 + 8)
    icon_data = [take(16) for _ in icon_ids]
    group_data = take(16)
    icon_payloads = [take(len(images[i])) for i in icon_ids]
    group_payload = take(len(group))

    blob = bytearray(cursor)

    def put(offset: int, data: bytes) -> None:
        blob[offset : offset + len(data)] = data

    put(root, _directory([(RT_ICON, icon_type, True), (RT_GROUP_ICON, group_type, True)]))
    put(icon_type, _directory([(i, icon_langs[n], True) for n, i in enumerate(icon_ids)]))
    put(group_type, _directory([(1, group_lang, True)]))
    for index, identifier in enumerate(icon_ids):
        put(icon_langs[index], _directory([(_LANGUAGE, icon_data[index], False)]))
        put(
            icon_data[index],
            struct.pack("<IIII", RSRC_RVA + icon_payloads[index], len(images[identifier]), 0, 0),
        )
        put(icon_payloads[index], images[identifier])
    put(group_lang, _directory([(_LANGUAGE, group_data, False)]))
    put(group_data, struct.pack("<IIII", RSRC_RVA + group_payload, len(group), 0, 0))
    put(group_payload, group)
    return bytes(blob)


def icon_data_entry_offset(icon_count: int = 1) -> int:
    """Where :func:`resource_section` puts the first RT_ICON data entry.

    Mirrors the builder's own layout, so a test can corrupt one field without
    hard-coding an offset that drifts the moment the builder changes.
    """
    root = 16 + 2 * 8
    icon_type = 16 + icon_count * 8
    group_type = 16 + 8
    language_directories = (icon_count + 1) * (16 + 8)
    return root + icon_type + group_type + language_directories


def build_pe(section: bytes, *, plus: bool = False, resource_size: int | None = None) -> bytes:
    """Wrap a resource section in the smallest PE the parser will accept."""
    magic = 0x20B if plus else 0x10B
    optional_size = 240 if plus else 224
    count_offset, directory_offset = (108, 112) if plus else (92, 96)

    dos = bytearray(0x80)
    dos[0:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 0x80)

    coff = b"PE\x00\x00" + struct.pack(
        "<HHIIIHH", 0x8664 if plus else 0x14C, 1, 0, 0, 0, optional_size, 0x0102
    )
    optional = bytearray(optional_size)
    struct.pack_into("<H", optional, 0, magic)
    struct.pack_into("<I", optional, count_offset, 16)
    struct.pack_into(
        "<II",
        optional,
        directory_offset + 16,
        RSRC_RVA,
        len(section) if resource_size is None else resource_size,
    )

    header = struct.pack(
        "<8sIIIIIIHHI",
        b".rsrc\x00\x00\x00",
        len(section),
        RSRC_RVA,
        len(section),
        RSRC_FILE_OFFSET,
        0,
        0,
        0,
        0,
        0x40000040,
    )
    body = bytes(dos) + coff + bytes(optional) + header
    return body.ljust(RSRC_FILE_OFFSET, b"\x00") + section


def read_ico(blob: bytes) -> list[tuple[int, int, bytes]]:
    """``(width, height, payload)`` per image in an ``.ico`` file."""
    reserved, kind, count = struct.unpack_from("<HHH", blob, 0)
    assert reserved == 0 and kind == 1
    images = []
    for index in range(count):
        width, height, _c, _r, _p, _b, size, offset = struct.unpack_from(
            "<BBBBHHII", blob, 6 + index * 16
        )
        images.append((width, height, blob[offset : offset + size]))
    return images


class IconExtractionTests(unittest.TestCase):
    def test_group_and_images_round_trip_into_an_ico(self):
        small = dib_icon(16, 0x11)
        large = dib_icon(32, 0x22)
        section = resource_section(
            {1: small, 2: large},
            group_icon([(16, len(small), 1), (32, len(large), 2)]),
        )
        blob = icon_bytes(build_pe(section))
        self.assertIsNotNone(blob)
        images = read_ico(blob)
        self.assertEqual([(16, 16), (32, 32)], [(w, h) for w, h, _ in images])
        self.assertEqual([small, large], [payload for _w, _h, payload in images])

    def test_pe32_plus_executables_are_read_too(self):
        payload = dib_icon(16, 0x33)
        section = resource_section({1: payload}, group_icon([(16, len(payload), 1)]))
        blob = icon_bytes(build_pe(section, plus=True))
        self.assertIsNotNone(blob)
        self.assertEqual([payload], [image for _w, _h, image in read_ico(blob)])

    def test_a_group_entry_with_no_image_is_dropped_not_fatal(self):
        payload = dib_icon(16, 0x44)
        section = resource_section(
            {1: payload},
            group_icon([(16, len(payload), 1), (48, 999, 7)]),  # id 7 does not exist
        )
        images = read_ico(icon_bytes(build_pe(section)))
        self.assertEqual(1, len(images))
        self.assertEqual(payload, images[0][2])

    def test_the_largest_available_size_survives(self):
        biggest = dib_icon(64, 0x55)
        section = resource_section({1: biggest}, group_icon([(0, len(biggest), 1)]))
        images = read_ico(icon_bytes(build_pe(section)))
        # A 256-wide icon records its width as 0; the byte is copied verbatim.
        self.assertEqual((0, 0), images[0][:2])
        self.assertEqual(biggest, images[0][2])


class MalformedInputTests(unittest.TestCase):
    def test_a_file_that_is_not_a_pe_yields_no_icon(self):
        self.assertIsNone(icon_bytes(b"not an executable" * 8))

    def test_a_truncated_executable_yields_no_icon(self):
        payload = dib_icon(16, 0x66)
        section = resource_section({1: payload}, group_icon([(16, len(payload), 1)]))
        full = build_pe(section)
        self.assertIsNone(icon_bytes(full[: RSRC_FILE_OFFSET + 8]))

    def test_a_resource_pointing_past_the_section_yields_no_icon(self):
        payload = dib_icon(16, 0x77)
        section = resource_section({1: payload}, group_icon([(16, len(payload), 1)]))
        # Send the image's data entry past the end of the mapped section.
        entry = icon_data_entry_offset()
        broken = bytearray(build_pe(section))
        struct.pack_into("<I", broken, RSRC_FILE_OFFSET + entry, RSRC_RVA + 0xFFFF)
        self.assertIsNone(icon_bytes(bytes(broken)))

    def test_an_executable_without_resources_yields_no_icon(self):
        self.assertIsNone(icon_bytes(build_pe(b"", resource_size=0)))


class ExtractFromDiskTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_reads_an_executable_off_disk(self):
        payload = dib_icon(16, 0x88)
        section = resource_section({1: payload}, group_icon([(16, len(payload), 1)]))
        exe = self.root / "app.exe"
        exe.write_bytes(build_pe(section))
        self.assertEqual([payload], [image for _w, _h, image in read_ico(extract_icon(exe))])

    def test_missing_and_empty_files_are_not_errors(self):
        self.assertIsNone(extract_icon(self.root / "nothing.exe"))
        empty = self.root / "empty.exe"
        empty.write_bytes(b"")
        self.assertIsNone(extract_icon(empty))

    def test_a_directory_is_not_an_executable(self):
        self.assertIsNone(extract_icon(self.root))


if __name__ == "__main__":
    unittest.main()
