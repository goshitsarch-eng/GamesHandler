"""Dialog for adding or editing a game in the library."""

from __future__ import annotations

import threading
import uuid
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .covers import DEFAULT_CATEGORIES, copy_custom_cover, fetch_cover  # noqa: E402
from .models import Game  # noqa: E402
from .plugins import PLUGINS, install_plugin  # noqa: E402
from .runners import SYSTEM_WINE  # noqa: E402


class AddGameDialog(Adw.Dialog):
    def __init__(
        self,
        runner_manager,
        on_saved,
        game: Game | None = None,
        default_runner: str = SYSTEM_WINE,
        default_mangohud: bool = False,
        default_gamemode: bool = False,
        default_prefer_sdl: bool = False,
        extra_categories: list[str] | None = None,
        toast=None,
    ) -> None:
        super().__init__(title="Edit Game" if game else "Add Game")
        self.set_content_width(540)
        self.set_content_height(720)
        self.runner_manager = runner_manager
        self.on_saved = on_saved
        self.existing = game
        self.toast = toast or (lambda _msg: None)
        self.game_id = game.id if game else uuid.uuid4().hex
        self.cover_path = game.cover_path if game else ""
        self.steam_appid = game.steam_appid if game else 0
        self._cover_preview = None

        toolbar_view = Adw.ToolbarView()
        self.set_child(toolbar_view)

        header = Adw.HeaderBar()
        toolbar_view.add_top_bar(header)

        cancel = Gtk.Button(label="Cancel")
        cancel.connect("clicked", lambda *_: self.close())
        header.pack_start(cancel)

        self.save_button = Gtk.Button(label="Save" if game else "Add")
        self.save_button.add_css_class("suggested-action")
        self.save_button.set_sensitive(bool(game and game.name.strip()))
        self.save_button.connect("clicked", self._on_save)
        header.pack_end(self.save_button)

        page = Adw.PreferencesPage()
        toolbar_view.set_content(page)

        basics = Adw.PreferencesGroup(title="Game")
        page.add(basics)

        self.name_row = Adw.EntryRow(title="Name")
        self.name_row.connect("changed", self._validate)
        basics.add(self.name_row)

        kind_model = Gtk.StringList()
        kind_model.append("Windows (Wine / Proton)")
        kind_model.append("Linux native")
        self.kind_row = Adw.ComboRow(title="Type", model=kind_model)
        self.kind_row.connect("notify::selected", self._on_kind_changed)
        basics.add(self.kind_row)

        self.exe_row = Adw.EntryRow(title="Executable")
        browse = Gtk.Button(icon_name="document-open-symbolic")
        browse.set_valign(Gtk.Align.CENTER)
        browse.add_css_class("flat")
        browse.set_tooltip_text("Browse for an executable")
        browse.connect("clicked", self._on_browse)
        self.exe_row.add_suffix(browse)
        basics.add(self.exe_row)

        self.args_row = Adw.EntryRow(title="Launch arguments")
        basics.add(self.args_row)

        self.cwd_row = Adw.EntryRow(title="Working directory")
        basics.add(self.cwd_row)

        library = Adw.PreferencesGroup(
            title="Library",
            description="Categorize the game and set a cover. GameHandler can pull artwork from Steam by name.",
        )
        page.add(library)

        self.category_names = list(DEFAULT_CATEGORIES)
        for name in extra_categories or []:
            if name and name not in self.category_names:
                self.category_names.append(name)
        self.category_names.append("Custom…")
        category_model = Gtk.StringList()
        for name in self.category_names:
            category_model.append(name)
        self.category_row = Adw.ComboRow(title="Category", model=category_model)
        self.category_row.connect("notify::selected", self._on_category_changed)
        library.add(self.category_row)

        self.custom_category_row = Adw.EntryRow(title="Custom category")
        self.custom_category_row.set_visible(False)
        library.add(self.custom_category_row)

        self.cover_row = Adw.ActionRow(
            title="Cover art",
            subtitle="No cover yet — browse a file or fetch one from Steam",
        )
        fetch = Gtk.Button(label="Find cover")
        fetch.add_css_class("pill")
        fetch.set_valign(Gtk.Align.CENTER)
        fetch.set_tooltip_text("Search Steam for a matching cover")
        fetch.connect("clicked", self._on_fetch_cover)
        self.cover_row.add_suffix(fetch)
        cover_browse = Gtk.Button(icon_name="document-open-symbolic")
        cover_browse.add_css_class("flat")
        cover_browse.set_valign(Gtk.Align.CENTER)
        cover_browse.set_tooltip_text("Choose a custom cover image")
        cover_browse.connect("clicked", self._on_browse_cover)
        self.cover_row.add_suffix(cover_browse)
        library.add(self.cover_row)

        self.runner_group = Adw.PreferencesGroup(
            title="Compatibility tool",
            description="Choose which Proton or Wine build launches this game. See Runners for when to use each family.",
        )
        page.add(self.runner_group)

        self.runner_ids: list[str] = []
        model = Gtk.StringList()
        selected = 0
        preferred = game.runner if game else default_runner
        for index, (runner_id, label) in enumerate(runner_manager.choices()):
            self.runner_ids.append(runner_id)
            model.append(label)
            if runner_id == preferred:
                selected = index
        self.runner_row = Adw.ComboRow(title="Runner", model=model)
        self.runner_row.set_selected(selected)
        self.runner_group.add(self.runner_row)

        self.prefix_row = Adw.EntryRow(title="Wine prefix (optional)")
        self.prefix_row.set_tooltip_text("Leave empty to use an isolated prefix per game.")
        self.runner_group.add(self.prefix_row)

        tools = Adw.PreferencesGroup(
            title="Launch options",
            description="Optional helpers. Missing plugins can be installed from here.",
        )
        page.add(tools)

        self.mangohud_row = Adw.SwitchRow(
            title="MangoHud",
            subtitle="Performance overlay when MangoHud is installed",
        )
        self._attach_plugin_install(self.mangohud_row, "mangohud")
        tools.add(self.mangohud_row)

        self.gamemode_row = Adw.SwitchRow(
            title="Feral GameMode",
            subtitle="Ask the system to boost performance while the game runs",
        )
        self._attach_plugin_install(self.gamemode_row, "gamemode")
        tools.add(self.gamemode_row)

        self.sdl_row = Adw.SwitchRow(
            title="Prefer SDL",
            subtitle="Can fix controller issues in some games",
        )
        tools.add(self.sdl_row)

        self.wayland_row = Adw.SwitchRow(
            title="Wine Wayland driver",
            subtitle="Experimental. Works best on Proton-EM and recent GE-Proton.",
        )
        tools.add(self.wayland_row)

        self.hdr_row = Adw.SwitchRow(
            title="HDR",
            subtitle="Experimental. Requires a compatible Proton build and display.",
        )
        tools.add(self.hdr_row)

        extras = Adw.PreferencesGroup(title="Advanced")
        page.add(extras)
        self.extra_row = Adw.EntryRow(title="Additional application")
        self.extra_row.set_tooltip_text("Optional helper launched in the same prefix (trainer, overlay, …).")
        extras.add(self.extra_row)

        if game:
            self._populate(game)
        else:
            self.mangohud_row.set_active(default_mangohud)
            self.gamemode_row.set_active(default_gamemode)
            self.sdl_row.set_active(default_prefer_sdl)
        self._on_kind_changed()
        self._refresh_cover_row()

    def _attach_plugin_install(self, row, plugin_id: str):
        plugin = next((item for item in PLUGINS if item.id == plugin_id), None)
        if plugin is None or plugin.is_installed():
            return
        button = Gtk.Button(label="Install")
        button.add_css_class("pill")
        button.set_valign(Gtk.Align.CENTER)
        button.set_tooltip_text(f"{plugin.name} is not installed")
        button.connect("clicked", self._on_install_plugin, plugin, button)
        row.set_subtitle(f"{row.get_subtitle()} — not installed")
        row.add_suffix(button)

    def _on_install_plugin(self, _button, plugin, button):
        button.set_sensitive(False)
        button.set_label("Installing…")
        self.toast(f"Installing {plugin.name}…")

        def worker():
            try:
                result = install_plugin(plugin)
                ok = result.returncode == 0 and plugin.is_installed()
                GLib.idle_add(self._plugin_install_done, plugin, button, ok, result.returncode)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._plugin_install_failed, plugin, button, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _plugin_install_done(self, plugin, button, ok, code):
        if ok:
            button.set_label("Installed")
            self.toast(f"{plugin.name} is ready to use")
        else:
            button.set_sensitive(True)
            button.set_label("Install")
            self.toast(f"Could not install {plugin.name} (exit {code}). See Plugins for the command.")
        return False

    def _plugin_install_failed(self, plugin, button, message):
        button.set_sensitive(True)
        button.set_label("Install")
        self.toast(f"Could not install {plugin.name}: {message}")
        return False

    def _populate(self, game: Game) -> None:
        self.name_row.set_text(game.name)
        self.exe_row.set_text(game.exe_path)
        self.args_row.set_text(game.arguments)
        self.cwd_row.set_text(game.working_directory)
        self.prefix_row.set_text(game.prefix_path)
        self.extra_row.set_text(game.additional_app)
        self.kind_row.set_selected(1 if game.is_linux else 0)
        self.mangohud_row.set_active(game.mangohud)
        self.gamemode_row.set_active(game.gamemode)
        self.sdl_row.set_active(game.prefer_sdl)
        self.wayland_row.set_active(game.wayland)
        self.hdr_row.set_active(game.hdr)
        category = game.category or "Uncategorized"
        if category in self.category_names:
            self.category_row.set_selected(self.category_names.index(category))
        else:
            self.category_row.set_selected(self.category_names.index("Custom…"))
            self.custom_category_row.set_text(category)
            self.custom_category_row.set_visible(True)

    def _on_category_changed(self, *_args):
        custom = self._selected_category_name() == "Custom…"
        self.custom_category_row.set_visible(custom)

    def _selected_category_name(self) -> str:
        idx = self.category_row.get_selected()
        if 0 <= idx < len(self.category_names):
            return self.category_names[idx]
        return "Uncategorized"

    def _selected_category(self) -> str:
        name = self._selected_category_name()
        if name == "Custom…":
            return self.custom_category_row.get_text().strip() or "Uncategorized"
        return name

    def _refresh_cover_row(self):
        if self._cover_preview is not None:
            self.cover_row.remove(self._cover_preview)
            self._cover_preview = None
        if self.cover_path and Path(self.cover_path).is_file():
            picture = Gtk.Picture.new_for_filename(self.cover_path)
            picture.set_size_request(42, 64)
            picture.set_valign(Gtk.Align.CENTER)
            if hasattr(Gtk, "ContentFit"):
                picture.set_content_fit(Gtk.ContentFit.COVER)
            self.cover_row.add_prefix(picture)
            self._cover_preview = picture
            source = "Steam artwork" if self.steam_appid else "Custom image"
            self.cover_row.set_subtitle(f"{source} ready")
        else:
            self.cover_row.set_subtitle("No cover yet — browse a file or fetch one from Steam")

    def _on_kind_changed(self, *_args):
        linux = self.kind_row.get_selected() == 1
        self.runner_row.set_sensitive(not linux)
        self.prefix_row.set_sensitive(not linux)
        self.wayland_row.set_sensitive(not linux)
        self.hdr_row.set_sensitive(not linux)

    def _validate(self, *_args):
        self.save_button.set_sensitive(bool(self.name_row.get_text().strip()))

    def _on_browse(self, *_args):
        dialog = Gtk.FileDialog(title="Select an executable")
        filters = Gio.ListStore.new(Gtk.FileFilter)
        exe_filter = Gtk.FileFilter()
        exe_filter.set_name("Windows executables")
        exe_filter.add_pattern("*.exe")
        exe_filter.add_pattern("*.EXE")
        filters.append(exe_filter)
        all_filter = Gtk.FileFilter()
        all_filter.set_name("All files")
        all_filter.add_pattern("*")
        filters.append(all_filter)
        dialog.set_filters(filters)
        dialog.open(self.get_root(), None, self._on_file_chosen)

    def _on_file_chosen(self, dialog, result):
        try:
            file = dialog.open_finish(result)
        except Exception:  # noqa: BLE001 - user cancelled or error
            return
        if file:
            path = file.get_path()
            self.exe_row.set_text(path or "")
            if not self.name_row.get_text().strip() and path:
                stem = path.rsplit("/", 1)[-1].rsplit(".", 1)[0]
                self.name_row.set_text(stem)

    def _on_browse_cover(self, *_args):
        dialog = Gtk.FileDialog(title="Select a cover image")
        filters = Gio.ListStore.new(Gtk.FileFilter)
        images = Gtk.FileFilter()
        images.set_name("Images")
        for pattern in ("*.png", "*.jpg", "*.jpeg", "*.webp", "*.PNG", "*.JPG"):
            images.add_pattern(pattern)
        filters.append(images)
        dialog.set_filters(filters)
        dialog.open(self.get_root(), None, self._on_cover_chosen)

    def _on_cover_chosen(self, dialog, result):
        try:
            file = dialog.open_finish(result)
        except Exception:  # noqa: BLE001
            return
        if not file or not file.get_path():
            return
        try:
            path = copy_custom_cover(file.get_path(), self.game_id)
        except OSError as exc:
            self.toast(f"Could not copy cover: {exc}")
            return
        self.cover_path = str(path)
        self._refresh_cover_row()
        self.toast("Custom cover added")

    def _on_fetch_cover(self, *_args):
        name = self.name_row.get_text().strip()
        if not name:
            self.toast("Enter a game name first")
            return
        self.toast(f"Searching Steam for “{name}”…")

        def worker():
            try:
                hit = fetch_cover(name, self.game_id)
                GLib.idle_add(self._cover_fetched, hit)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._cover_failed, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _cover_fetched(self, hit):
        self.cover_path = hit.cover_path
        self.steam_appid = hit.appid
        self._refresh_cover_row()
        if hit.category and hit.category != "Uncategorized":
            if hit.category in self.category_names:
                self.category_row.set_selected(self.category_names.index(hit.category))
        self.toast(f"Cover found: {hit.name}")
        return False

    def _cover_failed(self, message):
        self.toast(message)
        return False

    def _selected_runner(self) -> str:
        if self.kind_row.get_selected() == 1:
            return SYSTEM_WINE
        idx = self.runner_row.get_selected()
        if 0 <= idx < len(self.runner_ids):
            return self.runner_ids[idx]
        return SYSTEM_WINE

    def _on_save(self, *_args):
        linux = self.kind_row.get_selected() == 1
        values = dict(
            name=self.name_row.get_text().strip(),
            exe_path=self.exe_row.get_text().strip(),
            arguments=self.args_row.get_text().strip(),
            working_directory=self.cwd_row.get_text().strip(),
            runner=self._selected_runner(),
            prefix_path=self.prefix_row.get_text().strip(),
            additional_app=self.extra_row.get_text().strip(),
            kind="linux" if linux else "windows",
            mangohud=self.mangohud_row.get_active(),
            gamemode=self.gamemode_row.get_active(),
            prefer_sdl=self.sdl_row.get_active(),
            wayland=self.wayland_row.get_active(),
            hdr=self.hdr_row.get_active(),
            category=self._selected_category(),
            cover_path=self.cover_path,
            steam_appid=self.steam_appid,
        )
        if self.existing:
            game = self.existing
            for key, value in values.items():
                setattr(game, key, value)
        else:
            game = Game(id=self.game_id, **values)
        self.on_saved(game)
        self.close()
