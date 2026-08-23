"""Installers page: curated one-click setup for store launchers and apps."""

from __future__ import annotations

import subprocess
import threading
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .installers import (  # noqa: E402
    APPS,
    LAUNCHERS,
    Installer,
    build_installer_command,
    download_installer,
    find_prefix_exe,
    game_from_install,
    prepare_prefix,
    search_installers,
)
from .models import Game  # noqa: E402
from .runners import SYSTEM_WINE  # noqa: E402


class InstallersPage(Gtk.Box):
    def __init__(self, runner_manager, settings, toast, on_installed) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.runner_manager = runner_manager
        self.settings = settings
        self.toast = toast
        self.on_installed = on_installed
        self._busy = False
        self._reloading = False

        toolbar = Adw.ToolbarView()
        self.append(toolbar)
        self.set_hexpand(True)
        self.set_vexpand(True)

        header = Adw.HeaderBar()
        header.set_title_widget(
            Adw.WindowTitle(title="Installers", subtitle="One-click store launchers and apps")
        )
        toolbar.add_top_bar(header)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.progress = Gtk.ProgressBar()
        self.progress.add_css_class("runner-progress")
        self.progress.set_visible(False)
        content.append(self.progress)

        page = Adw.PreferencesPage()
        content.append(page)
        toolbar.set_content(content)

        intro = Adw.PreferencesGroup(
            title="Easy install",
            description=(
                "Download the official Windows installer, run it in an isolated "
                "prefix, and add the result to your library. You finish the vendor "
                "wizard yourself — GameHandler just sets up Wine/Proton."
            ),
        )
        page.add(intro)

        self.runner_ids: list[str] = []
        self.runner_row = Adw.ComboRow(title="Runner for new installs")
        intro.add(self.runner_row)
        self.reload_runners()

        self.search_row = Adw.EntryRow(title="Search")
        self.search_row.connect("changed", lambda *_: self._rebuild_list())
        intro.add(self.search_row)

        filter_model = Gtk.StringList()
        self.filter_ids = ["All", LAUNCHERS, APPS]
        for name in self.filter_ids:
            filter_model.append(name)
        self.filter_row = Adw.ComboRow(title="Show", model=filter_model)
        self.filter_row.set_selected(0)
        self.filter_row.connect("notify::selected", lambda *_: self._rebuild_list())
        intro.add(self.filter_row)

        self.catalog_group = Adw.PreferencesGroup(
            title="Catalog",
            description="Official vendor downloads only. No game files are redistributed.",
        )
        page.add(self.catalog_group)
        self._rebuild_list()

    def reload_runners(self):
        self._reloading = True
        model = Gtk.StringList()
        self.runner_ids = []
        selected = 0
        preferred = self.settings.default_runner
        for index, (runner_id, label) in enumerate(self.runner_manager.choices()):
            self.runner_ids.append(runner_id)
            model.append(label)
            if runner_id == preferred:
                selected = index
        self.runner_row.set_model(model)
        if self.runner_ids:
            self.runner_row.set_selected(selected)
        self._reloading = False

    def _selected_runner(self) -> str:
        idx = self.runner_row.get_selected()
        if 0 <= idx < len(self.runner_ids):
            return self.runner_ids[idx]
        return SYSTEM_WINE

    def _selected_category(self) -> str:
        idx = self.filter_row.get_selected()
        if 0 <= idx < len(self.filter_ids):
            return self.filter_ids[idx]
        return "All"

    def _rebuild_list(self):
        for row in _iter_rows(self.catalog_group):
            self.catalog_group.remove(row)

        items = search_installers(self.search_row.get_text(), self._selected_category())
        if not items:
            empty = Adw.ActionRow(title="No matching installers")
            empty.set_sensitive(False)
            self.catalog_group.add(empty)
            return
        for installer in items:
            row = Adw.ActionRow(title=installer.name, subtitle=installer.description)
            badge = Gtk.Label(label=installer.category)
            badge.add_css_class("game-badge")
            badge.set_valign(Gtk.Align.CENTER)
            row.add_suffix(badge)
            button = Gtk.Button(label="Install")
            button.add_css_class("suggested-action")
            button.add_css_class("pill")
            button.set_valign(Gtk.Align.CENTER)
            button.set_sensitive(not self._busy)
            button.connect("clicked", self._on_install, installer)
            row.add_suffix(button)
            self.catalog_group.add(row)

    def _set_busy(self, busy: bool):
        self._busy = busy
        self.progress.set_visible(busy)
        if not busy:
            self.progress.set_fraction(0)
        self._rebuild_list()

    def _on_install(self, _button, installer: Installer):
        if self._busy:
            return
        runner_id = self._selected_runner()
        runner = self.runner_manager.get(runner_id)
        if not runner.is_available():
            self.toast(f"{runner.name} is not available. Download a runner first.")
            return
        game_id = Game(name=installer.name).id
        prefix = prepare_prefix(game_id)
        self._set_busy(True)
        self.progress.set_fraction(0)
        self.toast(f"Downloading {installer.name}…")

        def progress(fraction):
            GLib.idle_add(self.progress.set_fraction, fraction)

        def worker():
            try:
                archive = download_installer(installer, progress_cb=progress)
                GLib.idle_add(
                    self._download_done, installer, runner, runner_id, prefix, game_id, archive
                )
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._install_failed, installer, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _download_done(self, installer, runner, runner_id, prefix, game_id, archive: Path):
        self.toast(
            f"Launching the {installer.name} installer… Finish the vendor wizard, then close it."
        )

        def worker():
            try:
                argv, env = build_installer_command(runner, prefix, installer, archive)
                Path(prefix).mkdir(parents=True, exist_ok=True)
                completed = subprocess.run(argv, env=env, cwd=str(archive.parent), check=False)
                GLib.idle_add(
                    self._installer_finished,
                    installer,
                    runner_id,
                    prefix,
                    game_id,
                    completed.returncode,
                )
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._install_failed, installer, str(exc))

        threading.Thread(target=worker, daemon=True).start()
        return False

    def _installer_finished(self, installer, runner_id, prefix, game_id, returncode):
        found = find_prefix_exe(prefix, installer.expected_exe)
        if found is not None:
            self._finish_install(installer, found, prefix, runner_id, game_id)
            return False
        suffix = f" (installer exited {returncode})" if returncode else ""
        self.toast(f"Could not find the {installer.name} executable in the prefix{suffix}. Browse for it?")
        self._offer_browse(installer, prefix, runner_id, game_id)
        return False

    def _finish_install(self, installer, exe_path, prefix, runner_id, game_id):
        game = game_from_install(installer, exe_path, prefix, runner_id, game_id=game_id)
        self._set_busy(False)
        self.on_installed(game)
        self.toast(f"Added “{game.name}” to your library")
        return False

    def _offer_browse(self, installer, prefix, runner_id, game_id):
        dialog = Gtk.FileDialog(title=f"Locate {installer.name}")
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
        initial = Path(prefix) / "drive_c"
        if initial.is_dir():
            dialog.set_initial_folder(Gio.File.new_for_path(str(initial)))
        dialog.open(
            self.get_root(),
            None,
            lambda dlg, result: self._on_browse_chosen(
                dlg, result, installer, prefix, runner_id, game_id
            ),
        )

    def _on_browse_chosen(self, dialog, result, installer, prefix, runner_id, game_id):
        try:
            file = dialog.open_finish(result)
        except Exception:  # noqa: BLE001 - cancelled
            self._set_busy(False)
            self.toast(f"Kept the {installer.name} prefix. Add it later from Add Game if you want.")
            return
        if not file or not file.get_path():
            self._set_busy(False)
            return
        self._finish_install(installer, Path(file.get_path()), prefix, runner_id, game_id)

    def _install_failed(self, installer, message):
        self._set_busy(False)
        self.toast(f"Could not install {installer.name}: {message}")
        return False


def _iter_rows(group):
    """Yield the rows currently held by an Adw.PreferencesGroup."""
    rows = []
    stack = [group]
    while stack:
        widget = stack.pop()
        child = widget.get_first_child() if hasattr(widget, "get_first_child") else None
        while child is not None:
            if isinstance(child, (Adw.ActionRow, Adw.EntryRow, Adw.ComboRow, Adw.SwitchRow, Adw.ExpanderRow)):
                rows.append(child)
            else:
                stack.append(child)
            child = child.get_next_sibling()
    return rows
