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
from .credits import ACKNOWLEDGEMENT, about_credit_sections  # noqa: E402
from .models import Library  # noqa: E402
from .runners import ProtonManager, RunnerManager, launch  # noqa: E402
from .settings import Settings  # noqa: E402
from .window import MainWindow  # noqa: E402

_SCHEME = {
    "system": Adw.ColorScheme.DEFAULT,
    "light": Adw.ColorScheme.FORCE_LIGHT,
    "dark": Adw.ColorScheme.FORCE_DARK,
}

SHORTCUTS = (
    ("Library", (
        ("<primary>N", "Add a game"),
        ("<primary>F", "Search the library"),
        ("<primary>comma", "Preferences"),
        ("<primary>Q", "Quit"),
    )),
)


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
        self._add_action("credits", self._on_credits)
        self._add_action("preferences", self._on_preferences, ["<primary>comma"])
        self._add_action("shortcuts", self._on_shortcuts, ["<primary>question"])
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
        _register_source_icons()
        _load_style()

    def do_activate(self):
        if not self.window:
            self.window = MainWindow(application=self)
        self.window.set_visible(True)
        self.window.present()

    def apply_color_scheme(self, scheme: str) -> None:
        manager = Adw.StyleManager.get_default()
        manager.set_color_scheme(_SCHEME.get(scheme, Adw.ColorScheme.FORCE_DARK))

    def _show_page(self, name: str):
        if self.window:
            self.window.show_page(name)

    def _on_add_game(self, *_args):
        if self.window:
            self.window.open_add_game_dialog()

    def _on_installers(self, *_args):
        self._show_page("installers")

    def _on_manage_runners(self, *_args):
        self._show_page("runners")

    def _on_manage_plugins(self, *_args):
        self._show_page("plugins")

    def _on_credits(self, *_args):
        self._show_page("credits")

    def _on_preferences(self, *_args):
        self._show_page("settings")

    def _on_quit(self, *_args):
        self.quit()

    def _on_shortcuts(self, *_args):
        dialog = Adw.AlertDialog(heading="Keyboard shortcuts", body="")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        for group_title, entries in SHORTCUTS:
            heading = Gtk.Label(label=group_title, xalign=0)
            heading.add_css_class("heading")
            box.append(heading)
            for accel, description in entries:
                row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
                label = Gtk.Label(label=description, xalign=0, hexpand=True)
                shortcut = Gtk.ShortcutLabel(accelerator=accel)
                row.append(label)
                row.append(shortcut)
                box.append(row)
        dialog.set_extra_child(box)
        dialog.add_response("close", "Close")
        dialog.set_default_response("close")
        dialog.set_close_response("close")
        dialog.present(self.window)

    def _on_about(self, *_args):
        comments = (
            "A modern game manager for running Windows games on Linux with Wine "
            "and Proton. GameHandler implements none of that compatibility work "
            "itself — it downloads the upstream projects' own builds, sets up "
            "isolated prefixes, and gets out of their way.\n\n" + ACKNOWLEDGEMENT
        )
        # Adw.AboutDialog superseded Adw.AboutWindow in libadwaita 1.5.
        factory = getattr(Adw, "AboutDialog", None)
        about = (factory or Adw.AboutWindow)(
            application_name="GameHandler",
            application_icon=APP_ID,
            developer_name="GameHandler contributors",
            version=__version__,
            comments=comments,
            website="https://github.com/goshitsarch-eng/GamesHandler",
            issue_url="https://github.com/goshitsarch-eng/GamesHandler/issues",
            license_type=Gtk.License.GPL_3_0,
        )
        for title, people in about_credit_sections():
            about.add_credit_section(title, people)
        about.add_legal_section(
            "Upstream runtimes",
            None,
            Gtk.License.CUSTOM,
            "Proton and Wine builds are downloaded from their maintainers at your "
            "request and remain under their own licenses. The bundled DXVK runtime "
            "is distributed under the zlib license; its full text is installed "
            "alongside this application.",
        )
        if factory is not None:
            about.present(self.window)
        else:  # pragma: no cover - libadwaita < 1.5
            about.set_transient_for(self.window)
            about.present()


def _register_source_icons() -> None:
    """Make the app icon resolvable when running from a source checkout.

    An installed build gets it from the icon theme; ``python3 -m gamehandler``
    otherwise shows a broken-image placeholder in the About dialog.
    """
    display = Gdk.Display.get_default()
    if display is None:
        return
    icons = Path(__file__).resolve().parent.parent / "data" / "icons"
    if icons.is_dir():
        Gtk.IconTheme.get_for_display(display).add_search_path(str(icons))


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
    app = GameHandlerApplication()
    return app.run([argv[0], *remaining])
