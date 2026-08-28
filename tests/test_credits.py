import unittest
from pathlib import Path

from gamehandler import credits as credits_module
from gamehandler.credits import (
    ACKNOWLEDGEMENT,
    CREDIT_SECTIONS,
    WHY_ALL_IN_ONE,
    about_credit_sections,
    all_credits,
    credit_by_name,
    markdown,
    section_by_id,
)
from gamehandler.runners import RUNNER_FAMILIES

ROOT = Path(__file__).resolve().parents[1]

# Every project whose code actually runs when a game launches must be credited.
REQUIRED = (
    "Wine",
    "Proton",
    "DXVK",
    "VKD3D-Proton",
    "DXVK-NVAPI",
    "umu-launcher",
    "MangoHud",
    "Feral GameMode",
    "Gamescope",
    "Winetricks",
    "Lutris",
    "Faugus Launcher",
    "Bottles",
    "ProtonPlus",
    "Qt",
    "Kirigami",
    "PySide6",
)


class CreditCatalogTests(unittest.TestCase):
    def test_every_upstream_dependency_is_credited(self):
        names = {credit.name for credit in all_credits()}
        for required in REQUIRED:
            self.assertIn(required, names)

    def test_credits_carry_a_role_and_a_link(self):
        for credit in all_credits():
            with self.subTest(credit=credit.name):
                self.assertTrue(credit.role.strip(), "every credit explains what it is used for")
                self.assertTrue(credit.url.startswith("https://"))

    def test_section_ids_are_unique_and_populated(self):
        ids = [section.id for section in CREDIT_SECTIONS]
        self.assertEqual(len(ids), len(set(ids)))
        for section in CREDIT_SECTIONS:
            self.assertTrue(section.entries, f"{section.id} has no entries")
            self.assertTrue(section.title.strip())
            self.assertTrue(section.summary.strip())

    def test_lookup_helpers(self):
        self.assertEqual(section_by_id("layers").title, "Compatibility layers")
        self.assertEqual(credit_by_name("wine").name, "Wine")
        with self.assertRaises(KeyError):
            section_by_id("nope")
        with self.assertRaises(KeyError):
            credit_by_name("nope")

    def test_label_includes_authors_when_known(self):
        self.assertIn("—", credit_by_name("DXVK").label)
        self.assertEqual(credit_by_name("Proton-GE").label, "Proton-GE — GloriousEggroll")

    def test_every_runner_family_maintainer_is_acknowledged(self):
        text = markdown()
        for family in RUNNER_FAMILIES:
            with self.subTest(family=family.id):
                self.assertTrue(family.maintainer, "each family names its maintainer")
                self.assertIn(family.maintainer, text)

    def test_about_sections_pair_names_with_urls(self):
        rendered = about_credit_sections()
        self.assertEqual(len(rendered), len(CREDIT_SECTIONS))
        first_title, people = rendered[0]
        self.assertEqual(first_title, CREDIT_SECTIONS[0].title)
        self.assertIn("Wine https://www.winehq.org", people)

    def test_rationale_entries_are_headed_paragraphs(self):
        self.assertGreaterEqual(len(WHY_ALL_IN_ONE), 3)
        for heading, body in WHY_ALL_IN_ONE:
            self.assertTrue(heading.strip())
            self.assertGreater(len(body.split()), 15)

    def test_module_is_ui_toolkit_free(self):
        source = Path(credits_module.__file__).read_text(encoding="utf-8")
        self.assertNotIn("import gi", source)
        self.assertNotIn("PySide6 import", source)
        self.assertNotIn("import PySide6", source)


class ReadmeSyncTests(unittest.TestCase):
    """The README acknowledgements are generated from credits.py, so they agree."""

    def test_readme_contains_the_generated_acknowledgements(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn(ACKNOWLEDGEMENT, readme)
        for block in markdown().split("\n\n"):
            block = block.strip()
            if block:
                self.assertIn(block, readme, f"README is out of sync with credits.py:\n{block}")

    def test_readme_explains_why_it_is_one_app(self):
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("Why one app instead of assembling the stack yourself", readme)


if __name__ == "__main__":
    unittest.main()
