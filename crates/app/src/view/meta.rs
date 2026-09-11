//! The one line of text under a game's name, and the pieces it is built from.
//!
//! Ported from `Backend._game_row` (`bridge.py:305-327`), which is where the
//! subtitle a user actually reads is composed. It is a pure function of data
//! here rather than a `format!` at the call site for two reasons: the row and
//! the tile must not drift apart, and this is testable without a renderer.

use gamehandler_core::models::UNCATEGORIZED;

/// The runner label for a native Linux game.
///
/// `bridge.py:301-302`: a Linux game is not run by Wine, so naming a runner for
/// it would be wrong rather than merely redundant.
pub const LINUX_NATIVE: &str = "Linux native";

/// The separator between a category and a runner.
///
/// U+00B7 MIDDLE DOT with a space either side. Spelled as an escape so the
/// character is unambiguous in the source and in a test expectation.
pub const SEPARATOR: &str = " \u{b7} ";

/// What to call the thing that launches a game.
///
/// `manager_label` is the runner manager's own label for the configured runner
/// (`runners.py:825`, e.g. `"Proton-GE · Proton"`), resolved by the caller —
/// this function deliberately does not take a [`RunnerManager`] so that it stays
/// a pure function of data. When the game is a native Linux build the manager is
/// not consulted at all, which is why the argument is `&str` and not a closure.
///
/// [`RunnerManager`]: gamehandler_core::runners::RunnerManager
pub fn runner_label(is_linux: bool, manager_label: &str) -> String {
    if is_linux {
        LINUX_NATIVE.to_string()
    } else {
        manager_label.to_string()
    }
}

/// The subtitle: the category and the runner, with either half left out when
/// there is nothing to show rather than left as a dangling separator.
///
/// `display_category` is [`Game::display_category`], i.e. already folded — but
/// a blank is *also* treated as absent here. `bridge.py:323` compares against
/// the literal `"Uncategorized"` and so would render `" · Proton"` for a blank
/// it was handed directly; folding blanks too is the same answer for every
/// input the reference can actually produce, and one fewer way to get a stray
/// separator.
///
/// The same argument applies to an empty `runner_label`, and that branch is
/// **history rather than a live defect** — which is worth stating, because the
/// reason it was written is no longer true and a reader who checks the old
/// reason would conclude the branch is dead and delete it.
///
/// It was written as the fix for **#30**. At that point `subtitle_of`
/// (`widgets.rs`) passed the literal `""` as every game's runner label — the
/// view had no runner manager anywhere in it — so without the guard a Windows
/// game read `"Shooter · "`, a trailing separator with nothing after it:
/// precisely the bug the category branch above exists to prevent, one argument
/// over. `#30` was closed in `13e9806`, and the caller that could pass `""` is
/// gone. The label now arrives from
/// [`resolved_runner_label`](super::widgets::resolved_runner_label), which is
/// [`RunnerManager::label`], and that function cannot return an empty string
/// for **any** input: an empty or unknown `runner_id` folds to `"System Wine"`
/// (`runners/mod.rs:1099-1116`). So no game the application can draw reaches
/// this branch.
///
/// That last step is the one worth not taking on faith, since it says the
/// branch is dead — and it is a fact about the crate's call sites, not about
/// this file, so it is written here with the command that checks it. The only
/// two places that hand a label to a widget are `view/library.rs:343` and
/// `:360`, and each resolves it on the line before it uses it:
///
/// ```text
/// $ grep -rn 'resolved_runner_label(' crates/app/src/view/library.rs
/// 343:        let label = widgets::resolved_runner_label(runners, game);
/// 360:        let label = widgets::resolved_runner_label(runners, game);
/// ```
///
/// So the branch is unreachable *and stays anyway*: it is a property of this
/// function's contract, not a workaround for one caller's bug. `subtitle` is
/// public and pure over two
/// strings it does not validate, and the rule it states — an empty half is
/// omitted, never left as a dangling separator — is the rule the category
/// branch already implements. Guarding one half and not the other is an
/// accident of which bug was reported first, and the next caller to hand this
/// function a label from somewhere other than the manager should not have to
/// rediscover why the separator appears.
///
/// The test below therefore pins the **contract**, not a reproduction: it is
/// the reason the branch may not be deleted as unreachable code.
///
/// [`Game::display_category`]: gamehandler_core::models::Game::display_category
/// [`RunnerManager::label`]: gamehandler_core::runners::RunnerManager::label
pub fn subtitle(display_category: &str, runner_label: &str) -> String {
    let category = display_category.trim();
    let runner = runner_label.trim();

    let has_category = !category.is_empty() && category != UNCATEGORIZED;
    match (has_category, runner.is_empty()) {
        (false, _) => runner.to_string(),
        (true, true) => category.to_string(),
        (true, false) => format!("{category}{SEPARATOR}{runner}"),
    }
}

/// The line under a game's name as a user reads it: [`subtitle`], then the
/// last-played label. Parity items P-02 and P-16.
///
/// The reference splits this across two files, and the split is invisible in
/// the rendered string: `_game_row` (`bridge.py:305-327`) builds `subtitle`
/// from the category and the runner, and then `LibraryPage.qml:269` draws
///
/// ```text
/// text: row.modelData.subtitle + " · " + row.modelData.lastPlayed
/// ```
///
/// so the concatenation is the QML's, not Python's. Joining them here rather
/// than in the builder is what keeps the separator identical to [`SEPARATOR`]:
/// the QML writes its own `" · "` literal, and a port that copied that literal
/// into a widget would have two spellings of the same delimiter, one of which
/// the tests could not reach. It also puts the whole visible string in one
/// function, which is the only form an assertion can read without a renderer
/// (the wall #46, #51 and #57 all hit).
///
/// `last_played` is `format_last_played`'s result and **cannot be empty** for
/// any input — the zero timestamp is `"Never played"`, not `""`. It is folded
/// like the other two anyway, for the reason [`subtitle`] gives at length: this
/// function is total over its inputs, and "an empty part is omitted rather than
/// left as a dangling separator" is the rule, not a workaround for a caller.
/// Deleting that fold because `format_last_played` happens never to return
/// empty would make the rule true by accident of one function's range.
///
/// [`format_last_played`]: gamehandler_core::models::format_last_played
pub fn row_subtitle(display_category: &str, runner_label: &str, last_played: &str) -> String {
    let head = subtitle(display_category, runner_label);
    let played = last_played.trim();

    // The three-way shape is `subtitle`'s, one argument over: neither part,
    // one part, or both joined. `head` is empty when the category is absent —
    // blank, or `UNCATEGORIZED`, which [`Game::display_category`] folds a blank
    // into — *and* the runner label is empty as well. That is not reachable
    // through the application (`resolved_runner_label` cannot return empty),
    // but it is reachable from a library file whose entry stores `"runner": ""`
    // and no category, which `Library::load` accepts, so the branch is written
    // rather than assumed.
    //
    // [`Game::display_category`]: gamehandler_core::models::Game::display_category
    if played.is_empty() {
        head
    } else if head.is_empty() {
        played.to_string()
    } else {
        format!("{head}{SEPARATOR}{played}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A categorised game shows both halves, with the middle dot between them.
    /// The expected string is written out literally, so a change to
    /// [`SEPARATOR`] fails here rather than silently altering every subtitle.
    #[test]
    fn a_categorised_game_shows_the_category_then_the_runner() {
        assert_eq!(
            subtitle("Action", "Proton-GE"),
            "Action \u{b7} Proton-GE"
        );
    }

    /// An uncategorised game shows the runner alone. `UNCATEGORIZED` is the
    /// constant both Python (`models.py:59-62`) and the port fold blanks into,
    /// and it is imported rather than pasted so a rename cannot make this test
    /// agree with the implementation about the wrong string.
    #[test]
    fn an_uncategorised_game_shows_the_runner_alone() {
        assert_eq!(subtitle(UNCATEGORIZED, "Proton-GE"), "Proton-GE");
        assert!(
            !subtitle(UNCATEGORIZED, "Proton-GE").contains('\u{b7}'),
            "a separator with nothing before it is the bug this branch exists \
             to prevent"
        );
    }

    /// The blank case above, spelled out: no leading separator for a category
    /// that is empty or only whitespace.
    #[test]
    fn a_blank_category_shows_the_runner_alone() {
        assert_eq!(subtitle("", "Wine"), "Wine");
        assert_eq!(subtitle("   ", "Wine"), "Wine");
    }

    /// A category with surrounding whitespace is trimmed rather than shown with
    /// it, and is not mistaken for a blank.
    #[test]
    fn a_padded_category_is_trimmed_but_kept() {
        assert_eq!(subtitle("  RPG  ", "Wine"), "RPG \u{b7} Wine");
    }

    /// A native Linux game is never labelled with a runner, whatever the
    /// manager would have said.
    #[test]
    fn a_linux_game_is_labelled_native_and_the_manager_is_not_consulted() {
        assert_eq!(runner_label(true, "Proton-GE"), LINUX_NATIVE);
        assert_eq!(runner_label(true, ""), LINUX_NATIVE);
    }

    /// A Windows game gets the manager's label verbatim — including an empty
    /// one, which is the manager's business and not something to paper over
    /// with a placeholder here.
    #[test]
    fn a_windows_game_gets_the_managers_label_verbatim() {
        assert_eq!(runner_label(false, "Proton-GE \u{b7} Proton"), "Proton-GE \u{b7} Proton");
        assert_eq!(runner_label(false, ""), "");
    }

    /// The two pieces compose the way `_game_row` composes them: a categorised
    /// Linux game reads `"RPG · Linux native"`, which is the string a user sees
    /// and the one a screenshot would be compared against.
    #[test]
    fn the_pieces_compose_into_the_row_a_user_reads() {
        assert_eq!(
            subtitle("RPG", &runner_label(true, "ignored")),
            "RPG \u{b7} Linux native"
        );
        assert_eq!(
            subtitle(UNCATEGORIZED, &runner_label(false, "System Wine")),
            "System Wine"
        );
    }

    /// **The regression a test caught**, now guarding the contract rather than
    /// reproducing a live bug. When it was written this port passed `""` for
    /// every Windows game — there was no runner manager in the view at all — so
    /// `"Shooter · "` was what a row actually showed; `#30` fixed that caller
    /// (`13e9806`) and the label now arrives non-empty from
    /// `RunnerManager::label`, which folds an empty id to `"System Wine"`.
    ///
    /// This test is therefore what keeps the branch undeletable: it asserts the
    /// function's rule, which is the only thing still holding it up. See
    /// `subtitle`'s doc for that argument in full.
    ///
    /// The assertion checks the tail explicitly as well as the whole string,
    /// because "no trailing dot" is the claim and a `trim()` at the call site
    /// would satisfy the first assertion without satisfying this one.
    #[test]
    fn a_categorised_game_with_no_runner_label_has_no_trailing_separator() {
        assert_eq!(subtitle("Shooter", ""), "Shooter");
        assert!(
            !subtitle("Shooter", "").ends_with('\u{b7}'),
            "a dangling separator is the bug this branch exists to prevent"
        );
        assert_eq!(subtitle("Shooter", "   "), "Shooter");
    }

    // -----------------------------------------------------------------------
    // P-02 / P-16: the row's one line, last-played label included
    // -----------------------------------------------------------------------

    /// The job the application actually does: a categorised Windows game that
    /// has been played reads the three parts in the reference's order.
    ///
    /// `"Played 3 days ago"` is `format_last_played`'s own wording for that
    /// input, not a literal invented here — the string is checked against the
    /// real function in [`super::widgets`]' test and against CPython in
    /// `core`'s `format_last_played` corpus, so this test is about the
    /// *composition* and nothing else.
    #[test]
    fn a_played_categorised_game_shows_all_three_parts() {
        assert_eq!(
            row_subtitle("Shooter", "System Wine", "Played 3 days ago"),
            "Shooter \u{b7} System Wine \u{b7} Played 3 days ago"
        );
    }

    /// A game that has never been played still shows a last-played label, and
    /// it is `"Never played"` rather than an empty string.
    ///
    /// This is the case `format_last_played(0.0)` produces, and the reason the
    /// reference's row builder is unconditional: Python concatenates
    /// `+ " · " + lastPlayed` with no test, so the port must not invent a
    /// suppression the reference does not have.
    #[test]
    fn an_unplayed_game_shows_the_never_played_label_and_not_nothing() {
        assert_eq!(
            row_subtitle("Shooter", "System Wine", "Never played"),
            "Shooter \u{b7} System Wine \u{b7} Never played"
        );
    }

    /// An uncategorised game drops the category and keeps the other two, which
    /// is `subtitle`'s branch composed with the new part.
    #[test]
    fn an_uncategorised_game_shows_the_runner_and_the_timestamp() {
        assert_eq!(
            row_subtitle(UNCATEGORIZED, "System Wine", "Never played"),
            "System Wine \u{b7} Never played"
        );
    }

    /// **The property the acceptance asks for**: no stray separator, whichever
    /// part is missing.
    ///
    /// Every combination of present/absent is asserted, because "no dangling
    /// dot" is a claim about all of them and a test that checked only the one
    /// combination the application can currently produce would pass while the
    /// rule was broken for the rest.
    #[test]
    fn no_combination_of_missing_parts_leaves_a_dangling_separator() {
        let cases = [
            ("Shooter", "System Wine", "Never played"),
            ("Shooter", "System Wine", ""),
            ("Shooter", "", "Never played"),
            ("Shooter", "", ""),
            (UNCATEGORIZED, "System Wine", "Never played"),
            (UNCATEGORIZED, "", "Never played"),
            ("", "", "Never played"),
            ("", "", ""),
        ];
        for (category, runner, played) in cases {
            let rendered = row_subtitle(category, runner, played);
            assert!(
                !rendered.starts_with('\u{b7}') && !rendered.ends_with('\u{b7}'),
                "row_subtitle({category:?}, {runner:?}, {played:?}) = {rendered:?} \
                 has a dangling separator"
            );
            assert!(
                !rendered.contains("\u{b7} \u{b7}") && !rendered.contains("  "),
                "row_subtitle({category:?}, {runner:?}, {played:?}) = {rendered:?} \
                 has an empty part between two separators"
            );
        }
        // And the all-absent case is the empty string, not a bare separator —
        // the one input where "omit empty parts" could degenerate to `" · "`.
        assert_eq!(row_subtitle("", "", ""), "");
    }

    /// The separator this module joins with is the same character the
    /// reference's QML uses, read out of the QML rather than restated.
    ///
    /// `LibraryPage.qml:269` is the whole reason this function exists — it
    /// concatenates the two halves the port joins here — and it writes its own
    /// `" · "` literal. If the QML's delimiter and [`SEPARATOR`] disagree, the
    /// port and the reference render different strings and no unit test can see
    /// it, because both sides are self-consistent.
    ///
    /// This is the anti-rot pattern the icon transcription and the nav-label
    /// test already use: read the reference at test time instead of trusting a
    /// hand-copied constant. `KNOWN_`-style records are deliberately not used —
    /// there is no divergence to defer here, only one to prevent.
    #[test]
    fn the_separator_is_the_one_the_reference_qml_writes() {
        let qml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../gamehandler/qml/LibraryPage.qml"
        ))
        .expect("LibraryPage.qml is part of the reference and must be readable");

        // The line that concatenates them. Asserted to exist rather than
        // silently skipped: a `find` that returned `None` would make the
        // separator check below vacuous.
        let line = qml
            .lines()
            .find(|line| line.contains("modelData.subtitle +") && line.contains("lastPlayed"))
            .expect(
                "LibraryPage.qml no longer concatenates subtitle and lastPlayed — \
                 if the reference changed, this port's row must change with it",
            );

        assert!(
            line.contains(&format!("subtitle + \"{SEPARATOR}\" +")),
            "the QML joins the subtitle and the last-played label with a \
             different separator than SEPARATOR ({SEPARATOR:?}); the reference \
             line is: {}",
            line.trim()
        );
    }
}
