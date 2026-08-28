"""The QML-facing application backend.

One QObject exposes the library, settings, runners, installers, plugins, and
credits to the Kirigami interface. All long work (downloads, vendor install
wizards, cover lookups, launch watching) runs on Python threads; results are
marshalled back to the UI thread through a queued signal so QML property
updates always happen where Qt expects them.
"""

from __future__ import annotations

import shlex
import shutil
import subprocess
import threading
import uuid
from pathlib import Path

from PySide6.QtCore import Property, QObject, Qt, QUrl, Signal, Slot
from PySide6.QtGui import QDesktopServices

from . import APP_ID, __version__, config
from .covers import (
    DEFAULT_CATEGORIES,
    accent_index,
    copy_custom_cover,
    fetch_cover,
    initials,
    save_exe_icon,
)
from .credits import ACKNOWLEDGEMENT, WHY_ALL_IN_ONE, sections
from .installers import (
    INSTALLER_CATEGORIES,
    build_installer_command,
    download_installer,
    game_from_install,
    installer_by_id,
    prepare_prefix,
    search_installers,
    wait_for_installer,
)
from .models import Game, Library, format_last_played
from .netpaths import as_local_path
from .plugins import (
    PLUGINS,
    detect_package_manager,
    format_command,
    in_flatpak,
    install_command,
    install_plugin,
    plugin_by_id,
    privileged_command,
)
from .runners import (
    SYSTEM_WINE,
    ProtonManager,
    RunnerManager,
    create_desktop_shortcut,
    families,
    launch,
    prefix_drive_c,
    runner_guide_details,
    tool_command,
)
from .settings import COLOR_SCHEMES, VIEW_MODES, Settings

SORT_OPTIONS = (
    ("name", "Name"),
    ("recent", "Recently played"),
    ("added", "Recently added"),
)

# Booleans shared between Game, Settings defaults, and the game form.
_TOGGLE_FIELDS = (
    "mangohud", "gamemode", "prefer_sdl", "wayland", "hdr", "esync", "fsync",
    "dxvk", "vkd3d", "nvapi", "fsr", "battleye", "eac", "gamescope",
    "virtual_desktop",
)
# Toggles that have a matching default in Settings (wayland/hdr do not).
_DEFAULTED_TOGGLES = tuple(
    name for name in _TOGGLE_FIELDS if hasattr(Settings(), f"default_{name}")
)

_GAME_TEXT_FIELDS = (
    "name", "exe_path", "arguments", "working_directory", "runner",
    "prefix_path", "additional_app", "environment", "category",
    "virtual_desktop_size", "cover_path", "kind",
)


def _launcher_command() -> str:
    """The command a desktop shortcut runs to reach this install."""
    found = shutil.which("gamehandler")
    if not found:
        return "python3 -m gamehandler"
    return shlex.quote(found) if " " in found else found


class Backend(QObject):
    """Everything the QML layer talks to."""

    # UI feedback
    notify = Signal(str)
    gameInstalled = Signal(str, str)  # game id, message
    requestHide = Signal()
    requestShow = Signal()

    # Data change notifications
    gamesChanged = Signal()
    categoriesChanged = Signal()
    settingsChanged = Signal()
    runnersChanged = Signal()
    releasesChanged = Signal()
    installersChanged = Signal()
    pluginsChanged = Signal()
    busyChanged = Signal()
    progressChanged = Signal()
    coverFetched = Signal(str, "QVariantMap")  # form token, cover info
    easyInstallNeedsExe = Signal(str, str, str)  # token, name, start folder url

    _dispatch = Signal(object)

    def __init__(self, theme_manager=None, parent=None) -> None:
        super().__init__(parent)
        config.ensure_dirs()
        self._theme = theme_manager
        self.settings = Settings.load()
        self.library = Library()
        self.runner_manager = RunnerManager()
        self.proton_manager = ProtonManager()

        self._search_text = ""
        self._category_filter = "All"
        self._releases: list = []
        self._releases_family = ""
        self._releases_status = "idle"
        self._installer_search = ""
        self._installer_category = "All"
        self._runner_busy = False
        self._easy_busy = False
        self._progress = -1.0
        self._pending_installs: dict[str, dict] = {}

        # Queued so worker threads can hand callables to the UI thread.
        self._dispatch.connect(self._run_dispatched, Qt.ConnectionType.QueuedConnection)

    # ------------------------------------------------------------- threading

    def _run_dispatched(self, callback) -> None:
        callback()

    def _async(self, work, done=None, fail=None) -> None:
        def runner():
            try:
                result = work()
            except Exception as exc:  # noqa: BLE001 - surfaced to the user
                message = str(exc) or exc.__class__.__name__
                if fail is not None:
                    self._dispatch.emit(lambda: fail(message))
                else:
                    self._dispatch.emit(lambda: self.notify.emit(message))
                return
            if done is not None:
                self._dispatch.emit(lambda: done(result))

        threading.Thread(target=runner, daemon=True).start()

    # ------------------------------------------------------------ app info

    @Property(str, constant=True)
    def appVersion(self) -> str:
        return __version__

    @Property(str, constant=True)
    def appId(self) -> str:
        return APP_ID

    @Property(str, constant=True)
    def acknowledgement(self) -> str:
        return ACKNOWLEDGEMENT

    @Slot()
    def quit(self) -> None:
        from PySide6.QtCore import QCoreApplication

        QCoreApplication.quit()

    # ------------------------------------------------------------- settings

    def _save_settings(self) -> None:
        self.settings.save()
        self.settingsChanged.emit()

    def _get_color_scheme(self) -> str:
        return self.settings.color_scheme

    def _set_color_scheme(self, value: str) -> None:
        if value not in COLOR_SCHEMES or value == self.settings.color_scheme:
            return
        self.settings.color_scheme = value
        if self._theme is not None:
            self._theme.apply(value)
        self._save_settings()

    colorScheme = Property(str, _get_color_scheme, _set_color_scheme, notify=settingsChanged)

    def _get_view_mode(self) -> str:
        return self.settings.view_mode

    def _set_view_mode(self, value: str) -> None:
        if value not in VIEW_MODES or value == self.settings.view_mode:
            return
        self.settings.view_mode = value
        self._save_settings()

    viewMode = Property(str, _get_view_mode, _set_view_mode, notify=settingsChanged)

    def _get_sort_mode(self) -> str:
        return self.settings.sort_mode

    def _set_sort_mode(self, value: str) -> None:
        if value == self.settings.sort_mode:
            return
        if value not in {key for key, _label in SORT_OPTIONS}:
            return
        self.settings.sort_mode = value
        self._save_settings()
        self.gamesChanged.emit()

    sortMode = Property(str, _get_sort_mode, _set_sort_mode, notify=settingsChanged)

    def _get_default_runner(self) -> str:
        return self.settings.default_runner

    def _set_default_runner(self, value: str) -> None:
        if value and value != self.settings.default_runner:
            self.settings.default_runner = value
            self._save_settings()

    defaultRunner = Property(
        str, _get_default_runner, _set_default_runner, notify=settingsChanged
    )

    def _get_close_on_launch(self) -> bool:
        return self.settings.close_on_launch

    def _set_close_on_launch(self, value: bool) -> None:
        if bool(value) != self.settings.close_on_launch:
            self.settings.close_on_launch = bool(value)
            self._save_settings()

    closeOnLaunch = Property(
        bool, _get_close_on_launch, _set_close_on_launch, notify=settingsChanged
    )

    def _get_defaults(self) -> dict:
        return {
            name: getattr(self.settings, f"default_{name}")
            for name in _DEFAULTED_TOGGLES
        }

    defaultToggles = Property("QVariantMap", _get_defaults, notify=settingsChanged)

    @Slot(str, bool)
    def setDefaultToggle(self, name: str, value: bool) -> None:
        field = f"default_{name}"
        if name in _DEFAULTED_TOGGLES and getattr(self.settings, field) != bool(value):
            setattr(self.settings, field, bool(value))
            self._save_settings()

    @Property("QVariantList", constant=True)
    def sortOptions(self) -> list:
        return [{"key": key, "label": label} for key, label in SORT_OPTIONS]

    # -------------------------------------------------------------- library

    def _get_search_text(self) -> str:
        return self._search_text

    def _set_search_text(self, value: str) -> None:
        if value != self._search_text:
            self._search_text = value
            self.gamesChanged.emit()

    searchText = Property(str, _get_search_text, _set_search_text, notify=gamesChanged)

    def _get_category_filter(self) -> str:
        return self._category_filter

    def _set_category_filter(self, value: str) -> None:
        if value != self._category_filter:
            self._category_filter = value or "All"
            self.gamesChanged.emit()

    categoryFilter = Property(
        str, _get_category_filter, _set_category_filter, notify=gamesChanged
    )

    def _runner_label(self, game: Game) -> str:
        if game.is_linux:
            return "Linux native"
        return self.runner_manager.label(game.runner)

    def _game_row(self, game: Game) -> dict:
        cover = game.cover_path if game.cover_path and Path(game.cover_path).is_file() else ""
        row = {
            "gameId": game.id,
            "name": game.name,
            "category": game.display_category,
            "runnerLabel": self._runner_label(game),
            "lastPlayed": format_last_played(game.last_played),
            "coverUrl": QUrl.fromLocalFile(cover).toString() if cover else "",
            "coverIsIcon": cover.lower().endswith(".ico"),
            "initials": initials(game.name),
            "accent": accent_index(game.id or game.name),
            "isLinux": game.is_linux,
        }
        meta = row["runnerLabel"]
        if row["category"] != "Uncategorized":
            meta = f"{row['category']} · {meta}"
        row["subtitle"] = meta
        return row

    def _get_games(self) -> list:
        found = self.library.search(
            self._search_text,
            category=self._category_filter,
            sort=self.settings.sort_mode,
        )
        return [self._game_row(game) for game in found]

    games = Property("QVariantList", _get_games, notify=gamesChanged)

    def _get_library_size(self) -> int:
        return len(self.library)

    librarySize = Property(int, _get_library_size, notify=gamesChanged)

    def _get_categories(self) -> list:
        return ["All", *self.library.categories()]

    categories = Property("QVariantList", _get_categories, notify=categoriesChanged)

    @Property("QVariantList", constant=True)
    def formCategories(self) -> list:
        names = list(DEFAULT_CATEGORIES)
        for name in self.library.categories():
            if name not in names:
                names.append(name)
        return names

    def _library_updated(self) -> None:
        self.gamesChanged.emit()
        self.categoriesChanged.emit()

    @Slot(str, result="QVariantMap")
    def getGame(self, game_id: str) -> dict:
        game = self.library.get(game_id)
        if game is None:
            return {}
        data = {
            "gameId": game.id,
            "name": game.name,
            "exePath": game.exe_path,
            "arguments": game.arguments,
            "workingDirectory": game.working_directory,
            "runner": game.runner,
            "prefixPath": game.prefix_path,
            "additionalApp": game.additional_app,
            "environment": game.environment,
            "category": game.display_category,
            "virtualDesktopSize": game.virtual_desktop_size,
            "coverPath": game.cover_path,
            "steamAppid": game.steam_appid,
            "isLinux": game.is_linux,
        }
        for name in _TOGGLE_FIELDS:
            data[name] = getattr(game, name)
        return data

    @Slot(result="QVariantMap")
    def newGameTemplate(self) -> dict:
        data = {
            "gameId": uuid.uuid4().hex,
            "name": "",
            "exePath": "",
            "arguments": "",
            "workingDirectory": "",
            "runner": self.settings.default_runner,
            "prefixPath": "",
            "additionalApp": "",
            "environment": "",
            "category": "Uncategorized",
            "virtualDesktopSize": "1920x1080",
            "coverPath": "",
            "steamAppid": 0,
            "isLinux": False,
        }
        for name in _TOGGLE_FIELDS:
            default = f"default_{name}"
            data[name] = getattr(self.settings, default, Game.__dataclass_fields__[name].default)
        return data

    @Slot("QVariantMap")
    def saveGame(self, values) -> None:
        values = dict(values)
        game_id = str(values.get("gameId") or uuid.uuid4().hex)
        existing = self.library.get(game_id)
        game = existing or Game(name="", id=game_id)
        game.name = str(values.get("name") or "").strip()
        if not game.name:
            self.notify.emit("A game needs a name")
            return
        game.exe_path = as_local_path(str(values.get("exePath") or "").strip())
        game.arguments = str(values.get("arguments") or "").strip()
        game.working_directory = as_local_path(
            str(values.get("workingDirectory") or "").strip()
        )
        game.kind = "linux" if values.get("isLinux") else "windows"
        game.runner = str(values.get("runner") or SYSTEM_WINE) if not values.get("isLinux") else SYSTEM_WINE
        game.prefix_path = str(values.get("prefixPath") or "").strip()
        game.additional_app = str(values.get("additionalApp") or "").strip()
        game.environment = str(values.get("environment") or "").strip()
        game.category = str(values.get("category") or "").strip() or "Uncategorized"
        game.virtual_desktop_size = (
            str(values.get("virtualDesktopSize") or "").strip() or "1920x1080"
        )
        game.cover_path = str(values.get("coverPath") or "")
        try:
            game.steam_appid = int(values.get("steamAppid") or 0)
        except (TypeError, ValueError):
            game.steam_appid = 0
        for name in _TOGGLE_FIELDS:
            if name in values:
                setattr(game, name, bool(values[name]))

        if existing is None:
            self.library.add(game)
            self.notify.emit(f"Added “{game.name}”")
        else:
            self.library.update(game)
            self.notify.emit(f"Updated “{game.name}”")
        self._library_updated()
        if not game.cover_path:
            self.fetchCover(game.id)

    @Slot(str)
    def removeGame(self, game_id: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            return
        self.library.remove(game_id)
        self._library_updated()
        self.notify.emit(f"Removed “{game.name}”")

    @Slot(str)
    def playGame(self, game_id: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            self.notify.emit("Select a game first")
            return
        try:
            started = launch(game, self.runner_manager)
        except Exception as exc:  # noqa: BLE001 - surface any launch failure
            self.notify.emit(f"Could not launch “{game.name}”: {exc}")
            return
        self.library.mark_played(game.id)
        self.notify.emit(f"Launching “{game.name}”…")
        self.gamesChanged.emit()
        if self.settings.close_on_launch:
            self.requestHide.emit()

        name = game.name

        def watch():
            return started.failure()

        def report(reason):
            if reason:
                # close-on-launch may have hidden the window; an error nobody
                # can see is no better than the silence it replaced.
                self.requestShow.emit()
                self.notify.emit(f"“{name}” stopped right away: {reason}")

        self._async(watch, done=report)

    @Slot(str, str)
    def runPrefixTool(self, game_id: str, tool: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            return
        if game.is_linux:
            self.notify.emit("Prefix tools are only available for Windows games")
            return
        try:
            argv, env = tool_command(game, self.runner_manager, tool)
            subprocess.Popen(argv, env=env)
        except Exception as exc:  # noqa: BLE001
            self.notify.emit(str(exc))
            return
        self.notify.emit(f"Opening {tool} for “{game.name}”")

    @Slot(str)
    def openPrefix(self, game_id: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            return
        if game.is_linux:
            self.notify.emit("Linux games do not use a Wine prefix")
            return
        prefix = Path(game.prefix_path or str(config.prefixes_dir() / game.id))
        drive_c = prefix_drive_c(prefix)
        target = drive_c.parent if drive_c is not None else prefix
        try:
            target.mkdir(parents=True, exist_ok=True)
        except OSError as exc:
            self.notify.emit(f"Could not open the prefix folder: {exc}")
            return
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(target.resolve())))

    @Slot(str)
    def createShortcut(self, game_id: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            return
        command = f"{_launcher_command()} --launch {game.id}"
        try:
            path = create_desktop_shortcut(game, command)
        except OSError as exc:
            self.notify.emit(f"Could not create the shortcut: {exc}")
            return
        self.notify.emit(f"Shortcut created at {path}")

    # ---------------------------------------------------------------- covers

    @Slot(str)
    def fetchCover(self, game_id: str) -> None:
        game = self.library.get(game_id)
        if game is None:
            return
        exe_path = "" if game.is_linux else game.exe_path

        def work():
            return fetch_cover(game.name, game.id, exe_path=exe_path)

        def done(hit):
            current = self.library.get(game_id)
            if current is None:
                return
            current.cover_path = hit.cover_path
            if hit.appid:
                current.steam_appid = hit.appid
            if current.display_category == "Uncategorized" and hit.category:
                current.category = hit.category
            self.library.update(current)
            self._library_updated()
            self.notify.emit(f"Cover set from {hit.origin_label}: {hit.name}")

        self._async(work, done=done)

    @Slot(str, str, str, str)
    def fetchCoverForForm(self, token: str, game_id: str, name: str, exe_path: str) -> None:
        """Cover lookup for the add/edit form, reported via ``coverFetched``."""
        name = name.strip()
        if not name:
            self.notify.emit("Enter a game name first")
            return
        self.notify.emit(f"Looking for artwork for “{name}”…")
        exe = as_local_path(exe_path.strip())

        def work():
            return fetch_cover(name, game_id, exe_path=exe)

        def done(hit):
            self.coverFetched.emit(
                token,
                {
                    "coverPath": hit.cover_path,
                    "steamAppid": hit.appid,
                    "category": hit.category,
                    "origin": hit.origin_label,
                    "name": hit.name,
                },
            )
            self.notify.emit(f"Cover found via {hit.origin_label}: {hit.name}")

        self._async(work, done=done)

    @Slot(str, str, result=str)
    def importCustomCover(self, game_id: str, file_url: str) -> str:
        source = as_local_path(file_url)
        try:
            path = copy_custom_cover(source, game_id)
        except (OSError, FileNotFoundError) as exc:
            self.notify.emit(f"Could not copy cover: {exc}")
            return ""
        self.notify.emit("Custom cover added")
        return str(path)

    @Slot(str, result=str)
    def urlToLocalFile(self, url: str) -> str:
        """Turn a file-dialog result (including network shares) into a path."""
        return as_local_path(url)

    @Slot(str, result=str)
    def coverUrlFor(self, cover_path: str) -> str:
        if cover_path and Path(cover_path).is_file():
            return QUrl.fromLocalFile(cover_path).toString()
        return ""

    # ---------------------------------------------------------------- runners

    def _get_runner_choices(self) -> list:
        return [
            {"runnerId": runner_id, "label": label}
            for runner_id, label in self.runner_manager.choices()
        ]

    runnerChoices = Property("QVariantList", _get_runner_choices, notify=runnersChanged)

    def _get_installed_runners(self) -> list:
        wine = self.runner_manager.system_wine()
        rows = [
            {
                "runnerId": SYSTEM_WINE,
                "name": "System Wine",
                "detail": wine.version() if wine.is_available() else "Not installed on this system",
                "available": wine.is_available(),
                "removable": False,
            }
        ]
        for proton in self.runner_manager.installed_protons():
            rows.append(
                {
                    "runnerId": proton.id,
                    "name": proton.name,
                    "detail": proton.family_label(),
                    "available": True,
                    "removable": True,
                }
            )
        return rows

    installedRunners = Property(
        "QVariantList", _get_installed_runners, notify=runnersChanged
    )

    @Property("QVariantList", constant=True)
    def runnerFamilies(self) -> list:
        return [
            {
                "familyId": family.id,
                "name": family.name,
                "description": family.description,
                "maintainer": family.maintainer,
                "homepage": family.homepage,
                "kind": family.kind,
            }
            for family in families()
        ]

    @Property("QVariantList", constant=True)
    def runnerGuide(self) -> list:
        return [
            {
                "title": row.title,
                "kind": row.kind.capitalize(),
                "advice": row.advice,
                "maintainer": row.maintainer,
                "homepage": row.homepage,
            }
            for row in runner_guide_details()
        ]

    def _get_releases(self) -> list:
        return [
            {
                "tag": release.tag,
                "assetName": release.name,
                "sizeMb": round(release.size_mb),
                "familyName": release.family.name,
                "installed": self.proton_manager.is_release_installed(release),
            }
            for release in self._releases
        ]

    releases = Property("QVariantList", _get_releases, notify=releasesChanged)

    def _get_releases_status(self) -> str:
        return self._releases_status

    releasesStatus = Property(str, _get_releases_status, notify=releasesChanged)

    @Slot(str)
    def fetchReleases(self, family_id: str) -> None:
        self._releases_family = family_id
        self._releases_status = "loading"
        self._releases = []
        self.releasesChanged.emit()

        def work():
            return self.proton_manager.fetch_available(limit=12, family=family_id)

        def done(found):
            if family_id != self._releases_family:
                return
            self._releases = found
            self._releases_status = "ready"
            self.releasesChanged.emit()

        def fail(message):
            if family_id != self._releases_family:
                return
            self._releases_status = f"error: {message}"
            self.releasesChanged.emit()

        self._async(work, done=done, fail=fail)

    def _get_busy(self) -> bool:
        return self._runner_busy or self._easy_busy

    busy = Property(bool, _get_busy, notify=busyChanged)

    def _get_progress(self) -> float:
        return self._progress

    progress = Property(float, _get_progress, notify=progressChanged)

    def _set_progress(self, fraction: float) -> None:
        self._progress = fraction
        self.progressChanged.emit()

    def _progress_cb(self, fraction: float) -> None:
        # Called from a worker thread; queue the property update.
        self._dispatch.emit(lambda: self._set_progress(fraction))

    @Slot(str)
    def installRelease(self, tag: str) -> None:
        if self._runner_busy:
            return
        release = next((item for item in self._releases if item.tag == tag), None)
        if release is None:
            return
        self._runner_busy = True
        self._set_progress(0.0)
        self.busyChanged.emit()
        self.notify.emit(f"Downloading {release.tag}…")

        def work():
            self.proton_manager.install(release, progress_cb=self._progress_cb)

        def done(_result):
            self._runner_busy = False
            self._set_progress(-1.0)
            self.busyChanged.emit()
            self.notify.emit(
                f"Installed {release.tag}. You can now choose it when adding or editing a game."
            )
            self.runnersChanged.emit()
            self.releasesChanged.emit()
            self._library_updated()

        def fail(message):
            self._runner_busy = False
            self._set_progress(-1.0)
            self.busyChanged.emit()
            self.notify.emit(f"Failed to install {release.tag}: {message}")

        self._async(work, done=done, fail=fail)

    @Slot(str)
    def uninstallRunner(self, runner_id: str) -> None:
        try:
            self.proton_manager.uninstall(runner_id)
        except Exception as exc:  # noqa: BLE001
            self.notify.emit(f"Could not remove {runner_id}: {exc}")
            return
        self.notify.emit(f"Removed {runner_id}")
        self.runnersChanged.emit()
        self.releasesChanged.emit()
        self._library_updated()

    # -------------------------------------------------------------- installers

    @Property("QVariantList", constant=True)
    def installerCategories(self) -> list:
        return ["All", *INSTALLER_CATEGORIES]

    def _get_installer_search(self) -> str:
        return self._installer_search

    def _set_installer_search(self, value: str) -> None:
        if value != self._installer_search:
            self._installer_search = value
            self.installersChanged.emit()

    installerSearch = Property(
        str, _get_installer_search, _set_installer_search, notify=installersChanged
    )

    def _get_installer_category(self) -> str:
        return self._installer_category

    def _set_installer_category(self, value: str) -> None:
        if value != self._installer_category:
            self._installer_category = value or "All"
            self.installersChanged.emit()

    installerCategory = Property(
        str, _get_installer_category, _set_installer_category, notify=installersChanged
    )

    def _get_installers(self) -> list:
        rows = []
        for item in search_installers(self._installer_search, self._installer_category):
            subtitle = item.description
            if item.notes:
                subtitle = f"{subtitle}\n{item.notes}"
            rows.append(
                {
                    "installerId": item.id,
                    "name": item.name,
                    "subtitle": subtitle,
                    "category": item.category,
                }
            )
        return rows

    installers = Property("QVariantList", _get_installers, notify=installersChanged)

    @Slot(str, str)
    def installEasy(self, installer_id: str, runner_id: str) -> None:
        if self._easy_busy:
            self.notify.emit("Another install is already running")
            return
        try:
            installer = installer_by_id(installer_id)
        except KeyError:
            return
        runner = self.runner_manager.get(runner_id or self.settings.default_runner)
        if not runner.is_available():
            self.notify.emit(f"{runner.name} is not available. Download a runner first.")
            return
        game_id = uuid.uuid4().hex
        try:
            prefix = prepare_prefix(game_id)
        except OSError as exc:
            self.notify.emit(f"Could not create a prefix for {installer.name}: {exc}")
            return

        self._easy_busy = True
        self._set_progress(0.0)
        self.busyChanged.emit()
        self.notify.emit(f"Downloading {installer.name}…")
        resolved_runner_id = runner_id or self.settings.default_runner

        def work():
            archive = download_installer(installer, progress_cb=self._progress_cb)
            self._dispatch.emit(
                lambda: self.notify.emit(
                    f"Launching the {installer.name} installer… Finish the vendor "
                    "wizard, then close it — GameHandler adds it as soon as the "
                    "install lands."
                )
            )
            argv, env = build_installer_command(runner, prefix, installer, archive)
            Path(prefix).mkdir(parents=True, exist_ok=True)
            completed = subprocess.run(
                argv, env=env, cwd=str(archive.parent), check=False
            )
            found = wait_for_installer(runner, env, prefix, installer.expected_exe)
            return completed.returncode, found

        def done(result):
            returncode, found = result
            if found is not None:
                self._finish_easy_install(
                    installer, found, prefix, resolved_runner_id, game_id
                )
                return
            suffix = f" (installer exited {returncode})" if returncode else ""
            self.notify.emit(
                f"Could not find the {installer.name} executable in the prefix{suffix}. "
                "Pick it yourself if the install finished."
            )
            token = game_id
            self._pending_installs[token] = {
                "installer": installer,
                "prefix": prefix,
                "runner_id": resolved_runner_id,
                "game_id": game_id,
            }
            start = prefix_drive_c(prefix)
            start_url = QUrl.fromLocalFile(str(start or prefix)).toString()
            self.easyInstallNeedsExe.emit(token, installer.name, start_url)

        def fail(message):
            self._easy_busy = False
            self._set_progress(-1.0)
            self.busyChanged.emit()
            self.notify.emit(f"Could not install {installer.name}: {message}")

        self._async(work, done=done, fail=fail)

    def _finish_easy_install(self, installer, exe_path, prefix, runner_id, game_id) -> None:
        game = game_from_install(installer, exe_path, prefix, runner_id, game_id=game_id)
        # A store launcher is not a Steam product; the executable the vendor
        # just installed carries the right artwork already.
        try:
            game.cover_path = str(save_exe_icon(exe_path, game_id))
        except (OSError, RuntimeError):
            game.cover_path = ""
        self.library.add(game)
        self._easy_busy = False
        self._set_progress(-1.0)
        self.busyChanged.emit()
        self._library_updated()
        self.gameInstalled.emit(game.id, f"Installed “{game.name}”")

    @Slot(str, str)
    def completeEasyInstall(self, token: str, file_url: str) -> None:
        pending = self._pending_installs.pop(token, None)
        if pending is None:
            return
        exe = as_local_path(file_url)
        if not exe:
            self.cancelEasyInstall(token)
            return
        self._finish_easy_install(
            pending["installer"],
            Path(exe),
            pending["prefix"],
            pending["runner_id"],
            pending["game_id"],
        )

    @Slot(str)
    def cancelEasyInstall(self, token: str) -> None:
        pending = self._pending_installs.pop(token, None)
        self._easy_busy = False
        self._set_progress(-1.0)
        self.busyChanged.emit()
        if pending is not None:
            self.notify.emit(
                f"Kept the {pending['installer'].name} prefix. "
                "Add it later from Add Game if you want."
            )

    # ---------------------------------------------------------------- plugins

    def _plugin_row(self, plugin) -> dict:
        installed = plugin.is_installed()
        sandboxed = in_flatpak()
        if installed:
            subtitle = f"{plugin.description} {plugin.used_for}"
            state = "installed"
        elif sandboxed:
            subtitle = (
                f"{plugin.description} Not bundled in this Flatpak. Installing it "
                "on the host would not expose it to the sandbox."
            )
            state = "unavailable"
        else:
            try:
                command = format_command(privileged_command(install_command(plugin)))
                subtitle = f"{plugin.description} Not installed. Install with: {command}"
                state = "missing"
            except RuntimeError:
                subtitle = (
                    f"{plugin.description} Not installed. "
                    "No package mapping is available for this system."
                )
                state = "unavailable"
        return {
            "pluginId": plugin.id,
            "name": plugin.name,
            "subtitle": subtitle,
            "state": state,
        }

    def _get_plugins(self) -> list:
        return [self._plugin_row(plugin) for plugin in PLUGINS]

    plugins = Property("QVariantList", _get_plugins, notify=pluginsChanged)

    @Property(str, constant=True)
    def pluginsIntro(self) -> str:
        if in_flatpak():
            return (
                "These tools are optional. GameHandler offers them on each game. "
                "The Flatpak can only use helpers bundled in its sandbox; host "
                "package installation is intentionally disabled."
            )
        manager = detect_package_manager()
        detected = f" ({manager})." if manager else "."
        return (
            "These tools are optional. GameHandler offers them on each game. "
            "Missing helpers can be installed with the detected host package "
            f"manager{detected}"
        )

    @Slot()
    def refreshPlugins(self) -> None:
        self.pluginsChanged.emit()

    @Slot(str)
    def installPlugin(self, plugin_id: str) -> None:
        try:
            plugin = plugin_by_id(plugin_id)
        except KeyError:
            return
        self.notify.emit(f"Installing {plugin.name}…")

        def work():
            return install_plugin(plugin)

        def done(result):
            self.pluginsChanged.emit()
            if result.returncode == 0 and plugin.is_installed():
                self.notify.emit(f"{plugin.name} is installed")
            else:
                self.notify.emit(
                    f"{plugin.name} did not install. The command is shown on the "
                    "Plugins page."
                )

        def fail(message):
            self.pluginsChanged.emit()
            self.notify.emit(f"Could not install {plugin.name}: {message}")

        self._async(work, done=done, fail=fail)

    # ---------------------------------------------------------------- credits

    @Property("QVariantList", constant=True)
    def creditSections(self) -> list:
        rendered = []
        for section in sections():
            rendered.append(
                {
                    "title": section.title,
                    "summary": section.summary,
                    "entries": [
                        {
                            "label": credit.label,
                            "role": credit.role,
                            "license": credit.license,
                            "url": credit.url,
                        }
                        for credit in section.entries
                    ],
                }
            )
        return rendered

    @Property("QVariantList", constant=True)
    def whyAllInOne(self) -> list:
        return [
            {"heading": heading, "body": body} for heading, body in WHY_ALL_IN_ONE
        ]

    @Property(str, constant=True)
    def aboutText(self) -> str:
        return (
            "A modern game manager for running Windows games on Linux with Wine "
            "and Proton. GameHandler implements none of that compatibility work "
            "itself — it downloads the upstream projects' own builds, sets up "
            "isolated prefixes, and gets out of their way.\n\n" + ACKNOWLEDGEMENT
        )


__all__ = ["Backend", "SORT_OPTIONS"]
