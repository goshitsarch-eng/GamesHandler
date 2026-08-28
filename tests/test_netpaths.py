"""Tests for network-share resolution and the launch paths that rely on it.

The bug these lock in: file choosers list SMB/SFTP shares but hand back
``smb://`` URLs, and the old UI could only ask another application to open
them. Resolving through the GVFS FUSE mount makes a share-hosted executable
launch like a local one.
"""

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from gamehandler.models import Game
from gamehandler.netpaths import (
    as_local_path,
    gvfs_root,
    is_remote_url,
    unreachable_share_message,
)
from gamehandler.runners import WineRunner, resolve_game_paths, tool_command


class UrlDetectionTests(unittest.TestCase):
    def test_share_urls_are_remote(self):
        for url in (
            "smb://nas/games/Doom/doom.exe",
            "sftp://user@host/srv/game.exe",
            "davs://cloud.example.com/games/setup.exe",
        ):
            self.assertTrue(is_remote_url(url), url)

    def test_local_paths_and_other_schemes_are_not(self):
        for value in ("", "/home/me/game.exe", "file:///home/me/game.exe", "https://x/y"):
            self.assertFalse(is_remote_url(value), value)


class AsLocalPathTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        patcher = mock.patch.dict(
            os.environ, {"GAMEHANDLER_GVFS_ROOT": str(self.root)}
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_plain_paths_pass_through(self):
        self.assertEqual(as_local_path("/games/doom.exe"), "/games/doom.exe")
        self.assertEqual(as_local_path(""), "")

    def test_file_urls_are_unwrapped_and_unquoted(self):
        self.assertEqual(
            as_local_path("file:///home/me/My%20Games/doom.exe"),
            "/home/me/My Games/doom.exe",
        )

    def test_smb_url_resolves_to_the_gvfs_mount(self):
        mount = self.root / "smb-share:server=nas,share=games"
        target = mount / "Doom" / "doom.exe"
        target.parent.mkdir(parents=True)
        target.write_text("MZ")
        self.assertEqual(
            as_local_path("smb://NAS/games/Doom/doom.exe"), str(target)
        )

    def test_smb_url_with_user_prefers_the_user_mount(self):
        mount = self.root / "smb-share:server=nas,share=games,user=me"
        target = mount / "doom.exe"
        target.parent.mkdir(parents=True)
        target.write_text("MZ")
        self.assertEqual(as_local_path("smb://me@nas/games/doom.exe"), str(target))

    def test_sftp_url_resolves_to_the_gvfs_mount(self):
        mount = self.root / "sftp:host=host"
        target = mount / "srv" / "game.exe"
        target.parent.mkdir(parents=True)
        target.write_text("MZ")
        self.assertEqual(as_local_path("sftp://host/srv/game.exe"), str(target))

    def test_scan_fallback_finds_mounts_with_extra_details(self):
        mount = self.root / "smb-share:domain=WORKGROUP,server=nas,share=games"
        target = mount / "doom.exe"
        target.parent.mkdir(parents=True)
        target.write_text("MZ")
        self.assertEqual(as_local_path("smb://nas/games/doom.exe"), str(target))

    def test_unmounted_share_returns_the_original_url(self):
        url = "smb://nowhere/games/doom.exe"
        self.assertEqual(as_local_path(url), url)
        self.assertIn("nowhere", unreachable_share_message(url))

    def test_gvfs_root_env_override_wins(self):
        self.assertEqual(gvfs_root(), self.root)


class ResolveGamePathsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        patcher = mock.patch.dict(
            os.environ, {"GAMEHANDLER_GVFS_ROOT": str(self.root)}
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_share_hosted_exe_is_rewritten_to_the_mounted_path(self):
        mount = self.root / "smb-share:server=nas,share=games"
        exe = mount / "Doom" / "doom.exe"
        exe.parent.mkdir(parents=True)
        exe.write_text("MZ")
        game = Game(name="Doom", exe_path="smb://nas/games/Doom/doom.exe")
        resolved = resolve_game_paths(game)
        self.assertEqual(resolved.exe_path, str(exe))
        # The original entry is left untouched for persistence.
        self.assertEqual(game.exe_path, "smb://nas/games/Doom/doom.exe")

    def test_unmounted_share_fails_with_an_instruction(self):
        game = Game(name="Doom", exe_path="smb://nas/games/doom.exe")
        with self.assertRaises(RuntimeError) as caught:
            resolve_game_paths(game)
        self.assertIn("not mounted", str(caught.exception))

    def test_local_games_come_back_unchanged(self):
        game = Game(name="Doom", exe_path="/games/doom.exe")
        self.assertIs(resolve_game_paths(game), game)


class ToolEnvironmentTests(unittest.TestCase):
    """Winetricks must run against the game's own Wine, not whatever is first
    on PATH — a prefix built by a newer Proton/Wine build is otherwise refused
    or silently migrated by the system Wine."""

    def test_tool_command_pins_wine_and_wineserver(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        bindir = Path(tmp.name) / "bin"
        bindir.mkdir()
        wine = bindir / "wine"
        wine.write_text("#!/bin/sh\n")
        wine.chmod(0o755)
        server = bindir / "wineserver"
        server.write_text("#!/bin/sh\n")
        server.chmod(0o755)

        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary=str(wine))
        game = Game(name="Steam", prefix_path=str(Path(tmp.name) / "prefix"))
        argv, env = tool_command(game, manager, "winecfg")
        self.assertEqual(env["WINE"], str(wine))
        self.assertEqual(env["WINESERVER"], str(server))
        self.assertEqual(argv[0], str(wine))


if __name__ == "__main__":
    unittest.main()
