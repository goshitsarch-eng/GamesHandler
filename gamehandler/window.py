"""Main application window: library, installers, runners, plugins, and settings."""

from __future__ import annotations

import subprocess
import threading
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .add_game_dialog import AddGameDialog  # noqa: E402
from .covers import accent_index, fetch_cover, initials  # noqa: E402
from .credits_page import CreditsPage  # noqa: E402
from .installers_page import InstallersPage  # noqa: E402
from .models import Game, format_last_played  # noqa: E402
from .plugins_page import PluginsPage  # noqa: E402
from .runners import create_desktop_shortcut, launch, tool_command  # noqa: E402
from .runners_dialog import RunnersPage  # noqa: E402
from .settings_page import SettingsPage  # noqa: E402

# (stack name, sidebar title, content subtitle, icon)
NAV_PAGES: tuple[tuple[str, str, str, str], ...] = (
    ("library", "Library", "Your games", "applications-games-symbolic"),
    ("installers", "Installers", "One-click launchers and apps", "system-software-install-symbolic"),
    ("runners", "Runners", "Proton and Wine builds", "folder-download-symbolic"),
    ("plugins", "Plugins", "Optional launch helpers", "application-x-addon-symbolic"),
    ("credits", "Credits", "The projects we stand on", "emblem-favorite-symbolic"),
    ("settings", "Settings", "Appearance and defaults", "emblem-system-symbolic"),
)
_PAGE_INDEX = {name: index for index, (name, _t, _s, _i) in enumerate(NAV_PAGES)}

SORT_LABELS = (("name", "Name"), ("recent", "Recently played"), ("added", "Recently added"))

# Card geometry. The tile is fixed-width on purpose: GtkFlowBox decides how many
# children fit per line from their *natural* width, so an unbounded title label
# would make every tile as wide as the longest game name and collapse the grid
# into a single column.
CARD_WIDTH = 188
COVER_HEIGHT = 172
TITLE_MAX_CHARS = 18


class GameCard(Gtk.Box):
    """A single tile in the library grid."""

    def __init__(self, game: Game, runner_label: str, window: "MainWindow") -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.game = game
        self.window = window
        self.add_css_class("card")
        self.add_css_class("game-card")
        self.set_size_request(CARD_WIDTH, -1)
        self.set_valign(Gtk.Align.START)
        self.set_tooltip_text(f"{game.name}\n{runner_label} · {format_last_played(game.last_played)}")

        click = Gtk.GestureClick()
        click.set_button(0)  # any button, so button 3 reaches _on_pressed
        click.connect("pressed", self._on_pressed)
        self.add_controller(click)

        artwork = cover_widget(game, width=CARD_WIDTH - 16, height=COVER_HEIGHT)
        artwork.set_margin_top(8)
        artwork.set_margin_start(8)
        artwork.set_margin_end(8)
        self.append(artwork)

        title = Gtk.Label(label=game.name)
        title.add_css_class("game-title")
        title.set_ellipsize(3)  # Pango.EllipsizeMode.END
        title.set_max_width_chars(TITLE_MAX_CHARS)
        title.set_width_chars(TITLE_MAX_CHARS)
        title.set_single_line_mode(True)
        title.set_margin_start(10)
        title.set_margin_end(10)
        title.set_margin_top(8)
        self.append(title)

        meta = game.display_category
        if meta == "Uncategorized":
            meta = runner_label
        else:
            meta = f"{meta} · {runner_label}"
        subtitle = Gtk.Label(label=meta)
        subtitle.add_css_class("caption")
        subtitle.add_css_class("dim-label")
        subtitle.set_ellipsize(3)
        subtitle.set_max_width_chars(TITLE_MAX_CHARS + 4)
        subtitle.set_width_chars(TITLE_MAX_CHARS + 4)
        subtitle.set_single_line_mode(True)
        subtitle.set_margin_start(10)
        subtitle.set_margin_end(10)
        self.append(subtitle)

        buttons = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        buttons.set_halign(Gtk.Align.CENTER)
        buttons.set_margin_top(10)
        buttons.set_margin_bottom(10)

        play = Gtk.Button()
        play.set_child(
            Adw.ButtonContent(icon_name="media-playback-start-symbolic", label="Play")
        )
        play.set_tooltip_text(f"Play {game.name}")
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
        menu_button.connect("notify::active", self._on_menu_active)
        buttons.append(menu_button)

        self.append(buttons)

    def _on_menu_active(self, button, *_args):
        if button.get_active():
            self.window.select_game(self.game)

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
        self.set_default_size(1180, 760)
        self.set_size_request(420, 480)
        self.selected_game: Game | None = None
        self._refreshing = False
        self._syncing_view = False
        self._reloading_categories = False

        self.toasts = Adw.ToastOverlay()
        self.set_content(self.toasts)

        split = Adw.NavigationSplitView(min_sidebar_width=210, max_sidebar_width=260)
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
        for _name, title, _subtitle, icon in NAV_PAGES:
            row = Adw.ActionRow(title=title)
            row.add_prefix(Gtk.Image.new_from_icon_name(icon))
            self.nav_list.append(row)
        sidebar_view.set_content(self.nav_list)

        sidebar_footer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        sidebar_footer.add_css_class("gh-sidebar-footer")
        credit_hint = Gtk.Button(label="Powered by Wine, Proton & DXVK")
        credit_hint.add_css_class("flat")
        credit_hint.add_css_class("gh-credit-link")
        credit_hint.set_tooltip_text("See everyone GameHandler is built on")
        credit_hint.connect("clicked", lambda *_: self.show_page("credits"))
        credit_hint.set_margin_top(6)
        credit_hint.set_margin_bottom(6)
        credit_hint.set_margin_start(8)
        credit_hint.set_margin_end(8)
        sidebar_footer.append(credit_hint)
        sidebar_view.add_bottom_bar(sidebar_footer)

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
        self.credits_page = CreditsPage()
        self.content_stack.add_named(self.credits_page, "credits")
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
            "remove": lambda *_: self.confirm_remove_selected(),
            "winecfg": lambda *_: self.run_tool("winecfg"),
            "winetricks": lambda *_: self.run_tool("winetricks"),
            "open-prefix": lambda *_: self.open_prefix(),
            "shortcut": lambda *_: self.create_shortcut(),
            "fetch-cover": lambda *_: self.fetch_selected_cover(),
            "search": lambda *_: self.focus_search(),
        }
        for name, callback in mapping.items():
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", callback)
            self.add_action(action)
        app = self.get_application()
        if app is not None:
            app.set_accels_for_action("win.search", ["<primary>f"])

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
        add_button.set_tooltip_text("Add a game to your library (Ctrl+N)")
        add_button.connect("clicked", lambda *_: self.open_add_game_dialog())
        header.pack_start(add_button)

        menu_button = Gtk.MenuButton(icon_name="open-menu-symbolic")
        menu_button.set_tooltip_text("Main menu")
        menu_button.set_menu_model(_app_menu())
        header.pack_end(menu_button)

        self.search_button = Gtk.ToggleButton(icon_name="system-search-symbolic")
        self.search_button.set_tooltip_text("Search library (Ctrl+F)")
        header.pack_end(self.search_button)

        self.view_button = Gtk.ToggleButton(icon_name="view-list-symbolic")
        self.view_button.set_tooltip_text("Toggle list view")
        self.view_button.set_active(self.app.settings.view_mode == "list")
        self.view_button.connect("toggled", self._on_view_toggle)
        header.pack_end(self.view_button)

        self.sort_button = Gtk.DropDown()
        self.sort_button.set_tooltip_text("Sort library")
        sort_model = Gtk.StringList()
        for _key, label in SORT_LABELS:
            sort_model.append(label)
        self.sort_button.set_model(sort_model)
        self.sort_button.set_selected(self._sort_index(self.app.settings.sort_mode))
        self.sort_button.connect("notify::selected", self._on_sort_changed)
        header.pack_end(self.sort_button)

        self.category_button = Gtk.DropDown()
        self.category_button.set_tooltip_text("Filter by category")
        header.pack_end(self.category_button)

        self.search_bar = Gtk.SearchBar()
        self.search_entry = Gtk.SearchEntry(placeholder_text="Search games…")
        self.search_entry.set_hexpand(True)
        self.search_entry.connect("search-changed", lambda *_: self.refresh())
        clamp = Adw.Clamp(maximum_size=680, tightening_threshold=480)
        clamp.set_child(self.search_entry)
        self.search_bar.set_child(clamp)
        self.search_bar.connect_entry(self.search_entry)
        self.search_bar.set_key_capture_widget(self)
        self.search_button.bind_property(
            "active",
            self.search_bar,
            "search-mode-enabled",
            2 | 1,  # BIDIRECTIONAL | SYNC_CREATE
        )
        self.search_bar.connect("notify::search-mode-enabled", lambda *_: self.refresh())
        toolbar_view.add_top_bar(self.search_bar)

        self.library_stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
        toolbar_view.set_content(self.library_stack)

        self.empty_state = Adw.StatusPage(
            icon_name="applications-games-symbolic",
            title="No games yet",
            description=(
                "Add a Windows or Linux game, install a store launcher in one click, "
                "or download a Proton build to get started."
            ),
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
            description="Try a different search, or clear the category filter.",
        )
        clear_filters = Gtk.Button(label="Clear filters")
        clear_filters.add_css_class("pill")
        clear_filters.set_halign(Gtk.Align.CENTER)
        clear_filters.connect("clicked", lambda *_: self.clear_filters())
        self.no_results.set_child(clear_filters)
        self.library_stack.add_named(self.no_results, "no-results")

        scrolled = Gtk.ScrolledWindow(hexpand=True, vexpand=True)
        scrolled.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scrolled.add_css_class("library-scroller")
        self.flowbox = Gtk.FlowBox(
            valign=Gtk.Align.START,
            halign=Gtk.Align.CENTER,
            max_children_per_line=8,
            min_children_per_line=1,
            selection_mode=Gtk.SelectionMode.NONE,
            homogeneous=True,
            column_spacing=14,
            row_spacing=14,
        )
        self.flowbox.set_margin_top(18)
        self.flowbox.set_margin_bottom(18)
        self.flowbox.set_margin_start(18)
        self.flowbox.set_margin_end(18)
        scrolled.set_child(self.flowbox)
        self.library_stack.add_named(scrolled, "grid")

        list_scrolled = Gtk.ScrolledWindow(hexpand=True, vexpand=True)
        list_scrolled.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        list_clamp = Adw.Clamp(maximum_size=920, tightening_threshold=680)
        self.listbox = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.listbox.add_css_class("boxed-list")
        self.listbox.set_valign(Gtk.Align.START)
        self.listbox.set_margin_top(18)
        self.listbox.set_margin_bottom(18)
        self.listbox.set_margin_start(18)
        self.listbox.set_margin_end(18)
        list_clamp.set_child(self.listbox)
        list_scrolled.set_child(list_clamp)
        self.library_stack.add_named(list_scrolled, "list")
        self._reload_category_filter()
        self.category_button.connect("notify::selected", self._on_category_changed)
        return toolbar_view

    def _on_category_changed(self, *_args):
        # Swapping the model mid-rebuild emits this twice; the caller refreshes.
        if not self._reloading_categories:
            self.refresh()

    # ------------------------------------------------------------------ navigation

    def show_page(self, name: str):
        index = _PAGE_INDEX.get(name, 0)
        row = self.nav_list.get_row_at_index(index)
        if row is not None and not row.is_selected():
            self.nav_list.select_row(row)
        self._show_index(index)

    def _on_nav(self, _list, row):
        if row is None:
            return
        self._show_index(row.get_index())

    def _show_index(self, index: int):
        if not 0 <= index < len(NAV_PAGES):
            index = 0
        name, title, subtitle, _icon = NAV_PAGES[index]
        self.content_stack.set_visible_child_name(name)
        self.content_page.set_title(title)
        self.split.set_show_content(True)
        if name == "plugins":
            self.plugins_page.refresh()
        elif name == "installers":
            self.installers_page.reload_runners()

    # ------------------------------------------------------------------- filtering

    def focus_search(self):
        self.show_page("library")
        self.search_button.set_active(True)
        self.search_entry.grab_focus()

    def clear_filters(self):
        self.search_entry.set_text("")
        self.search_button.set_active(False)
        if self.category_button.get_model() is not None:
            self.category_button.set_selected(0)
        self.refresh()

    def _sort_index(self, mode: str) -> int:
        for index, (key, _label) in enumerate(SORT_LABELS):
            if key == mode:
                return index
        return 0

    def _selected_sort(self) -> str:
        index = self.sort_button.get_selected()
        if 0 <= index < len(SORT_LABELS):
            return SORT_LABELS[index][0]
        return "name"

    def _on_sort_changed(self, *_args):
        mode = self._selected_sort()
        if mode != self.app.settings.sort_mode:
            self.app.settings.sort_mode = mode
            self.app.settings.save()
        self.refresh()

    def _reload_category_filter(self):
        current = self._selected_category_filter()
        names = ["All"] + self.app.library.categories()
        model = Gtk.StringList()
        selected = 0
        for index, name in enumerate(names):
            model.append(name)
            if name == current:
                selected = index
        self._reloading_categories = True
        try:
            self.category_button.set_model(model)
            self.category_button.set_selected(selected)
        finally:
            self._reloading_categories = False

    def _selected_category_filter(self) -> str:
        model = self.category_button.get_model()
        if model is None:
            return "All"
        idx = self.category_button.get_selected()
        if 0 <= idx < model.get_n_items():
            return model.get_string(idx)
        return "All"

    def _on_view_toggle(self, button):
        mode = "list" if button.get_active() else "grid"
        self.app.settings.view_mode = mode
        self.app.settings.save()
        if not self._syncing_view:
            self._syncing_view = True
            try:
                self.settings_page.sync_view_mode(mode)
            finally:
                self._syncing_view = False
        self.refresh()

    def _on_settings_changed(self, settings):
        self.app.apply_color_scheme(settings.color_scheme)
        if not self._syncing_view:
            self._syncing_view = True
            try:
                self.view_button.set_active(settings.view_mode == "list")
            finally:
                self._syncing_view = False
        self.refresh()

    def _on_runners_changed(self):
        self.settings_page.reload_runners()
        self.installers_page.reload_runners()
        self.refresh()

    # ---------------------------------------------------------------------- render

    def refresh(self):
        if self._refreshing:
            return
        self._refreshing = True
        try:
            self._render_library()
        finally:
            self._refreshing = False

    def _render_library(self):
        _clear_container(self.flowbox)
        _clear_container(self.listbox)

        query = self.search_entry.get_text() if self.search_bar.get_search_mode() else ""
        category = self._selected_category_filter()
        games = self.app.library.search(query, category=category, sort=self._selected_sort())

        if not len(self.app.library):
            self.library_stack.set_visible_child_name("empty")
            return
        if not games:
            self.library_stack.set_visible_child_name("no-results")
            return

        # Build only the view being shown; constructing both doubles the widget
        # and cover-loading cost of every refresh for a view nobody is looking at.
        list_view = self.app.settings.view_mode == "list"
        self.library_stack.set_visible_child_name("list" if list_view else "grid")
        for game in games:
            label = self._runner_label(game)
            if list_view:
                self.listbox.append(self._list_row(game, label))
            else:
                self.flowbox.append(GameCard(game, label, self))

    def _runner_label(self, game: Game) -> str:
        if game.is_linux:
            return "Linux native"
        return self.app.runner_manager.label(game.runner)

    def _list_row(self, game: Game, runner_label: str) -> Gtk.Widget:
        parts = [runner_label, format_last_played(game.last_played)]
        if game.display_category != "Uncategorized":
            parts.insert(0, game.display_category)
        row = Adw.ActionRow(title=game.name, subtitle=" · ".join(parts))
        row.set_title_lines(1)
        row.set_subtitle_lines(1)
        row.add_prefix(cover_widget(game, width=40, height=56, compact=True))

        play = Gtk.Button(icon_name="media-playback-start-symbolic")
        play.add_css_class("suggested-action")
        play.add_css_class("circular")
        play.set_valign(Gtk.Align.CENTER)
        play.set_tooltip_text(f"Play {game.name}")
        play.connect("clicked", lambda *_: self.play_game(game))
        row.add_suffix(play)

        more = Gtk.MenuButton(icon_name="view-more-symbolic")
        more.add_css_class("flat")
        more.add_css_class("circular")
        more.set_valign(Gtk.Align.CENTER)
        more.set_tooltip_text("More actions")
        more.set_menu_model(self.game_menu_model())
        more.connect(
            "notify::active",
            lambda button, *_: self.select_game(game) if button.get_active() else None,
        )
        row.add_suffix(more)

        row.set_activatable(True)
        row.connect("activated", lambda *_: (self.select_game(game), self.play_game(game)))
        gesture = Gtk.GestureClick()
        gesture.set_button(3)
        gesture.connect("pressed", lambda *_: self.popup_game_menu(row, game))
        row.add_controller(gesture)
        return row

    def game_menu_model(self) -> Gio.Menu:
        menu = Gio.Menu()
        primary = Gio.Menu()
        primary.append("Play", "win.play")
        primary.append("Edit", "win.edit")
        primary.append("Find cover art", "win.fetch-cover")
        menu.append_section(None, primary)
        tools = Gio.Menu()
        tools.append("Winecfg", "win.winecfg")
        tools.append("Winetricks", "win.winetricks")
        tools.append("Open prefix folder", "win.open-prefix")
        menu.append_submenu("Prefix tools", tools)
        trailing = Gio.Menu()
        trailing.append("Create desktop shortcut", "win.shortcut")
        trailing.append("Remove from library", "win.remove")
        menu.append_section(None, trailing)
        return menu

    def select_game(self, game: Game):
        self.selected_game = game

    def popup_game_menu(self, widget, game: Game):
        self.select_game(game)
        self._popover.unparent()
        self._popover.set_parent(widget)
        self._popover.popup()

    # --------------------------------------------------------------------- actions

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
        self.selected_game = game
        self._reload_category_filter()
        self.refresh()
        if not game.cover_path:
            self._autofetch_cover(game)

    def play_game(self, game: Game | None = None):
        game = game or self.selected_game
        if not game:
            self.toast("Select a game first")
            return
        try:
            launch(game, self.app.runner_manager)
        except Exception as exc:  # noqa: BLE001 - surface any launch failure
            self.toast(f"Could not launch “{game.name}”: {exc}")
            return
        self.app.library.mark_played(game.id)
        self.toast(f"Launching “{game.name}”…")
        self.refresh()
        if self.app.settings.close_on_launch:
            self.set_visible(False)

    def edit_selected(self):
        if self.selected_game:
            self.open_add_game_dialog(self.selected_game)
        else:
            self.toast("Select a game first")

    def confirm_remove_selected(self):
        game = self.selected_game
        if not game:
            self.toast("Select a game first")
            return
        dialog = Adw.AlertDialog(
            heading=f"Remove “{game.name}”?",
            body=(
                "This removes the game from your GameHandler library. "
                "Its Wine prefix and game files are left on disk."
            ),
        )
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("remove", "Remove")
        dialog.set_response_appearance("remove", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect("response", self._on_remove_response, game)
        dialog.present(self)

    def _on_remove_response(self, _dialog, response, game):
        if response != "remove":
            return
        self.app.library.remove(game.id)
        if self.selected_game is not None and self.selected_game.id == game.id:
            self.selected_game = None
        self._reload_category_filter()
        self.refresh()
        self.toast(f"Removed “{game.name}”")

    def fetch_selected_cover(self):
        if self.selected_game:
            self._autofetch_cover(self.selected_game, announce=True)
        else:
            self.toast("Select a game first")

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
        if game.display_category == "Uncategorized" and hit.category:
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
            self.toast("Select a game first")
            return
        if game.is_linux:
            self.toast("Linux games do not use a Wine prefix")
            return
        from . import config

        prefix = game.prefix_path or str(config.prefixes_dir() / game.id)
        try:
            Path(prefix).mkdir(parents=True, exist_ok=True)
            Gio.AppInfo.launch_default_for_uri(Path(prefix).resolve().as_uri(), None)
        except (OSError, GLib.Error) as exc:
            self.toast(f"Could not open the prefix folder: {exc}")

    def create_shortcut(self):
        game = self.selected_game
        if not game:
            self.toast("Select a game first")
            return
        command = f"{launcher_command()} --launch {game.id}"
        try:
            path = create_desktop_shortcut(game, command)
        except OSError as exc:
            self.toast(f"Could not create the shortcut: {exc}")
            return
        self.toast(f"Shortcut created at {path}")

    def toast(self, message: str):
        self.toasts.add_toast(Adw.Toast(title=message, timeout=4))


def _clear_container(container) -> None:
    child = container.get_first_child()
    while child is not None:
        nxt = child.get_next_sibling()
        container.remove(child)
        child = nxt


def cover_widget(
    game: Game, width: int = 172, height: int = 172, compact: bool = False
) -> Gtk.Widget:
    """A game's cover, or a coloured initials plate when it has none."""
    path = game.cover_path
    if path and Path(path).is_file():
        picture = Gtk.Picture.new_for_filename(path)
        picture.set_size_request(width, height)
        picture.set_can_shrink(True)
        if hasattr(Gtk, "ContentFit"):
            picture.set_content_fit(Gtk.ContentFit.COVER)
        picture.add_css_class("game-cover")
        picture.set_overflow(Gtk.Overflow.HIDDEN)
        return picture

    # An icon-name image renders as a broken-image glyph when the theme lacks the
    # symbolic, so draw our own placeholder instead of trusting the icon theme.
    plate = Gtk.Label(label=initials(game.name))
    plate.set_size_request(width, height)
    plate.add_css_class("game-cover")
    plate.add_css_class("game-art-plate")
    plate.add_css_class(f"gh-art-{accent_index(game.id or game.name)}")
    if compact:
        plate.add_css_class("game-art-plate-compact")
    plate.set_valign(Gtk.Align.CENTER)
    plate.set_halign(Gtk.Align.CENTER)
    return plate


def launcher_command() -> str:
    """The command a desktop shortcut should run to reach this install."""
    from shutil import which

    found = which("gamehandler")
    if not found:
        return "python3 -m gamehandler"
    return GLib.shell_quote(found) if " " in found else found


def _app_menu():
    menu = Gio.Menu()
    library = Gio.Menu()
    library.append("Add Game", "app.add-game")
    library.append("Easy Installers", "app.installers")
    menu.append_section(None, library)
    manage = Gio.Menu()
    manage.append("Manage Runners", "app.manage-runners")
    manage.append("Plugins", "app.manage-plugins")
    manage.append("Preferences", "app.preferences")
    menu.append_section(None, manage)
    about = Gio.Menu()
    about.append("Credits", "app.credits")
    about.append("Keyboard Shortcuts", "app.shortcuts")
    about.append("About GameHandler", "app.about")
    about.append("Quit", "app.quit")
    menu.append_section(None, about)
    return menu
