import json
import tempfile
import unittest
from pathlib import Path

from gamehandler.runners import SYSTEM_WINE
from gamehandler.settings import Settings


class SettingsTests(unittest.TestCase):
    def test_defaults_are_dark_grid(self):
        settings = Settings()
        self.assertEqual(settings.color_scheme, "dark")
        self.assertEqual(settings.view_mode, "grid")
        self.assertEqual(settings.default_runner, SYSTEM_WINE)

    def test_roundtrip(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "settings.json"
        settings = Settings(
            color_scheme="light",
            view_mode="list",
            default_runner="GE-Proton9-5",
            default_mangohud=True,
        )
        settings.save(path)
        loaded = Settings.load(path)
        self.assertEqual(loaded.color_scheme, "light")
        self.assertEqual(loaded.view_mode, "list")
        self.assertEqual(loaded.default_runner, "GE-Proton9-5")
        self.assertTrue(loaded.default_mangohud)

    def test_invalid_values_fall_back(self):
        settings = Settings.from_dict({"color_scheme": "neon", "view_mode": "mosaic"})
        self.assertEqual(settings.color_scheme, "dark")
        self.assertEqual(settings.view_mode, "grid")

    def test_corrupt_file_is_tolerated(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "settings.json"
        path.write_text("{ not json")
        self.assertEqual(Settings.load(path).color_scheme, "dark")

    def test_unknown_keys_are_ignored(self):
        settings = Settings.from_dict({"color_scheme": "system", "legacy": 1})
        self.assertEqual(settings.color_scheme, "system")
        self.assertNotIn("legacy", settings.to_dict())
        self.assertTrue(json.dumps(settings.to_dict()))


if __name__ == "__main__":
    unittest.main()
