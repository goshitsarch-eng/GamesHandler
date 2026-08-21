import tempfile
import unittest
from pathlib import Path
from unittest import mock

from gamehandler.covers import (
    copy_custom_cover,
    cover_urls_for_app,
    map_steam_genre,
    parse_store_search,
    pick_best_match,
    score_title,
)


class TitleScoreTests(unittest.TestCase):
    def test_exact_match_is_perfect(self):
        self.assertEqual(score_title("Half-Life", "Half-Life"), 1.0)

    def test_ignores_punctuation_and_trademark(self):
        self.assertGreater(score_title("Portal 2", "Portal 2™"), 0.9)

    def test_unrelated_titles_score_low(self):
        self.assertLess(score_title("Celeste", "Age of Empires"), 0.3)


class StoreSearchTests(unittest.TestCase):
    PAYLOAD = {
        "total": 2,
        "items": [
            {"id": 70, "name": "Half-Life", "tiny_image": "https://example/tiny.jpg", "type": "app"},
            {"id": 220, "name": "Half-Life 2", "tiny_image": "", "type": "app"},
            {"id": 1, "name": "Something else", "type": "music"},
        ],
    }

    def test_parse_and_pick_best(self):
        items = parse_store_search(self.PAYLOAD)
        self.assertEqual([item["appid"] for item in items], [70, 220, 1])
        best = pick_best_match("Half-Life", items)
        self.assertEqual(best["appid"], 70)

    def test_cover_urls_include_library_art(self):
        urls = cover_urls_for_app(70, "https://example/tiny.jpg")
        self.assertTrue(any("library_600x900.jpg" in url for url in urls))
        self.assertIn("https://example/tiny.jpg", urls)

    def test_genre_mapping(self):
        self.assertEqual(map_steam_genre(["Action", "Adventure"]), "Action")
        self.assertEqual(map_steam_genre(["Role-Playing"]), "RPG")
        self.assertEqual(map_steam_genre(["Utilities"]), "Utility")
        self.assertEqual(map_steam_genre([]), "Uncategorized")


class CustomCoverTests(unittest.TestCase):
    def test_copies_into_covers_dir(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        source = Path(tmp.name) / "art.png"
        source.write_bytes(b"\x89PNG fake")
        with mock.patch("gamehandler.covers.config.covers_dir", return_value=Path(tmp.name) / "covers"):
            dest = copy_custom_cover(source, "abc123")
        self.assertTrue(dest.exists())
        self.assertEqual(dest.name, "abc123.png")
        self.assertEqual(dest.read_bytes(), source.read_bytes())


if __name__ == "__main__":
    unittest.main()
