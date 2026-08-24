"""Credits page: the upstream projects GameHandler runs on, and why it exists."""

from __future__ import annotations

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gtk  # noqa: E402

from .credits import ACKNOWLEDGEMENT, WHY_ALL_IN_ONE, sections  # noqa: E402


class CreditsPage(Gtk.Box):
    """A read-only page; every row links to the project it credits."""

    def __init__(self) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.set_hexpand(True)
        self.set_vexpand(True)

        toolbar = Adw.ToolbarView()
        self.append(toolbar)

        header = Adw.HeaderBar()
        header.set_title_widget(
            Adw.WindowTitle(title="Credits", subtitle="The projects we stand on")
        )
        toolbar.add_top_bar(header)

        page = Adw.PreferencesPage()
        toolbar.set_content(page)

        intro = Adw.PreferencesGroup(title="Standing on other people's work")
        banner = Gtk.Label(label=ACKNOWLEDGEMENT)
        banner.set_wrap(True)
        banner.set_xalign(0)
        banner.add_css_class("gh-credit-intro")
        intro.add(banner)
        page.add(intro)

        for section in sections():
            group = Adw.PreferencesGroup(title=section.title, description=section.summary)
            for credit in section.entries:
                group.add(_credit_row(credit))
            page.add(group)

        why = Adw.PreferencesGroup(
            title="Why one app instead of assembling the stack yourself",
            description=(
                "GameHandler replaces none of these projects. It removes the "
                "assembly work between them."
            ),
        )
        for heading, body in WHY_ALL_IN_ONE:
            row = Adw.ActionRow(title=heading, subtitle=body)
            row.set_title_lines(0)
            row.set_subtitle_lines(0)
            row.set_activatable(False)
            why.add(row)
        page.add(why)


def _credit_row(credit) -> Adw.ActionRow:
    subtitle = credit.role
    if credit.license:
        subtitle = f"{subtitle}\nLicense: {credit.license}"
    row = Adw.ActionRow(title=credit.label, subtitle=subtitle)
    row.set_title_lines(0)
    row.set_subtitle_lines(0)
    if credit.url:
        link = Gtk.LinkButton(uri=credit.url)
        link.set_child(Gtk.Image.new_from_icon_name("adw-external-link-symbolic"))
        link.add_css_class("flat")
        link.add_css_class("circular")
        link.set_valign(Gtk.Align.CENTER)
        link.set_tooltip_text(credit.url)
        row.add_suffix(link)
        row.set_activatable_widget(link)
    return row


__all__ = ["CreditsPage"]
