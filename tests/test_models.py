import json
import tempfile
import unittest
from pathlib import Path

from gamehandler.models import Game, Library


class GameTests(unittest.TestCase):
    def test_roundtrip_serialization(self):
        game = Game(name="Half-Life", exe_path="/games/hl.exe", runner="GE-Proton9-5")
        restored = Game.from_dict(game.to_dict())
        self.assertEqual(restored.name, "Half-Life")
        self.assertEqual(restored.exe_path, "/games/hl.exe")
        self.assertEqual(restored.runner, "GE-Proton9-5")
        self.assertEqual(restored.id, game.id)

    def test_from_dict_ignores_unknown_keys(self):
        game = Game.from_dict({"name": "Doom", "legacy_field": 1})
        self.assertEqual(game.name, "Doom")

    def test_launch_options_roundtrip(self):
        game = Game(
            name="Celeste",
            kind="linux",
            mangohud=True,
            gamemode=True,
            prefer_sdl=True,
            wayland=True,
            hdr=True,
            additional_app="/opt/helper.exe",
        )
        restored = Game.from_dict(game.to_dict())
        self.assertTrue(restored.is_linux)
        self.assertTrue(restored.mangohud)
        self.assertTrue(restored.gamemode)
        self.assertTrue(restored.prefer_sdl)
        self.assertEqual(restored.additional_app, "/opt/helper.exe")


class LibraryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "games.json"

    def tearDown(self):
        self.tmp.cleanup()

    def test_add_persists_to_disk(self):
        lib = Library(self.path)
        lib.add(Game(name="Portal"))
        self.assertTrue(self.path.exists())
        data = json.loads(self.path.read_text())
        self.assertEqual(len(data), 1)
        self.assertEqual(data[0]["name"], "Portal")

    def test_reload_from_disk(self):
        lib = Library(self.path)
        game = lib.add(Game(name="Celeste"))
        reloaded = Library(self.path)
        self.assertEqual(len(reloaded), 1)
        self.assertIsNotNone(reloaded.get(game.id))

    def test_remove(self):
        lib = Library(self.path)
        game = lib.add(Game(name="Hades"))
        lib.remove(game.id)
        self.assertEqual(len(lib), 0)
        self.assertEqual(len(Library(self.path)), 0)

    def test_sorted_and_search(self):
        lib = Library(self.path)
        lib.add(Game(name="Zelda"))
        lib.add(Game(name="Age of Empires"))
        lib.add(Game(name="Baldur's Gate"))
        names = [g.name for g in lib.all()]
        self.assertEqual(names, ["Age of Empires", "Baldur's Gate", "Zelda"])
        self.assertEqual([g.name for g in lib.search("age")], ["Age of Empires"])
        self.assertEqual(len(lib.search("")), 3)

    def test_category_filter_and_search(self):
        lib = Library(self.path)
        lib.add(Game(name="Doom", category="Shooter"))
        lib.add(Game(name="Civilization", category="Strategy"))
        lib.add(Game(name="Hades", category="Action"))
        self.assertEqual(lib.categories(), ["Action", "Shooter", "Strategy"])
        self.assertEqual([g.name for g in lib.search("", category="Strategy")], ["Civilization"])
        self.assertEqual([g.name for g in lib.search("shoot")], ["Doom"])

    def test_corrupt_file_is_tolerated(self):
        self.path.write_text("{ not valid json")
        lib = Library(self.path)
        self.assertEqual(len(lib), 0)


if __name__ == "__main__":
    unittest.main()
