"""Settings page: appearance, defaults, and launch helpers."""

from __future__ import annotations

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gtk  # noqa: E402

from .runners import SYSTEM_WINE  # noqa: E402
from .settings import COLOR_SCHEMES, VIEW_MODES  # noqa: E402


class SettingsPage(Gtk.Box):
    def __init__(self, settings, runner_manager, on_changed) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.settings = settings
        self.runner_manager = runner_manager
        self.on_changed = on_changed
        self._reloading = False

        toolbar = Adw.ToolbarView()
        self.append(toolbar)
        self.set_hexpand(True)
        self.set_vexpand(True)

        header = Adw.HeaderBar()
        header.set_title_widget(Adw.WindowTitle(title="Settings", subtitle="Appearance and defaults"))
        toolbar.add_top_bar(header)

        page = Adw.PreferencesPage()
        toolbar.set_content(page)

        appearance = Adw.PreferencesGroup(
            title="Appearance",
            description="Dark mode is the default. You can follow the system theme or force light.",
        )
        page.add(appearance)

        scheme_model = Gtk.StringList()
        for label in ("Match system", "Light", "Dark"):
            scheme_model.append(label)
        self.scheme_row = Adw.ComboRow(title="Color scheme", model=scheme_model)
        self.scheme_row.set_selected(COLOR_SCHEMES.index(settings.color_scheme))
        self.scheme_row.connect("notify::selected", self._on_scheme)
        appearance.add(self.scheme_row)

        view_model = Gtk.StringList()
        view_model.append("Grid")
        view_model.append("List")
        self.view_row = Adw.ComboRow(title="Library layout", model=view_model)
        self.view_row.set_selected(VIEW_MODES.index(settings.view_mode))
        self.view_row.connect("notify::selected", self._on_view)
        appearance.add(self.view_row)

        defaults = Adw.PreferencesGroup(
            title="New games",
            description="Used when you add a game. You can still change the runner on each title later.",
        )
        page.add(defaults)

        self.runner_ids: list[str] = []
        self.runner_row = Adw.ComboRow(title="Default runner")
        defaults.add(self.runner_row)
        self.reload_runners()
        self.runner_row.connect("notify::selected", self._on_default_runner)

        self.mangohud_row = Adw.SwitchRow(title="Enable MangoHud by default")
        self.mangohud_row.set_active(settings.default_mangohud)
        self.mangohud_row.connect("notify::active", self._on_toggle, "default_mangohud")
        defaults.add(self.mangohud_row)

        self.gamemode_row = Adw.SwitchRow(title="Enable GameMode by default")
        self.gamemode_row.set_active(settings.default_gamemode)
        self.gamemode_row.connect("notify::active", self._on_toggle, "default_gamemode")
        defaults.add(self.gamemode_row)

        self.sdl_row = Adw.SwitchRow(title="Prefer SDL by default")
        self.sdl_row.set_active(settings.default_prefer_sdl)
        self.sdl_row.connect("notify::active", self._on_toggle, "default_prefer_sdl")
        defaults.add(self.sdl_row)

        behavior = Adw.PreferencesGroup(title="Behavior")
        page.add(behavior)
        self.close_row = Adw.SwitchRow(
            title="Hide window when launching",
            subtitle="Keeps the launcher out of the way while a game starts.",
        )
        self.close_row.set_active(settings.close_on_launch)
        self.close_row.connect("notify::active", self._on_toggle, "close_on_launch")
        behavior.add(self.close_row)

    def reload_runners(self):
        self._reloading = True
        model = Gtk.StringList()
        self.runner_ids = []
        selected = 0
        for index, (runner_id, label) in enumerate(self.runner_manager.choices()):
            self.runner_ids.append(runner_id)
            model.append(label)
            if runner_id == self.settings.default_runner:
                selected = index
        self.runner_row.set_model(model)
        if self.runner_ids:
            self.runner_row.set_selected(selected)
        self._reloading = False

    def _persist(self):
        if self._reloading:
            return
        self.settings.save()
        self.on_changed(self.settings)

    def _on_scheme(self, *_args):
        if self._reloading:
            return
        idx = self.scheme_row.get_selected()
        if 0 <= idx < len(COLOR_SCHEMES):
            self.settings.color_scheme = COLOR_SCHEMES[idx]
            self._persist()

    def _on_view(self, *_args):
        if self._reloading:
            return
        idx = self.view_row.get_selected()
        if 0 <= idx < len(VIEW_MODES):
            self.settings.view_mode = VIEW_MODES[idx]
            self._persist()

    def _on_default_runner(self, *_args):
        if self._reloading:
            return
        idx = self.runner_row.get_selected()
        if 0 <= idx < len(self.runner_ids):
            self.settings.default_runner = self.runner_ids[idx]
        else:
            self.settings.default_runner = SYSTEM_WINE
        self._persist()

    def _on_toggle(self, row, _pspec, field_name):
        if self._reloading:
            return
        setattr(self.settings, field_name, row.get_active())
        self._persist()
