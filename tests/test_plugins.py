import unittest
from unittest import mock

from gamehandler.plugins import (
    detect_package_manager,
    format_command,
    in_flatpak,
    install_command,
    plugin_by_id,
    privileged_command,
)
from gamehandler.runners import runner_guides


class PluginCatalogTests(unittest.TestCase):
    def test_mangohud_is_offered(self):
        plugin = plugin_by_id("mangohud")
        self.assertEqual(plugin.binary, "mangohud")
        self.assertIn("apt", plugin.packages)

    def test_apt_install_command(self):
        argv = install_command(plugin_by_id("mangohud"), manager="apt")
        self.assertEqual(argv, ["apt-get", "install", "-y", "mangohud"])

    def test_unknown_manager_raises(self):
        with self.assertRaises(RuntimeError):
            install_command(plugin_by_id("mangohud"), manager="nix")

    def test_privileged_prefix_uses_pkexec(self):
        with mock.patch("gamehandler.plugins.os.geteuid", return_value=1000):
            with mock.patch("gamehandler.plugins.shutil.which", side_effect=lambda name: "/usr/bin/pkexec" if name == "pkexec" else None):
                argv = privileged_command(["apt-get", "install", "-y", "mangohud"])
        self.assertEqual(argv[0], "/usr/bin/pkexec")
        self.assertIn("mangohud", argv)

    def test_format_command(self):
        self.assertEqual(format_command(["apt-get", "install", "-y", "mangohud"]), "apt-get install -y mangohud")

    def test_package_manager_detection_is_a_string(self):
        manager = detect_package_manager()
        self.assertIsInstance(manager, str)

    def test_flatpak_never_offers_host_package_commands(self):
        with mock.patch.dict("os.environ", {"FLATPAK_ID": "com.goshapps.GameHandler"}):
            self.assertTrue(in_flatpak())
            self.assertEqual(detect_package_manager(), "")
            with self.assertRaisesRegex(RuntimeError, "unavailable inside Flatpak"):
                install_command(plugin_by_id("mangohud"), manager="apt")


class RunnerGuideTests(unittest.TestCase):
    def test_guide_covers_system_wine_and_proton_ge(self):
        titles = [title for title, _kind, _advice in runner_guides()]
        self.assertIn("System Wine", titles)
        self.assertIn("Proton-GE", titles)
        self.assertIn("Wine-Vanilla", titles)
        advice = {title: text for title, _kind, text in runner_guides()}
        self.assertIn("most Windows games", advice["Proton-GE"])


if __name__ == "__main__":
    unittest.main()
