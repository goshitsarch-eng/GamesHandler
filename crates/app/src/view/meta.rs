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
/// The same argument applies to an empty `runner_label`, and it is the case
/// that actually bites this port: `bridge.py` resolves the runner label before
/// it gets here and so can never pass an empty one, whereas the Rust view has
/// no runner manager in its contract yet (T-07) and passes `""`. Without the
/// guard a Windows game reads `"Shooter · "` — a trailing separator with
/// nothing after it, which is precisely the bug the category branch above
/// exists to prevent, one argument over. Found by a test, not by reading.
///
/// [`Game::display_category`]: gamehandler_core::models::Game::display_category
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

    /// **The regression a test caught.** A categorised game with no runner
    /// label must not render a trailing separator: this port passes `""` for
    /// every Windows game until the runner manager is in the view's contract,
    /// so `"Shooter · "` was what a row actually showed.
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
}
