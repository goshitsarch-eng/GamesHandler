"""A minimal, test-only stand-in for the ``org.kde.kirigami`` QML module.

The real Kirigami is a C++ QML module that cannot be assumed on a headless
test machine. This stub registers just enough lookalike types — with the same
names, properties, and attached types the application actually uses — for the
whole QML interface to be instantiated against the real backend in an
offscreen QML engine. It exists purely so a typo in a page, a renamed backend
property, or an invalid property assignment fails a unit test instead of a
user's first launch.

Nothing here is shipped; the application always runs on the real Kirigami.
"""

from __future__ import annotations

from enum import IntEnum
from pathlib import Path

from PySide6.QtCore import Property, QEnum, QObject, QUrl, Signal
from PySide6.QtGui import QColor, QFont
from PySide6.QtQml import (
    QmlAnonymous,
    QmlAttached,
    QmlElement,
    QmlSingleton,
    qmlRegisterType,
)

QML_IMPORT_NAME = "org.kde.kirigami"
QML_IMPORT_MAJOR_VERSION = 6
QML_IMPORT_MINOR_VERSION = 0

URI = QML_IMPORT_NAME
_QML_DIR = Path(__file__).resolve().parent


@QmlAnonymous
class IconSizes(QObject):
    @Property(int, constant=True)
    def small(self):
        return 16

    @Property(int, constant=True)
    def smallMedium(self):
        return 22

    @Property(int, constant=True)
    def medium(self):
        return 32

    @Property(int, constant=True)
    def large(self):
        return 48


@QmlElement
@QmlSingleton
class Units(QObject):
    def __init__(self, parent=None):
        super().__init__(parent)
        self._icon_sizes = IconSizes(self)

    @Property(int, constant=True)
    def gridUnit(self):
        return 18

    @Property(int, constant=True)
    def smallSpacing(self):
        return 4

    @Property(int, constant=True)
    def largeSpacing(self):
        return 8

    @Property(int, constant=True)
    def shortDuration(self):
        return 150

    @Property(int, constant=True)
    def longDuration(self):
        return 250

    @Property(QObject, constant=True)
    def iconSizes(self):
        return self._icon_sizes


@QmlAnonymous
class ThemeAttached(QObject):
    """The subset of Kirigami.Theme the interface reads."""

    changed = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._small_font = QFont()
        self._small_font.setPointSize(8)

    @Property(QFont, notify=changed)
    def smallFont(self):
        return self._small_font

    @Property(QFont, notify=changed)
    def defaultFont(self):
        return QFont()

    @Property(QColor, notify=changed)
    def highlightColor(self):
        return QColor("#3daee9")

    @Property(QColor, notify=changed)
    def textColor(self):
        return QColor("#fcfcfc")

    @Property(QColor, notify=changed)
    def backgroundColor(self):
        return QColor("#232629")

    @Property(QColor, notify=changed)
    def alternateBackgroundColor(self):
        return QColor("#31363b")


@QmlElement
@QmlAttached(ThemeAttached)
class Theme(QObject):
    @staticmethod
    def qmlAttachedProperties(self, obj):  # noqa: N802,PLW0211 - Qt calling shape
        return ThemeAttached(obj)


@QmlAnonymous
class FormDataAttached(QObject):
    changed = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._label = ""
        self._is_section = False

    def _get_label(self):
        return self._label

    def _set_label(self, value):
        self._label = value
        self.changed.emit()

    label = Property(str, _get_label, _set_label, notify=changed)

    def _get_is_section(self):
        return self._is_section

    def _set_is_section(self, value):
        self._is_section = bool(value)
        self.changed.emit()

    isSection = Property(bool, _get_is_section, _set_is_section, notify=changed)


@QmlElement
@QmlAttached(FormDataAttached)
class FormData(QObject):
    @staticmethod
    def qmlAttachedProperties(self, obj):  # noqa: N802,PLW0211 - Qt calling shape
        return FormDataAttached(obj)


@QmlElement
class Dialog(QObject):
    """Only exists for the ``Kirigami.Dialog.*`` standard-button enums."""

    @QEnum
    class StandardButton(IntEnum):
        NoButton = 0x0
        Ok = 0x400
        Close = 0x200000
        Cancel = 0x400000


_QML_TYPES = (
    "ApplicationWindow",
    "Page",
    "ScrollablePage",
    "GlobalDrawer",
    "Action",
    "Heading",
    "Separator",
    "SearchField",
    "PlaceholderMessage",
    "AbstractCard",
    "Chip",
    "Icon",
    "UrlButton",
    "PromptDialog",
    "InlineMessage",
    "FormLayout",
)


def register() -> None:
    """Register the QML-file halves of the stub (Python types self-register)."""
    for name in _QML_TYPES:
        qmlRegisterType(
            QUrl.fromLocalFile(str(_QML_DIR / f"{name}.qml")), URI, 6, 0, name
        )
