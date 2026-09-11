import json
import re
import unittest
from pathlib import Path

from gamehandler import APP_ID, __version__


ROOT = Path(__file__).resolve().parents[1]
APP_ID_EXPECTED = "com.goshapps.GameHandler"


class PackagingTests(unittest.TestCase):
    def test_permanent_identity_is_consistent(self):
        self.assertEqual(APP_ID, APP_ID_EXPECTED)
        desktop = (ROOT / "data" / f"{APP_ID_EXPECTED}.desktop").read_text()
        metainfo = (ROOT / "data" / f"{APP_ID_EXPECTED}.metainfo.xml").read_text()
        self.assertIn(f"Icon={APP_ID_EXPECTED}", desktop)
        self.assertIn(f"<id>{APP_ID_EXPECTED}</id>", metainfo)
        self.assertIn(
            f'<launchable type="desktop-id">{APP_ID_EXPECTED}.desktop</launchable>',
            metainfo,
        )
        self.assertIn("<project_license>GPL-3.0-or-later</project_license>", metainfo)

    def test_flatpak_has_reviewed_launcher_permissions_and_multilib(self):
        manifest_path = ROOT / "build-aux" / "flatpak" / f"{APP_ID_EXPECTED}.json"
        manifest = json.loads(manifest_path.read_text())
        self.assertEqual(manifest["app-id"], APP_ID_EXPECTED)
        self.assertEqual(manifest["branch"], "stable")

        # Rust/libcosmic stack: Freedesktop 25.08 plus the rust-stable SDK
        # extension, which ships Rust 1.98.1 (libcosmic's floor is 1.93).
        # DECISIONS D-10. Equality, not membership: the Qt/LLVM extensions must
        # not creep back.
        self.assertEqual(manifest["runtime"], "org.freedesktop.Platform")
        self.assertEqual(manifest["runtime-version"], "25.08")
        self.assertEqual(manifest["sdk"], "org.freedesktop.Sdk")
        self.assertEqual(
            manifest["sdk-extensions"],
            ["org.freedesktop.Sdk.Extension.rust-stable"],
        )

        # The Wine BaseApp is a parity requirement, not a packaging detail:
        # launching Windows games is the application's whole purpose. Its
        # stable-25.08 branch already sits on Freedesktop 25.08, so the two
        # track each other with no runtime skew (DECISIONS D-10).
        self.assertEqual(manifest["base"], "org.winehq.Wine")
        self.assertEqual(manifest["base-version"], "stable-25.08")

        # Reviewed sandbox exceptions. Each of these is a deliberate decision
        # recorded in docs/migration/packaging.md section 3, not a default:
        # network for runner downloads, multiarch for 32-bit Windows games and
        # downloaded Wine/Proton builds, home for libraries in arbitrary
        # locations, gvfs for network-share games, and --device=all for
        # controllers passed through to launched games (PLAN.md Q-2 keeps it
        # until a real gamepad test justifies narrowing it to --device=dri).
        finish_args = manifest["finish-args"]
        for argument in (
            "--share=network",
            "--share=ipc",
            "--socket=fallback-x11",
            "--socket=wayland",
            "--socket=pulseaudio",
            "--allow=multiarch",
            "--device=all",
            "--filesystem=home",
            "--filesystem=xdg-run/gvfs",
            "--filesystem=~/.var/app/com.valvesoftware.Steam/data/Steam:ro",
        ):
            with self.subTest(argument=argument):
                self.assertIn(argument, finish_args)

        # 32-bit GL and the i386 compat layer are Wine needs, not Qt needs.
        self.assertIn("org.freedesktop.Platform.Compat.i386", manifest["inherit-extensions"])
        self.assertIn("org.freedesktop.Platform.GL32", manifest["inherit-extensions"])

        module_names = [module["name"] for module in manifest["modules"] if isinstance(module, dict)]
        # Toolchain-independent modules that survive the toolkit change:
        # osslsigncode verifies Easy Installer signatures and dxvk-runtime ships
        # the Direct3D translation DLLs installed into raw-Wine prefixes.
        self.assertIn("osslsigncode", module_names)
        self.assertIn("dxvk-runtime", module_names)
        self.assertIn("gamehandler", module_names)

        osslsigncode = next(
            module
            for module in manifest["modules"]
            if isinstance(module, dict) and module["name"] == "osslsigncode"
        )
        self.assertIn("/osslsigncode/archive/refs/tags/2.14.tar.gz", osslsigncode["sources"][0]["url"])
        self.assertEqual(
            osslsigncode["sources"][0]["sha256"],
            "0f033fd6069387d2e489fbd2187e62f624764eb8c2758ee94e3e793e5150b5c5",
        )

        dxvk = next(
            module
            for module in manifest["modules"]
            if isinstance(module, dict) and module["name"] == "dxvk-runtime"
        )
        self.assertIn("dxvk-3.0.2", dxvk["sources"][0]["url"])
        self.assertEqual(
            dxvk["sources"][0]["sha256"],
            "9c538924110a7cdef871ca36dee218c0774124374ffdeb38af4b76be55bdf7c2",
        )

        # The app module builds the Rust workspace offline against vendored
        # sources: the rust-stable extension on PATH, CARGO_HOME inside the
        # build dir, and cargo --offline over the generated sources.
        gamehandler = next(
            module
            for module in manifest["modules"]
            if isinstance(module, dict) and module["name"] == "gamehandler"
        )
        self.assertEqual(gamehandler["buildsystem"], "simple")
        self.assertEqual(
            gamehandler["build-options"]["append-path"],
            "/usr/lib/sdk/rust-stable/bin",
        )
        self.assertEqual(gamehandler["build-options"]["env"]["CARGO_HOME"], "/run/build/gamehandler/cargo")
        self.assertTrue(
            any("cargo --offline build" in command for command in gamehandler["build-commands"])
        )
        self.assertIn("cargo-sources.json", gamehandler["sources"])

    def test_cargo_sources_are_present_and_well_formed(self):
        """The vendored offline tree the Flatpak builds against (PLAN.md T-16).

        `cargo --offline fetch` inside the sandbox cannot reach the network, so
        a missing or malformed cargo-sources.json is a build failure, not a
        warning. It is generated from Cargo.lock by flatpak-cargo-generator.py
        and committed; scripts/verify.sh additionally fails when it is stale.
        """
        lock = ROOT / "Cargo.lock"
        sources_path = ROOT / "build-aux" / "flatpak" / "cargo-sources.json"
        self.assertTrue(lock.is_file(), "the workspace Cargo.lock is committed")
        self.assertTrue(sources_path.is_file(), "cargo-sources.json is committed")
        sources = json.loads(sources_path.read_text())
        self.assertTrue(sources, "cargo-sources.json is not empty")

        types = {source["type"] for source in sources if isinstance(source, dict)}
        self.assertIn("archive", types, "crates.io dependencies are vendored")
        self.assertIn("git", types, "git dependencies are vendored")

        # Every git dependency in the lockfile must have a matching git source.
        # This is the regression test for the transitive-git-dependency trap
        # (DECISIONS D-09): a libcosmic rev bump can silently add another one.
        locked_git_urls = {
            re.sub(r"\.git$", "", match.split("#")[0].split("?")[0])
            for match in re.findall(
                r'source = "git\+([^"#?]+)', lock.read_text()
            )
        }
        vendored_git_urls = {
            re.sub(r"\.git$", "", source["url"])
            for source in sources
            if isinstance(source, dict) and source["type"] == "git"
        }
        self.assertTrue(locked_git_urls, "the lockfile has git dependencies")
        self.assertEqual(
            locked_git_urls - vendored_git_urls,
            set(),
            "every git dependency in Cargo.lock has a type:git source",
        )

    def test_about_and_public_metadata_use_gosh_without_a_personal_name(self):
        main = (ROOT / "gamehandler" / "qml" / "Main.qml").read_text()
        credits = (ROOT / "gamehandler" / "qml" / "CreditsPage.qml").read_text()
        metainfo = (ROOT / "data" / f"{APP_ID_EXPECTED}.metainfo.xml").read_text()
        readme = (ROOT / "README.md").read_text()
        self.assertIn('text: "About & Credits"', main)
        self.assertIn('title: "About & Credits"', credits)
        self.assertIn('text: "Made by Gosh."', credits)
        self.assertIn('text: "Version " + backend.appVersion', credits)
        self.assertIn("<name>Gosh</name>", metainfo)
        public_text = "\n".join((main, credits, metainfo, readme))
        self.assertNotIn("Vaughan", public_text)
        self.assertNotIn("Jones", public_text)

    def test_no_gtk_or_adwaita_remains_anywhere(self):
        """The Qt rewrite must leave nothing of the old stack behind."""
        tracked = [
            *sorted((ROOT / "gamehandler").rglob("*.py")),
            *sorted((ROOT / "gamehandler").rglob("*.qml")),
        ]
        for path in tracked:
            if "__pycache__" in path.parts:
                continue
            source = path.read_text(encoding="utf-8", errors="ignore")
            with self.subTest(file=path.name):
                self.assertNotIn("import gi", source)
                self.assertNotIn("gi.repository", source)
                self.assertNotIn("Adwaita", source)
                self.assertNotIn("libadwaita", source)
                self.assertNotIn("Gtk.", source)

    def test_gpl_license_is_present_and_installed(self):
        license_text = (ROOT / "LICENSE").read_text()
        data_meson = (ROOT / "data" / "meson.build").read_text()
        self.assertIn("GNU GENERAL PUBLIC LICENSE", license_text)
        self.assertIn("Version 3, 29 June 2007", license_text)
        self.assertGreater(len(license_text.splitlines()), 600)
        self.assertIn("meson.project_source_root() / 'LICENSE'", data_meson)
        self.assertIn("'licenses' / app_id", data_meson)


class VersionLockstepTests(unittest.TestCase):
    """__init__.py, meson.build, and the metainfo release notes must agree."""

    def test_meson_project_version_matches_the_package(self):
        meson = (ROOT / "meson.build").read_text()
        match = re.search(r"version:\s*'([^']+)'", meson)
        self.assertIsNotNone(match, "meson.build declares a project version")
        self.assertEqual(match.group(1), __version__)

    def test_metainfo_documents_the_current_version_first(self):
        metainfo = (ROOT / "data" / f"{APP_ID_EXPECTED}.metainfo.xml").read_text()
        versions = re.findall(r'<release version="([^"]+)"', metainfo)
        self.assertTrue(versions, "the metainfo lists releases")
        self.assertEqual(versions[0], __version__, "newest release note is this version")

    def test_readme_flatpak_bundle_name_matches_the_version(self):
        readme = (ROOT / "README.md").read_text()
        self.assertIn(f"gamehandler-{__version__}.flatpak", readme)
        self.assertIn(f"Current release: **{__version__}**", readme)
        self.assertIn("unittest discover -s tests -t .", readme)

    def test_every_python_module_is_installed_by_meson(self):
        listed = (ROOT / "gamehandler" / "meson.build").read_text()
        for module in sorted((ROOT / "gamehandler").glob("*.py")):
            with self.subTest(module=module.name):
                self.assertIn(f"'{module.name}'", listed)


if __name__ == "__main__":
    unittest.main()