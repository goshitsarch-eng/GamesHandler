//! The interface: the decisions a screen makes, and the widgets that draw them.
//!
//! # What this module is allowed to know
//!
//! **This section used to claim something false**, and it is worth saying so
//! rather than quietly replacing the sentence: it said "Nothing here reads
//! [`State`], and nothing here emits a [`Message`]", and that every widget
//! function is generic over the message type. Seven of the fifteen modules below
//! do both, and one spawns an operating-system process. A contract stated in the
//! file that defines the layer boundary, and contradicted by the two largest
//! files under it, teaches a reader to distrust the boundary — which is
//! `ARCHITECTURE.md` `ARCH-02`, and this is its fix.
//!
//! What is actually here is **two layers sharing one module tree**, and they are
//! distinguished by what they are allowed to touch:
//!
//! | Kind | Modules | May read `State`? | May emit `Message`? |
//! |---|---|---|---|
//! | **Pure layout** — widget builders generic over `M` | [`a11y`], [`badge`], [`widgets`] | no | no |
//! | **Pure decisions** — data in, data out | [`cover`], [`cover_cache`], [`meta`], [`metrics`] | no | no |
//! | **Page modules** — bound to this app's state and messages | [`credits`], [`form`], [`installers`], [`library`], [`plugins`], [`runners`], [`settings`] | yes | yes |
//!
//! The page modules are the imperative shell for their screen: they own an
//! `update`, they take `&State` or `&mut State`, and [`plugins`] runs a real
//! `std::process::Command` for the package-manager install. That last one is a
//! deliberate seam rather than a leak — `core` describes the command as *data*
//! ([`plugins::install_command`], [`plugins::privileged_command`]) and this layer
//! runs the blocking work off the UI thread, exactly as [`runners`] runs its
//! downloads. The [module docs of `plugins`](plugins) carry that argument.
//!
//! `the_layer_split_the_docs_describe_is_the_split_the_code_has` below reads this
//! table back out of the source and fails if the two disagree, because the
//! previous revision's problem was not that it was wrong — it was that being
//! wrong cost nothing.
//!
//! # The split that *is* worth keeping, and why
//!
//! The **decisions** are pure functions over data in the modules the table calls
//! *pure decisions*, and the widget builder that calls one is thin:
//!
//! | Decision | Module |
//! |---|---|
//! | which cover to draw, and how it fits | [`cover`] |
//! | the text under a game's name | [`meta`] |
//! | how big a tile and its cover are | [`metrics`] |
//!
//! That is deliberate, and it is the project's own rule rather than a
//! preference: `main.rs` records that "every non-trivial decision lives in
//! `gamehandler-core`, where it is tested without a display". These functions
//! are view-shaped so they cannot live there, but they are written to the same
//! standard — no renderer, no display, no fixture that can only be built by
//! running the app. The page modules follow the same rule one level up: the
//! decisions *they* make are free functions a test can call without a window,
//! which is why `view::plugins`'s install-success rule and `view::runners`'s
//! fetch-error rendering are both tested directly.
//!
//! # What the tests here are for
//!
//! The tests assert the *data* — the chosen cover source, the subtitle string,
//! the arithmetic — not the widget tree. A test that rendered a widget and
//! looked for text would need a display, a font stack and an event loop, and
//! would still be asserting the same strings.
//!
//! [`State`]: crate::state::State
//! [`Message`]: crate::Message
//! [`plugins::install_command`]: gamehandler_core::plugins::install_command
//! [`plugins::privileged_command`]: gamehandler_core::plugins::privileged_command

/// The gutter every top-level view pads its body by — UX-10.
///
/// Five of the seven views ended in `container(scrollable(body)).padding(18)`
/// and two — [`installers`] and [`runners`] — ended in a bare
/// `scrollable(body)`, so their text ran flush against the window edge and,
/// with the nav bar condensed (every window under
/// `Core::is_condensed_update`'s 648 px), flush against the hamburger.
///
/// The value is the five existing sites' own, which is why it is `18` and not a
/// number this port chose: `view::installers`'s
/// `the_page_pads_its_body_by_the_same_gutter_as_the_other_views` and
/// `view::runners`'s test of the same name both assert their whole page is drawn
/// inside it, so the five that had it are the control the two that did not are
/// measured against. It lives here rather than in either page because it has two
/// users and is a property of the layer, not of a screen.
pub const GUTTER: u16 = 18;

pub mod a11y;
pub mod badge;
pub mod cover;
pub mod cover_cache;
pub mod credits;
pub mod form;
pub mod installers;
pub mod library;
pub mod meta;
pub mod metrics;
pub mod plugins;
pub mod runners;
pub mod settings;
pub mod widgets;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// The whole `view/` tree, read at test time.
    ///
    /// `CARGO_MANIFEST_DIR` is `crates/app`, so this file sits at
    /// `<manifest>/src/view/mod.rs` beside the modules it declares.
    fn sources() -> Vec<(String, String)> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("view");
        let mut files: Vec<(String, String)> = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", directory.display()))
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension()?.to_str()? != "rs" {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let text = std::fs::read_to_string(&path).ok()?;
                Some((name, text))
            })
            .collect();
        files.sort();
        assert!(
            files.len() > 10,
            "read only {} files from {}; this test would pass vacuously",
            files.len(),
            directory.display()
        );
        files
    }

    /// Every `pub mod NAME;` this file declares.
    ///
    /// The declaration list is the input, not the directory listing: a module
    /// on disk that this file does not declare is not part of the layer, and a
    /// test that walked the directory would grade files the compiler ignores.
    fn declared_modules() -> BTreeSet<String> {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("view")
                .join("mod.rs"),
        )
        .expect("view/mod.rs must be readable");
        let declared: BTreeSet<String> = text
            .lines()
            .filter_map(|line| line.strip_prefix("pub mod "))
            .map(|line| line.trim_end_matches(';').trim().to_string())
            .collect();
        assert!(
            declared.len() > 10,
            "parsed only {} modules out of view/mod.rs; the parse is broken, not the file",
            declared.len()
        );
        declared
    }

    /// Whether a module reads this app's state or emits its messages.
    ///
    /// Both spellings matter: `&State` in a signature, and the `use
    /// crate::Message` that a module needs to name a variant. A module that
    /// only *mentions* either in a doc comment reads as bound, which is the safe
    /// direction for a check whose purpose is to catch an undeclared reading.
    fn reads_state_or_emits_messages(text: &str) -> bool {
        // The first version of this matched the literal `&State` and nothing
        // else, and a control that added `fn f(state: &crate::state::State)`
        // to a *pure* module passed the check — `&crate::state::State` does not
        // contain the substring `&State`. That is the defect this file is about,
        // in the check written to catch it, so the fully-qualified spellings are
        // matched too.
        text.contains("&State")
            || text.contains("&mut State")
            || text.contains("::state::State")
            || text.contains("use crate::Message")
            || text.contains("crate::Message::")
    }

    /// The doc comment's table, as a classification of each module.
    ///
    /// The three lists are the contract, written here so a change to the code
    /// that the docs do not describe fails a test rather than a review.
    const PURE_LAYOUT: &[&str] = &["a11y", "badge", "widgets"];
    const PURE_DECISIONS: &[&str] = &["cover", "cover_cache", "meta", "metrics"];
    const PAGE_MODULES: &[&str] = &[
        "credits",
        "form",
        "installers",
        "library",
        "plugins",
        "runners",
        "settings",
    ];

    #[test]
    fn the_layer_split_the_docs_describe_is_the_split_the_code_has() {
        // `ARCHITECTURE.md` `ARCH-02`: `view/mod.rs` claimed nothing in the
        // layer reads `State` or emits a `Message`, and seven modules did both.
        // The claim was wrong for a long time and nothing noticed, because prose
        // that nothing reads back is prose that cannot be wrong. This is the
        // reading-back.
        let sources = sources();
        let by_name: std::collections::BTreeMap<&str, &str> = sources
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();

        // Every declared module is classified exactly once. A new module that
        // appears in none of the three lists is the failure this catches: it
        // would be a module whose contract nobody wrote down.
        let declared = declared_modules();
        let classified: BTreeSet<String> = PURE_LAYOUT
            .iter()
            .chain(PURE_DECISIONS)
            .chain(PAGE_MODULES)
            .map(|name| (*name).to_string())
            .collect();
        let unclassified: Vec<&String> = declared.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "these modules are declared by view/mod.rs and classified by neither the \
             pure-layout, pure-decision nor page-module list, so the contract in the \
             module docs does not say what they may touch: {unclassified:?}. Add each \
             to the right list in `view/mod.rs`'s docs and to the constant here."
        );
        let missing: Vec<&String> = classified.difference(&declared).collect();
        assert!(
            missing.is_empty(),
            "these modules are classified here and not declared by view/mod.rs: {missing:?}"
        );

        // The pure two may not read state or emit messages — that is the whole
        // of what "pure" means, and it is the half of the old claim that was
        // true and is worth keeping true.
        for name in PURE_LAYOUT.iter().chain(PURE_DECISIONS) {
            let text = by_name
                .get(name)
                .unwrap_or_else(|| panic!("{name} is classified but has no file under view/"));
            assert!(
                !reads_state_or_emits_messages(text),
                "`view::{name}` is listed as pure in view/mod.rs's contract, but it reads \
                 `State` or emits a `Message`. Either the module moved a layer — in which \
                 case move it to the page-module list and say so — or the reading is \
                 unintended."
            );
        }

        // And the page modules really are bound to this app, so the list cannot
        // be padded to make the check above pass trivially by reclassifying
        // everything as pure.
        for name in PAGE_MODULES {
            let text = by_name
                .get(name)
                .unwrap_or_else(|| panic!("{name} is classified but has no file under view/"));
            assert!(
                reads_state_or_emits_messages(text),
                "`view::{name}` is listed as a page module, but it neither reads `State` nor \
                 emits a `Message`. If it became pure, move it to a pure list rather than \
                 leaving the contract describing something the code no longer is."
            );
        }
    }
}
