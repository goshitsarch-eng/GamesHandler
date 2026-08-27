import io
import tempfile
import unittest
import unittest.mock
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from gamehandler.installers import (
    APPS,
    INSTALLER_CATEGORIES,
    LAUNCHERS,
    MAX_SCAN_DEPTH,
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
    wait_for_installer,
    wait_for_prefix_idle,
    wineserver_binary,
)
from gamehandler.runners import ProtonRunner, WineRunner


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

    def test_raw_wine_installer_run_gets_no_proton_only_variables(self):
        """Mirrors the gating launch() applies; Proton vars mean nothing to Wine.

        The anti-cheat runtimes are stubbed as present, otherwise the lookup
        finds nothing on a test machine and the assertion proves nothing.
        """
        runner = WineRunner(binary="/usr/bin/wine")
        with unittest.mock.patch(
            "gamehandler.runners.find_anticheat_runtime", lambda *a, **k: "/opt/runtime"
        ):
            _argv, env = build_installer_command(
                runner, "/tmp/prefix", installer_by_id("steam"), "/tmp/SteamSetup.exe"
            )
        for key in ("PROTON_BATTLEYE_RUNTIME", "PROTON_EAC_RUNTIME"):
            self.assertNotIn(key, env, f"{key} leaked into a raw Wine installer run")
        # Wine's own knobs still apply.
        self.assertEqual(env["WINEESYNC"], "1")
        self.assertEqual(env["WINEFSYNC"], "1")

    def test_proton_installer_run_keeps_proton_features(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name) / "GE-Proton"
        (root / "files" / "bin").mkdir(parents=True)
        (root / "files" / "bin" / "wine").write_text("#!/bin/sh\n")
        (root / "proton").write_text("#!/usr/bin/env python\n")
        runner = ProtonRunner(root)
        with unittest.mock.patch(
            "gamehandler.runners.shutil.which", lambda name: "/usr/bin/umu-run"
        ):
            argv, env = build_installer_command(
                runner, "/tmp/prefix", installer_by_id("steam"), "/tmp/SteamSetup.exe"
            )
        self.assertEqual(Path(argv[0]).name, "umu-run")
        self.assertEqual(env["PROTONPATH"], str(root))

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


class PrefixScanBoundsTests(unittest.TestCase):
    """A finished prefix is huge; the fallback scan must stay bounded."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.prefix = Path(self.tmp.name)
        self.drive_c = self.prefix / "drive_c"

    def _touch(self, relative: str):
        target = self.drive_c / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("")
        return target

    def test_fallback_finds_a_relocated_executable(self):
        moved = self._touch("Program Files/Somewhere Else/steam.exe")
        found = find_prefix_exe(self.prefix, ("Program Files (x86)/Steam/steam.exe",))
        self.assertEqual(found, moved)

    def test_scan_does_not_descend_past_the_depth_limit(self):
        deep = "/".join(f"d{i}" for i in range(MAX_SCAN_DEPTH + 4))
        self._touch(f"Program Files/{deep}/steam.exe")
        self.assertIsNone(find_prefix_exe(self.prefix, ("Program Files/Steam/steam.exe",)))

    def test_windows_system_directories_are_skipped(self):
        self._touch("Program Files/windows/system32/steam.exe")
        self.assertIsNone(find_prefix_exe(self.prefix, ("Program Files/Steam/steam.exe",)))

    def test_extra_candidate_names_do_not_multiply_the_fallback_scan(self):
        """The scan used to restart from scratch for every expected basename."""
        target = self._touch("Program Files/Epic Games/EpicGamesLauncher.exe")
        for i in range(30):
            self._touch(f"Program Files/Filler{i}/thing.dat")

        def scan_reads(expected):
            reads = 0
            real_iterdir = Path.iterdir

            def counting(self):
                nonlocal reads
                reads += 1
                return real_iterdir(self)

            with unittest.mock.patch.object(Path, "iterdir", counting):
                found = find_prefix_exe(self.prefix, expected)
            return found, reads

        one_found, one_reads = scan_reads(("Program Files (x86)/Epic/A.exe",))
        many_found, many_reads = scan_reads(
            (
                "Program Files (x86)/Epic/A.exe",
                "Program Files (x86)/Epic/B.exe",
                "Program Files (x86)/Epic/C.exe",
                "Program Files (x86)/Epic/D.exe",
            )
        )
        self.assertIsNone(one_found)
        self.assertIsNone(many_found)
        # Four basenames instead of one must not cost four times the directory
        # reads; the exact-path probes add a little, the scan itself is shared.
        self.assertLess(many_reads, one_reads * 2)
        self.assertTrue(target.exists())


class CategoryConstantTests(unittest.TestCase):
    def test_exported_categories_match_the_catalog(self):
        self.assertEqual(INSTALLER_CATEGORIES, (LAUNCHERS, APPS))
        used = {item.category for item in installers()}
        self.assertTrue(used.issubset(set(INSTALLER_CATEGORIES)))

    def test_notes_are_present_where_the_vendor_needs_an_explanation(self):
        notes = {item.id: item.notes for item in installers()}
        self.assertTrue(notes["battlenet"])
        self.assertTrue(notes["discord"])


class ProtonPrefixLayoutTests(unittest.TestCase):
    """Proton keeps drive_c under "pfx"; an install there is still an install.

    umu-run passes WINEPREFIX to Proton as STEAM_COMPAT_DATA_PATH, so a title
    installed with any Proton runner lands one directory deeper than a raw Wine
    one. Searching only the Wine layout reported every Proton install as a
    missing executable.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.prefix = Path(self.tmp.name)

    def _install(self, drive_c: Path) -> Path:
        target = drive_c / "Program Files (x86)" / "Steam" / "steam.exe"
        target.parent.mkdir(parents=True)
        target.write_text("stub")
        return target

    def test_an_executable_under_pfx_is_found(self):
        target = self._install(self.prefix / "pfx" / "drive_c")
        found = find_prefix_exe(self.prefix, installer_by_id("steam").expected_exe)
        self.assertEqual(target, found)

    def test_the_wine_layout_still_works(self):
        target = self._install(self.prefix / "drive_c")
        found = find_prefix_exe(self.prefix, installer_by_id("steam").expected_exe)
        self.assertEqual(target, found)

    def test_the_proton_layout_wins_when_both_directories_exist(self):
        (self.prefix / "drive_c").mkdir()
        target = self._install(self.prefix / "pfx" / "drive_c")
        found = find_prefix_exe(self.prefix, installer_by_id("steam").expected_exe)
        self.assertEqual(target, found)

    def test_an_empty_expected_list_finds_nothing(self):
        self._install(self.prefix / "drive_c")
        self.assertIsNone(find_prefix_exe(self.prefix, []))


class InstallerHandoffTests(unittest.TestCase):
    """Vendor installers bootstrap: the process we start is not the wizard.

    Battle.net, the EA App, GOG Galaxy and Discord all unpack a payload, start
    the real installer as a separate process, and exit within seconds. Scanning
    the prefix at that moment reported "could not find the executable" for an
    install that then went on to succeed.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.prefix = Path(self.tmp.name)
        (self.prefix / "drive_c").mkdir()
        self.runner = WineRunner(binary="/usr/bin/wine")
        self.expected = installer_by_id("steam").expected_exe
        self.now = 0.0

    def _install_now(self):
        target = self.prefix / "drive_c" / "Program Files (x86)" / "Steam" / "steam.exe"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("stub")
        return target

    def _run(self, *, wait_seconds: float, install_after: float | None = None):
        """Drive one wait on a virtual clock.

        *wait_seconds* is how long the wineserver wait appears to take, and
        *install_after* the simulated moment the wizard writes its executable.
        The idle stub honours ``timeout`` so a leftover store client cannot
        hide an already-written launcher. Returns ``(found, elapsed, idle_calls)``.
        """
        self.now = 0.0
        remaining = [wait_seconds]
        idle_calls = []

        def clock():
            return self.now

        def maybe_install():
            if install_after is not None and self.now >= install_after:
                self._install_now()

        def sleep(seconds):
            self.now += seconds
            maybe_install()

        def idle(_runner, _env, timeout=6 * 60 * 60, **_kwargs):
            idle_calls.append(timeout)
            step = min(remaining[0], float(timeout))
            remaining[0] -= step
            self.now += step
            maybe_install()
            return remaining[0] <= 0

        with unittest.mock.patch("gamehandler.installers.wait_for_prefix_idle", idle):
            found = wait_for_installer(
                self.runner,
                {"WINEPREFIX": str(self.prefix)},
                self.prefix,
                self.expected,
                sleep=sleep,
                clock=clock,
            )
        return found, self.now, idle_calls

    def test_polls_until_the_wizard_writes_the_executable(self):
        found, _elapsed, _idle = self._run(wait_seconds=0, install_after=60)
        self.assertIsNotNone(found)
        self.assertTrue(found.is_file())

    def test_gives_up_at_the_deadline_instead_of_polling_forever(self):
        found, elapsed, _idle = self._run(wait_seconds=0)
        self.assertIsNone(found)
        self.assertLess(elapsed, 60 * 60, "the wizard window is bounded")

    def test_a_wineserver_that_really_waited_shortens_the_window(self):
        """The wizard is provably gone, so do not sit here for ten more minutes."""
        _found, elapsed, _idle = self._run(wait_seconds=30)
        self.assertLess(elapsed, 60)

    def test_a_wineserver_that_returned_at_once_proves_nothing(self):
        """umu runs Proton's wineserver where a host one cannot attach to it.

        Treating that instant answer as "the wizard has finished" would cut
        Proton installs off after a few seconds, straight back to the bug.
        """
        _found, elapsed, _idle = self._run(wait_seconds=0)
        self.assertGreater(elapsed, 60)

    def test_an_already_installed_executable_is_returned_immediately(self):
        target = self._install_now()
        found, elapsed, idle_calls = self._run(wait_seconds=30)
        self.assertEqual(target, found)
        self.assertEqual([], idle_calls)
        self.assertEqual(3.0, elapsed, "only the bootstrap handoff, no wineserver wait")

    def test_finds_the_launcher_while_wineserver_is_still_busy(self):
        """Steam/EA/Battle.net keep wineserver alive after writing the exe."""
        found, elapsed, idle_calls = self._run(
            wait_seconds=6 * 60 * 60,
            install_after=10,
        )
        self.assertIsNotNone(found)
        self.assertTrue(found.is_file())
        self.assertLess(elapsed, 60)
        self.assertGreater(len(idle_calls), 0)
        self.assertTrue(all(timeout <= 2 for timeout in idle_calls))


class WineserverLookupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_prefers_the_wineserver_beside_the_runners_own_wine(self):
        binaries = self.root / "files" / "bin"
        binaries.mkdir(parents=True)
        (binaries / "wine").write_text("#!/bin/sh\n")
        server = binaries / "wineserver"
        server.write_text("#!/bin/sh\n")
        server.chmod(0o755)
        self.assertEqual(str(server), wineserver_binary(WineRunner(binary=str(binaries / "wine"))))

    def test_falls_back_to_the_one_on_PATH(self):
        with unittest.mock.patch(
            "gamehandler.installers.shutil.which", return_value="/usr/bin/wineserver"
        ):
            self.assertEqual(
                "/usr/bin/wineserver", wineserver_binary(WineRunner(binary="/nowhere/wine"))
            )

    def test_no_wineserver_means_no_wait_rather_than_a_crash(self):
        with unittest.mock.patch("gamehandler.installers.shutil.which", return_value=None):
            self.assertFalse(
                wait_for_prefix_idle(WineRunner(binary=None), {"WINEPREFIX": str(self.root)})
            )

    def test_a_prefixless_environment_is_never_waited_on(self):
        with unittest.mock.patch(
            "gamehandler.installers.shutil.which", return_value="/usr/bin/wineserver"
        ):
            self.assertFalse(wait_for_prefix_idle(WineRunner(binary=None), {}))


if __name__ == "__main__":
    unittest.main()
