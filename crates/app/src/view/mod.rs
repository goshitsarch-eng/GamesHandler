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
//! | **Test support** — compiled only under `cfg(test)` | `testkit` | no | no |
//!
//! The *test support* row is not a layer so much as the absence of one: `testkit`
//! is `#[cfg(test)]`, so it is not in the binary at all, and it is listed here
//! because the test below reads this table as the complete classification of
//! every declared module — a module in none of the rows is a module whose
//! contract nobody wrote down, which is what that test exists to catch. It
//! holds no app types: it lays an element out and reports what the framework
//! says about the tree, which is why it may read no `State` and emit no
//! `Message` and why it can be shared by a page module and a layout module
//! alike (`ARCH-14`).
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

/// The label-and-control row the Settings page and the game form both draw
/// (`UX-20`).
///
/// **This calls [`cosmic::widget::settings::item_row()`] rather than
/// reimplementing it.** The row it replaces was hand-written twice —
/// `view::settings`'s `row` and `view::form`'s `field_row`, with byte-identical
/// bodies — and the rationale recorded against replacing it argued about
/// *`iced`'s `form`*, which is a layout with its own opinion about where a label
/// goes. `item_row` is not that: it is a plain `Row` carrying the theme's own
/// spacing and centre alignment and leaving the label to its caller. So the
/// recorded objection was to a widget that was never the alternative here.
///
/// Passing [`cosmic::widget::text::body`] keeps the reference's label
/// typography, and keeps the label a real `Text` child, which is the property
/// `view::settings`'s
/// `the_close_on_launch_label_is_drawn_and_its_explanation_is_not_observable`
/// measures against — [`cosmic::widget::settings::item()`] would have used the
/// plain `text()` preset instead, which is why the helper is used at
/// `item_row`'s level rather than through `item`.
///
/// **The metrics are the same as the version this replaces, measured rather
/// than assumed.** `item_row` sets `spacing(theme::spacing().space_xs)`,
/// `align_y(Center)` and `width(Fill)` (`libcosmic
/// src/widget/settings/item.rs:52-58`), and `cosmic-theme`'s default gives
/// `space_xs: 12` (`src/model/spacing.rs:34`) — exactly the literal the
/// hand-rolled pair wrote. Adopting it is a change of *who owns the number*,
/// not of what the page looks like, which is the whole reason to use the
/// framework's helper for the job.
///
/// **Why this is here and not in either page.** The same reason [`GUTTER`] is:
/// it has two users, and it is a property of the layer rather than of a screen.
/// Written generic over `M` so that it stays on the pure side of the split the
/// module docs describe — it names no `Message` and reads no `State`.
pub fn settings_row<'a, M: 'static>(
    label: &'a str,
    control: cosmic::Element<'a, M>,
) -> cosmic::Element<'a, M> {
    use cosmic::widget::{settings, space, text};
    settings::item_row(vec![
        text::body(label).into(),
        space::horizontal().into(),
        control,
    ])
    .into()
}

/// The section heading the Settings page and the game form both draw (`UX-20`).
///
/// De-duplicated from two byte-identical private copies, but **deliberately not
/// replaced with [`cosmic::widget::settings::section()`]**, which is the other
/// half of `UX-20`'s recommendation. Two measured reasons, and the second is the
/// one that decides it:
///
/// * `Section::title` renders [`cosmic::widget::text::heading`], which is 14 px
///   bold (`libcosmic src/widget/text.rs:80-87`), where every heading in this app
///   is `title4`, 20 px bold (`:67-74`). Swapping them resizes every section
///   heading on two pages — a visible change, and a typography decision, not a
///   refactor.
/// * `Section` is not a heading widget. It is a `ListColumn` with an optional
///   header, so adopting it changes the *structure* of each page body: the rows
///   under a heading stop being siblings in the page column and become list
///   entries, which is what gives COSMIC Settings its separators. The reference
///   this port answers to draws section headings as `Kirigami.Separator`s with
///   no list grouping between the rows, so adopting `Section` would introduce a
///   structural difference from the reference to remove a duplication.
///
/// The heading *level* question is real and separate, and it is `UX-22`'s — a
/// row that asks for it can decide it there, once, for every heading in the app,
/// instead of as a side effect of this one.
pub fn settings_section<'a, M: 'static>(heading: &'a str) -> cosmic::Element<'a, M> {
    cosmic::widget::text::title4(heading).into()
}

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
#[cfg(test)]
pub mod testkit;
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
    /// `#[cfg(test)]`, so not in the binary — see the docs table above.
    const TEST_SUPPORT: &[&str] = &["testkit"];
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
            .chain(TEST_SUPPORT)
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

    /// The shared row is the framework's, and it renders what the duplicated
    /// pair rendered (`UX-20`).
    ///
    /// **Measured, not read off the source.** The finding was that
    /// `cosmic::widget::settings::` had *zero* call sites while two
    /// byte-identical private helpers did its job, so the thing worth pinning is
    /// that the shared helper produces the toolkit's row shape — not that some
    /// file contains the string `item_row`.
    ///
    /// Three properties are asserted, all of them `item_row`'s own
    /// (`libcosmic src/widget/settings/item.rs:50-58`):
    ///
    /// * the label reaches the traversal as its own `Text` child, which is what
    ///   makes `view::settings`'s drawn-strings assertions able to see the
    ///   reference's form labels, and which `settings::item` would have lost —
    ///   it uses the plain `text()` preset, so this is also why the helper is
    ///   built on `item_row` rather than on `item`;
    /// * the control sits **trailing** the label, which is what makes this a
    ///   settings row rather than a stack;
    /// * the label and the control are **vertically centred** on each other,
    ///   which is `item_row`'s `align_y(Alignment::Center)`.
    ///
    /// The last two are asserted against a `Column` holding the same three
    /// children as the sensitivity control: a `Column` puts the control below the
    /// label rather than to its right, so if these assertions could not tell the
    /// two apart they would be asserting nothing about which composition is
    /// drawn. That negative half is the point — the first draft of this test
    /// compared row *heights* and passed against a hand-rolled row with a
    /// deliberately wrong spacing, because a horizontal `Row`'s spacing does not
    /// change its height at all.
    #[test]
    fn the_shared_row_is_the_frameworks_row() {
        use cosmic::iced::Length;
        use cosmic::widget::{Column, Id, Space, container, text};

        fn laid_out<M: Clone + 'static>(
            el: &mut cosmic::Element<'_, M>,
        ) -> (Vec<String>, f32, f32, f32, f32) {
            let seen = super::testkit::traversal_at_width(el, 600.0);
            let strings = super::testkit::texts(&seen)
                .into_iter()
                .map(str::to_string)
                .collect();
            let label = seen
                .iter()
                .find(|s| s.text.as_deref() == Some("Label"))
                .expect("the traversal reported the label's bounds");
            // The control carries an `Id` because a bare `Space` reports no
            // bounds at all — it is neither a container nor a text — so a helper
            // that looked for it by size alone would find nothing and the test
            // below would fail for a reason unrelated to what it is checking.
            let control = seen
                .iter()
                .find(|s| s.id.as_ref() == Some(&Id::new("control")))
                .expect("the traversal reported the control's bounds");
            (
                strings,
                control.bounds.x,
                label.bounds.x,
                label.bounds.width,
                (control.bounds.center_y() - label.bounds.center_y()).abs(),
            )
        }

        let control = || {
            container(Space::new())
                .id(Id::new("control"))
                .width(Length::Fixed(40.0))
                .height(Length::Fixed(40.0))
        };
        let mut shared: cosmic::Element<'_, ()> = super::settings_row("Label", control().into());
        let (strings, control_x, label_x, label_w, centre_delta) = laid_out(&mut shared);

        assert!(
            strings.iter().any(|s| s == "Label"),
            "the shared row's label must reach the traversal as its own `Text` child, \
             which is what makes the settings page's drawn-strings assertions able to \
             see the reference's form labels. Reported: {strings:?}"
        );
        assert!(
            control_x >= label_x + label_w,
            "the control must sit trailing the label, not over it: control at {control_x}, \
             label at {label_x} with width {label_w}"
        );
        assert!(
            centre_delta < 1.0,
            "the label and the control must be vertically centred on each other \
             (`item_row`'s `align_y(Center)`); their centres differ by {centre_delta} px"
        );

        // Sensitivity: the same three children in a `Column` are stacked, not
        // laid out as a row, so the trailing assertion above must not hold for it.
        let mut stacked: cosmic::Element<'_, ()> = Column::new()
            .push(text::body("Label"))
            .push(control())
            .spacing(12)
            .align_x(cosmic::iced::Alignment::Center)
            .into();
        let (_, stacked_x, stacked_label_x, _, _) = laid_out(&mut stacked);
        assert!(
            stacked_x < stacked_label_x + 200.0,
            "a `Column` put the control at {stacked_x} against a label at {stacked_label_x}, \
             which the trailing assertion would also have accepted — this test cannot \
             distinguish a settings row from a stack and is therefore asserting nothing"
        );
    }

    /// The heading is deliberately *not* `settings::section`, and the reason is
    /// a measurement (`UX-20`).
    ///
    /// The row's recommendation was to replace both duplicated helpers with the
    /// toolkit's two. The row is right about the *rows* and this file follows it
    /// there; it would silently resize every section heading in the app if it
    /// were followed here. `Section::title` renders `text::heading` — 14 px — and
    /// every heading in this app is `title4`, 20 px
    /// (`libcosmic src/widget/text.rs:67-74` against `:80-87`).
    ///
    /// Pinned as a measurement so that adopting `settings::section` later is a
    /// decision someone makes on purpose: if the sizes are made to agree, this
    /// test fails and points at the row that should record it.
    #[test]
    fn the_shared_heading_is_title4_and_not_the_toolkits_section() {
        fn size_of(el: &mut cosmic::Element<'_, ()>) -> f32 {
            super::testkit::traversal(el)
                .first()
                .map(|seen| seen.bounds.height)
                .expect("the traversal reported the heading's own bounds")
        }

        let mut shared = super::settings_section("Behaviour");
        let mut toolkit = cosmic::widget::text::heading("Behaviour").into();
        let mut title4 = cosmic::widget::text::title4("Behaviour").into();

        assert_eq!(
            size_of(&mut shared),
            size_of(&mut title4),
            "the shared heading must render exactly as `title4` does"
        );
        assert!(
            size_of(&mut toolkit) < size_of(&mut title4),
            "`text::heading` and `text::title4` now measure the same, so \
             `settings::section`'s smaller header is no longer a reason to keep this \
             helper — `UX-22` owns that decision, and this is where it gets made"
        );
    }
}
