"""Application entry point: CLI handling and the Qt 6 / Kirigami interface.

The command-line paths (``--list``, ``--launch``, ``--version``) stay free of
any Qt import so desktop shortcuts and scripts work headlessly. The GUI is a
QML application built on Kirigami; PySide6 and the Kirigami QML modules are
only required once a window is actually wanted.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from . import APP_ID, __version__, config
from .models import Library
from .runners import RunnerManager, launch

_KIRIGAMI_HINT = (
    "GameHandler needs the Kirigami QML modules and a Qt Quick Controls "
    "desktop style. On Debian/Ubuntu install: qml6-module-org-kde-kirigami "
    "qml6-module-qtquick-dialogs qqc2-desktop-style. On Arch: kirigami "
    "qqc2-desktop-style. On Fedora: kf6-kirigami qqc2-desktop-style."
)


def _launch_from_cli(game_id: str) -> int:
    config.ensure_dirs()
    library = Library()
    game = library.get(game_id)
    if game is None:
        print(f"GameHandler: no game with id {game_id}", file=sys.stderr)
        return 1
    try:
        started = launch(game, RunnerManager())
    except Exception as exc:  # noqa: BLE001 - a shortcut must fail with a message
        print(f"GameHandler: could not launch {game.name}: {exc}", file=sys.stderr)
        return 1
    library.mark_played(game.id)
    # A shortcut that opens nothing and exits 0 tells the user nothing at all.
    reason = started.failure()
    if reason:
        print(f"GameHandler: {game.name} stopped right away: {reason}", file=sys.stderr)
        return 1
    return 0


def _list_games() -> int:
    config.ensure_dirs()
    games = Library().all()
    if not games:
        print("GameHandler: the library is empty")
        return 0
    for game in games:
        print(f"{game.id}\t{game.name}")
    return 0


def _source_icon_path() -> Path | None:
    """The scalable app icon, resolvable from a source checkout too."""
    candidate = (
        Path(__file__).resolve().parent.parent
        / "data"
        / "icons"
        / "hicolor"
        / "scalable"
        / "apps"
        / f"{APP_ID}.svg"
    )
    return candidate if candidate.is_file() else None


def run_gui(argv: list[str]) -> int:
    # qqc2-desktop-style is the Qt Quick Controls style Kirigami is designed
    # for; respect an explicit user override.
    os.environ.setdefault("QT_QUICK_CONTROLS_STYLE", "org.kde.desktop")

    from PySide6.QtGui import QIcon
    from PySide6.QtQml import QQmlApplicationEngine
    from PySide6.QtWidgets import QApplication

    from .bridge import Backend
    from .theme import ThemeManager

    app = QApplication(argv)
    app.setApplicationName("GameHandler")
    app.setApplicationDisplayName("GameHandler")
    app.setApplicationVersion(__version__)
    app.setOrganizationDomain("goshapps.com")
    app.setDesktopFileName(APP_ID)
    # Outside Plasma the active icon theme may carry none of the KDE icon
    # names the interface uses; Breeze as the fallback keeps the KDE look.
    if QIcon.fallbackThemeName() == "":
        QIcon.setFallbackThemeName("breeze")
    icon = QIcon.fromTheme(APP_ID)
    source_icon = _source_icon_path()
    if icon.isNull() and source_icon is not None:
        icon = QIcon(str(source_icon))
    if not icon.isNull():
        app.setWindowIcon(icon)

    config.ensure_dirs()
    theme = ThemeManager(app)
    backend = Backend(theme_manager=theme)
    theme.apply(backend.settings.color_scheme)

    engine = QQmlApplicationEngine()
    engine.rootContext().setContextProperty("backend", backend)
    main_qml = Path(__file__).resolve().parent / "qml" / "Main.qml"
    engine.load(str(main_qml))
    if not engine.rootObjects():
        print(f"GameHandler: could not load the interface.\n{_KIRIGAMI_HINT}", file=sys.stderr)
        return 1
    return app.exec()


def main(argv: list[str] | None = None) -> int:
    argv = list(argv if argv is not None else sys.argv)
    parser = argparse.ArgumentParser(prog="gamehandler", add_help=True)
    parser.add_argument("--version", action="version", version=f"GameHandler {__version__}")
    parser.add_argument("--launch", metavar="GAME_ID", help="Launch a library game by id")
    parser.add_argument(
        "--list", action="store_true", help="Print library game ids and names"
    )
    args, remaining = parser.parse_known_args(argv[1:])
    if args.list:
        return _list_games()
    if args.launch:
        return _launch_from_cli(args.launch)
    return run_gui([argv[0], *remaining])
