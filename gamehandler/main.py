"""Application object and entry point."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
gi.require_version("Gdk", "4.0")

from gi.repository import Adw, Gdk, Gio, Gtk  # noqa: E402

from . import APP_ID, __version__, config  # noqa: E402
from .models import Library  # noqa: E402
from .runners import ProtonManager, RunnerManager, launch  # noqa: E402
from .settings import Settings  # noqa: E402
from .window import MainWindow  # noqa: E402

_SCHEME = {
    "system": Adw.ColorScheme.DEFAULT,
    "light": Adw.ColorScheme.FORCE_LIGHT,
    "dark": Adw.ColorScheme.FORCE_DARK,
}


class GameHandlerApplication(Adw.Application):
    def __init__(self) -> None:
        super().__init__(
            application_id=APP_ID,
            flags=Gio.ApplicationFlags.DEFAULT_FLAGS,
        )
        config.ensure_dirs()
        self.settings = Settings.load()
        self.library = Library()
        self.runner_manager = RunnerManager()
        self.proton_manager = ProtonManager()
        self.window: MainWindow | None = None

        self._add_action("add-game", self._on_add_game, ["<primary>n"])
        self._add_action("installers", self._on_installers)
        self._add_action("manage-runners", self._on_manage_runners)
        self._add_action("manage-plugins", self._on_manage_plugins)
        self._add_action("preferences", self._on_preferences, ["<primary>comma"])
        self._add_action("about", self._on_about)
        self._add_action("quit", self._on_quit, ["<primary>q"])

    def _add_action(self, name, callback, accels=None):
        action = Gio.SimpleAction.new(name, None)
        action.connect("activate", callback)
        self.add_action(action)
        if accels:
            self.set_accels_for_action(f"app.{name}", accels)

    def do_startup(self):
        Adw.Application.do_startup(self)
        self.apply_color_scheme(self.settings.color_scheme)
        _load_style()

    def do_activate(self):
        if not self.window:
            self.window = MainWindow(application=self)
        self.window.present()

    def apply_color_scheme(self, scheme: str) -> None:
        manager = Adw.StyleManager.get_default()
        manager.set_color_scheme(_SCHEME.get(scheme, Adw.ColorScheme.FORCE_DARK))

    def _on_add_game(self, *_args):
        if self.window:
            self.window.open_add_game_dialog()

    def _on_installers(self, *_args):
        if self.window:
            self.window.show_page("installers")

    def _on_manage_runners(self, *_args):
        if self.window:
            self.window.show_page("runners")

    def _on_manage_plugins(self, *_args):
        if self.window:
            self.window.show_page("plugins")

    def _on_preferences(self, *_args):
        if self.window:
            self.window.show_page("settings")

    def _on_quit(self, *_args):
        self.quit()

    def _on_about(self, *_args):
        about = Adw.AboutWindow(
            transient_for=self.window,
            application_name="GameHandler",
            application_icon=APP_ID,
            developer_name="GameHandler contributors",
            version=__version__,
            comments=(
                "A modern game manager for running Windows games on Linux "
                "with Wine and Proton. Download Proton-GE, Proton-CachyOS, "
                "Proton-EM, Kron4ek Wine, and more — then pick a runner per game."
            ),
            website="https://github.com/goshitsarch-eng/GamesHandler",
            license_type=Gtk.License.GPL_3_0,
        )
        about.present()


def _load_style() -> None:
    display = Gdk.Display.get_default()
    if display is None:
        return
    css_path = Path(__file__).with_name("style.css")
    if not css_path.exists():
        return
    provider = Gtk.CssProvider()
    provider.load_from_path(str(css_path))
    Gtk.StyleContext.add_provider_for_display(
        display,
        provider,
        Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
    )


def _launch_from_cli(game_id: str) -> int:
    config.ensure_dirs()
    library = Library()
    game = library.get(game_id)
    if game is None:
        print(f"GameHandler: no game with id {game_id}", file=sys.stderr)
        return 1
    launch(game, RunnerManager())
    library.mark_played(game.id)
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = list(argv if argv is not None else sys.argv)
    parser = argparse.ArgumentParser(prog="gamehandler", add_help=True)
    parser.add_argument("--launch", metavar="GAME_ID", help="Launch a library game by id")
    args, remaining = parser.parse_known_args(argv[1:])
    if args.launch:
        return _launch_from_cli(args.launch)
    app = GameHandlerApplication()
    return app.run([argv[0], *remaining])
