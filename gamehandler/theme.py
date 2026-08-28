"""Light and dark color schemes for the Qt 6 / Kirigami interface.

Kirigami takes its colors from the active Qt palette (through
qqc2-desktop-style on a desktop). Under a full Plasma session the platform
theme supplies that palette; everywhere else — and whenever the user forces
light or dark in Settings — GameHandler applies its own Breeze-flavoured
palette so both modes work on any desktop.
"""

from __future__ import annotations

from PySide6.QtGui import QColor, QPalette

ACCENT = "#3daee9"

# (window, window_text, base, alternate_base, text, button, button_text,
#  tooltip_base, tooltip_text, highlight, highlighted_text, link, disabled_text)
_DARK = (
    "#232629", "#fcfcfc", "#1b1e20", "#232629", "#fcfcfc", "#31363b",
    "#fcfcfc", "#31363b", "#fcfcfc", ACCENT, "#fcfcfc", "#1d99f3", "#6e7175",
)
_LIGHT = (
    "#eff0f1", "#232629", "#ffffff", "#f7f7f8", "#232629", "#fcfcfc",
    "#232629", "#fcfcfc", "#232629", ACCENT, "#ffffff", "#2980b9", "#a0a2a5",
)

# The generated cover plates: the same gradients the library used before, so
# a game's tile keeps its shade across the rewrite.
COVER_GRADIENTS = (
    ("#3f6fd8", "#23407f"),
    ("#8a4fd6", "#4b2380"),
    ("#1f8f78", "#10513f"),
    ("#c1533f", "#6f2a20"),
    ("#b8862c", "#6d4a12"),
    ("#2c7fa8", "#164b64"),
    ("#a4406e", "#5d1f3c"),
    ("#4c6b8a", "#2a3c50"),
)


def build_palette(scheme: str) -> QPalette | None:
    """A ready-to-apply palette for ``light``/``dark``; ``None`` for system."""
    if scheme == "dark":
        values = _DARK
    elif scheme == "light":
        values = _LIGHT
    else:
        return None
    (
        window, window_text, base, alternate, text, button, button_text,
        tip_base, tip_text, highlight, highlighted, link, disabled,
    ) = values
    palette = QPalette()
    palette.setColor(QPalette.ColorRole.Window, QColor(window))
    palette.setColor(QPalette.ColorRole.WindowText, QColor(window_text))
    palette.setColor(QPalette.ColorRole.Base, QColor(base))
    palette.setColor(QPalette.ColorRole.AlternateBase, QColor(alternate))
    palette.setColor(QPalette.ColorRole.Text, QColor(text))
    palette.setColor(QPalette.ColorRole.PlaceholderText, QColor(disabled))
    palette.setColor(QPalette.ColorRole.Button, QColor(button))
    palette.setColor(QPalette.ColorRole.ButtonText, QColor(button_text))
    palette.setColor(QPalette.ColorRole.ToolTipBase, QColor(tip_base))
    palette.setColor(QPalette.ColorRole.ToolTipText, QColor(tip_text))
    palette.setColor(QPalette.ColorRole.Highlight, QColor(highlight))
    palette.setColor(QPalette.ColorRole.HighlightedText, QColor(highlighted))
    palette.setColor(QPalette.ColorRole.Link, QColor(link))
    palette.setColor(QPalette.ColorRole.BrightText, QColor("#ffffff"))
    for role in (
        QPalette.ColorRole.WindowText,
        QPalette.ColorRole.Text,
        QPalette.ColorRole.ButtonText,
    ):
        palette.setColor(QPalette.ColorGroup.Disabled, role, QColor(disabled))
    return palette


class ThemeManager:
    """Applies the persisted color scheme to a running application."""

    def __init__(self, app) -> None:
        self._app = app
        # The palette the platform handed us, restored for "system".
        self._system_palette = QPalette(app.palette())

    def apply(self, scheme: str) -> None:
        palette = build_palette(scheme)
        self._app.setPalette(
            palette if palette is not None else self._system_palette
        )


__all__ = ["ACCENT", "COVER_GRADIENTS", "ThemeManager", "build_palette"]
