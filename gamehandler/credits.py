"""Acknowledgements for the upstream projects GameHandler is built on.

GameHandler runs no Windows game by itself. Every title it launches is running
on someone else's compatibility layer, someone else's Proton build, and
someone else's Vulkan translation layer. This module is the single source of
truth for crediting them: the About dialog, the in-app Credits page, and the
README acknowledgements section are all generated from the data here, so they
cannot drift apart.

Kept free of GTK imports so it can be unit tested headlessly.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Credit:
    """One upstream project, and what GameHandler actually uses it for."""

    name: str
    url: str
    role: str
    license: str = ""
    authors: str = ""

    @property
    def label(self) -> str:
        """``Name — Authors`` for list rows that show attribution inline."""
        return f"{self.name} — {self.authors}" if self.authors else self.name


@dataclass(frozen=True)
class CreditSection:
    id: str
    title: str
    summary: str
    entries: tuple[Credit, ...]


# Licenses are listed only where they are unambiguous. Proton/Wine derivative
# builds inherit their upstreams' terms rather than carrying one SPDX id, so
# they are described by role instead of being labelled with a guess.
CREDIT_SECTIONS: tuple[CreditSection, ...] = (
    CreditSection(
        id="layers",
        title="Compatibility layers",
        summary=(
            "The projects that actually run Windows games. GameHandler does not "
            "reimplement any of this — it configures it and gets out of the way."
        ),
        entries=(
            Credit(
                name="Wine",
                url="https://www.winehq.org",
                role=(
                    "The Windows compatibility layer underneath every Windows title "
                    "GameHandler launches, including every Proton build. Without Wine "
                    "there is no GameHandler."
                ),
                license="LGPL-2.1-or-later",
                authors="WineHQ and hundreds of contributors since 1993",
            ),
            Credit(
                name="Proton",
                url="https://github.com/ValveSoftware/Proton",
                role=(
                    "Valve's Wine distribution with gaming patches and the Steam Linux "
                    "Runtime. Every Proton family GameHandler downloads is a fork of it."
                ),
                license="BSD-3-Clause, plus each bundled component's own license",
                authors="Valve Software and CodeWeavers",
            ),
            Credit(
                name="DXVK",
                url="https://github.com/doitsujin/dxvk",
                role=(
                    "Direct3D 8/9/10/11 over Vulkan — the DXVK toggle, and the runtime "
                    "GameHandler bundles for raw Wine runners that ship without it."
                ),
                license="zlib",
                authors="Philip Rebohle and contributors",
            ),
            Credit(
                name="VKD3D-Proton",
                url="https://github.com/HansKristian-Work/vkd3d-proton",
                role="Direct3D 12 over Vulkan, behind the VKD3D toggle.",
                license="LGPL-2.1-or-later",
                authors="Hans-Kristian Arntzen, Philip Rebohle, and contributors",
            ),
            Credit(
                name="DXVK-NVAPI",
                url="https://github.com/jp7677/dxvk-nvapi",
                role="NVIDIA NVAPI and DLSS support, behind the NVAPI/DLSS toggle.",
                license="MIT",
                authors="Jens Peters and contributors",
            ),
        ),
    ),
    CreditSection(
        id="builds",
        title="Proton and Wine builds",
        summary=(
            "GameHandler ships none of these. It downloads the maintainers' own "
            "official release archives, from their own release pages, on request."
        ),
        entries=(
            Credit(
                name="Proton-GE",
                url="https://github.com/GloriousEggroll/proton-ge-custom",
                role="Community Proton with codecs, protonfixes, and broad game fixes.",
                authors="GloriousEggroll",
            ),
            Credit(
                name="proton-rtsp",
                url="https://github.com/SpookySkeletons/proton-rtsp",
                role="Proton with RTSP and in-game media playback patches.",
                authors="SpookySkeletons",
            ),
            Credit(
                name="Proton-CachyOS",
                url="https://github.com/CachyOS/proton-cachyos",
                role="Performance-oriented Proton with additional Wayland work.",
                authors="The CachyOS project",
            ),
            Credit(
                name="Proton-EM",
                url="https://github.com/Etaash-mathamsetty/Proton",
                role="Proton with Wine Wayland, HDR, and FSR additions.",
                authors="Etaash Mathamsetty",
            ),
            Credit(
                name="Wine-Builds",
                url="https://github.com/Kron4ek/Wine-Builds",
                role=(
                    "The standalone Wine, Wine-Staging, Staging-TkG, and Wine-Proton "
                    "builds behind GameHandler's four Wine families."
                ),
                authors="Kron4ek",
            ),
        ),
    ),
    CreditSection(
        id="launchers",
        title="Launchers that shaped GameHandler",
        summary=(
            "Prior art we studied and learned from. No code was copied from any of "
            "them; what GameHandler borrowed is their design thinking."
        ),
        entries=(
            Credit(
                name="Lutris",
                url="https://lutris.net",
                role=(
                    "The per-game runner and environment model, and the shape of the "
                    "Esync/Fsync/DXVK/VKD3D/FSR compatibility toggles."
                ),
                license="GPL-3.0-or-later",
                authors="Mathieu Comandon and contributors",
            ),
            Credit(
                name="Faugus Launcher",
                url="https://github.com/Faugus/faugus-launcher",
                role=(
                    "The case for a small, focused launcher, and the idea of "
                    "one-click store-launcher installs."
                ),
                license="MIT",
                authors="Faugus",
            ),
            Credit(
                name="Bottles",
                url="https://usebottles.com",
                role="Isolated per-title prefixes and the installable-programs catalog idea.",
                license="GPL-3.0-or-later",
                authors="The Bottles project",
            ),
            Credit(
                name="ProtonPlus",
                url="https://github.com/Vysp3r/ProtonPlus",
                role=(
                    "The compatibility-tool family list and the upstream release "
                    "sources GameHandler fetches Proton and Wine builds from."
                ),
                license="GPL-3.0-or-later",
                authors="Vysp3r and contributors",
            ),
            Credit(
                name="Heroic Games Launcher",
                url="https://heroicgameslauncher.com",
                role="Prior art for treating store launchers as first-class library entries.",
                license="GPL-3.0-or-later",
                authors="The Heroic Games Launcher team",
            ),
        ),
    ),
    CreditSection(
        id="helpers",
        title="Runtime helpers",
        summary=(
            "Optional tools GameHandler detects and wraps launches with. They are "
            "installed from your distribution, never vendored here."
        ),
        entries=(
            Credit(
                name="umu-launcher",
                url="https://github.com/Open-Wine-Components/umu-launcher",
                role=(
                    "Runs Proton builds with the Steam Linux Runtime outside Steam — "
                    "how GameHandler launches Proton runners at all."
                ),
                license="GPL-3.0-or-later",
                authors="Open Wine Components",
            ),
            Credit(
                name="MangoHud",
                url="https://github.com/flightlessmango/MangoHud",
                role="The performance overlay behind the per-game MangoHud switch.",
                license="MIT",
                authors="FlightlessMango and contributors",
            ),
            Credit(
                name="Feral GameMode",
                url="https://github.com/FeralInteractive/gamemode",
                role="Temporary system tuning while a game runs.",
                license="BSD-3-Clause",
                authors="Feral Interactive",
            ),
            Credit(
                name="Gamescope",
                url="https://github.com/ValveSoftware/gamescope",
                role="The nested compositor behind scaling, a stable session, and HDR.",
                license="BSD-2-Clause",
                authors="Valve Software",
            ),
            Credit(
                name="Winetricks",
                url="https://github.com/Winetricks/winetricks",
                role="Installs Windows runtimes and fonts from a game's Prefix tools menu.",
                license="LGPL-2.1-or-later",
                authors="The Winetricks maintainers",
            ),
        ),
    ),
    CreditSection(
        id="platform",
        title="Platform",
        summary="What GameHandler itself is built and shipped with.",
        entries=(
            Credit(
                name="GTK",
                url="https://www.gtk.org",
                role="The toolkit the whole interface is built on.",
                license="LGPL-2.1-or-later",
                authors="The GNOME Project",
            ),
            Credit(
                name="libadwaita",
                url="https://gitlab.gnome.org/GNOME/libadwaita",
                role="Adaptive widgets, the dark theme, and the GNOME look.",
                license="LGPL-2.1-or-later",
                authors="The GNOME Project",
            ),
            Credit(
                name="PyGObject",
                url="https://pygobject.gnome.org",
                role="The Python bindings that let GameHandler drive GTK.",
                license="LGPL-2.1-or-later",
                authors="The PyGObject maintainers",
            ),
            Credit(
                name="Meson and Flatpak",
                url="https://flatpak.org",
                role="How GameHandler is built and packaged.",
                authors="The Meson and Flatpak projects",
            ),
            Credit(
                name="Steam store web API",
                url="https://store.steampowered.com",
                role="Public artwork and genre lookup for automatic cover art.",
                authors="Valve Software",
            ),
        ),
    ),
)

ACKNOWLEDGEMENT = (
    "GameHandler is a front-end. It does not implement Windows compatibility, "
    "Direct3D translation, or a Proton build of its own — it configures the work "
    "of the projects below and stays out of their way. All of them are "
    "independent; none endorse GameHandler."
)

# Why one app instead of asking people to assemble the stack themselves.
WHY_ALL_IN_ONE: tuple[tuple[str, str], ...] = (
    (
        "The stack was always the hard part, not the games",
        "Running a Windows game on Linux takes a compatibility layer, a Proton or "
        "Wine build, Vulkan translation for Direct3D, a clean prefix, and the right "
        "environment variables. Each of those is a mature, excellent project. "
        "Assembling them is the part that stops people.",
    ),
    (
        "One app instead of five",
        "Without GameHandler the usual route is ProtonPlus (or a hand-extracted "
        "tarball) for runners, Lutris or Bottles for prefixes, winetricks by hand "
        "for runtimes, a separate wiki tab to learn which Proton fork a game needs, "
        "and a vendor installer run manually for each store launcher. GameHandler "
        "does those five jobs in one GTK4 window.",
    ),
    (
        "Bundling the workflow, not the projects",
        "GameHandler vendors none of the runners it offers. It downloads the "
        "maintainers' own official release archives from their own release pages, "
        "on request, and keeps them in your data directory. Optional helpers like "
        "MangoHud and GameMode come from your distribution. Upgrading is still the "
        "upstream project's business, not ours.",
    ),
    (
        "Defaults that match what the projects recommend",
        "Esync, Fsync, DXVK, VKD3D, and the anti-cheat runtimes are on by default "
        "because that is what Proton does. Every one of them is a switch you can "
        "turn off per game, because the projects that wrote them documented when "
        "you should.",
    ),
    (
        "The credit stays visible",
        "Every runner family names its maintainer in the app, every compatibility "
        "toggle names the project that implements it, and this list ships inside "
        "the application rather than only in a file on a repository page.",
    ),
)


def sections() -> tuple[CreditSection, ...]:
    return CREDIT_SECTIONS


def section_by_id(section_id: str) -> CreditSection:
    for section in CREDIT_SECTIONS:
        if section.id == section_id:
            return section
    raise KeyError(f"Unknown credit section: {section_id}")


def all_credits() -> list[Credit]:
    return [credit for section in CREDIT_SECTIONS for credit in section.entries]


def credit_by_name(name: str) -> Credit:
    for credit in all_credits():
        if credit.name.lower() == name.lower():
            return credit
    raise KeyError(f"Unknown credit: {name}")


def about_credit_sections() -> list[tuple[str, list[str]]]:
    """``(title, ["Name https://url", ...])`` pairs for Adw.AboutDialog.

    libadwaita renders a trailing URL in each entry as a link, so the name and
    its homepage are joined into one string per project.
    """
    rendered = []
    for section in CREDIT_SECTIONS:
        people = [
            f"{credit.name} {credit.url}" if credit.url else credit.name
            for credit in section.entries
        ]
        rendered.append((section.title, people))
    return rendered


def markdown() -> str:
    """Render the acknowledgements as the README section, so the two agree."""
    lines = ["## Thanks to the projects GameHandler stands on", "", ACKNOWLEDGEMENT, ""]
    for section in CREDIT_SECTIONS:
        lines.append(f"### {section.title}")
        lines.append("")
        lines.append(section.summary)
        lines.append("")
        for credit in section.entries:
            suffix = f" _({credit.license})_" if credit.license else ""
            author = f" — {credit.authors}" if credit.authors else ""
            lines.append(f"- **[{credit.name}]({credit.url})**{author}{suffix}  ")
            lines.append(f"  {credit.role}")
        lines.append("")
    lines.append("## Why one app instead of assembling the stack yourself")
    lines.append("")
    for heading, body in WHY_ALL_IN_ONE:
        lines.append(f"### {heading}")
        lines.append("")
        lines.append(body)
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


__all__ = [
    "ACKNOWLEDGEMENT",
    "CREDIT_SECTIONS",
    "WHY_ALL_IN_ONE",
    "Credit",
    "CreditSection",
    "about_credit_sections",
    "all_credits",
    "credit_by_name",
    "markdown",
    "section_by_id",
    "sections",
]
