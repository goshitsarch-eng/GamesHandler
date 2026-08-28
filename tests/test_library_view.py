"""Tests for library filtering, sorting, and the presentation helpers.

These cover the UI-free logic behind the library grid and list, so a broken
filter or sort shows up here rather than as an empty window.
"""

import json
import tempfile
import time
import unittest
from pathlib import Path

from gamehandler.covers import COVER_ACCENTS, accent_index, initials
from gamehandler.models import SORT_MODES, UNCATEGORIZED, Game, Library, format_last_played
from gamehandler.settings import Settings


class CategoryFilterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.library = Library(Path(self.tmp.name) / "games.json")

    def test_blank_category_is_listed_and_filterable_as_uncategorized(self):
        blank = self.library.add(Game(name="Blank", category=""))
        spaces = self.library.add(Game(name="Spaces", category="   "))
        self.library.add(Game(name="Rpg", category="RPG"))

        self.assertEqual(self.library.categories(), ["RPG", UNCATEGORIZED])
        names = [g.name for g in self.library.search("", category=UNCATEGORIZED)]
        self.assertEqual(names, ["Blank", "Spaces"])
        self.assertEqual(blank.display_category, UNCATEGORIZED)
        self.assertEqual(spaces.display_category, UNCATEGORIZED)

    def test_all_and_empty_category_return_everything(self):
        self.library.add(Game(name="A", category=""))
        self.library.add(Game(name="B", category="RPG"))
        self.assertEqual(len(self.library.search("", category="All")), 2)
        self.assertEqual(len(self.library.search("", category="")), 2)

    def test_query_matches_name_or_category(self):
        self.library.add(Game(name="Elden Ring", category="RPG"))
        self.library.add(Game(name="Factorio", category="Strategy"))
        self.assertEqual([g.name for g in self.library.search("elden")], ["Elden Ring"])
        self.assertEqual([g.name for g in self.library.search("strategy")], ["Factorio"])
        self.assertEqual([g.name for g in self.library.search("uncategor")], [])


class SortTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.library = Library(Path(self.tmp.name) / "games.json")
        now = time.time()
        self.library.add(Game(name="Charlie", added=now - 300, last_played=now - 10))
        self.library.add(Game(name="alpha", added=now - 100, last_played=0))
        self.library.add(Game(name="Bravo", added=now - 200, last_played=now - 5000))

    def test_default_sort_is_case_insensitive_by_name(self):
        self.assertEqual([g.name for g in self.library.all()], ["alpha", "Bravo", "Charlie"])

    def test_recent_sort_puts_never_played_last(self):
        self.assertEqual(
            [g.name for g in self.library.all(sort="recent")], ["Charlie", "Bravo", "alpha"]
        )

    def test_added_sort_is_newest_first(self):
        self.assertEqual(
            [g.name for g in self.library.all(sort="added")], ["alpha", "Bravo", "Charlie"]
        )

    def test_search_honours_the_sort_mode(self):
        names = [g.name for g in self.library.search("", sort="recent")]
        self.assertEqual(names[0], "Charlie")

    def test_unknown_sort_falls_back_to_name(self):
        self.assertEqual(
            [g.name for g in self.library.all(sort="nonsense")], ["alpha", "Bravo", "Charlie"]
        )

    def test_settings_reject_an_unknown_sort_mode(self):
        self.assertIn(Settings().sort_mode, SORT_MODES)
        self.assertEqual(Settings.from_dict({"sort_mode": "sideways"}).sort_mode, "name")
        self.assertEqual(Settings.from_dict({"sort_mode": "recent"}).sort_mode, "recent")


class CorruptLibraryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "games.json"

    def test_a_json_object_instead_of_a_list_loads_empty(self):
        self.path.write_text(json.dumps({"oops": True}), encoding="utf-8")
        self.assertEqual(len(Library(self.path)), 0)

    def test_non_dict_entries_are_skipped(self):
        self.path.write_text(
            json.dumps([{"name": "Good"}, "bad", 42, None, {"name": ""}]), encoding="utf-8"
        )
        library = Library(self.path)
        self.assertEqual([g.name for g in library.all()], ["Good"])

    def test_unknown_keys_and_truncated_files_do_not_raise(self):
        self.path.write_text('[{"name": "X", "from_the_future": 1}', encoding="utf-8")
        self.assertEqual(len(Library(self.path)), 0)
        self.path.write_text('[{"name": "X", "from_the_future": 1}]', encoding="utf-8")
        self.assertEqual([g.name for g in Library(self.path).all()], ["X"])

    def test_invalid_persisted_timestamps_are_normalized(self):
        self.path.write_text(
            json.dumps(
                [
                    {"name": "Text", "added": "yesterday", "last_played": "never"},
                    {"name": "Boolean", "added": True, "last_played": False},
                    {"name": "Nonfinite", "added": float("inf"), "last_played": float("nan")},
                    {"name": "Overflow", "added": 10**400, "last_played": 10**400},
                ]
            ),
            encoding="utf-8",
        )
        library = Library(self.path)
        self.assertEqual(
            {game.name for game in library.all(sort="added")},
            {"Text", "Boolean", "Nonfinite", "Overflow"},
        )
        for game in library.all(sort="recent"):
            self.assertIsInstance(game.added, float)
            self.assertEqual(game.last_played, 0.0)
            self.assertEqual(format_last_played(game.last_played), "Never played")


class PresentationHelperTests(unittest.TestCase):
    def test_initials_cover_the_shapes_a_library_contains(self):
        self.assertEqual(initials("Elden Ring"), "ER")
        self.assertEqual(initials("DOOM"), "DO")
        self.assertEqual(initials("A"), "A")
        self.assertEqual(initials(""), "?")
        self.assertEqual(initials("   "), "?")
        self.assertEqual(initials("007 GoldenEye"), "0G")
        self.assertEqual(initials("Battle.net"), "BN")

    def test_accent_index_is_stable_and_in_range(self):
        first = accent_index("some-game-id")
        self.assertEqual(first, accent_index("some-game-id"))
        for seed in ("", "a", "b", "a much longer identifier"):
            self.assertIn(accent_index(seed), range(COVER_ACCENTS))

    def test_last_played_labels(self):
        now = 1_000_000.0
        self.assertEqual(format_last_played(0, now), "Never played")
        self.assertEqual(format_last_played(now - 30, now), "Played just now")
        self.assertEqual(format_last_played(now - 60 * 20, now), "Played 20 min ago")
        self.assertEqual(format_last_played(now - 3600, now), "Played 1 hour ago")
        self.assertEqual(format_last_played(now - 3600 * 5, now), "Played 5 hours ago")
        self.assertEqual(format_last_played(now - 86400, now), "Played yesterday")
        self.assertEqual(format_last_played(now - 86400 * 10, now), "Played 10 days ago")
        self.assertEqual(format_last_played(now - 86400 * 60, now), "Played 2 months ago")
        self.assertEqual(format_last_played(now - 86400 * 400, now), "Played 1 year ago")

    def test_a_clock_skewed_future_timestamp_does_not_produce_a_negative_label(self):
        self.assertEqual(format_last_played(2_000_000.0, 1_000_000.0), "Played just now")


if __name__ == "__main__":
    unittest.main()
