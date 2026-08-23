"""Main application window: library, runners, and settings."""

from __future__ import annotations

import subprocess
import threading
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .add_game_dialog import AddGameDialog  # noqa: E402
from .covers import fetch_cover  # noqa: E402
from .installers_page import InstallersPage  # noqa: E402
from .models import Game  # noqa: E402
from .plugins_page import PluginsPage  # noqa: E402
from .runners import create_desktop_shortcut, launch, tool_command  # noqa: E402
from .runners_dialog import RunnersPage  # noqa: E402
from .settings_page import SettingsPage  # noqa: E402


class GameCard(Gtk.Box):
    """A single tile in the library grid."""

    def __init__(self, game: Game, runner_label: str, window: "MainWindow") -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.game = game
        self.window = window
        self.add_css_class("card")
        self.add_css_class("game-card")
        self.set_size_request(196, 228)

        click = Gtk.GestureClick()
        click.connect("pressed", self._on_pressed)
        self.add_controller(click)

        artwork = _cover_picture(game, pixel_size=64, width=176, height=168)
        artwork.add_css_class("game-art")
        self.append(artwork)

        if game.category and game.category != "Uncategorized":
            badge = Gtk.Label(label=game.category)
            badge.add_css_class("game-badge")
            badge.set_halign(Gtk.Align.CENTER)
            self.append(badge)

        title = Gtk.Label(label=game.name)
        title.add_css_class("game-title")
        title.add_css_class("heading")
        title.set_ellipsize(3)
        title.set_margin_start(8)
        title.set_margin_end(8)
        self.append(title)

        subtitle = Gtk.Label(label=runner_label)
        subtitle.add_css_class("caption")
        subtitle.add_css_class("dim-label")
        subtitle.set_ellipsize(3)
        self.append(subtitle)

        buttons = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        buttons.set_halign(Gtk.Align.CENTER)
        buttons.set_margin_top(2)
        buttons.set_margin_bottom(10)

        play = Gtk.Button(label="Play")
        play.set_tooltip_text("Play")
        play.add_css_class("suggested-action")
        play.add_css_class("pill")
        play.add_css_class("play-pill")
        play.connect("clicked", lambda *_: window.play_game(game))
        buttons.append(play)

        menu_button = Gtk.MenuButton(icon_name="view-more-symbolic")
        menu_button.add_css_class("circular")
        menu_button.add_css_class("flat")
        menu_button.set_tooltip_text("More actions")
        menu_button.set_menu_model(window.game_menu_model())
        menu_button.connect("notify::active", lambda btn, *_: window.select_game(game) if btn.get_active() else None)
        buttons.append(menu_button)

        self.append(buttons)

    def _on_pressed(self, gesture, n_press, _x, _y):
        self.window.select_game(self.game)
        if gesture.get_current_button() == 3:
            self.window.popup_game_menu(self, self.game)
        elif n_press == 2:
            self.window.play_game(self.game)


class MainWindow(Adw.ApplicationWindow):
    def __init__(self, application) -> None:
        super().__init__(application=application, title="GameHandler")
        self.app = application
        self.set_default_size(1100, 720)
        self.selected_game: Game | None = None

        self.toasts = Adw.ToastOverlay()
        self.set_content(self.toasts)

        split = Adw.NavigationSplitView(min_sidebar_width=220, max_sidebar_width=280)
        self.toasts.set_child(split)

        sidebar_page = Adw.NavigationPage(title="GameHandler")
        sidebar_view = Adw.ToolbarView()
        sidebar_page.set_child(sidebar_view)
        sidebar_header = Adw.HeaderBar()
        sidebar_header.set_title_widget(
            Adw.WindowTitle(title="GameHandler", subtitle="Wine & Proton")
        )
        sidebar_view.add_top_bar(sidebar_header)

        self.nav_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.nav_list.add_css_class("navigation-sidebar")
        self.nav_list.add_css_class("gh-sidebar")
        self.nav_list.set_activate_on_single_click(True)
        self.nav_list.set_margin_top(8)
        # row-selected fires on a single click; row-activated does not for ActionRows.
        self.nav_list.connect("row-selected", self._on_nav)
        for icon, title in (
            ("applications-games-symbolic", "Library"),
            ("application-x-executable-symbolic", "Installers"),
            ("folder-download-symbolic", "Runners"),
            ("system-software-install-symbolic", "Plugins"),
            ("emblem-system-symbolic", "Settings"),
        ):
            row = Adw.ActionRow(title=title)
            row.add_prefix(Gtk.Image.new_from_icon_name(icon))
            self.nav_list.append(row)
        sidebar_view.set_content(self.nav_list)
        split.set_sidebar(sidebar_page)

        content_page = Adw.NavigationPage(title="Library")
        self.content_stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
        content_page.set_child(self.content_stack)
        split.set_content(content_page)
        self.split = split
        self.content_page = content_page

        self.content_stack.add_named(self._build_library(), "library")
        self.installers_page = InstallersPage(
            self.app.runner_manager,
            self.app.settings,
            self.toast,
            on_installed=self._on_game_saved,
        )
        self.content_stack.add_named(self.installers_page, "installers")
        self.runners_page = RunnersPage(
            self.app.runner_manager,
            self.app.proton_manager,
            self.toast,
            on_changed=self._on_runners_changed,
        )
        self.content_stack.add_named(self.runners_page, "runners")
        self.plugins_page = PluginsPage(self.toast)
        self.content_stack.add_named(self.plugins_page, "plugins")
        self.settings_page = SettingsPage(
            self.app.settings, self.app.runner_manager, self._on_settings_changed
        )
        self.content_stack.add_named(self.settings_page, "settings")

        self.nav_list.select_row(self.nav_list.get_row_at_index(0))
        self._popover = Gtk.PopoverMenu()
        self._popover.set_menu_model(self.game_menu_model())
        self._popover.set_parent(self)
        self._add_window_actions()

        self.refresh()

    def _add_window_actions(self):
        mapping = {
            "play": lambda *_: self.play_game(),
            "edit": lambda *_: self.edit_selected(),
            "remove": lambda *_: self.remove_selected(),
            "winecfg": lambda *_: self.run_tool("winecfg"),
            "winetricks": lambda *_: self.run_tool("winetricks"),
            "open-prefix": lambda *_: self.open_prefix(),
            "shortcut": lambda *_: self.create_shortcut(),
            "fetch-cover": lambda *_: self.fetch_selected_cover(),
        }
        for name, callback in mapping.items():
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", callback)
            self.add_action(action)

    def _build_library(self) -> Gtk.Widget:
        toolbar_view = Adw.ToolbarView()
        header = Adw.HeaderBar()
        header.set_title_widget(Adw.WindowTitle(title="Library", subtitle="Your games"))
        toolbar_view.add_top_bar(header)

        add_button = Gtk.Button()
        add_button.set_child(
            Adw.ButtonContent(icon_name="list-add-symbolic", label="Add Game")
        )
        add_button.add_css_class("suggested-action")
        add_button.connect("clicked", lambda *_: self.open_add_game_dialog())
        header.pack_start(add_button)

        self.search_button = Gtk.ToggleButton(icon_name="system-search-symbolic")
        self.search_button.set_tooltip_text("Search library")
        header.pack_end(self.search_button)

        self.view_button = Gtk.ToggleButton(icon_name="view-list-symbolic")
        self.view_button.set_tooltip_text("Toggle list view")
        self.view_button.set_active(self.app.settings.view_mode == "list")
        self.view_button.connect("toggled", self._on_view_toggle)
        header.pack_end(self.view_button)

        self.category_button = Gtk.DropDown()
        self.category_button.set_tooltip_text("Filter by category")
        header.pack_end(self.category_button)

        menu_button = Gtk.MenuButton(icon_name="open-menu-symbolic")
        menu_button.set_menu_model(_app_menu())
        header.pack_end(menu_button)

        self.search_bar = Gtk.SearchBar()
        self.search_entry = Gtk.SearchEntry(placeholder_text="Search games…")
        self.search_entry.connect("search-changed", lambda *_: self.refresh())
        self.search_bar.set_child(self.search_entry)
        self.search_bar.connect_entry(self.search_entry)
        self.search_button.bind_property(
            "active",
            self.search_bar,
            "search-mode-enabled",
            2 | 1,  # BIDIRECTIONAL | SYNC_CREATE
        )
        toolbar_view.add_top_bar(self.search_bar)

        self.library_stack = Gtk.Stack()
        toolbar_view.set_content(self.library_stack)

        self.empty_state = Adw.StatusPage(
            icon_name="applications-games-symbolic",
            title="No games yet",
            description="Add a Windows or Linux game, pick a Proton or Wine runner, and play.",
        )
        empty_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        empty_box.set_halign(Gtk.Align.CENTER)
        empty_add = Gtk.Button(label="Add your first game")
        empty_add.add_css_class("pill")
        empty_add.add_css_class("suggested-action")
        empty_add.connect("clicked", lambda *_: self.open_add_game_dialog())
        empty_install = Gtk.Button(label="Easy install")
        empty_install.add_css_class("pill")
        empty_install.connect("clicked", lambda *_: self.show_page("installers"))
        empty_runners = Gtk.Button(label="Download a runner")
        empty_runners.add_css_class("pill")
        empty_runners.connect("clicked", lambda *_: self.show_page("runners"))
        empty_box.append(empty_add)
        empty_box.append(empty_install)
        empty_box.append(empty_runners)
        self.empty_state.set_child(empty_box)
        self.library_stack.add_named(self.empty_state, "empty")

        self.no_results = Adw.StatusPage(
            icon_name="system-search-symbolic",
            title="No matching games",
            description="Try a different search.",
        )
        self.library_stack.add_named(self.no_results, "no-results")

        scrolled = Gtk.ScrolledWindow(hexpand=True, vexpand=True)
        scrolled.add_css_class("library-scroller")
        self.flowbox = Gtk.FlowBox(
            valign=Gtk.Align.START,
            max_children_per_line=6,
            min_children_per_line=1,
            selection_mode=Gtk.SelectionMode.NONE,
            homogeneous=True,
            column_spacing=10,
            row_spacing=10,
        )
        self.flowbox.set_margin_top(16)
        self.flowbox.set_margin_bottom(16)
        self.flowbox.set_margin_start(16)
        self.flowbox.set_margin_end(16)
        scrolled.set_child(self.flowbox)
        self.library_stack.add_named(scrolled, "grid")

        list_scrolled = Gtk.ScrolledWindow(hexpand=True, vexpand=True)
        self.listbox = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.listbox.add_css_class("boxed-list")
        self.listbox.set_margin_top(16)
        self.listbox.set_margin_bottom(16)
        self.listbox.set_margin_start(20)
        self.listbox.set_margin_end(20)
        list_scrolled.set_child(self.listbox)
        self.library_stack.add_named(list_scrolled, "list")
        self._reload_category_filter()
        self.category_button.connect("notify::selected", lambda *_: self.refresh())
        return toolbar_view

    def show_page(self, name: str):
        mapping = {"library": 0, "installers": 1, "runners": 2, "plugins": 3, "settings": 4}
        index = mapping.get(name, 0)
        self.nav_list.select_row(self.nav_list.get_row_at_index(index))
        self._show_index(index)

    def _on_nav(self, _list, row):
        if row is None:
            return
        self._show_index(row.get_index())

    def _show_index(self, index: int):
        names = ("library", "installers", "runners", "plugins", "settings")
        titles = ("Library", "Installers", "Runners", "Plugins", "Settings")
        name = names[index]
        self.content_stack.set_visible_child_name(name)
        self.content_page.set_title(titles[index])
        if name == "plugins":
            self.plugins_page.refresh()
        if name == "installers":
            self.installers_page.reload_runners()

    def _reload_category_filter(self):
        current = self._selected_category_filter()
        names = ["All"] + self.app.library.categories()
        model = Gtk.StringList()
        selected = 0
        for index, name in enumerate(names):
            model.append(name)
            if name == current:
                selected = index
        self.category_button.set_model(model)
        self.category_button.set_selected(selected)

    def _selected_category_filter(self) -> str:
        model = self.category_button.get_model()
        if model is None:
            return "All"
        idx = self.category_button.get_selected()
        if 0 <= idx < model.get_n_items():
            return model.get_string(idx)
        return "All"

    def _on_view_toggle(self, button):
        self.app.settings.view_mode = "list" if button.get_active() else "grid"
        self.app.settings.save()
        self.settings_page.view_row.set_selected(1 if button.get_active() else 0)
        self.refresh()

    def _on_settings_changed(self, settings):
        self.app.apply_color_scheme(settings.color_scheme)
        self.view_button.set_active(settings.view_mode == "list")
        self.refresh()

    def _on_runners_changed(self):
        self.settings_page.reload_runners()
        self.installers_page.reload_runners()
        self.refresh()

    def refresh(self):
        child = self.flowbox.get_first_child()
        while child is not None:
            nxt = child.get_next_sibling()
            self.flowbox.remove(child)
            child = nxt
        row = self.listbox.get_first_child()
        while row is not None:
            nxt = row.get_next_sibling()
            self.listbox.remove(row)
            row = nxt

        query = self.search_entry.get_text() if self.search_bar.get_search_mode() else ""
        category = self._selected_category_filter()
        games = self.app.library.search(query, category=category)

        if not self.app.library.all():
            self.library_stack.set_visible_child_name("empty")
            return
        if not games:
            self.library_stack.set_visible_child_name("no-results")
            return

        view = "list" if self.app.settings.view_mode == "list" else "grid"
        self.library_stack.set_visible_child_name(view)
        for game in games:
            label = self._runner_label(game)
            self.flowbox.append(GameCard(game, label, self))
            self.listbox.append(self._list_row(game, label))

    def _runner_label(self, game: Game) -> str:
        if game.is_linux:
            return "Linux native"
        return self.app.runner_manager.label(game.runner)

    def _list_row(self, game: Game, runner_label: str) -> Gtk.Widget:
        subtitle = runner_label
        if game.category and game.category != "Uncategorized":
            subtitle = f"{game.category} · {runner_label}"
        row = Adw.ActionRow(title=game.name, subtitle=subtitle)
        row.add_prefix(_cover_picture(game, pixel_size=32, width=40, height=56))
        play = Gtk.Button(icon_name="media-playback-start-symbolic")
        play.add_css_class("suggested-action")
        play.add_css_class("circular")
        play.set_valign(Gtk.Align.CENTER)
        play.set_tooltip_text("Play")
        play.connect("clicked", lambda *_: self.play_game(game))
        row.add_suffix(play)
        more = Gtk.MenuButton(icon_name="view-more-symbolic")
        more.add_css_class("flat")
        more.set_valign(Gtk.Align.CENTER)
        more.set_menu_model(self.game_menu_model())
        more.connect("notify::active", lambda *_: self.select_game(game))
        row.add_suffix(more)
        gesture = Gtk.GestureClick()
        gesture.connect("pressed", lambda *_: self.select_game(game))
        row.add_controller(gesture)
        return row

    def game_menu_model(self) -> Gio.Menu:
        menu = Gio.Menu()
        menu.append("Play", "win.play")
        menu.append("Edit", "win.edit")
        menu.append("Find cover", "win.fetch-cover")
        tools = Gio.Menu()
        tools.append("Winecfg", "win.winecfg")
        tools.append("Winetricks", "win.winetricks")
        tools.append("Open prefix folder", "win.open-prefix")
        menu.append_submenu("Prefix tools", tools)
        menu.append("Create desktop shortcut", "win.shortcut")
        menu.append("Remove", "win.remove")
        return menu

    def select_game(self, game: Game):
        self.selected_game = game

    def popup_game_menu(self, widget, game: Game):
        self.select_game(game)
        self._popover.unparent()
        self._popover.set_parent(widget)
        self._popover.popup()

    def open_add_game_dialog(self, game: Game | None = None):
        settings = self.app.settings
        dialog = AddGameDialog(
            self.app.runner_manager,
            self._on_game_saved,
            game=game,
            default_runner=settings.default_runner,
            default_mangohud=settings.default_mangohud,
            default_gamemode=settings.default_gamemode,
            default_prefer_sdl=settings.default_prefer_sdl,
            default_esync=settings.default_esync,
            default_fsync=settings.default_fsync,
            default_dxvk=settings.default_dxvk,
            default_vkd3d=settings.default_vkd3d,
            default_nvapi=settings.default_nvapi,
            default_fsr=settings.default_fsr,
            default_battleye=settings.default_battleye,
            default_eac=settings.default_eac,
            default_gamescope=settings.default_gamescope,
            default_virtual_desktop=settings.default_virtual_desktop,
            extra_categories=self.app.library.categories(),
            toast=self.toast,
        )
        dialog.present(self)

    def open_runners_window(self):
        self.show_page("runners")

    def _on_game_saved(self, game: Game):
        existing = self.app.library.get(game.id)
        if existing:
            self.app.library.update(game)
            self.toast(f"Updated “{game.name}”")
        else:
            self.app.library.add(game)
            self.toast(f"Added “{game.name}”")
        self._reload_category_filter()
        self.refresh()
        if not game.cover_path:
            self._autofetch_cover(game)

    def play_game(self, game: Game | None = None):
        game = game or self.selected_game
        if not game:
            return
        try:
            launch(game, self.app.runner_manager)
        except Exception as exc:  # noqa: BLE001 - surface any launch failure
            self.toast(f"Could not launch “{game.name}”: {exc}")
            return
        self.app.library.mark_played(game.id)
        self.toast(f"Launching “{game.name}”…")
        if self.app.settings.close_on_launch:
            self.set_visible(False)

    def edit_selected(self):
        if self.selected_game:
            self.open_add_game_dialog(self.selected_game)

    def remove_selected(self):
        game = self.selected_game
        if not game:
            return
        self.app.library.remove(game.id)
        self.selected_game = None
        self._reload_category_filter()
        self.refresh()
        self.toast(f"Removed “{game.name}”")

    def fetch_selected_cover(self):
        if self.selected_game:
            self._autofetch_cover(self.selected_game, announce=True)

    def _autofetch_cover(self, game: Game, announce: bool = False):
        if announce:
            self.toast(f"Searching Steam for “{game.name}”…")

        def worker():
            try:
                hit = fetch_cover(game.name, game.id)
                GLib.idle_add(self._cover_ready, game.id, hit)
            except Exception as exc:  # noqa: BLE001
                if announce:
                    GLib.idle_add(self.toast, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _cover_ready(self, game_id: str, hit):
        game = self.app.library.get(game_id)
        if not game:
            return False
        game.cover_path = hit.cover_path
        game.steam_appid = hit.appid
        if (not game.category or game.category == "Uncategorized") and hit.category:
            game.category = hit.category
        self.app.library.update(game)
        self._reload_category_filter()
        self.refresh()
        self.toast(f"Cover set from Steam: {hit.name}")
        return False

    def run_tool(self, tool: str):
        game = self.selected_game
        if not game:
            self.toast("Select a game first")
            return
        if game.is_linux:
            self.toast("Prefix tools are only available for Windows games")
            return
        try:
            argv, env = tool_command(game, self.app.runner_manager, tool)
            subprocess.Popen(argv, env=env)
        except Exception as exc:  # noqa: BLE001
            self.toast(str(exc))
            return
        self.toast(f"Opening {tool} for “{game.name}”")

    def open_prefix(self):
        game = self.selected_game
        if not game:
            return
        from . import config

        prefix = game.prefix_path or str(config.prefixes_dir() / game.id)
        Path(prefix).mkdir(parents=True, exist_ok=True)
        Gio.AppInfo.launch_default_for_uri(Path(prefix).resolve().as_uri(), None)

    def create_shortcut(self):
        game = self.selected_game
        if not game:
            return
        exe = shutil_which_gamehandler()
        command = f"{exe} --launch {game.id}"
        path = create_desktop_shortcut(game, command)
        self.toast(f"Shortcut created at {path}")

    def toast(self, message: str):
        self.toasts.add_toast(Adw.Toast(title=message, timeout=3))


def _cover_picture(game: Game, pixel_size: int = 64, width: int = 180, height: int = 240) -> Gtk.Widget:
    path = game.cover_path
    if path and Path(path).is_file():
        picture = Gtk.Picture.new_for_filename(path)
        picture.set_size_request(width, height)
        picture.set_can_shrink(True)
        if hasattr(Gtk, "ContentFit"):
            picture.set_content_fit(Gtk.ContentFit.COVER)
        picture.add_css_class("game-cover")
        return picture
    artwork = Gtk.Image.new_from_icon_name("applications-games-symbolic")
    artwork.set_pixel_size(pixel_size)
    artwork.set_vexpand(pixel_size >= 48)
    artwork.add_css_class("dim-label")
    return artwork


def shutil_which_gamehandler() -> str:
    from shutil import which

    return which("gamehandler") or "python3 -m gamehandler"


def _app_menu():
    menu = Gio.Menu()
    menu.append("Add Game", "app.add-game")
    menu.append("Easy Installers", "app.installers")
    menu.append("Manage Runners", "app.manage-runners")
    menu.append("Plugins", "app.manage-plugins")
    menu.append("Preferences", "app.preferences")
    menu.append("About GameHandler", "app.about")
    menu.append("Quit", "app.quit")
    return menu
