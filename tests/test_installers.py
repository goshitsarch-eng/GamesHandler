import io
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from gamehandler.installers import (
    APPS,
    LAUNCHERS,
    build_installer_command,
    download_installer,
    find_prefix_exe,
    game_from_install,
    installer_argv,
    installer_by_id,
    installers,
    resolve_case_insensitive,
    search_installers,
    verify_installer_authenticity,
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
            self.assertTrue(item.allowed_hosts, item.id)
            self.assertTrue(item.publishers, item.id)

    def test_epic_is_msi(self):
        self.assertEqual(installer_by_id("epic").kind, "msi")
        self.assertTrue(installer_by_id("epic").filename.lower().endswith(".msi"))

    def test_gog_requires_the_full_publisher_identity(self):
        self.assertEqual(
            installer_by_id("gog").publishers,
            ("CN=GOG  sp. z o.o,O=GOG  sp. z o.o",),
        )

    def test_search_and_category_filter(self):
        hits = search_installers("epic")
        self.assertEqual([item.id for item in hits], ["epic"])
        apps = search_installers(category=APPS)
        self.assertEqual([item.id for item in apps], ["discord"])
        none = search_installers("no-such-launcher")
        self.assertEqual(none, [])


class FakeResponse(io.BytesIO):
    def __init__(self, body: bytes, final_url: str):
        super().__init__(body)
        self._final_url = final_url
        self.headers = {"Content-Length": str(len(body))}

    def geturl(self):
        return self._final_url


class DownloadSecurityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dest = Path(self.tmp.name)
        self.installer = installer_by_id("steam")

    def tearDown(self):
        self.tmp.cleanup()

    def test_download_is_authenticated_before_atomic_install(self):
        body = b"MZ" + b"safe-installer"
        response = FakeResponse(body, self.installer.download_url)
        with (
            mock.patch("gamehandler.installers.urlopen", return_value=response),
            mock.patch("gamehandler.installers.verify_installer_authenticity") as verify,
        ):
            target = download_installer(self.installer, self.dest)
        self.assertEqual(target.read_bytes(), body)
        verify.assert_called_once()
        self.assertEqual([item for item in self.dest.iterdir() if item.name.endswith(".part")], [])

    def test_untrusted_redirect_is_rejected_and_removed(self):
        response = FakeResponse(b"MZpayload", "https://evil.example/SteamSetup.exe")
        with (
            mock.patch("gamehandler.installers.urlopen", return_value=response),
            mock.patch("gamehandler.installers.verify_installer_authenticity") as verify,
            self.assertRaisesRegex(RuntimeError, "untrusted download origin"),
        ):
            download_installer(self.installer, self.dest)
        verify.assert_not_called()
        self.assertFalse((self.dest / self.installer.filename).exists())
        self.assertEqual([item for item in self.dest.iterdir() if item.name.endswith(".part")], [])

    def test_non_pe_payload_is_rejected_before_signature_check(self):
        response = FakeResponse(b"not-an-executable", self.installer.download_url)
        with (
            mock.patch("gamehandler.installers.urlopen", return_value=response),
            mock.patch("gamehandler.installers.verify_installer_authenticity") as verify,
            self.assertRaisesRegex(RuntimeError, "not a valid EXE"),
        ):
            download_installer(self.installer, self.dest)
        verify.assert_not_called()

    def test_signature_requires_approved_publisher(self):
        result = SimpleNamespace(
            returncode=0,
            stdout="Signature verification: ok\nSubject: /O=Impostor Corp./CN=Impostor Corp.",
        )
        with (
            mock.patch("gamehandler.installers.shutil.which", return_value="/usr/bin/osslsigncode"),
            mock.patch("gamehandler.installers.subprocess.run", return_value=result),
            self.assertRaisesRegex(RuntimeError, "approved publisher"),
        ):
            verify_installer_authenticity(self.installer, self.dest / "SteamSetup.exe")

    def test_signature_accepts_verified_approved_publisher(self):
        result = SimpleNamespace(
            returncode=0,
            stdout="Signature verification: ok\nSubject: /O=Valve Corp./CN=Valve Corp.",
        )
        with (
            mock.patch("gamehandler.installers.shutil.which", return_value="/usr/bin/osslsigncode"),
            mock.patch("gamehandler.installers.subprocess.run", return_value=result),
        ):
            verify_installer_authenticity(self.installer, self.dest / "SteamSetup.exe")


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
