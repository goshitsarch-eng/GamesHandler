"""Instantiate the whole QML interface against the real backend.

Runs the offscreen QML engine with a stub ``org.kde.kirigami`` module (see
``tests/kirigami_stub``) so that a typo in a page, a renamed backend property,
or an invalid property assignment fails here instead of on a user's first
launch. Skipped automatically when PySide6 is not installed.
"""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path

try:  # pragma: no cover - environment probe
    import PySide6  # noqa: F401

    HAVE_PYSIDE6 = True
except ImportError:  # pragma: no cover
    HAVE_PYSIDE6 = False

QML_DIR = Path(__file__).resolve().parents[1] / "gamehandler" / "qml"

# Messages that mean the interface is actually broken, as opposed to cosmetic
# warnings (missing theme icons, network being unavailable, and so on).
_FATAL_MARKERS = (
    "is not a type",
    "Cannot assign",
    "is not defined",
    "TypeError",
    "ReferenceError",
    "Syntax error",
    "LAYER-ERROR",
    "is not a function",
    "Unable to assign",
    "Invalid property",
    "Duplicate",
    "non-existent property",
)


@unittest.skipUnless(HAVE_PYSIDE6, "PySide6 is not installed")
class QmlSmokeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        cls._tmp = tempfile.TemporaryDirectory()
        os.environ["GAMEHANDLER_CONFIG_HOME"] = str(Path(cls._tmp.name) / "config")
        os.environ["GAMEHANDLER_DATA_HOME"] = str(Path(cls._tmp.name) / "data")

        from PySide6.QtWidgets import QApplication

        cls.app = QApplication.instance() or QApplication([])

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _fatal(self, messages):
        return [
            message
            for message in messages
            if any(marker in message for marker in _FATAL_MARKERS)
        ]

    def test_every_page_instantiates_against_the_real_backend(self):
        from PySide6.QtCore import Q_ARG, QMetaObject, Qt
        from PySide6.QtQml import QQmlApplicationEngine

        from gamehandler.bridge import Backend
        from gamehandler.theme import ThemeManager
        from tests import kirigami_stub

        kirigami_stub.register()

        backend = Backend(theme_manager=ThemeManager(self.app))
        # The Runners page fetches releases when it appears, and saving a
        # coverless game starts a cover lookup; keep the test offline and
        # deterministic.
        backend.proton_manager.fetch_available = lambda **_kwargs: []
        backend.fetchCover = lambda _game_id: None

        # A populated library instantiates the grid, list, and cover
        # delegates rather than only the empty states.
        backend.saveGame(
            {
                "gameId": "smoke1",
                "name": "Smoke Test Game",
                "exePath": "/tmp/smoke.exe",
                "isLinux": False,
            }
        )

        engine = QQmlApplicationEngine()
        problems: list[str] = []
        engine.warnings.connect(
            lambda warnings: problems.extend(str(w) for w in warnings)
        )
        engine.rootContext().setContextProperty("backend", backend)
        engine.load(str(QML_DIR / "Main.qml"))
        self.assertTrue(engine.rootObjects(), f"Main.qml failed to load: {problems}")
        root = engine.rootObjects()[0]

        # Visit every page, then open the add-game form layer.
        for name in ("installers", "runners", "plugins", "credits", "settings", "library"):
            QMetaObject.invokeMethod(
                root,
                "showPage",
                Qt.ConnectionType.DirectConnection,
                Q_ARG("QVariant", name),
            )
            self.app.processEvents()
        # Both library layouts, then the add-game form layer.
        backend.settings.view_mode = "list"
        backend.settingsChanged.emit()
        backend.gamesChanged.emit()
        self.app.processEvents()
        backend.settings.view_mode = "grid"
        backend.settingsChanged.emit()
        backend.gamesChanged.emit()
        self.app.processEvents()
        QMetaObject.invokeMethod(
            root,
            "openGameForm",
            Qt.ConnectionType.DirectConnection,
            Q_ARG("QVariant", ""),
        )
        for _ in range(4):
            self.app.processEvents()

        fatal = self._fatal(problems)
        self.assertEqual(fatal, [], "QML reported errors:\n" + "\n".join(fatal))

        # Keep the engine and backend alive until interpreter exit: dropping
        # either mid-teardown floods stderr with teardown-only binding errors
        # that would drown real failures.
        self.__class__._engine = engine
        self.__class__._backend = backend
        # Engine teardown at interpreter exit re-evaluates bindings against a
        # nulled context property; silence that expected, harmless chatter.
        from PySide6.QtCore import qInstallMessageHandler

        qInstallMessageHandler(lambda _mode, _context, _message: None)


if __name__ == "__main__":
    unittest.main()
