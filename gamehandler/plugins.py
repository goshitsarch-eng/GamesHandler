"""Optional launch helpers (MangoHud, GameMode, Winetricks, UMU, Gamescope).

Source installations can offer a package-manager command when a helper is
missing. Flatpak builds must never run sandbox package managers as if they
could modify the host, so installation is explicitly disabled there.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Plugin:
    id: str
    name: str
    binary: str
    description: str
    used_for: str
    packages: dict[str, str]

    def is_installed(self) -> bool:
        return shutil.which(self.binary) is not None


PLUGINS: tuple[Plugin, ...] = (
    Plugin(
        id="mangohud",
        name="MangoHud",
        binary="mangohud",
        description="On-screen overlay for FPS, frame times, GPU, and CPU.",
        used_for="Enable it per game under Launch options, or as the default in Settings.",
        packages={
            "apt": "mangohud",
            "pacman": "mangohud",
            "dnf": "mangohud",
            "zypper": "mangohud",
            "flatpak": "org.freedesktop.Platform.VulkanLayer.MangoHud",
        },
    ),
    Plugin(
        id="gamemode",
        name="Feral GameMode",
        binary="gamemoderun",
        description="Temporarily tunes the system for better game performance.",
        used_for="Wraps the launch command when GameMode is enabled on a game.",
        packages={
            "apt": "gamemode",
            "pacman": "gamemode",
            "dnf": "gamemode",
            "zypper": "gamemode",
        },
    ),
    Plugin(
        id="winetricks",
        name="Winetricks",
        binary="winetricks",
        description="Installs common Windows runtimes, fonts, and DLL overrides.",
        used_for="Open it from a game’s Prefix tools menu.",
        packages={
            "apt": "winetricks",
            "pacman": "winetricks",
            "dnf": "winetricks",
            "zypper": "winetricks",
        },
    ),
    Plugin(
        id="umu",
        name="UMU Launcher",
        binary="umu-run",
        description="Runs Proton builds with the Steam runtime outside Steam.",
        used_for="Used automatically when a downloaded Proton build includes a proton script.",
        packages={
            "apt": "umu-launcher",
            "pacman": "umu-launcher",
            "dnf": "umu-launcher",
        },
    ),
    Plugin(
        id="gamescope",
        name="Gamescope",
        binary="gamescope",
        description="Nested compositor for scaling, HDR, and a stable game session.",
        used_for="Wraps the launch command when Gamescope is enabled on a game.",
        packages={
            "apt": "gamescope",
            "pacman": "gamescope",
            "dnf": "gamescope",
            "zypper": "gamescope",
        },
    ),
)


def plugin_by_id(plugin_id: str) -> Plugin:
    for plugin in PLUGINS:
        if plugin.id == plugin_id:
            return plugin
    raise KeyError(plugin_id)


def in_flatpak() -> bool:
    """Return whether GameHandler is running in a Flatpak sandbox."""
    return bool(os.environ.get("FLATPAK_ID")) or Path("/.flatpak-info").exists()


def detect_package_manager() -> str:
    """Return a short id for the host package manager, or empty."""
    if in_flatpak():
        return ""
    if shutil.which("apt-get") and Path("/etc/debian_version").exists():
        return "apt"
    if shutil.which("pacman") and Path("/etc/arch-release").exists():
        return "pacman"
    if shutil.which("dnf"):
        return "dnf"
    if shutil.which("zypper"):
        return "zypper"
    if shutil.which("flatpak"):
        return "flatpak"
    return ""


def install_command(plugin: Plugin, manager: str | None = None) -> list[str]:
    """Return the argv used to install *plugin*, without a privilege helper."""
    if in_flatpak():
        raise RuntimeError(
            "Plugin installation is unavailable inside Flatpak; sandbox package "
            "managers cannot install or expose host packages"
        )
    manager = manager or detect_package_manager()
    package = plugin.packages.get(manager)
    if not manager or not package:
        raise RuntimeError(f"No install package is known for {plugin.name} on this system")
    if manager == "apt":
        return ["apt-get", "install", "-y", package]
    if manager == "pacman":
        return ["pacman", "-S", "--noconfirm", package]
    if manager == "dnf":
        return ["dnf", "install", "-y", package]
    if manager == "zypper":
        return ["zypper", "--non-interactive", "install", package]
    if manager == "flatpak":
        return ["flatpak", "install", "-y", "flathub", package]
    raise RuntimeError(f"Unsupported package manager: {manager}")


def privileged_command(argv: list[str]) -> list[str]:
    """Prefix *argv* with pkexec or sudo when installing system packages."""
    if argv and argv[0] == "flatpak":
        return argv
    if os.geteuid() == 0:
        return argv
    pkexec = shutil.which("pkexec")
    if pkexec:
        return [pkexec, *argv]
    sudo = shutil.which("sudo")
    if sudo:
        return [sudo, *argv]
    return argv


def format_command(argv: list[str]) -> str:
    return " ".join(argv)


def install_plugin(plugin: Plugin, timeout: int = 300) -> subprocess.CompletedProcess:
    """Run the privileged install command for *plugin*."""
    argv = privileged_command(install_command(plugin))
    return subprocess.run(argv, check=False, timeout=timeout)


__all__ = [
    "Plugin",
    "PLUGINS",
    "plugin_by_id",
    "in_flatpak",
    "detect_package_manager",
    "install_command",
    "privileged_command",
    "format_command",
    "install_plugin",
]
