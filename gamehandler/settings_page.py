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

        self.esync_row = Adw.SwitchRow(
            title="Enable Esync by default",
            subtitle="Eventfd-based Wine sync. Usually leave this on.",
        )
        self.esync_row.set_active(settings.default_esync)
        self.esync_row.connect("notify::active", self._on_toggle, "default_esync")
        defaults.add(self.esync_row)

        self.fsync_row = Adw.SwitchRow(
            title="Enable Fsync by default",
            subtitle="Futex-based Wine sync. Preferred when the kernel supports it.",
        )
        self.fsync_row.set_active(settings.default_fsync)
        self.fsync_row.connect("notify::active", self._on_toggle, "default_fsync")
        defaults.add(self.fsync_row)

        compat = Adw.PreferencesGroup(
            title="New games · compatibility",
            description="Lutris-style Wine/Proton defaults. You can still change them on each title.",
        )
        page.add(compat)

        self.dxvk_row = Adw.SwitchRow(
            title="Enable DXVK by default",
            subtitle="Direct3D 8–11 through Vulkan.",
        )
        self.dxvk_row.set_active(settings.default_dxvk)
        self.dxvk_row.connect("notify::active", self._on_toggle, "default_dxvk")
        compat.add(self.dxvk_row)

        self.vkd3d_row = Adw.SwitchRow(
            title="Enable VKD3D by default",
            subtitle="Direct3D 12 through Vulkan.",
        )
        self.vkd3d_row.set_active(settings.default_vkd3d)
        self.vkd3d_row.connect("notify::active", self._on_toggle, "default_vkd3d")
        compat.add(self.vkd3d_row)

        self.nvapi_row = Adw.SwitchRow(
            title="Enable DXVK-NVAPI / DLSS by default",
            subtitle="Only needed for some NVIDIA / DLSS titles.",
        )
        self.nvapi_row.set_active(settings.default_nvapi)
        self.nvapi_row.connect("notify::active", self._on_toggle, "default_nvapi")
        compat.add(self.nvapi_row)

        self.fsr_row = Adw.SwitchRow(title="Enable AMD FSR by default")
        self.fsr_row.set_active(settings.default_fsr)
        self.fsr_row.connect("notify::active", self._on_toggle, "default_fsr")
        compat.add(self.fsr_row)

        self.battleye_row = Adw.SwitchRow(title="Enable BattlEye runtime by default")
        self.battleye_row.set_active(settings.default_battleye)
        self.battleye_row.connect("notify::active", self._on_toggle, "default_battleye")
        compat.add(self.battleye_row)

        self.eac_row = Adw.SwitchRow(title="Enable Easy Anti-Cheat runtime by default")
        self.eac_row.set_active(settings.default_eac)
        self.eac_row.connect("notify::active", self._on_toggle, "default_eac")
        compat.add(self.eac_row)

        self.gamescope_row = Adw.SwitchRow(title="Enable Gamescope by default")
        self.gamescope_row.set_active(settings.default_gamescope)
        self.gamescope_row.connect("notify::active", self._on_toggle, "default_gamescope")
        compat.add(self.gamescope_row)

        self.desktop_row = Adw.SwitchRow(title="Enable virtual desktop by default")
        self.desktop_row.set_active(settings.default_virtual_desktop)
        self.desktop_row.connect("notify::active", self._on_toggle, "default_virtual_desktop")
        compat.add(self.desktop_row)

        behavior = Adw.PreferencesGroup(title="Behavior")
        page.add(behavior)
        self.close_row = Adw.SwitchRow(
            title="Hide window when launching",
            subtitle="Keeps the launcher out of the way while a game starts.",
        )
        self.close_row.set_active(settings.close_on_launch)
        self.close_row.connect("notify::active", self._on_toggle, "close_on_launch")
        behavior.add(self.close_row)

    def sync_view_mode(self, mode: str):
        """Mirror the library toolbar's grid/list toggle without re-saving."""
        if mode not in VIEW_MODES:
            return
        self._reloading = True
        try:
            self.view_row.set_selected(VIEW_MODES.index(mode))
        finally:
            self._reloading = False

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
