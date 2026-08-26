"""Installers page: curated one-click setup for store launchers and apps."""

from __future__ import annotations

import subprocess
import threading
import uuid
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .covers import save_exe_icon  # noqa: E402
from .installers import (  # noqa: E402
    INSTALLER_CATEGORIES,
    Installer,
    build_installer_command,
    download_installer,
    game_from_install,
    prefix_drive_c,
    prepare_prefix,
    search_installers,
    wait_for_installer,
)
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

        self.search_button = Gtk.ToggleButton(icon_name="system-search-symbolic")
        self.search_button.set_tooltip_text("Search the catalog")
        header.pack_end(self.search_button)

        self.search_bar = Gtk.SearchBar()
        self.search_entry = Gtk.SearchEntry(placeholder_text="Search installers…")
        self.search_entry.set_hexpand(True)
        self.search_entry.connect("search-changed", lambda *_: self._rebuild_list())
        clamp = Adw.Clamp(maximum_size=640, tightening_threshold=420)
        clamp.set_child(self.search_entry)
        self.search_bar.set_child(clamp)
        self.search_bar.connect_entry(self.search_entry)
        self.search_button.bind_property(
            "active", self.search_bar, "search-mode-enabled", 2 | 1
        )
        self.search_bar.connect("notify::search-mode-enabled", lambda *_: self._rebuild_list())
        toolbar.add_top_bar(self.search_bar)

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
                "GameHandler downloads the vendor's official Windows installer, runs "
                "it in a fresh isolated Wine prefix, then adds the result to your "
                "library. You complete the vendor's own wizard — silent-install "
                "flags are unreliable under Wine."
            ),
        )
        page.add(intro)

        self.runner_ids: list[str] = []
        self.runner_row = Adw.ComboRow(title="Runner for new installs")
        self.runner_row.set_subtitle("Each install gets its own prefix under your data directory.")
        intro.add(self.runner_row)
        self.reload_runners()

        filter_model = Gtk.StringList()
        self.filter_ids = ["All", *INSTALLER_CATEGORIES]
        for name in self.filter_ids:
            filter_model.append(name)
        self.filter_row = Adw.ComboRow(title="Show", model=filter_model)
        self.filter_row.set_selected(0)
        self.filter_row.connect("notify::selected", lambda *_: self._rebuild_list())
        intro.add(self.filter_row)

        self.catalog_group = Adw.PreferencesGroup(
            title="Catalog",
            description="Official vendor downloads only. No game or launcher files are redistributed.",
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

    def _query(self) -> str:
        return self.search_entry.get_text() if self.search_bar.get_search_mode() else ""

    def _rebuild_list(self):
        for row in _iter_rows(self.catalog_group):
            self.catalog_group.remove(row)

        items = search_installers(self._query(), self._selected_category())
        if not items:
            empty = Adw.ActionRow(
                title="No matching installers",
                subtitle="Try a different search, or switch the Show filter back to All.",
            )
            empty.set_sensitive(False)
            self.catalog_group.add(empty)
            return
        for installer in items:
            # notes carry vendor-specific gotchas; showing them here is the only
            # place a user finds out before the wizard opens.
            subtitle = installer.description
            if installer.notes:
                subtitle = f"{subtitle}\n{installer.notes}"
            row = Adw.ActionRow(title=installer.name, subtitle=subtitle)
            row.set_subtitle_lines(0)
            badge = Gtk.Label(label=installer.category)
            badge.add_css_class("game-badge")
            badge.set_valign(Gtk.Align.CENTER)
            row.add_suffix(badge)
            button = Gtk.Button(label="Install")
            button.add_css_class("suggested-action")
            button.add_css_class("pill")
            button.set_valign(Gtk.Align.CENTER)
            button.set_sensitive(not self._busy)
            button.set_tooltip_text(f"Download and run the official {installer.name} installer")
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
        game_id = uuid.uuid4().hex
        try:
            prefix = prepare_prefix(game_id)
        except OSError as exc:
            self.toast(f"Could not create a prefix for {installer.name}: {exc}")
            return
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
            f"Launching the {installer.name} installer… Finish the vendor wizard, "
            "then close it — GameHandler adds it as soon as the install lands."
        )

        def worker():
            try:
                argv, env = build_installer_command(runner, prefix, installer, archive)
                Path(prefix).mkdir(parents=True, exist_ok=True)
                completed = subprocess.run(argv, env=env, cwd=str(archive.parent), check=False)
                # The process we started is usually only the bootstrapper, so
                # the real wizard is still on screen when it exits. Waiting
                # here is what turns a finished install into a library entry
                # instead of a "could not find the executable" dead end.
                found = wait_for_installer(runner, env, prefix, installer.expected_exe)
                GLib.idle_add(
                    self._installer_finished,
                    installer,
                    runner_id,
                    prefix,
                    game_id,
                    completed.returncode,
                    found,
                )
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._install_failed, installer, str(exc))

        threading.Thread(target=worker, daemon=True).start()
        return False

    def _installer_finished(self, installer, runner_id, prefix, game_id, returncode, found):
        if found is not None:
            self._finish_install(installer, found, prefix, runner_id, game_id)
            return False
        suffix = f" (installer exited {returncode})" if returncode else ""
        self.toast(f"Could not find the {installer.name} executable in the prefix{suffix}.")
        self._offer_browse(installer, prefix, runner_id, game_id)
        return False

    def _finish_install(self, installer, exe_path, prefix, runner_id, game_id):
        game = game_from_install(installer, exe_path, prefix, runner_id, game_id=game_id)
        # A store launcher is not a Steam store product, so searching Steam for
        # its name finds nothing or something else entirely. The executable the
        # vendor just installed carries the right artwork already.
        try:
            game.cover_path = str(save_exe_icon(exe_path, game_id))
        except (OSError, RuntimeError):
            game.cover_path = ""
        self._set_busy(False)
        self.on_installed(game)
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
        # Proton keeps drive_c one level down, under "pfx".
        initial = prefix_drive_c(prefix)
        if initial is not None:
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


__all__ = ["InstallersPage"]
