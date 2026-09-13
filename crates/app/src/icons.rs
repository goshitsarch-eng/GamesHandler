//! The app's action glyphs, embedded rather than looked up.
//!
//! The reference sets Breeze as its icon fallback outside Plasma
//! (`main.py:92-95`): without it, the KDE icon names the QML uses resolve to
//! nothing. The port's equivalent problem measured worse — a probe over the
//! sixteen `icon::from_name` names this tree used resolved **none** of them on
//! a box with Adwaita and hicolor installed, because libcosmic searches the
//! configured theme (here `Cosmic`, which is not installed) plus hicolor, and
//! none of the names lives in either. `bundle::get` — the only other fallback
//! in the lookup — is non-unix code, so on Linux an unresolvable name renders
//! empty bytes. Every icon in the app rendered empty, everywhere the Cosmic
//! theme is absent — which the T-19 walk never caught, because the buttons
//! still click.
//!
//! So the glyphs ship in the binary, in the exact shape libcosmic itself uses
//! for its must-render icon (`button::icon`'s external-link: `from_svg_bytes`
//! over `include_bytes!`, `.symbolic(true)`). They are drawn black on a 16px
//! grid; the symbolic recolor replaces every pixel's RGB with the theme's icon
//! color and keeps only the alpha (measured in both the tiny-skia and wgpu
//! backends), so the glyphs theme-tint like named symbolic icons while
//! rendering identically on every system, installed or not.
//!
//! Fifteen names collapse to thirteen drawings: the menu's Play shares the
//! play triangle with the nav's `run-install`, and the menu's `folder-open`
//! shares the folder with the browse buttons. The sharing is drawn once but
//! named twice — [`Icon::Play`] and [`Icon::FolderOpen`] are their own
//! variants over shared bytes — so every variant keeps exactly one legacy
//! name and the transcription tests still read the reference.
//!
//! # What is tested, and what is read
//!
//! The mapping — every [`Icon`] has non-empty bytes that start an SVG
//! document, and every legacy name is distinct — is pinned below, as is the
//! `symbolic` flag the tint depends on. The glyphs' shapes are read: they
//! were rendered through librsvg during U7 and eyeballed. A path typo that
//! turns one into abstract art survives the suite; keep the render check in
//! the loop when touching them.
use cosmic::widget::icon::{self, Handle};

/// The app's action glyphs, one per freedesktop name (not per drawing: two
/// drawings are shared — see the module note).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// The runners page's remove button and the menu's Remove.
    Delete,
    /// The release rows' install button.
    Download,
    /// The nav Installers row and the installers page's install button.
    Install,
    /// The menu's Play and the cover Play control.
    Play,
    /// The form's two browse buttons.
    Open,
    /// The menu's Open prefix folder.
    FolderOpen,
    /// The nav Library row.
    Games,
    /// The nav Runners row.
    FolderDownload,
    /// The nav Plugins row.
    Plugins,
    /// The nav Credits row.
    Help,
    /// The nav Settings row.
    Configure,
    /// An installed runner's available emblem.
    Checked,
    /// An installed runner's missing emblem.
    Warning,
    /// The menu's Edit.
    Edit,
    /// The menu's Find cover art.
    Image,
    /// The card's and the row's "More actions" control (UX-16).
    ///
    /// `view-more-symbolic`, the name `LibraryPage.qml:210` and `:282` give it:
    /// the three dots that mean "the rest of this item's actions". The keyboard
    /// route the reference has and this port did not — the QQC2 `ToolButton`
    /// that opens `gameMenu` — so it is drawn on both delegates, where the QML
    /// draws it.
    ViewMore,
}

impl Icon {
    /// The freedesktop name this glyph stands in for: the `icon.name` the QML
    /// used at the glyph's site, kept so the transcription tests still read
    /// the reference instead of a copy. Test-only: production renders bytes,
    /// never names.
    #[cfg(test)]
    pub fn legacy_name(self) -> &'static str {
        match self {
            Icon::Delete => "delete",
            Icon::Download => "download",
            Icon::Install => "run-install",
            Icon::Play => "media-playback-start",
            Icon::Open => "document-open",
            Icon::FolderOpen => "folder-open",
            Icon::Games => "applications-games",
            Icon::FolderDownload => "folder-download",
            Icon::Plugins => "plugins",
            Icon::Help => "help-about",
            Icon::Configure => "configure",
            Icon::Checked => "emblem-checked",
            Icon::Warning => "data-warning",
            Icon::Edit => "edit-entry",
            Icon::Image => "viewimage",
            Icon::ViewMore => "view-more-symbolic",
        }
    }
}

/// The embedded bytes of an [`Icon`]. [`Icon::Play`] shares
/// [`Icon::Install`]'s triangle: a play glyph is a play glyph, and two copies
/// of the same path would drift.
fn bytes(icon: Icon) -> &'static [u8] {
    match icon {
        Icon::Delete => include_bytes!("icons/delete.svg"),
        Icon::Download => include_bytes!("icons/download.svg"),
        Icon::Install | Icon::Play => include_bytes!("icons/install.svg"),
        Icon::Open | Icon::FolderOpen => include_bytes!("icons/open.svg"),
        Icon::Games => include_bytes!("icons/games.svg"),
        Icon::FolderDownload => include_bytes!("icons/folder-download.svg"),
        Icon::Plugins => include_bytes!("icons/plugins.svg"),
        Icon::Help => include_bytes!("icons/help.svg"),
        Icon::Configure => include_bytes!("icons/configure.svg"),
        Icon::Checked => include_bytes!("icons/checked.svg"),
        Icon::Warning => include_bytes!("icons/warning.svg"),
        Icon::Edit => include_bytes!("icons/edit.svg"),
        Icon::Image => include_bytes!("icons/image.svg"),
        Icon::ViewMore => include_bytes!("icons/view-more.svg"),
    }
}

/// An [`Icon`] as a symbolic handle, ready for `button::icon`,
/// `leading_icon`, or `icon::icon`. Symbolic, so the theme tints it;
/// embedded, so it renders with or without any icon theme installed.
pub fn handle(icon: Icon) -> Handle {
    icon::from_svg_bytes(bytes(icon)).symbolic(true)
}

/// Every [`Icon`] there is, for the mapping tests.
#[cfg(test)]
const ALL: [Icon; 15] = [
    Icon::Delete,
    Icon::Download,
    Icon::Install,
    Icon::Play,
    Icon::Open,
    Icon::FolderOpen,
    Icon::Games,
    Icon::FolderDownload,
    Icon::Plugins,
    Icon::Help,
    Icon::Configure,
    Icon::Checked,
    Icon::Warning,
    Icon::Edit,
    Icon::Image,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_icon_embeds_a_parseable_svg() {
        for icon in ALL {
            let bytes = bytes(icon);
            assert!(!bytes.is_empty(), "{icon:?} embeds no bytes");
            assert!(
                bytes.starts_with(b"<svg"),
                "{icon:?} does not start an SVG document: {:?}",
                &bytes[..bytes.len().min(32)]
            );
            let grid = b"viewBox=\"0 0 16 16\"";
            assert!(
                bytes.windows(grid.len()).any(|w| w == grid),
                "{icon:?} is not drawn on the 16px grid"
            );
        }
    }

    #[test]
    fn handles_are_symbolic_so_the_theme_tints_them() {
        for icon in ALL {
            assert!(
                handle(icon).symbolic,
                "{icon:?} must render symbolic — without the flag the glyph keeps its authored black on every theme"
            );
        }
    }

    #[test]
    fn every_legacy_name_is_distinct() {
        let names: BTreeSet<&str> = ALL.iter().map(|icon| icon.legacy_name()).collect();
        assert_eq!(
            names.len(),
            ALL.len(),
            "two glyphs claim one legacy name, so a transcription test cannot tell them apart"
        );
    }
}
