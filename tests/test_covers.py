import tempfile
import unittest
from pathlib import Path
from unittest import mock

from gamehandler.covers import (
    ICON_SOURCE,
    STEAM_SOURCE,
    CoverHit,
    copy_custom_cover,
    cover_urls_for_app,
    fetch_cover,
    icon_cover,
    map_steam_genre,
    parse_store_search,
    pick_best_match,
    save_exe_icon,
    score_title,
)

from .test_exe_icons import build_pe, dib_icon, group_icon, resource_section


def windows_executable(path: Path) -> Path:
    """Write a real PE carrying one icon, for the fallback lookup to read."""
    payload = dib_icon(32, 0x5A)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(build_pe(resource_section({1: payload}, group_icon([(32, len(payload), 1)]))))
    return path


class TitleScoreTests(unittest.TestCase):
    def test_exact_match_is_perfect(self):
        self.assertEqual(score_title("Half-Life", "Half-Life"), 1.0)

    def test_ignores_punctuation_and_trademark(self):
        self.assertGreater(score_title("Portal 2", "Portal 2™"), 0.9)

    def test_unrelated_titles_score_low(self):
        self.assertLess(score_title("Celeste", "Age of Empires"), 0.3)

    def test_extra_words_in_the_candidate_cost_score(self):
        """A launcher's name inside a longer game title is not a match.

        Steam answers "Steam" with "DCS World Steam Edition" and "Battle.net"
        with "Mega Man Battle Network"; both used to score high enough to hang
        someone else's artwork on the tile.
        """
        self.assertGreater(score_title("Half-Life", "Half-Life 2"), 0.8)
        self.assertLess(score_title("Steam", "DCS World Steam Edition"), 0.8)
        self.assertLess(score_title("Discord", "Bot Maker For Discord"), 0.8)
        self.assertLess(
            score_title("Battle.net", "Mega Man Battle Network Legacy Collection Vol. 1"),
            0.45,
        )
        self.assertLess(score_title("EA App", "Creatry — Easy Game Maker & Game Builder App"), 0.45)


class LauncherMatchTests(unittest.TestCase):
    """Store launchers are not Steam products; no hit beats the wrong hit."""

    def _hits(self, *names):
        return [{"appid": index + 1, "name": name, "type": "app"} for index, name in enumerate(names)]

    def test_a_one_word_launcher_name_does_not_match_a_longer_title(self):
        self.assertIsNone(
            pick_best_match("Steam", self._hits("DCS World Steam Edition", "Steamworld Dig"))
        )
        self.assertIsNone(pick_best_match("Discord", self._hits("Bot Maker For Discord")))

    def test_a_one_word_title_still_matches_its_own_store_page(self):
        best = pick_best_match("Celeste", self._hits("Celeste Classic", "Celeste"))
        self.assertEqual("Celeste", best["name"])

    def test_a_multi_word_launcher_name_does_not_match_a_lookalike(self):
        self.assertIsNone(
            pick_best_match(
                "Battle.net", self._hits("Mega Man Battle Network Legacy Collection Vol. 1")
            )
        )
        self.assertIsNone(
            pick_best_match("EA App", self._hits("Creatry — Easy Game Maker & Game Builder App"))
        )

    def test_the_exact_title_wins_over_a_longer_superset(self):
        """Ranking must not just take whichever hit the store listed first."""
        best = pick_best_match(
            "Portal 2", self._hits("Portal 2 Sixense Perceptual Pack", "Portal 2")
        )
        self.assertEqual("Portal 2", best["name"])


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


class ExecutableIconCoverTests(unittest.TestCase):
    """The fallback that gives launchers and plain Windows apps a tile."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.covers = self.root / "covers"
        patcher = mock.patch(
            "gamehandler.covers.config.covers_dir", return_value=self.covers
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_saves_the_executables_icon_into_the_covers_directory(self):
        exe = windows_executable(self.root / "Steam" / "steam.exe")
        path = save_exe_icon(exe, "abc123")
        self.assertEqual(self.covers / "abc123.ico", path)
        self.assertTrue(path.read_bytes().startswith(b"\x00\x00\x01\x00"))

    def test_an_executable_without_an_icon_is_reported_not_silently_empty(self):
        bare = self.root / "bare.exe"
        bare.write_bytes(b"MZ" + b"\x00" * 4096)
        with self.assertRaisesRegex(RuntimeError, "no icon"):
            save_exe_icon(bare, "abc123")

    def test_icon_cover_reports_its_own_origin(self):
        exe = windows_executable(self.root / "app.exe")
        hit = icon_cover("Battle.net", exe, "abc123")
        self.assertEqual(ICON_SOURCE, hit.source)
        self.assertEqual("the app icon", hit.origin_label)
        self.assertEqual(0, hit.appid)
        self.assertEqual("Battle.net", hit.name)

    def test_fetch_cover_falls_back_to_the_icon_when_steam_has_nothing(self):
        exe = windows_executable(self.root / "app.exe")
        with mock.patch(
            "gamehandler.covers.steam_cover", side_effect=RuntimeError("No Steam cover found")
        ):
            hit = fetch_cover("EA App", "abc123", exe_path=exe)
        self.assertEqual(ICON_SOURCE, hit.source)
        self.assertTrue(Path(hit.cover_path).is_file())

    def test_fetch_cover_prefers_steam_artwork_when_it_exists(self):
        exe = windows_executable(self.root / "app.exe")
        steam = CoverHit(70, "Half-Life", "Action", "/covers/hl.jpg", "https://cdn", STEAM_SOURCE)
        with mock.patch("gamehandler.covers.steam_cover", return_value=steam):
            hit = fetch_cover("Half-Life", "abc123", exe_path=exe)
        self.assertEqual(steam, hit)
        self.assertEqual("Steam", hit.origin_label)

    def test_with_neither_source_the_steam_failure_is_what_surfaces(self):
        with mock.patch(
            "gamehandler.covers.steam_cover", side_effect=RuntimeError("No Steam cover found for X")
        ):
            with self.assertRaisesRegex(RuntimeError, "No Steam cover found for X"):
                fetch_cover("X", "abc123", exe_path=self.root / "missing.exe")


if __name__ == "__main__":
    unittest.main()
