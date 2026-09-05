"""Keep package-aware unittest discovery usable without GUI dependencies."""

import subprocess
import sys
import unittest
from pathlib import Path


class DiscoveryTests(unittest.TestCase):
    def test_discovery_without_pyside_imports_or_loader_errors(self):
        # -S hides site-packages even when the parent test runner has PySide6.
        # Discover (but do not run) the full suite to avoid recursive subprocesses.
        probe = """
import importlib.util
import sys
import unittest

assert importlib.util.find_spec('PySide6') is None
loader = unittest.TestLoader()
suite = loader.discover('tests', top_level_dir='.')
assert not loader.errors, '\\n'.join(loader.errors)
assert suite.countTestCases() > 0
assert not any(name == 'PySide6' or name.startswith('PySide6.') for name in sys.modules)
from tests.test_qml_smoke import QmlSmokeTests
result = unittest.TestResult()
loader.loadTestsFromTestCase(QmlSmokeTests).run(result)
assert result.wasSuccessful(), (result.errors, result.failures)
assert result.testsRun == len(result.skipped) == 1, result.skipped
assert result.skipped[0][1] == 'PySide6 is not installed'
"""
        result = subprocess.run(
            [sys.executable, "-S", "-c", probe],
            cwd=Path(__file__).resolve().parents[1],
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
