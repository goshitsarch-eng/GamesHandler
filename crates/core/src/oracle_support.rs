//! Oracles shared by the reference-reading tests of both crates.
//!
//! This module exists because of ARCH-13. `join_adjacent_literals` was written
//! twice — once in this crate's `plugins` tests and once in the application's —
//! with the two bodies identical down to the loop tail. Everything both copies
//! assert is read out of the same reference file, so the duplication had a
//! specific cost rather than a general one: a lexer that mishandles an edge case
//! (an unhandled `'''`, a backslash before a newline, a comment between adjacent
//! literals) makes both crates' tests agree with each other *and* disagree with
//! the reference, which is this project's own named defect class — a green test
//! whose assertion a different mechanism satisfies. A fix applied to one copy
//! and not the other is invisible to both test suites.
//!
//! ## Why this is a module and not a `#[cfg(test)]` module
//!
//! The row's suggested home was "core's test-support surface", and the obvious
//! reading of that — a `#[cfg(test)] mod` — does not work here, for a reason
//! worth recording because it reads like an oversight. `cfg(test)` is set for
//! the crate under test and *not* for its dependencies, so an application test
//! cannot call into `gamehandler-core`'s `#[cfg(test)]` items no matter how
//! they are declared. A `#[cfg(test)]` module would have deduplicated nothing
//! while looking like it had.
//!
//! So this is a real module behind a non-default feature that only the test
//! path turns on. The dependency edges are the guarantee that it cannot reach a
//! release binary: `gamehandler` asks for it under `[dev-dependencies]` and
//! `gamehandler-core` under `[dev-dependencies]` of itself, and neither is
//! enabled by `cargo build` or `cargo build --release`. `scripts/verify.sh`
//! builds both ways, so a release binary that had grown a link to this module
//! would fail there rather than ship.
//!
//! ## Adding to this module
//!
//! The rule for what belongs here is the reverse of the usual one: add a
//! helper when the *second* copy appears, not the third. The two copies this
//! module replaced each carried a comment explaining why the duplication was
//! acceptable, and both of those comments were still in the tree after the
//! duplication had stopped being acceptable — that is ARCH-14's defect, found
//! in the same audit as this one.

/// Python's implicit string concatenation, undone.
///
/// `bridge.py` writes its notifications as adjacent literals across lines, and
/// the joined text is what the user reads. Restoring `"` + whitespace + `"`
/// into one literal lets a sentence be looked up in the source exactly as the
/// page shows it, instead of as the source happens to wrap it. The whitespace
/// *inside* a literal is left alone — only the newline and indentation between
/// two literals are dropped.
///
/// A following literal may carry a prefix, and `pluginsIntro` is where that
/// matters: it is written `"... host package " f"manager{detected}"`, so the
/// boundary is `"` + whitespace + `f"` and a joiner that only knew about a bare
/// quote found nothing. Up to two prefix letters are accepted (`f`, `r`, `b`,
/// `u` and their combinations), and the whitespace requirement is what keeps
/// this from merging anything that is not a concatenation — `"Wine",` on the
/// next line is protected by its comma.
///
/// This is a lexer, and it is deliberately not a general one: it knows the four
/// things the reference file actually contains and nothing else. That is the
/// point of having it in one place — its limits are now stated once, next to
/// the one body, instead of being implied twice.
pub fn join_adjacent_literals(source: &str) -> String {
    fn is_prefix(character: char) -> bool {
        matches!(character, 'f' | 'F' | 'r' | 'R' | 'b' | 'B' | 'u' | 'U')
    }
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '"' {
            let mut probe = index + 1;
            while probe < chars.len() && chars[probe].is_whitespace() {
                probe += 1;
            }
            let mut quote = probe;
            while quote < chars.len() && quote - probe < 2 && is_prefix(chars[quote]) {
                quote += 1;
            }
            if probe > index + 1 && quote < chars.len() && chars[quote] == '"' {
                index = quote + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// A file from the repository root, read at test time (`ARCH-15`).
///
/// The reference implementation is in the tree — `gamehandler/*.py`, the QML,
/// `data/` — and a test that asserts the port matches it has to read it. This
/// is that read, once.
///
/// **It was four copies with three derivations.** `env!("CARGO_MANIFEST_DIR")`
/// is `crates/core` or `crates/app` depending on which crate is compiling, so
/// each copy had to climb two levels and each climbed differently:
/// `.join("..").join("..")` in `core::plugins` and `core::credits`,
/// `.ancestors().nth(2)` in `app::view::credits`, and a bare
/// `join("../../…")` chain at six further sites. The four also carried four
/// different panic messages, so a missing file produced four different
/// explanations of the same problem.
///
/// The cost of that is not tidiness. These reads are the *oracle* for tests
/// about the reference: a restructure that moved one file would break some
/// tests with a panic and leave others passing, because the derivations do not
/// all resolve to the same place. A partial failure that looks like a data
/// problem is the worst of both, and it is what ARCH-15 is about.
///
/// Resolution is relative to this crate's `CARGO_MANIFEST_DIR`, so callers
/// pass a path from the repository root — `"gamehandler/plugins.py"`,
/// `"data/com.goshapps.GameHandler.metainfo.xml"` — and never compute the
/// climb themselves.
///
/// # Panics
///
/// If the file cannot be read. That is deliberate and it is the one category
/// this project allows to panic (category A): a test asserting against the
/// reference is meaningless if the reference is missing, and a silent empty
/// string would make every such assertion pass. The message names the resolved
/// path, so the failure says where it looked rather than only that it failed.
pub fn repo_file(relative: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/core sits two levels below the repository root");
    let path = root.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the reference file {} is unreadable: {error}. Every test that reads \
             it is asserting against the reference rather than against a copy of \
             the port's own constants, so this file missing is not a data \
             problem — it is the oracle being absent.",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::join_adjacent_literals;

    /// The boundary the reference file actually contains: a bare quote, a
    /// newline, indentation, then a prefixed quote.
    #[test]
    fn a_prefixed_literal_is_joined_to_the_one_before_it() {
        assert_eq!(
            join_adjacent_literals("\"a \"\n    f\"b\""),
            "\"a b\"",
            "the newline, the indentation and the second quote all go"
        );
    }

    /// The comma is the *only* thing that stops a join, and this is the test
    /// that says so — because the first version of it asserted otherwise.
    ///
    /// It claimed that without a comma `"a"` + newline + `"b"` would be left
    /// alone. It is not, and it should not be: in Python that *is* implicit
    /// concatenation, so joining it is the correct reading, and the lexer is
    /// right where the test was wrong. The assertion was written from a guess
    /// about the rule instead of from the code, which is the exact failure this
    /// audit keeps finding — and it is worth leaving the correction in place,
    /// since the wrong guess is the more intuitive reading of "adjacent
    /// literals" and the next person to touch this will make it too.
    #[test]
    fn a_comma_is_what_stops_a_join_and_nothing_else_does() {
        assert_eq!(
            join_adjacent_literals("\"a\",\n    \"b\""),
            "\"a\",\n    \"b\"",
            "the comma is what makes these two arguments, not one concatenation"
        );
        assert_eq!(
            join_adjacent_literals("\"a\"\n    \"b\""),
            "\"ab\"",
            "with no comma this is implicit concatenation and does join — the \
             comma above is the protection, not the line break"
        );
    }

    /// Whitespace *inside* a literal survives. This is the half that a joiner
    /// which simply deleted newlines would get wrong, and the sentences read
    /// out of the reference are compared against the page's own text.
    #[test]
    fn whitespace_inside_a_literal_survives() {
        assert_eq!(
            join_adjacent_literals("\"a\n b\""),
            "\"a\n b\"",
            "a literal spanning lines is one literal, not two"
        );
    }

    /// Three prefix letters exceed the two the scan accepts, so this is not a
    /// concatenation as far as the lexer is concerned. Recorded because it is
    /// a real limit, not a hypothetical: the reference file contains no such
    /// prefix today, and if it ever does this is where the behaviour is
    /// pinned rather than discovered.
    #[test]
    fn three_prefix_letters_are_not_treated_as_a_continuation() {
        assert_eq!(
            join_adjacent_literals("\"a \"\n    frb\"b\""),
            "\"a \"\n    frb\"b\"",
            "the scan accepts up to two prefix letters and no more"
        );
    }
}
