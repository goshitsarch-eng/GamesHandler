import json
import unittest
from pathlib import Path

from gamehandler import APP_ID


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
        self.assertEqual(manifest["runtime-version"], "50")
        self.assertEqual(manifest["base"], "org.winehq.Wine")
        self.assertEqual(manifest["base-version"], "stable-25.08")
        self.assertIn("--allow=multiarch", manifest["finish-args"])
        self.assertIn("--filesystem=home", manifest["finish-args"])
        self.assertIn("--device=all", manifest["finish-args"])
        self.assertIn("org.freedesktop.Platform.Compat.i386", manifest["inherit-extensions"])
        self.assertIn("org.freedesktop.Platform.GL32", manifest["inherit-extensions"])

    def test_gpl_license_is_present_and_installed(self):
        license_text = (ROOT / "LICENSE").read_text()
        data_meson = (ROOT / "data" / "meson.build").read_text()
        self.assertIn("GNU GENERAL PUBLIC LICENSE", license_text)
        self.assertIn("Version 3, 29 June 2007", license_text)
        self.assertGreater(len(license_text.splitlines()), 600)
        self.assertIn("meson.project_source_root() / 'LICENSE'", data_meson)
        self.assertIn("'licenses' / app_id", data_meson)


if __name__ == "__main__":
    unittest.main()