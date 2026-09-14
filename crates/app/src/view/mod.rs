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

/// The gutter every top-level view pads its body by — UX-10, and the theme's
/// own token since UX-23.
///
/// All seven views end in `container(scrollable(body)).padding(gutter())`. Two
/// of them — [`installers`] and [`runners`] — used to end in a bare
/// `scrollable(body)`, so their text ran flush against the window edge and,
/// with the nav bar condensed (every window under
/// `Core::is_condensed_update`'s 648 px), flush against the hamburger; UX-10
/// fixed that by giving those two the same gutter the other five had. UX-23 is
/// the second half: the five spelled it as the literal `18`, so the app's
/// outermost rhythm was a number the toolkit knew nothing about.
///
/// # Why this is a function and no longer a `const`
///
/// **`18` is not a COSMIC spacing value at any density, and that is the whole
/// finding.** `cosmic-theme`'s token set is `space_xxxs 4`, `space_xxs 8`,
/// `space_xs 12`, `space_s 16`, `space_m 24`, `space_l 32`, `space_xl 48`
/// (`src/model/spacing.rs:32-38`), and the two other densities scale it —
/// `Compact` gives `8, 4, 8, 8, 16, 24, 32` and `Spacious` `8, 12, 16, 24, 32,
/// 48, 64` (`:61-81`). There is no density at which `18` is a token, so a
/// hardcoded gutter cannot follow the user's density setting at all: a
/// `Compact` user gets a page whose margin is 18 while the `settings::item_row`
/// inside it spaces by 8, and a `Spacious` one gets 18 around content spaced by
/// 24 — the page margin reads as the tighter of the two, which is backwards.
///
/// **`space_s` is the token adopted, and it is the nearest by measurement.**
/// Against the literal it replaces, `|18 − 16| = 2` for `space_s` against
/// `|18 − 12| = 6` for `space_xs` and `|18 − 24| = 6` for `space_m`, and
/// `space_s` is what UX-23's own recommendation names. **What that costs, stated
/// rather than hidden: the gutter moves from 18 to 16 at the default density,
/// which is 2 px per side — a 4 px wider content band at every window width.**
/// That is a visual change and it is measured below, not asserted to look
/// better: there is no display in this environment, so "correct" here means
/// *off-token at every density before, nearest token and density-responsive
/// after*, and not "checked by eye". The `Compact`/`Spacious` responsiveness
/// this buys is untestable from here for [`crate::theme::apply`]'s reason —
/// `cosmic::theme::spacing()` reads a global that libcosmic exposes no writer
/// for (`src/theme/mod.rs:47` is `pub(crate)`), so a test cannot install a
/// density and watch the layout follow. It can only read the active token,
/// which is what the two page tests do.
///
/// # Why `metrics::GRID_UNIT` is still 18 and this is not
///
/// The two numbers agreeing was a coincidence, and the change separates them
/// deliberately. [`crate::view::metrics::GRID_UNIT`] is an *estimate of a
/// font-relative length* — Kirigami's `gridUnit`, which the QML's card sizes are
/// expressions over (`gridUnit * 3.4` and so on) — and it is tied to the 2:3
/// aspect the store art is authored for. It is geometry, not rhythm, and COSMIC
/// has no font-relative unit to translate it to; that module's own doc says so.
/// This is rhythm, and the toolkit has tokens for it.
pub fn gutter() -> u16 {
    cosmic::theme::spacing().space_s
}

/// The widest a page's content column is allowed to grow, in logical pixels —
/// UX-25.
///
/// Past this width a settings row stops reading as a pair ("label … far-away
/// control") and prose runs to a measure nobody can read, which is the failure
/// the row names at 2560 px. The number is the row's own: the reference bounds
/// nothing (its rows are `Layout.fillWidth` on the same stretch), so there is
/// no QML value to port and any figure here is a port decision — 1100 is wide
/// enough that the installer and runner cards keep their shape and narrow
/// enough that the failure mode is gone.
///
/// The Library page is deliberately not wrapped: its grid's column count *is*
/// the window width, so bounding it would take columns away on a wide display
/// — a divergence from the reference's `GridView`, which fills. Its list rows
/// stretch in both implementations.
pub const MAX_CONTENT_WIDTH: f32 = 1100.0;

/// `body`, centred and capped at [`MAX_CONTENT_WIDTH`].
///
/// The cap needs two containers because they are different jobs: the inner one
/// is the thing that is bounded (its `max_width` is what the scrollable's
/// viewport cannot grow past), and the outer one is the thing that is wide —
/// `center_x(Fill)` takes the viewport's width and centres the bounded box
/// inside it. A single `container(body).max_width(..)` alone resolves to the
/// bound but stays left-aligned, which reads as a bug at exactly the window
/// sizes the bound exists for.
///
/// Applied inside the page's scrollable — `scrollable(bounded_body(body))` —
/// so the scrollbars and the viewport still see the real content height while
/// the column itself stays narrow.
pub fn bounded_body<'a, M: 'static>(
    body: impl Into<cosmic::Element<'a, M>>,
) -> cosmic::Element<'a, M> {
    cosmic::widget::container(cosmic::widget::container(body).max_width(MAX_CONTENT_WIDTH))
        .center_x(cosmic::iced::Length::Fill)
        .into()
}

/// Which progress indicator a page draws — the UX-27 decision, shared so the
/// two pages that carry a bar cannot disagree about it.
///
/// The reference's bar is `visible: busy && progress >= 0`, which leaves the
/// phases with no byte fraction — extracting, the vendor wizard, the settle
/// poll, the releases fetch — showing nothing at all, or a bar frozen at its
/// last tick. The port's workers now send `-1.0` at those boundaries (the
/// reference's own "nothing to show" sentinel), which
/// `view::runners::progress_fraction` already filters to `None`: so `None`
/// *while a job is running* is exactly "working, unmeasurable", and it draws
/// [`cosmic::widget::progress_bar::indeterminate_linear`] rather than nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressCue {
    /// No bar. Idle, or a job that has nothing to report yet and is not
    /// expected to be long — the fetch path's callers pass `working` for it.
    Hidden,
    /// `determinate_linear(fraction)` — a byte-measurable phase is running.
    Determinate(f32),
    /// `indeterminate_linear()` — work is in flight with no fraction to show.
    Indeterminate,
}

/// [`ProgressCue`] from the two inputs every caller already holds.
///
/// `fraction` is `progress_fraction`'s filtered answer — `Some` means a real
/// measurement — and `working` is whatever the page's in-flight state is:
/// `state.busy()` on the Installers page, `busy || releases_status == Loading`
/// on Runners. A fraction wins over the indeterminate cue unconditionally:
/// `Some` already implies `busy`, so the case split has no third truth.
pub fn progress_cue(fraction: Option<f32>, working: bool) -> ProgressCue {
    match fraction {
        Some(fraction) => ProgressCue::Determinate(fraction),
        None if working => ProgressCue::Indeterminate,
        None => ProgressCue::Hidden,
    }
}

/// The `Id` the game-removal prompt's **Cancel** button carries, and the control
/// the arm that opens it moves the keyboard to — UX-24.
///
/// # Why the port has to name this control at all
///
/// The reference's prompt distinguishes its two actions by *kind*: Cancel is a
/// `standardButton` and the destructive Remove is a `customFooterAction`
/// (`gamehandler/qml/LibraryPage.qml:349` and `:350-359`, identically at
/// `RunnersPage.qml:260` and `:261-270`). libcosmic's `dialog()` keeps that
/// distinction — `secondary_action` and `primary_action` — but gives neither
/// one focus. Measured on the pinned revision: `src/widget/dialog.rs` mentions
/// `focus` nowhere, and the two buttons it is handed are built with the default
/// `Id::unique()` (`a401af8 src/widget/button/widget.rs:63`, `:88`). So nothing
/// in the toolkit decides which of the two the keyboard lands on.
///
/// In this port that leaves the keyboard where it was: the page stays in the
/// tree under the layer, so the Delete control that raised the prompt is *still
/// a registered focusable* and keeps the focus it had. The user's next Tab then
/// walks the rest of the page before reaching the two buttons they are being
/// asked about, and the destructive one is indistinguishable from the safe one
/// at the moment of decision. `Message::OpenGameMenu` answers exactly this for
/// the actions layer (**UX-16**, `crates/app/src/main.rs:2196-2207`); the two
/// destructive prompts are the same problem with a worse failure mode, so the
/// control named here is the **secondary** action and never the destructive
/// one.
///
/// # Why there are two of these and not one
///
/// A single shared id would read better — only one prompt is normally open —
/// but it cannot be shown not to collide: `view_with_overlays`'s own ordering
/// backstop names the state in which `confirm_delete` and
/// `confirm_remove_runner` are both pending as reachable, and in that tree one
/// shared id would name two widgets. One constant per prompt costs a line and
/// cannot be made to collide.
pub const REMOVE_GAME_CANCEL_ID: &str = "gamehandler.dialog.remove-game.cancel";

/// [`REMOVE_GAME_CANCEL_ID`]'s counterpart on the runner-removal prompt.
///
/// The same argument, the same failure mode, a different dialog — see that
/// constant's header rather than a second copy of it.
pub const REMOVE_RUNNER_CANCEL_ID: &str = "gamehandler.dialog.remove-runner.cancel";

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
/// **Why this is here and not in either page.** The same reason [`gutter`] is:
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
/// The heading *level* question was `UX-22`'s, and its answer is why this
/// helper is `title4` while every *reference-attested* section heading is
/// `title3`. The reference draws a `Kirigami.Heading level: 3` for each of
/// its section headings (Runners ×4, Installers "Catalog", Plugins "Host
/// plugins", Credits ×3) — those are `text::title3` in this port. On the two
/// pages this helper serves, the reference draws `Kirigami.Separator`s with
/// `FormData.isSection` — a rule, not a heading — so the label is this port's
/// own device and is held one rung below the headings the reference actually
/// draws, where it cannot be mistaken for one. The same level split applies
/// to the placeholder titles: the reference's `PlaceholderMessage` titles are
/// `title3` here (Library, Installers, Plugins), one level for one job.
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

    /// **All seven pages pad their body by [`gutter`], and none by a literal** —
    /// the layer half of UX-23.
    ///
    /// # Why this exists beside the two edge tests
    ///
    /// `view::installers` and `view::runners` each carry a test that lays the
    /// page out at a 420 px window and asserts the leftmost and rightmost
    /// published nodes sit at `cosmic::theme::spacing().space_s`. Those two are
    /// the **strong** half: they read the built tree, so they pin the token's
    /// *value* as well as the padding, and a `gutter` reverted to the literal
    /// fails them with `leftmost node at x = 18, expected 16`.
    ///
    /// They cover two of the seven pages. The other five — [`library`],
    /// [`settings`], [`credits`], [`plugins`], [`form`] — had their literal
    /// replaced by the same mechanical edit and had **nothing** that would
    /// notice it being put back: this test is that notice. It is deliberately
    /// the weaker instrument and is described as such — see the bound below.
    ///
    /// # The bound, stated rather than left for a reader to find
    ///
    /// This is a **source** check, and it matches text line by line. Comments
    /// are cut first, which the first draft of this test did not do — it failed
    /// on correct code, because *this very fix's* prose quotes the literal it
    /// removed (`"Five of the seven views ended in
    /// `container(scrollable(body)).padding(18)`"`). That is the same
    /// "a pointer that no longer lands is worse than no pointer" trap
    /// `ARCH-16` names, one level down: a guard that fires on the documentation
    /// of its own fix is a guard someone deletes. So [`code_lines`] drops
    /// `//`-prefixed lines, and both spelling lists below are matched against
    /// code only.
    ///
    /// What survives that cut is still text, not parsed Rust. `padding(18)` in
    /// a string literal would satisfy the negative half's match, and the
    /// positive half requires only the gutter's name as the argument of a
    /// `padding(`, which a code-shaped line that is not a call could also
    /// satisfy. What neither half can be fooled by is the *regression it is
    /// here for*: restoring `.padding(18)` is an edit to a code line, and that
    /// fails the first assertion. `wiring_claims.rs` states its equivalent
    /// bound in the same terms.
    ///
    /// The positive half is asserted per page rather than layer-wide so that a
    /// page losing its padding outright fails here too, and not only in the two
    /// edge tests that would not cover it.
    #[test]
    fn every_page_pads_its_body_by_the_theme_gutter_and_not_a_literal() {
        let sources = sources();

        /// The lines of a file that are code, not prose about code.
        ///
        /// A trimmed line starting `//` is a comment — `//`, `///` and `//!`
        /// alike — and the layer's doc comments quote both the old literal and
        /// the new call, so leaving them in would make both halves of the
        /// assertion below fire on documentation.
        fn code_lines(text: &str) -> String {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<&str>>()
                .join("\n")
        }

        // The literal UX-23 replaced, in the three spellings the tree had. Each
        // is matched as a whole call, so the number `18` stays legal elsewhere
        // in this layer (`metrics::GRID_UNIT`, `metrics::ICON_INSET`, the card
        // sizes) and this does not fail on correct code.
        //
        // Scoped to the page modules, which is the claim — *the pages* pad by
        // the gutter. The layer as a whole legitimately pads other things by
        // literals, and this test's own spelling list is a code line in this
        // file: run layer-wide, the guard would match the strings it is made of
        // and fail on itself.
        for spelling in [
            ".padding(18)",
            ".padding(GUTTER)",
            ".padding(super::GUTTER)",
        ] {
            let offenders: Vec<&str> = sources
                .iter()
                .filter(|(name, _)| PAGE_MODULES.contains(&name.as_str()))
                .filter(|(_, text)| code_lines(text).contains(spelling))
                .map(|(name, _)| name.as_str())
                .collect();
            assert!(
                offenders.is_empty(),
                "these pages still pad by the literal {spelling:?}, so their \
                 gutter cannot follow the user's density setting: {offenders:?}. \
                 It is `gutter()` now — see `view::gutter`'s header for why the \
                 token is `space_s` and what the 2 px costs."
            );
        }

        // And every page's body padding names the gutter. `PAGE_MODULES` is the
        // same list the layer-split test classifies with, so a page added to the
        // layer is covered here with no edit, and a page that loses its padding
        // is a failure rather than a smaller file.
        for page in PAGE_MODULES {
            let text = sources
                .iter()
                .find(|(name, _)| name == page)
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "`{page}` is in `PAGE_MODULES` and not on disk; the two \
                         lists have come apart"
                    )
                });
            let code = code_lines(text);
            assert!(
                code.contains("padding(gutter())") || code.contains("padding(super::gutter())"),
                "`view::{page}` no longer pads anything by `gutter()`. Every \
                 top-level page pads its body by it — UX-10 for the padding \
                 existing at all, UX-23 for it being the theme's token."
            );
        }

        // This test is worth nothing against an empty or partial read.
        assert!(
            sources.len() > 10,
            "read only {} files from the view tree; the assertions above would \
             pass vacuously",
            sources.len()
        );
    }

    /// **The six content pages bound and centre their body column; the
    /// Library does not** — UX-25.
    ///
    /// Two halves, the same split the gutter guard above uses. The measured
    /// half lays the Credits page out at a 2560 px window — the row's own
    /// figure — and requires the drawn text to sit inside a centred
    /// [`MAX_CONTENT_WIDTH`] column: a page that dropped the bound fails the
    /// width half, and one that kept the bound but lost the centring fails
    /// the edge half (left-aligned bounded content starts at the gutter,
    /// 16 px, not ~730). Credits is the instrument because its page takes no
    /// state — any of the six would measure the same wrapper.
    ///
    /// The scan half is the weaker instrument, stated as such: it requires the
    /// *call* in each page's source, which is what a reverted edit looks like
    /// — a page that calls `bounded_body` and then lays its body out wrongly
    /// is a failure this test cannot see and the measured half is for.
    /// `library` is excluded on purpose: its grid's column count is the window
    /// width, and bounding it would diverge from the reference's `GridView`,
    /// which fills — see [`MAX_CONTENT_WIDTH`]'s doc.
    #[test]
    fn the_content_pages_bound_and_centre_their_body_and_the_grid_does_not() {
        // Measured: Credits at the row's own 2560 px.
        let mut page: cosmic::Element<'_, crate::Message> =
            super::credits::view(super::credits::CreditsPage);
        let seen = super::testkit::traversal_at_width(&mut page, 2560.0);
        let (left, right) = seen.iter().filter(|node| node.text.is_some()).fold(
            (f32::INFINITY, 0.0f32),
            |(l, r), node| {
                (
                    l.min(node.bounds.x),
                    r.max(node.bounds.x + node.bounds.width),
                )
            },
        );
        assert!(left.is_finite(), "the Credits page draws no text at all");
        let cap = super::MAX_CONTENT_WIDTH;
        assert!(
            right - left <= cap + 1.0,
            "the content column spans {} px at a 2560 px window — the \
             {cap} px bound is not applied",
            right - left
        );
        assert!(
            left > 100.0,
            "the content column starts at x={left}: the bound is applied but \
             the column is not centred in the viewport"
        );

        // Scanned: every content page routes its body through the wrapper.
        let sources = sources();
        for page in [
            "credits",
            "form",
            "installers",
            "plugins",
            "runners",
            "settings",
        ] {
            let text = sources
                .iter()
                .find(|(name, _)| name == page)
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("`{page}` is not on disk"));
            let code: String = text
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<&str>>()
                .join("\n");
            assert!(
                code.contains("bounded_body(body)"),
                "`view::{page}` no longer bounds its body column — \
                 `scrollable(bounded_body(body))` is the call, and `MAX_CONTENT_WIDTH`'s \
                 doc records why the Library is the one page that must not have it"
            );
        }
        let library = sources
            .iter()
            .find(|(name, _)| name == "library")
            .map(|(_, text)| text.as_str())
            .expect("view::library is on disk");
        let library_code: String = library
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<&str>>()
            .join("\n");
        assert!(
            !library_code.contains("bounded_body("),
            "the Library's grid must keep the window's width — bounding it \
             takes columns away on a wide display, which the reference's \
             GridView does not do"
        );
    }

    /// UX-27: the three shapes a progress report can take, decided in one
    /// place so the two pages that draw a bar cannot disagree.
    ///
    /// The cases that matter are the ambiguous ones: `None` while a job runs
    /// is *work with no fraction* — the worker's `-1.0` marker, filtered by
    /// `progress_fraction` — and must draw the indeterminate bar rather than
    /// nothing (the finding) or a frozen bar (a last-tick replay). And a
    /// fraction wins unconditionally, because `Some` already implies busy.
    #[test]
    fn the_cue_is_determinate_indeterminate_or_hidden() {
        assert_eq!(
            super::progress_cue(Some(0.4), true),
            super::ProgressCue::Determinate(0.4)
        );
        // A stray fraction with no job still draws — the callers' `fraction`
        // input is `progress_fraction`'s, which already returns `None` when
        // nothing runs, so `Some` here is the worker's word and it is taken.
        assert_eq!(
            super::progress_cue(Some(0.4), false),
            super::ProgressCue::Determinate(0.4)
        );
        assert_eq!(
            super::progress_cue(None, true),
            super::ProgressCue::Indeterminate,
            "busy with no fraction must not render as *nothing* — that is the \
             bug UX-27 exists to fix"
        );
        assert_eq!(super::progress_cue(None, false), super::ProgressCue::Hidden);
    }
}
