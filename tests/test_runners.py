import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from gamehandler.models import Game
from gamehandler.runners import (
    ProtonManager,
    RunnerManager,
    SYSTEM_WINE,
    WineRunner,
    apply_launch_options,
    asset_matches,
    build_linux_command,
    create_desktop_shortcut,
    families,
    family_by_id,
    find_anticheat_runtime,
    find_wine_binary,
    install_bundled_dxvk,
    launch,
    merge_dll_overrides,
    normalize_desktop_size,
    parse_env_block,
    pick_asset,
    virtual_desktop_argv,
)


class WineCommandTests(unittest.TestCase):
    def test_build_command_uses_wine_binary_and_exe(self):
        runner = WineRunner(binary="/usr/bin/wine")
        game = Game(name="App", exe_path="/games/app.exe", arguments="-fullscreen -dx11")
        argv, env = runner.build_command(game)
        self.assertEqual(argv, ["/usr/bin/wine", "/games/app.exe", "-fullscreen", "-dx11"])
        self.assertIn("WINEPREFIX", env)

    def test_prefix_defaults_per_game(self):
        runner = WineRunner(binary="/usr/bin/wine")
        game = Game(name="App", exe_path="/games/app.exe")
        _, env = runner.build_command(game)
        self.assertTrue(env["WINEPREFIX"].endswith(game.id))

    def test_explicit_prefix_is_respected(self):
        runner = WineRunner(binary="/usr/bin/wine")
        game = Game(name="App", exe_path="/x.exe", prefix_path="/custom/prefix")
        _, env = runner.build_command(game)
        self.assertEqual(env["WINEPREFIX"], "/custom/prefix")

    def test_unavailable_runner_raises(self):
        runner = WineRunner(binary=None)
        with self.assertRaises(RuntimeError):
            runner.build_command(Game(name="App", exe_path="/x.exe"))


class RunnerManagerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.runners_dir = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def _make_fake_proton(self, tag):
        binpath = self.runners_dir / tag / "files" / "bin"
        binpath.mkdir(parents=True)
        wine = binpath / "wine"
        wine.write_text("#!/bin/sh\n")
        wine.chmod(0o755)

    def test_discovers_installed_protons(self):
        self._make_fake_proton("GE-Proton9-5")
        self._make_fake_proton("GE-Proton8-32")
        manager = RunnerManager(self.runners_dir)
        ids = [p.id for p in manager.installed_protons()]
        self.assertIn("GE-Proton9-5", ids)
        self.assertIn("GE-Proton8-32", ids)

    def test_choices_include_system_wine(self):
        self._make_fake_proton("GE-Proton9-5")
        manager = RunnerManager(self.runners_dir)
        choices = dict(manager.choices())
        self.assertEqual(choices[SYSTEM_WINE], "System Wine")
        self.assertIn("GE-Proton9-5", choices)

    def test_get_proton_builds_command_with_bundled_wine(self):
        self._make_fake_proton("GE-Proton9-5")
        manager = RunnerManager(self.runners_dir)
        game = Game(name="App", exe_path="/g/app.exe", runner="GE-Proton9-5")
        argv, _ = manager.get("GE-Proton9-5").build_command(game)
        self.assertTrue(argv[0].endswith("GE-Proton9-5/files/bin/wine"))
        self.assertEqual(argv[1], "/g/app.exe")

    def test_get_missing_runner_falls_back_to_system_wine(self):
        manager = RunnerManager(self.runners_dir)
        runner = manager.get("GE-Proton-does-not-exist")
        self.assertIsInstance(runner, WineRunner)


class ProtonManagerTests(unittest.TestCase):
    FIXTURE = [
        {
            "tag_name": "GE-Proton9-5",
            "assets": [
                {"name": "notes.txt", "browser_download_url": "u0", "size": 10},
                {
                    "name": "GE-Proton9-5.tar.gz",
                    "browser_download_url": "https://example/GE-Proton9-5.tar.gz",
                    "size": 419430400,
                },
            ],
        },
        {
            "tag_name": "GE-Proton8-32",
            "assets": [
                {
                    "name": "GE-Proton8-32.tar.gz",
                    "browser_download_url": "https://example/GE-Proton8-32.tar.gz",
                    "size": 400000000,
                }
            ],
        },
        {"tag_name": "no-assets", "assets": []},
    ]

    def test_parse_releases_selects_tarball_asset(self):
        releases = ProtonManager.parse_releases(self.FIXTURE)
        self.assertEqual([r.tag for r in releases], ["GE-Proton9-5", "GE-Proton8-32"])
        first = releases[0]
        self.assertEqual(first.name, "GE-Proton9-5.tar.gz")
        self.assertEqual(first.download_url, "https://example/GE-Proton9-5.tar.gz")
        self.assertAlmostEqual(first.size_mb, 400.0, places=1)

    def test_is_installed(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        manager = ProtonManager(Path(tmp.name))
        self.assertFalse(manager.is_installed("GE-Proton9-5"))
        binpath = Path(tmp.name) / "GE-Proton9-5" / "files" / "bin"
        binpath.mkdir(parents=True)
        (binpath / "wine").write_text("#!/bin/sh\n")
        self.assertTrue(manager.is_installed("GE-Proton9-5"))


KRON4EK_ASSETS = [
    {"name": "sha256sums.txt", "browser_download_url": "u0", "size": 10},
    {
        "name": "wine-11.15-amd64.tar.xz",
        "browser_download_url": "https://example/wine-vanilla.tar.xz",
        "size": 100,
    },
    {
        "name": "wine-11.15-amd64-wow64.tar.xz",
        "browser_download_url": "https://example/wine-wow64.tar.xz",
        "size": 90,
    },
    {
        "name": "wine-11.15-staging-amd64.tar.xz",
        "browser_download_url": "https://example/wine-staging.tar.xz",
        "size": 110,
    },
    {
        "name": "wine-11.15-staging-tkg-amd64.tar.xz",
        "browser_download_url": "https://example/wine-tkg.tar.xz",
        "size": 120,
    },
    {
        "name": "wine-11.15-proton-amd64.tar.xz",
        "browser_download_url": "https://example/wine-proton.tar.xz",
        "size": 130,
    },
]


class FamilyCatalogTests(unittest.TestCase):
    def test_expected_families_are_present(self):
        ids = [family.id for family in families()]
        for expected in (
            "proton-ge",
            "proton-ge-rtsp",
            "proton-cachyos",
            "proton-em",
            "wine-vanilla",
            "wine-staging",
            "wine-staging-tkg",
            "wine-proton",
        ):
            self.assertIn(expected, ids)

    def test_kron4ek_families_pick_distinct_assets(self):
        cases = {
            "wine-vanilla": "wine-11.15-amd64.tar.xz",
            "wine-staging": "wine-11.15-staging-amd64.tar.xz",
            "wine-staging-tkg": "wine-11.15-staging-tkg-amd64.tar.xz",
            "wine-proton": "wine-11.15-proton-amd64.tar.xz",
        }
        for family_id, asset_name in cases.items():
            chosen = pick_asset(KRON4EK_ASSETS, family_by_id(family_id))
            self.assertIsNotNone(chosen, family_id)
            self.assertEqual(chosen["name"], asset_name)

    def test_wow64_and_v3_builds_are_excluded(self):
        self.assertFalse(
            asset_matches("wine-11.15-amd64-wow64.tar.xz", family_by_id("wine-vanilla"))
        )
        self.assertFalse(
            asset_matches(
                "proton-cachyos-11.0-v3-x86_64.tar.xz", family_by_id("proton-cachyos")
            )
        )

    def test_parse_releases_honours_family(self):
        payload = [{"tag_name": "11.15", "assets": KRON4EK_ASSETS}]
        vanilla = ProtonManager.parse_releases(payload, "wine-vanilla")
        staging = ProtonManager.parse_releases(payload, "wine-staging")
        self.assertEqual(vanilla[0].name, "wine-11.15-amd64.tar.xz")
        self.assertEqual(vanilla[0].family_id, "wine-vanilla")
        self.assertEqual(vanilla[0].install_id, "wine-vanilla~f11.15")
        self.assertEqual(staging[0].name, "wine-11.15-staging-amd64.tar.xz")
        self.assertEqual(staging[0].install_id, "wine-staging~f11.15")

    def test_ge_proton_keeps_tag_as_install_id(self):
        release = ProtonManager.parse_releases(
            [
                {
                    "tag_name": "GE-Proton9-5",
                    "assets": [
                        {
                            "name": "GE-Proton9-5.tar.gz",
                            "browser_download_url": "https://example/ge.tar.gz",
                            "size": 1,
                        }
                    ],
                }
            ]
        )[0]
        self.assertEqual(release.install_id, "GE-Proton9-5")


class WineLayoutTests(unittest.TestCase):
    def test_finds_kron4ek_bin_wine(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name) / "wine-11.15-amd64"
        binary = root / "bin" / "wine"
        binary.parent.mkdir(parents=True)
        binary.write_text("#!/bin/sh\n")
        self.assertEqual(find_wine_binary(root), binary)

    def test_runner_manager_discovers_wine_layout(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        binary = root / "wine-vanilla-11.15" / "bin" / "wine"
        binary.parent.mkdir(parents=True)
        binary.write_text("#!/bin/sh\n")
        manager = RunnerManager(root)
        ids = [runner.id for runner in manager.installed_protons()]
        self.assertIn("wine-vanilla-11.15", ids)
        game = Game(name="App", exe_path="/g/app.exe", runner="wine-vanilla-11.15")
        argv, _ = manager.get("wine-vanilla-11.15").build_command(game)
        self.assertTrue(argv[0].endswith("wine-vanilla-11.15/bin/wine"))

    def test_uninstall_removes_build(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        target = root / "GE-Proton9-5" / "files" / "bin"
        target.mkdir(parents=True)
        (target / "wine").write_text("#!/bin/sh\n")
        manager = ProtonManager(root)
        self.assertTrue(manager.is_installed("GE-Proton9-5"))
        manager.uninstall("GE-Proton9-5")
        self.assertFalse(manager.is_installed("GE-Proton9-5"))


class LaunchOptionTests(unittest.TestCase):
    def test_linux_native_command(self):
        game = Game(name="Native", exe_path="/games/celeste", arguments="--fullscreen", kind="linux")
        argv, _ = build_linux_command(game)
        self.assertEqual(argv, ["/games/celeste", "--fullscreen"])

    def test_wayland_and_hdr_environment(self):
        game = Game(name="App", wayland=True, hdr=True, prefer_sdl=True)
        argv, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {"HOME": "/tmp"})
        self.assertEqual(env["PROTON_ENABLE_WAYLAND"], "1")
        self.assertEqual(env["PROTON_ENABLE_HDR"], "1")
        self.assertEqual(env["PROTON_ENABLE_HIDAPI"], "1")
        self.assertEqual(env["WINEESYNC"], "1")
        self.assertEqual(env["WINEFSYNC"], "1")
        self.assertNotIn("PROTON_NO_ESYNC", env)
        self.assertNotIn("PROTON_NO_FSYNC", env)
        self.assertEqual(argv[0], "/usr/bin/wine")

    def test_disabled_esync_fsync_set_proton_off_flags(self):
        game = Game(name="App", esync=False, fsync=False)
        _, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {"HOME": "/tmp"})
        self.assertEqual(env["WINEESYNC"], "0")
        self.assertEqual(env["WINEFSYNC"], "0")
        self.assertEqual(env["PROTON_NO_ESYNC"], "1")
        self.assertEqual(env["PROTON_NO_FSYNC"], "1")

    def test_linux_native_skips_wine_sync_env(self):
        game = Game(name="Native", kind="linux", esync=True, fsync=True)
        _, env = apply_launch_options(game, ["/games/celeste"], {"HOME": "/tmp"})
        self.assertNotIn("WINEESYNC", env)
        self.assertNotIn("WINEFSYNC", env)
        self.assertNotIn("PROTON_NO_ESYNC", env)
        self.assertNotIn("PROTON_USE_WINED3D", env)
        self.assertNotIn("PROTON_BATTLEYE_RUNTIME", env)

    def test_dxvk_off_uses_wined3d(self):
        game = Game(name="App", dxvk=False)
        _, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {"HOME": "/tmp"})
        self.assertEqual(env["PROTON_USE_WINED3D"], "1")
        self.assertIn("dxgi,d3d11,d3d10core,d3d9=b", env["WINEDLLOVERRIDES"])

    def test_vkd3d_off_overrides_d3d12(self):
        game = Game(name="App", vkd3d=False)
        _, env = apply_launch_options(
            game,
            ["/usr/bin/wine", "/g/app.exe"],
            {"WINEDLLOVERRIDES": "winemenubuilder.exe=d"},
        )
        self.assertIn("d3d12,d3d12core=b", env["WINEDLLOVERRIDES"])
        self.assertIn("winemenubuilder.exe=d", env["WINEDLLOVERRIDES"])

    def test_nvapi_and_fsr_environment(self):
        game = Game(name="App", nvapi=True, fsr=True)
        _, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {})
        self.assertEqual(env["PROTON_ENABLE_NVAPI"], "1")
        self.assertEqual(env["DXVK_ENABLE_NVAPI"], "1")
        self.assertEqual(env["DXVK_NVAPIHACK"], "0")
        self.assertEqual(env["WINE_FULLSCREEN_FSR"], "1")
        self.assertEqual(env["WINE_FULLSCREEN_FSR_STRENGTH"], "2")

    def test_anticheat_off_clears_runtime_paths(self):
        game = Game(name="App", battleye=False, eac=False)
        _, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {})
        self.assertEqual(env["PROTON_BATTLEYE_RUNTIME"], "")
        self.assertEqual(env["PROTON_EAC_RUNTIME"], "")

    def test_virtual_desktop_wraps_explorer(self):
        game = Game(name="Half-Life", virtual_desktop=True, virtual_desktop_size="1280x720")
        argv, _ = apply_launch_options(game, ["/usr/bin/wine", "/g/hl.exe"], {})
        self.assertEqual(argv, ["/usr/bin/wine", "explorer", "/desktop=HalfLife,1280x720", "/g/hl.exe"])

    def test_gamescope_wraps_command_and_hdr_flag(self):
        game = Game(name="App", gamescope=True, hdr=True)

        def which(name):
            return "/usr/bin/gamescope" if name == "gamescope" else None

        with mock.patch("gamehandler.runners.shutil.which", side_effect=which):
            argv, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {})
        self.assertEqual(
            argv,
            ["/usr/bin/gamescope", "--hdr-enabled", "--", "/usr/bin/wine", "/g/app.exe"],
        )
        self.assertEqual(env["PROTON_ENABLE_HDR"], "1")

    def test_gamescope_missing_fails_instead_of_silently_ignoring_toggle(self):
        game = Game(name="App", gamescope=True)
        with (
            mock.patch("gamehandler.runners.shutil.which", return_value=None),
            self.assertRaisesRegex(RuntimeError, "VulkanLayer.gamescope"),
        ):
            apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {})

    def test_additional_app_uses_runner_before_gamescope_wrapping(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            dxvk=False,
            gamescope=True,
            additional_app="/g/helper.exe",
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")

        def which(name):
            return "/usr/bin/gamescope" if name == "gamescope" else None

        with (
            mock.patch("gamehandler.runners.shutil.which", side_effect=which),
            mock.patch("gamehandler.runners.subprocess.Popen") as popen,
        ):
            launch(game, manager)

        self.assertEqual(popen.call_args_list[0].args[0], ["/usr/bin/wine", "/g/helper.exe"])
        self.assertEqual(
            popen.call_args_list[1].args[0],
            ["/usr/bin/gamescope", "--", "/usr/bin/wine", "/g/app.exe"],
        )

    def test_source_install_without_flatpak_dxvk_bundle_still_launches(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            dxvk=True,
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")
        missing = str(Path(tmp.name) / "missing-dxvk")
        with (
            mock.patch.dict(os.environ, {"GAMEHANDLER_DXVK_ROOT": missing}),
            mock.patch("gamehandler.runners.subprocess.Popen") as popen,
        ):
            launch(game, manager)
        self.assertEqual(popen.call_args.args[0], ["/usr/bin/wine", "/g/app.exe"])

    def test_per_game_winearch_reaches_dxvk_installer(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        dxvk_root = Path(tmp.name) / "dxvk"
        dxvk_root.mkdir()
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            environment="WINEARCH=win32",
            dxvk=True,
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")
        with (
            mock.patch.dict(os.environ, {"GAMEHANDLER_DXVK_ROOT": str(dxvk_root)}),
            mock.patch("gamehandler.runners.install_bundled_dxvk") as install,
            mock.patch("gamehandler.runners.subprocess.Popen"),
        ):
            launch(game, manager)
        self.assertEqual(install.call_args.args[0]["WINEARCH"], "win32")

    def test_nvapi_rejects_raw_wine_runner(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            dxvk=False,
            nvapi=True,
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")
        with self.assertRaisesRegex(RuntimeError, "NVAPI/DLSS requires a Proton runner"):
            launch(game, manager)

    def test_fsr_rejects_raw_wine_runner(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            dxvk=False,
            fsr=True,
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")
        with self.assertRaisesRegex(RuntimeError, "FSR requires a compatible Proton runner"):
            launch(game, manager)

    def test_wayland_rejects_raw_wine_runner(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(
            name="App",
            exe_path="/g/app.exe",
            prefix_path=str(Path(tmp.name) / "prefix"),
            dxvk=False,
            wayland=True,
        )
        manager = mock.Mock()
        manager.get.return_value = WineRunner(binary="/usr/bin/wine")
        with self.assertRaisesRegex(RuntimeError, "Wayland mode requires a Proton runner"):
            launch(game, manager)

    def test_raw_wine_omits_proton_only_anticheat_variables(self):
        game = Game(name="App", battleye=True, eac=True)
        with mock.patch("gamehandler.runners.find_anticheat_runtime") as find_runtime:
            _, env = apply_launch_options(
                game,
                ["/usr/bin/wine", "/g/app.exe"],
                {},
                proton_features=False,
            )
        find_runtime.assert_not_called()
        self.assertNotIn("PROTON_BATTLEYE_RUNTIME", env)
        self.assertNotIn("PROTON_EAC_RUNTIME", env)

    def test_raw_wine_keeps_generic_sdl_and_hdr_without_proton_variables(self):
        game = Game(name="App", prefer_sdl=True, hdr=True)
        _, env = apply_launch_options(
            game,
            ["/usr/bin/wine", "/g/app.exe"],
            {},
            proton_features=False,
        )
        self.assertEqual(env["SDL_JOYSTICK_HIDAPI"], "1")
        self.assertEqual(env["DXVK_HDR"], "1")
        self.assertNotIn("PROTON_ENABLE_HIDAPI", env)
        self.assertNotIn("PROTON_ENABLE_HDR", env)

    def test_user_environment_overrides_toggles(self):
        game = Game(name="App", esync=True, environment="WINEESYNC=0 FOO=bar")
        _, env = apply_launch_options(game, ["/usr/bin/wine", "/g/app.exe"], {})
        self.assertEqual(env["WINEESYNC"], "0")
        self.assertEqual(env["FOO"], "bar")

    def test_parse_env_block_and_desktop_size(self):
        parsed = parse_env_block("FOO=1; BAR=two words\n# comment\nBAZ=3")
        self.assertEqual(parsed["FOO"], "1")
        self.assertEqual(parsed["BAR"], "two words")
        self.assertEqual(parsed["BAZ"], "3")
        self.assertEqual(normalize_desktop_size("2560 x 1440"), "2560x1440")
        self.assertEqual(normalize_desktop_size("nope"), "1920x1080")
        env = {"WINEDLLOVERRIDES": "winemenubuilder.exe=d"}
        merge_dll_overrides(env, "d3d12=b")
        self.assertEqual(env["WINEDLLOVERRIDES"], "winemenubuilder.exe=d;d3d12=b")

    def test_installs_bundled_dxvk_into_raw_wine_prefix_once(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name) / "dxvk"
        prefix = Path(tmp.name) / "prefix"
        for arch in ("x32", "x64"):
            source = root / arch
            source.mkdir(parents=True)
            (source / "d3d11.dll").write_bytes(arch.encode())
            (source / "dxgi.dll").write_bytes(arch.encode())
        env = {"WINEPREFIX": str(prefix)}
        install_bundled_dxvk(env, root)
        install_bundled_dxvk(env, root)
        self.assertEqual(
            (prefix / "drive_c/windows/system32/d3d11.dll").read_bytes(), b"x64"
        )
        self.assertEqual(
            (prefix / "drive_c/windows/syswow64/d3d11.dll").read_bytes(), b"x32"
        )
        self.assertEqual(
            (prefix / ".gamehandler-dxvk-version").read_text().strip(), "3.0.2"
        )
        self.assertIn("d3d11", env["WINEDLLOVERRIDES"])

    def test_installs_x32_dxvk_for_existing_win32_prefix(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name) / "dxvk"
        prefix = Path(tmp.name) / "prefix"
        for arch in ("x32", "x64"):
            source = root / arch
            source.mkdir(parents=True)
            (source / "d3d11.dll").write_bytes(arch.encode())
        prefix.mkdir()
        (prefix / "system.reg").write_text("WINE REGISTRY Version 2\n#arch=win32\n")
        env = {"WINEPREFIX": str(prefix)}
        install_bundled_dxvk(env, root)
        self.assertEqual(
            (prefix / "drive_c/windows/system32/d3d11.dll").read_bytes(), b"x32"
        )
        self.assertFalse((prefix / "drive_c/windows/syswow64").exists())

    def test_parse_env_block_preserves_keys_after_quoted_values(self):
        parsed = parse_env_block('LABEL="Radeon GPU" WINEESYNC=0')
        self.assertEqual(parsed, {"LABEL": "Radeon GPU", "WINEESYNC": "0"})

    def test_parse_env_block_preserves_semicolons_inside_quoted_values(self):
        parsed = parse_env_block(
            'WINEDLLOVERRIDES="dinput8=n,b;winemenubuilder.exe=d";WINEESYNC=0'
        )
        self.assertEqual(
            parsed,
            {
                "WINEDLLOVERRIDES": "dinput8=n,b;winemenubuilder.exe=d",
                "WINEESYNC": "0",
            },
        )

    def test_parse_env_block_preserves_unquoted_windows_backslashes(self):
        parsed = parse_env_block(r"MOD_PATH=C:\Games\Mods WINEESYNC=0")
        self.assertEqual(
            parsed,
            {"MOD_PATH": r"C:\Games\Mods", "WINEESYNC": "0"},
        )

    def test_find_anticheat_runtime_in_extra_root(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        runtime = Path(tmp.name) / "battleye_runtime"
        runtime.mkdir()
        (runtime / "marker").write_text("ok")
        self.assertEqual(find_anticheat_runtime("battleye", extra_roots=[Path(tmp.name)]), str(runtime))
        self.assertEqual(virtual_desktop_argv(["/usr/bin/wine", "a.exe"], Game(name="App")), 
                         ["/usr/bin/wine", "explorer", "/desktop=App,1920x1080", "a.exe"])

    def test_finds_normal_steam_anticheat_runtime_names(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        battleye = root / "Proton BattlEye Runtime"
        eac = root / "Proton EasyAntiCheat Runtime"
        for runtime in (battleye, eac):
            runtime.mkdir()
            (runtime / "marker").write_text("ok")
        self.assertEqual(find_anticheat_runtime("battleye", extra_roots=[root]), str(battleye))
        self.assertEqual(find_anticheat_runtime("eac", extra_roots=[root]), str(eac))

    def test_finds_flatpak_steam_anticheat_runtime(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        home = Path(tmp.name)
        runtime = (
            home
            / ".var/app/com.valvesoftware.Steam/data/Steam/steamapps/common"
            / "Proton BattlEye Runtime"
        )
        runtime.mkdir(parents=True)
        (runtime / "marker").write_text("ok")
        with mock.patch("gamehandler.runners.Path.home", return_value=home):
            self.assertEqual(find_anticheat_runtime("battleye"), str(runtime))

    def test_desktop_shortcut(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        game = Game(name="Half-Life", id="abcd1234deadbeef")
        path = create_desktop_shortcut(game, "gamehandler --launch abcd1234deadbeef", Path(tmp.name))
        text = path.read_text()
        self.assertIn("Name=Half-Life", text)
        self.assertIn("Exec=gamehandler --launch abcd1234deadbeef", text)


if __name__ == "__main__":
    unittest.main()

