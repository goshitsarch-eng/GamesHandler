//! GameHandler core logic.
//!
//! Pure, synchronous and GUI-free. This crate must never depend on
//! `libcosmic`, `iced`, or any other GUI crate — that rule (DECISIONS D-03) is
//! what keeps the port's logic testable with
//! `cargo test -p gamehandler-core` on a headless machine, with no display
//! server, no GPU and no async runtime.
//!
//! # Layout
//!
//! Modules land in dependency order (see `docs/migration/PLAN.md` §6):
//! `paths`, `models` and `settings` (T-02), then `runners` (T-03),
//! `installers` (T-04), `covers` / `exe_icons` / `netpaths` (T-05) and
//! `plugins` / `credits` (T-06). Each one mirrors a module of the Python tree
//! named in `docs/migration/architecture.md` §1.2.
//!
//! # Blocking by design
//!
//! Every operation that touches the network, a subprocess or the filesystem is
//! a plain blocking function here, reporting progress through a
//! `progress: &dyn Fn(f32)` callback — the same shape as the Python app's
//! `progress_cb` (`runners.py`, `installers.py`). The binary crate is what
//! wraps those calls in `spawn_blocking`, so the iced executor is never
//! stalled (`docs/migration/architecture.md` §3.1).

/// Reverse-domain application id.
///
/// One value drives the COSMIC window's application id, the installed desktop
/// entry and the Flatpak manifest, so it must match `gamehandler.APP_ID` and
/// `data/` exactly.
pub const APP_ID: &str = "com.goshapps.GameHandler";

/// Human-readable application name, as printed by `--version`.
pub const APP_NAME: &str = "GameHandler";

/// Application version, read from the workspace `Cargo.toml`.
///
/// Every crate in the workspace inherits `version.workspace = true`, so this
/// constant, `gamehandler --version`, the desktop file and the metainfo file
/// cannot drift apart.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod hash;
pub mod covers;
pub mod credits;
pub mod exe_icons;
pub mod installers;
pub mod json;
pub mod models;
pub mod netpaths;
pub mod paths;
pub mod plugins;
pub mod runners;
pub mod settings;

#[cfg(test)]
mod oracle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_major_minor_patch() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "VERSION should be MAJOR.MINOR.PATCH, got {VERSION:?}"
        );
        for part in parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "non-numeric component {part:?} in VERSION {VERSION:?}"
            );
        }
    }

    #[test]
    fn app_id_is_reverse_domain() {
        // Dotted, at least three components, com. namespace — the shape
        // Flatpak's `--app-id` and the desktop-file naming rules require.
        let parts: Vec<&str> = APP_ID.split('.').collect();
        assert!(
            parts.len() >= 3,
            "APP_ID should be reverse-domain, got {APP_ID:?}"
        );
        assert!(
            APP_ID.starts_with("com."),
            "APP_ID should live in the com. namespace, got {APP_ID:?}"
        );
        assert!(
            parts.iter().all(|part| !part.is_empty()),
            "APP_ID has an empty component: {APP_ID:?}"
        );
    }

    #[test]
    fn app_name_is_the_display_name() {
        // `--version` prints "{APP_NAME} {VERSION}", matching main.py's
        // argparse output byte for byte (PLAN.md §5, P-70).
        assert_eq!(APP_NAME, "GameHandler");
    }
}
