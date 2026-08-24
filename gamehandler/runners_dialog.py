"""Runners page: download, switch, and remove Proton/Wine builds."""

from __future__ import annotations

import threading

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, GLib, Gtk  # noqa: E402

from .runners import SYSTEM_WINE, families, runner_guide_details  # noqa: E402


class RunnersPage(Gtk.Box):
    """Embedded runners manager used by the main window."""

    def __init__(self, runner_manager, proton_manager, toast, on_changed=None) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.runner_manager = runner_manager
        self.proton_manager = proton_manager
        self.toast = toast
        self.on_changed = on_changed
        self._releases = []
        self._installing = False

        toolbar = Adw.ToolbarView()
        self.append(toolbar)
        self.set_hexpand(True)
        self.set_vexpand(True)

        header = Adw.HeaderBar()
        header.set_title_widget(
            Adw.WindowTitle(title="Runners", subtitle="Download Proton and Wine")
        )
        toolbar.add_top_bar(header)

        self.refresh_button = Gtk.Button(icon_name="view-refresh-symbolic")
        self.refresh_button.set_tooltip_text("Fetch available builds for the selected family")
        self.refresh_button.connect("clicked", lambda *_: self.fetch_available())
        header.pack_end(self.refresh_button)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.progress = Gtk.ProgressBar()
        self.progress.add_css_class("runner-progress")
        self.progress.set_visible(False)
        content.append(self.progress)
        self.page = Adw.PreferencesPage()
        content.append(self.page)
        toolbar.set_content(content)

        # Installed builds and the download picker come first: the guide is
        # reference material, and burying the primary action under nine
        # expander rows pushed it off the first screen.
        self.installed_group = Adw.PreferencesGroup(
            title="Installed",
            description="Available for launching and for the per-game runner picker.",
        )
        self.page.add(self.installed_group)

        intro = Adw.PreferencesGroup(
            title="Download a build",
            description=(
                "GameHandler fetches these archives from each maintainer's own "
                "release page, the same upstream sources ProtonPlus uses. Nothing "
                "is bundled or re-hosted here."
            ),
        )
        self.page.add(intro)

        family_model = Gtk.StringList()
        self.family_ids = []
        for family in families():
            self.family_ids.append(family.id)
            family_model.append(family.name)
        self.family_row = Adw.ComboRow(title="Family", model=family_model)
        self.family_row.connect("notify::selected", self._on_family_changed)
        intro.add(self.family_row)

        self.family_detail = Adw.ActionRow()
        self.family_detail.set_activatable(False)
        self.family_detail.set_title_lines(0)
        self.family_detail.set_subtitle_lines(0)
        self._family_link = Gtk.LinkButton(uri="https://www.winehq.org", label="Project")
        self._family_link.add_css_class("flat")
        self._family_link.set_valign(Gtk.Align.CENTER)
        self.family_detail.add_suffix(self._family_link)
        intro.add(self.family_detail)

        self.available_group = Adw.PreferencesGroup(
            title="Available versions",
            description="Fetched from GitHub. Install the one you want to use.",
        )
        self.page.add(self.available_group)

        guide = Adw.PreferencesGroup(
            title="Which runner should I use?",
            description=(
                "Proton builds are the usual choice for Windows games; standalone "
                "Wine is lighter and better for some older titles. Each entry links "
                "to the project that maintains it."
            ),
        )
        for row in runner_guide_details():
            expander = Adw.ExpanderRow(
                title=row.title,
                subtitle=f"{row.kind.capitalize()} · maintained by {row.maintainer}"
                if row.maintainer
                else row.kind.capitalize(),
            )
            detail = Adw.ActionRow(title=row.advice)
            detail.set_title_lines(0)
            detail.set_activatable(False)
            if row.homepage:
                link = Gtk.LinkButton(uri=row.homepage, label="Visit project")
                link.add_css_class("flat")
                link.set_valign(Gtk.Align.CENTER)
                detail.add_suffix(link)
            expander.add_row(detail)
            guide.add(expander)
        self.page.add(guide)

        self.reload_installed()
        self._update_family_detail()
        self._set_available_placeholder("Fetching the latest Proton-GE builds…")
        GLib.idle_add(self.fetch_available)

    def selected_family_id(self) -> str:
        idx = self.family_row.get_selected()
        if 0 <= idx < len(self.family_ids):
            return self.family_ids[idx]
        return "proton-ge"

    def _selected_family(self):
        target = self.selected_family_id()
        for family in families():
            if family.id == target:
                return family
        return families()[0]

    def _on_family_changed(self, *_args):
        self._update_family_detail()
        self.fetch_available()

    def _update_family_detail(self):
        family = self._selected_family()
        self.family_detail.set_title(family.description)
        self.family_detail.set_subtitle(
            f"Maintained by {family.maintainer}" if family.maintainer else ""
        )
        self._family_link.set_uri(family.homepage)
        self._family_link.set_tooltip_text(family.homepage)

    def reload_installed(self):
        self._clear_group(self.installed_group)

        wine = self.runner_manager.system_wine()
        wine_row = Adw.ActionRow(
            title="System Wine",
            subtitle=wine.version() if wine.is_available() else "Not installed on this system",
        )
        icon = "emblem-ok-symbolic" if wine.is_available() else "dialog-warning-symbolic"
        wine_row.add_suffix(Gtk.Image.new_from_icon_name(icon))
        self.installed_group.add(wine_row)

        installed = self.runner_manager.installed_protons()
        if not installed:
            empty = Adw.ActionRow(
                title="No downloaded runners yet",
                subtitle="Install a Proton or Wine build below, then assign it to a game.",
            )
            empty.set_sensitive(False)
            self.installed_group.add(empty)
            return

        for proton in installed:
            row = Adw.ActionRow(title=proton.name, subtitle=proton.family_label())
            row.add_suffix(Gtk.Image.new_from_icon_name("emblem-ok-symbolic"))
            remove = Gtk.Button(icon_name="user-trash-symbolic")
            remove.add_css_class("flat")
            remove.add_css_class("circular")
            remove.set_valign(Gtk.Align.CENTER)
            remove.set_tooltip_text(f"Remove {proton.name}")
            remove.connect("clicked", self._on_uninstall, proton.id, proton.name)
            row.add_suffix(remove)
            self.installed_group.add(row)

    def _set_available_placeholder(self, text):
        self._clear_group(self.available_group)
        row = Adw.ActionRow(title=text)
        row.set_title_lines(0)
        row.set_sensitive(False)
        self.available_group.add(row)

    def fetch_available(self, *_args):
        family_id = self.selected_family_id()
        self.refresh_button.set_sensitive(False)
        self._set_available_placeholder("Fetching…")

        def worker():
            try:
                releases = self.proton_manager.fetch_available(limit=12, family=family_id)
                GLib.idle_add(self._populate_available, family_id, releases)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._fetch_failed, family_id, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _fetch_failed(self, family_id, message):
        if family_id != self.selected_family_id():
            return False
        self.refresh_button.set_sensitive(True)
        self._set_available_placeholder(f"Could not fetch builds: {message}")
        return False

    def _populate_available(self, family_id, releases):
        # A slower earlier request must not overwrite the family now selected.
        if family_id != self.selected_family_id():
            return False
        self.refresh_button.set_sensitive(True)
        self._releases = releases
        self._clear_group(self.available_group)
        if not releases:
            self._set_available_placeholder("No builds found for this family.")
            return False
        for release in releases:
            installed = self.proton_manager.is_release_installed(release)
            row = Adw.ActionRow(
                title=release.tag,
                subtitle=f"{release.family.name} · {release.name} · {release.size_mb:.0f} MB",
            )
            if installed:
                row.add_suffix(Gtk.Image.new_from_icon_name("emblem-ok-symbolic"))
                badge = Gtk.Label(label="Installed")
                badge.add_css_class("game-badge")
                badge.set_valign(Gtk.Align.CENTER)
                row.add_suffix(badge)
            else:
                button = Gtk.Button(label="Install")
                button.set_valign(Gtk.Align.CENTER)
                button.add_css_class("suggested-action")
                button.add_css_class("pill")
                button.set_sensitive(not self._installing)
                button.connect("clicked", self._on_install, release)
                row.add_suffix(button)
            self.available_group.add(row)
        return False

    def _on_install(self, button, release):
        if self._installing:
            return
        self._installing = True
        button.set_sensitive(False)
        button.set_label("Installing…")
        self.progress.set_visible(True)
        self.progress.set_fraction(0)
        self.toast(f"Downloading {release.tag}…")

        def progress(fraction):
            GLib.idle_add(self.progress.set_fraction, fraction)

        def worker():
            try:
                self.proton_manager.install(release, progress_cb=progress)
                GLib.idle_add(self._install_done, release)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._install_failed, release, str(exc))

        threading.Thread(target=worker, daemon=True).start()

    def _install_done(self, release):
        self._installing = False
        self.progress.set_visible(False)
        self.toast(f"Installed {release.tag}. You can now choose it when adding or editing a game.")
        self.reload_installed()
        self.fetch_available()
        if self.on_changed:
            self.on_changed()
        return False

    def _install_failed(self, release, message):
        self._installing = False
        self.progress.set_visible(False)
        self.toast(f"Failed to install {release.tag}: {message}")
        self.fetch_available()
        return False

    def _on_uninstall(self, _button, runner_id, name):
        if runner_id == SYSTEM_WINE:
            return
        dialog = Adw.AlertDialog(
            heading=f"Remove {name}?",
            body=(
                "The downloaded build is deleted from disk. Games using it fall "
                "back to System Wine until you pick another runner."
            ),
        )
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("remove", "Remove")
        dialog.set_response_appearance("remove", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect("response", self._on_uninstall_response, runner_id, name)
        dialog.present(self)

    def _on_uninstall_response(self, _dialog, response, runner_id, name):
        if response != "remove":
            return
        try:
            self.proton_manager.uninstall(runner_id)
        except Exception as exc:  # noqa: BLE001
            self.toast(f"Could not remove {name}: {exc}")
            return
        self.toast(f"Removed {name}")
        self.reload_installed()
        self.fetch_available()
        if self.on_changed:
            self.on_changed()

    def _clear_group(self, group):
        for row in _iter_rows(group):
            group.remove(row)


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


__all__ = ["RunnersPage"]
