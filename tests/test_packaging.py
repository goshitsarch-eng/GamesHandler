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
        self.assertEqual(manifest["runtime"], "org.kde.Platform")
        self.assertEqual(manifest["sdk"], "org.kde.Sdk")
        self.assertEqual(manifest["base"], "org.winehq.Wine")
        self.assertEqual(manifest["base-version"], "stable-25.08")
        self.assertIn("--allow=multiarch", manifest["finish-args"])
        self.assertIn("--filesystem=home", manifest["finish-args"])
        self.assertIn("--filesystem=xdg-run/gvfs", manifest["finish-args"])
        self.assertIn("--device=all", manifest["finish-args"])
        self.assertIn("org.freedesktop.Platform.Compat.i386", manifest["inherit-extensions"])
        self.assertIn("org.freedesktop.Platform.GL32", manifest["inherit-extensions"])
        module_names = [module["name"] for module in manifest["modules"]]
        self.assertIn("pyside6", module_names)

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

    def test_every_python_module_is_installed_by_meson(self):
        listed = (ROOT / "gamehandler" / "meson.build").read_text()
        for module in sorted((ROOT / "gamehandler").glob("*.py")):
            with self.subTest(module=module.name):
                self.assertIn(f"'{module.name}'", listed)


if __name__ == "__main__":
    unittest.main()