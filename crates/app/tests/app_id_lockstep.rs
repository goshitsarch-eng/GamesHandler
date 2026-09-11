//! One identifier, five artefacts: they must agree (DECISIONS D-28).
//!
//! `App::APP_ID` is handed to winit as the X11 `WM_CLASS` and the Wayland
//! `app_id` (`libcosmic/src/app/mod.rs:70` →
//! `iced/winit/src/conversion.rs:202,213`), and the same string is the desktop
//! entry's basename, its `StartupWMClass`, the Flatpak manifest's `app-id` and
//! the metainfo's `<launchable>`. Each of those five is validated — by
//! `desktop-file-validate`, by `appstreamcli`, by `flatpak-builder`, by
//! `cargo test` — and **no validator compares them to each other**. So a
//! mismatch is possible with every tool green, and it fails only at runtime, as
//! a window that does not associate with its launcher icon.
//!
//! This test is the missing comparison. It lives here rather than in
//! `scripts/verify.sh` because D-28 asks for a workspace test, so it also runs
//! under a plain `cargo test`; and it lives in `crates/app` because that is the
//! crate the identifier is consumed by (D-05 puts `crates/app` test scaffolding
//! with packaging). The desktop entry, the manifest and the metainfo are not
//! Rust sources, so they are read from the checkout through
//! `CARGO_MANIFEST_DIR` — the tree this test was compiled in, which is the tree
//! `verify.sh` validates.
//!
//! `gamehandler_core::APP_ID` is the source of truth. Nothing here restates the
//! literal: a test that pasted `com.goshapps.GameHandler` in would pass while
//! the constant drifted, which is precisely the defect D-28 was written about.
//! Two consequences worth knowing before editing this file:
//!
//! - the artefact *paths* are derived from `APP_ID` too, so renaming the
//!   constant without renaming the files is a failure here (file not found)
//!   rather than a silent pass;
//! - the metainfo's launchable is compared against `APP_ID + ".desktop"`, and
//!   the desktop entry's basename against `APP_ID` alone — the two spellings
//!   D-28 records.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root, derived rather than hardcoded.
///
/// `CARGO_MANIFEST_DIR` is `<root>/crates/app` for this target, so the root is
/// two levels up. Using the build-time value (rather than the process's working
/// directory) means the test reads the same checkout cargo just built, whatever
/// directory it is invoked from.
fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2)
        .expect("crates/app sits two levels below the repository root")
        .to_path_buf()
}

/// Panic with the right diagnosis when a path under the repo root cannot be
/// read.
///
/// There are two ways that happens and they call for completely different
/// actions, so telling them apart is the whole point. `CARGO_MANIFEST_DIR` is
/// baked in at compile time: a `target/` directory copied between checkouts (or
/// shared via `CARGO_TARGET_DIR`) can hand cargo an artifact built from a
/// *different* source tree, and cargo will reuse it whenever the fingerprints
/// match. The second case looks like a bug in this test and is not.
///
/// All three of `Cargo.toml`, `build-aux/` and `data/` are probed, not just the
/// first, because the tree that bites is a real checkout of something else: a
/// scratch copy at `/tmp/wire-test` has a `Cargo.toml` of its own, so a guard
/// keyed on that alone does not fire on it. What such a copy does not have is
/// the artefacts this test actually reads.
fn explain(path: &Path, err: &std::io::Error) -> ! {
    let root = repo_root();
    let absent: Vec<&str> = ["Cargo.toml", "build-aux", "data"]
        .into_iter()
        .filter(|name| !root.join(name).exists())
        .collect();
    if !absent.is_empty() {
        panic!(
            "cannot read {}: {err}\n\
             This test was COMPILED in {}, which is not the GameHandler \
             checkout: {} is not there. So it is stale build output rather \
             than a missing file — a target/ directory copied from another \
             tree will do this, and so will building that tree with \
             CARGO_TARGET_DIR pointing at this one. Run `cargo clean -p \
             gamehandler` (or touch this file) to rebuild it here.",
            path.display(),
            root.display(),
            absent.join(", ")
        );
    }
    panic!(
        "cannot read {}: {err}\n\
         If this artefact was renamed, note that the path is derived from \
         gamehandler_core::APP_ID and the two must change together.",
        path.display()
    )
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| explain(path, &err))
}

/// The paths in a directory, sorted. The listing goes through [`explain`] too,
/// so a stale build root says so here as well rather than printing a bare
/// `No such file or directory` that reads as a defect in this test.
fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|err| explain(dir, &err))
        .map(|entry| entry.expect("readable directory entry").path())
        .collect();
    paths.sort();
    paths
}

/// The value of the one `Key=Value` line with this key.
///
/// The desktop entry format is a flat key-file, so this is how
/// `desktop-file-validate` reads it too. Insisting on exactly one match is
/// deliberate: a second `StartupWMClass` line would otherwise let a mismatched
/// first one hide behind a matching second.
fn unique_key_value(text: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    let values: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    assert_eq!(
        values.len(),
        1,
        "expected exactly one `{key}=` line, found {}",
        values.len()
    );
    values[0].to_string()
}

/// The desktop entry is the one artefact the shell reads. Its basename is the
/// id the compositor is told, and `StartupWMClass` is the value the shell
/// matches a window against.
#[test]
fn app_id_matches_the_desktop_entry() {
    let app_id = gamehandler_core::APP_ID;
    let data = repo_root().join("data");

    let mut entries: Vec<PathBuf> = list_dir(&data)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
        .collect();
    entries.sort();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one desktop entry under data/, found {entries:?}"
    );

    let entry = &entries[0];
    assert_eq!(
        entry.file_stem().expect("a .desktop filename").to_string_lossy(),
        app_id,
        "the desktop entry's basename must be the application id: a launcher \
         looks up {} for the id the window reports",
        entry.display()
    );

    let text = read(entry);
    assert_eq!(
        unique_key_value(&text, "StartupWMClass"),
        app_id,
        "StartupWMClass is what the shell matches the window against; the \
         window reports gamehandler_core::APP_ID (D-28)"
    );
}

/// The Flatpak manifest's `app-id` is the sandbox's identity. The key is
/// `app-id`, *not* `id` — the manifest is not an AppStream file, and the pair
/// looks interchangeable at a glance.
#[test]
fn app_id_matches_the_flatpak_manifest() {
    let app_id = gamehandler_core::APP_ID;
    let manifest = repo_root()
        .join("build-aux")
        .join("flatpak")
        .join(format!("{app_id}.json"));

    let text = read(&manifest);

    let key = "\"app-id\"";
    assert_eq!(
        text.matches(key).count(),
        1,
        "expected exactly one {key} key in the manifest: a second one, nested \
         or not, would mean this scan could read the wrong value"
    );

    // A targeted scan rather than a JSON parse, because this crate has no JSON
    // dependency and the check does not need one: with the count above pinned to
    // one, the only question left is what that line says. It fails loudly if the
    // manifest's formatting changes out from under it (value not found), which
    // is the safe direction for a check like this.
    let value = text
        .lines()
        .find_map(|line| line.trim().strip_prefix(key))
        .and_then(|rest| rest.trim().strip_prefix(':'))
        .map(|rest| rest.trim().trim_end_matches(',').trim())
        .and_then(|rest| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
        .expect("the manifest's app-id should be a quoted string on its own key");

    assert_eq!(
        value, app_id,
        "the Flatpak builds the sandbox under its own id; a mismatch here is \
         not caught by flatpak-builder, which never reads the Rust constant"
    );
}

/// The metainfo's `<launchable>` is what ties the AppStream component to the
/// desktop entry, so it carries the desktop id — `APP_ID.desktop`, not `APP_ID`.
#[test]
fn app_id_matches_the_metainfo_launchable() {
    let app_id = gamehandler_core::APP_ID;
    let metainfo = repo_root().join("data").join(format!("{app_id}.metainfo.xml"));

    let text = read(&metainfo);
    assert_eq!(
        text.matches("<launchable").count(),
        1,
        "expected exactly one <launchable> element"
    );

    let open = text.find("<launchable").expect("counted above");
    let body = &text[open..];
    let (tag, rest) = body
        .split_once('>')
        .expect("the <launchable> tag should be closed");
    assert!(
        tag.contains("type=\"desktop-id\""),
        "<launchable> must be a desktop-id for the shell to associate the \
         window with it, found: {tag}"
    );
    let value = rest
        .split_once("</launchable>")
        .expect("a closing </launchable> tag")
        .0
        .trim();

    assert_eq!(
        value,
        format!("{app_id}.desktop"),
        "appstreamcli validates that <launchable> ends in .desktop, but not \
         that the id before it is this application's"
    );
}
