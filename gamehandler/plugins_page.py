"""Plugins page: detect optional helpers and offer to install them."""

from __future__ import annotations

import threading

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, GLib, Gtk  # noqa: E402

from .plugins import (
    PLUGINS,
    detect_package_manager,
    format_command,
    in_flatpak,
    install_command,
    install_plugin,
    privileged_command,
)


class PluginsPage(Gtk.Box):
    def __init__(self, toast) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.toast = toast
        self._rows: dict[str, Adw.ActionRow] = {}
        self._buttons: dict[str, Gtk.Button] = {}

        toolbar = Adw.ToolbarView()
        self.append(toolbar)
        self.set_hexpand(True)
        self.set_vexpand(True)

        header = Adw.HeaderBar()
        header.set_title_widget(
            Adw.WindowTitle(title="Plugins", subtitle="Optional launch helpers")
        )
        toolbar.add_top_bar(header)

        page = Adw.PreferencesPage()
        toolbar.set_content(page)

        sandboxed = in_flatpak()
        manager = detect_package_manager()
        intro = Adw.PreferencesGroup(
            title="Host plugins",
            description=(
                "These tools are optional. GameHandler offers them on each game. "
                + (
                    "The Flatpak can only use helpers bundled in its sandbox; "
                    "host package installation is intentionally disabled."
                    if sandboxed
                    else "Missing helpers can be installed with the detected host package manager"
                    + (f" ({manager})." if manager else ".")
                )
            ),
        )
        page.add(intro)

        for plugin in PLUGINS:
            row = Adw.ActionRow(title=plugin.name)
            self._rows[plugin.id] = row
            button = Gtk.Button()
            button.add_css_class("pill")
            button.set_valign(Gtk.Align.CENTER)
            button.connect("clicked", self._on_install, plugin)
            self._buttons[plugin.id] = button
            row.add_suffix(button)
            intro.add(row)

        self.refresh()

    def refresh(self):
        for plugin in PLUGINS:
            row = self._rows[plugin.id]
            button = self._buttons[plugin.id]
            if plugin.is_installed():
                row.set_subtitle(f"{plugin.description} {plugin.used_for}")
                button.set_label("Installed")
                button.set_sensitive(False)
                button.remove_css_class("suggested-action")
            else:
                if in_flatpak():
                    row.set_subtitle(
                        f"{plugin.description} Not bundled in this Flatpak. "
                        "Installing it on the host would not expose it to the sandbox."
                    )
                    button.set_label("Unavailable")
                    button.set_sensitive(False)
                    button.remove_css_class("suggested-action")
                    continue
                try:
                    command = format_command(privileged_command(install_command(plugin)))
                    extra = f" Install with: {command}"
                except RuntimeError:
                    extra = " No package mapping is available for this system."
                row.set_subtitle(f"{plugin.description} Not installed.{extra}")
                button.set_label("Install")
                button.set_sensitive(True)
                button.add_css_class("suggested-action")

    def _on_install(self, button, plugin):
        button.set_sensitive(False)
        button.set_label("Installing…")
        self.toast(f"Installing {plugin.name}…")

        def worker():
            try:
                result = install_plugin(plugin)
                GLib.idle_add(self._done, plugin, result.returncode == 0)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._failed, plugin, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _done(self, plugin, ok):
        self.refresh()
        if ok and plugin.is_installed():
            self.toast(f"{plugin.name} is installed")
        else:
            self.toast(f"{plugin.name} did not install. The command is shown on the Plugins page.")
        return False

    def _failed(self, plugin, message):
        self.refresh()
        self.toast(f"Could not install {plugin.name}: {message}")
        return False
