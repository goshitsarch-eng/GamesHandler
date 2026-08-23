import tempfile
import unittest
from pathlib import Path

from gamehandler.installers import (
    APPS,
    LAUNCHERS,
    build_installer_command,
    find_prefix_exe,
    game_from_install,
    installer_argv,
    installer_by_id,
    installers,
    resolve_case_insensitive,
    search_installers,
)
from gamehandler.runners import WineRunner


EXPECTED_IDS = (
    "battlenet",
    "epic",
    "ea-app",
    "ubisoft",
    "gog",
    "amazon",
    "rockstar",
    "steam",
    "discord",
)


class CatalogTests(unittest.TestCase):
    def test_catalog_contains_eight_launchers_and_discord(self):
        ids = [item.id for item in installers()]
        self.assertEqual(ids, list(EXPECTED_IDS))
        self.assertEqual(sum(1 for item in installers() if item.category == LAUNCHERS), 8)
        self.assertEqual(sum(1 for item in installers() if item.category == APPS), 1)
        self.assertEqual(installer_by_id("discord").category, APPS)

    def test_every_recipe_has_official_https_url_and_expected_exe(self):
        for item in installers():
            self.assertTrue(item.download_url.startswith("https://"), item.id)
            self.assertTrue(item.filename)
            self.assertIn(item.kind, {"exe", "msi"}, item.id)
            self.assertTrue(item.expected_exe, item.id)
            self.assertTrue(item.name)
            self.assertTrue(item.description)

    def test_epic_is_msi(self):
        self.assertEqual(installer_by_id("epic").kind, "msi")
        self.assertTrue(installer_by_id("epic").filename.lower().endswith(".msi"))

    def test_search_and_category_filter(self):
        hits = search_installers("epic")
        self.assertEqual([item.id for item in hits], ["epic"])
        apps = search_installers(category=APPS)
        self.assertEqual([item.id for item in apps], ["discord"])
        none = search_installers("no-such-launcher")
        self.assertEqual(none, [])


class MsiexecArgvTests(unittest.TestCase):
    def test_msi_uses_msiexec(self):
        argv = installer_argv("/usr/bin/wine", "/tmp/Epic.msi", "msi")
        self.assertEqual(argv, ["/usr/bin/wine", "msiexec", "/i", "/tmp/Epic.msi"])

    def test_exe_passes_installer_path(self):
        argv = installer_argv("/usr/bin/wine", "/tmp/SteamSetup.exe", "exe")
        self.assertEqual(argv, ["/usr/bin/wine", "/tmp/SteamSetup.exe"])

    def test_extra_arguments_are_appended(self):
        argv = installer_argv("/usr/bin/wine", "/tmp/setup.exe", "exe", "/S")
        self.assertEqual(argv[-1], "/S")

    def test_build_installer_command_sets_prefix_and_sync(self):
        runner = WineRunner(binary="/usr/bin/wine")
        installer = installer_by_id("epic")
        argv, env = build_installer_command(
            runner, "/tmp/prefix", installer, "/tmp/Epic.msi"
        )
        self.assertEqual(argv[:4], ["/usr/bin/wine", "msiexec", "/i", "/tmp/Epic.msi"])
        self.assertEqual(env["WINEPREFIX"], "/tmp/prefix")
        self.assertEqual(env["WINEESYNC"], "1")
        self.assertEqual(env["WINEFSYNC"], "1")


class PrefixLookupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.prefix = Path(self.tmp.name)
        self.drive_c = self.prefix / "drive_c"
        self.drive_c.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def test_case_insensitive_path_resolution(self):
        target = self.drive_c / "Program Files (x86)" / "Steam" / "steam.exe"
        target.parent.mkdir(parents=True)
        target.write_text("stub")
        found = resolve_case_insensitive(self.drive_c, "program files (x86)/STEAM/Steam.EXE")
        self.assertEqual(found, target)
        located = find_prefix_exe(
            self.prefix, ("Program Files (x86)/Steam/steam.exe",)
        )
        self.assertEqual(located, target)

    def test_user_profile_fallback(self):
        target = (
            self.drive_c
            / "users"
            / "alice"
            / "AppData"
            / "Local"
            / "Discord"
            / "Update.exe"
        )
        target.parent.mkdir(parents=True)
        target.write_text("stub")
        located = find_prefix_exe(
            self.prefix,
            ("users/steamuser/AppData/Local/Discord/Update.exe",),
        )
        self.assertEqual(located, target)

    def test_basename_search_under_program_files(self):
        target = self.drive_c / "Program Files" / "GOG Galaxy" / "GalaxyClient.exe"
        target.parent.mkdir(parents=True)
        target.write_text("stub")
        located = find_prefix_exe(
            self.prefix,
            ("Program Files (x86)/GOG Galaxy/GalaxyClient.exe",),
        )
        self.assertEqual(located, target)

    def test_missing_prefix_returns_none(self):
        self.assertIsNone(find_prefix_exe(self.prefix / "missing", ("Steam/steam.exe",)))


class GameFromInstallTests(unittest.TestCase):
    def test_library_entry_uses_recipe_defaults(self):
        installer = installer_by_id("steam")
        game = game_from_install(
            installer,
            "/pfx/drive_c/Program Files (x86)/Steam/steam.exe",
            "/pfx",
            "GE-Proton9-5",
            game_id="abc123",
        )
        self.assertEqual(game.id, "abc123")
        self.assertEqual(game.name, "Steam")
        self.assertEqual(game.runner, "GE-Proton9-5")
        self.assertEqual(game.prefix_path, "/pfx")
        self.assertEqual(game.category, LAUNCHERS)
        self.assertTrue(game.esync)
        self.assertTrue(game.fsync)
        self.assertFalse(game.is_linux)

    def test_discord_update_entry_keeps_required_process_start_arguments(self):
        installer = installer_by_id("discord")
        game = game_from_install(
            installer,
            "/pfx/drive_c/users/steamuser/AppData/Local/Discord/Update.exe",
            "/pfx",
            "wine-system",
        )
        self.assertEqual(game.arguments, "--processStart Discord.exe")


if __name__ == "__main__":
    unittest.main()
