//! Every "this is wired" claim `crates/core/src/installers.rs` makes must hold.
//!
//! That file's header spent a long time asserting the opposite of the truth:
//! that it "has no *caller*" and that five named functions "have no call site
//! outside their own tests". Every clause was false by the time it was read —
//! `easy_install_worker` in the app crate drives the download and wizard halves
//! directly, and the other three are called inside the module's own live
//! install path. (`ARCH-05`.)
//!
//! A false "not landed yet" is not a stale comment. A `core` crate is the half
//! that is supposed to be self-describing, and a maintainer told the install
//! path is unwired may re-wire it, delete it, or decline to touch it. The
//! module says so itself: `crates/app/src/view/installers.rs:46-50` records
//! that this project has already lost time to exactly this failure once.
//!
//! So the header now carries a **table of call sites**, and this test reads the
//! header rather than a second copy of the same facts. A hand-maintained
//! expectation list beside the header could disagree with it, and a
//! disagreement would be invisible — which is this project's own named defect
//! class, a check that passes without inspecting what it claims.
//!
//! # What is asserted
//!
//! For every `| `[`function`]` | `path:line` |` row of the header's `# Callers`
//! table:
//!
//! * the cited path exists and is readable;
//! * the cited file's **production** source — test modules cut, see
//!   [`production_src`] — contains a *call-shaped* occurrence of the function,
//!   `function(`. This is the claim: it is called from live code.
//! * the cited line number is the line the call is on, when the citation is
//!   into another file, and merely a real line when it is into this one — see
//!   the exception noted below.
//!
//! The line is a **locator**, not the claim. The assertion that matters is the
//! second one, and it does not drift when unrelated code above it is edited.
//!
//! The five functions the old sentence named are additionally required to still
//! appear as rows, so the table cannot shrink to nothing and pass vacuously.
//!
//! # What is deliberately not asserted, and why
//!
//! **The exact line.** The first version of this test required the function to
//! be named on the cited line, and it failed on the header this file was
//! written for: three of the five citations had already drifted. Pinning the
//! line would then fail on every unrelated insertion above it — `main.rs` is
//! five thousand lines and the install worker sits deep inside it. A test that
//! fails on correct code is a test someone deletes, and deleting *this* test
//! restores the defect it exists for. The line number is kept in the header for
//! a reader to jump to and is checked only for being a real line; when a call
//! moves, the failure message here prints where it actually is so the table can
//! be refreshed in the same commit.
//!
//! **The line, for rows that cite this same file.** The table sits above the
//! calls it cites in `installers.rs`, so any edit to the table moves their line
//! numbers while the table itself stays put. Requiring an exact line there
//! would make the test fail whenever the header is edited — on correct code —
//! and a test that does that is one someone deletes, which would restore the
//! defect it exists for. Those two rows are therefore checked for pointing
//! inside the file and no further. Rows into other files are exact, because
//! nothing done to this table can move them. This is a measured consequence of
//! where the table lives, not a tolerance.
//!
//! **That the occurrence is a call rather than a mention.** `function(` is
//! matched textually, so a comment reading `download_installer(` would satisfy
//! it. Resolving this properly means parsing Rust, which is a heavier thing
//! than the claim needs: the failure this guards against is a function that
//! lost its last caller, and that removes the text too. The bound is stated
//! here rather than left for a reader to find.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root, from this test binary's own location.
///
/// `CARGO_MANIFEST_DIR` is `crates/core`, so the root is two levels up. Taking
/// it from the environment rather than from the process's working directory
/// means the test reads the same tree however it was invoked.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/core has two parent directories")
        .to_path_buf()
}

/// One row of the header's `# Callers` table.
struct Row {
    function: String,
    path: PathBuf,
    line: usize,
}

/// Read the `# Callers` section of `installers.rs`'s module doc.
///
/// The section is *found* — everything after the `# Callers` heading up to the
/// next `//! #` heading — rather than assumed to be the first table in the
/// file, so a table that moves within the header is still read. A header that
/// loses the section yields no rows, and the caller asserts on the count rather
/// than passing vacuously.
fn caller_rows(source: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut in_section = false;
    for line in source.lines() {
        let Some(doc) = line.strip_prefix("//!") else {
            continue;
        };
        let doc = doc.trim();
        if doc.starts_with("# ") {
            in_section = doc == "# Callers";
            continue;
        }
        if !in_section {
            continue;
        }
        let cells: Vec<&str> = doc.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 2 {
            continue;
        }
        // `| [`name`] | `path:line` |`
        let name = cells[0]
            .trim_matches('`')
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_matches('`');
        if name.is_empty() || name == "Function" || name.starts_with('-') {
            continue;
        }
        if !cells[1].starts_with('`') {
            continue;
        }
        let citation = cells[1].trim_matches('`');
        let Some((path, line)) = citation.rsplit_once(':') else {
            panic!(
                "the `# Callers` row for `{name}` cites {citation:?}, which is \
                 not a `path:line` citation, so this test cannot check it and \
                 would pass without reading anything."
            );
        };
        let Ok(line) = line.parse::<usize>() else {
            panic!(
                "the `# Callers` row for `{name}` cites a line number this test \
                 cannot parse: {citation:?}"
            );
        };
        rows.push(Row {
            function: name.to_string(),
            path: PathBuf::from(path),
            line,
        });
    }
    rows
}

/// The source with test modules removed, and whether a cut happened.
///
/// The boolean is returned rather than discarded because a helper that
/// silently stops cutting would leave the haystack holding its own needle — the
/// failure this repository names as its own defect class. Callers assert on it.
///
/// Cut at the first `#[cfg(test)]` that is followed by a `mod`, skipping blank
/// lines, comments and further attributes. `installers.rs` has a
/// `#[cfg(test)]` on a `thread_local!` test counter well above the production
/// calls this file checks ([`crate::installers`], line 1642); cutting at that
/// one would delete the very code under test and fail the check for the wrong
/// reason. Requiring `mod` after the attribute distinguishes a test *module*
/// from a test-only *item*.
fn production_src(src: &str) -> (String, bool) {
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() != "#[cfg(test)]" {
            continue;
        }
        let opens_a_module = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("#["))
            .is_some_and(|l| l.starts_with("mod "));
        if opens_a_module {
            let mut cut = lines[..i].join("\n");
            if !cut.is_empty() {
                cut.push('\n');
            }
            return (cut, true);
        }
    }
    (src.to_string(), false)
}

/// Line numbers, 1-based, where `function` is **called** in `text`.
///
/// The needle is `function(`, and the definition `fn function(` also contains
/// it. That is not a cosmetic detail: this test's first version counted the
/// definition as a call site, and it passed with the function's last production
/// caller deleted — a check that did not inspect what it claimed, which is this
/// project's recurring defect. The definition is therefore excluded here, and
/// `the_needle_does_not_match_a_definition` pins that exclusion so the same
/// mistake cannot come back through a refactor of this helper.
///
/// A line that mentions the name without calling it — an import, a `[`doc
/// link`]` — does not carry the `(` and is not matched. Line comments are
/// skipped too: without that, a comment reading "this no longer calls
/// `function(`" would keep a dead function looking wired, which is the
/// false-negative this whole test is about. Block comments spanning lines are
/// not handled; none appear anywhere near the code this checks.
fn call_lines(text: &str, function: &str) -> Vec<usize> {
    let called = format!("{function}(");
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && line.contains(&called) && !defines(line, function)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Whether `line` is the item definition of `function` rather than a call to it.
///
/// Covers both paths by which a definition gets a call-shaped line: the `fn`
/// itself, and a signature that wraps so the name lands on its own line with
/// the parameter list following. Signatures here are `pub fn name(` or
/// `fn name(`, so testing for `fn` immediately before the name — with only
/// whitespace between — is exact rather than a heuristic.
fn defines(line: &str, function: &str) -> bool {
    let line = line.trim();
    let Some(head) = line.split_once("fn ").map(|(head, _)| head) else {
        return false;
    };
    // `head` must be nothing but leading whitespace and visibility keywords,
    // and the text after `fn ` must start with the function's name and `(`.
    if !head.is_empty() && head != "pub " && !head.starts_with("pub(") {
        return false;
    }
    line[head.len() + 3..].starts_with(&format!("{function}("))
}

#[test]
fn every_function_the_installers_header_claims_is_wired_has_a_call_in_live_code() {
    let root = repo_root();
    let header_path = root.join("crates/core/src/installers.rs");
    let source = fs::read_to_string(&header_path)
        .unwrap_or_else(|error| panic!("{} should be readable: {error}", header_path.display()));

    let rows = caller_rows(&source);

    // An empty section would make every assertion below vacuous, and a header
    // that quietly dropped its call-site table is exactly the regression this
    // guards: the old header made the same claim in prose, with no rows to read.
    assert!(
        rows.len() >= 5,
        "the header's `# Callers` table has {} rows, and the prose it replaced \
         named five functions. Either the section was removed — and the claim \
         went with it — or its shape changed and this reader no longer finds \
         it. Rows: {:?}",
        rows.len(),
        rows.iter().map(|r| &r.function).collect::<Vec<_>>()
    );

    // Asserted by name so that dropping a row fails here instead of shrinking
    // the table silently.
    for expected in [
        "download_installer",
        "wait_for_installer",
        "wait_for_prefix_idle",
        "verify_installer_authenticity",
        "wineserver_binary",
    ] {
        assert!(
            rows.iter().any(|r| r.function == expected),
            "the `# Callers` table no longer names `{expected}`, which the prose \
             it replaced called callerless. If it truly has no caller again, \
             say that in the header in those words rather than dropping the \
             row, because the row is what makes the claim checkable. Rows: {:?}",
            rows.iter().map(|r| &r.function).collect::<Vec<_>>()
        );
    }

    for row in &rows {
        let Row {
            function,
            path,
            line,
        } = row;
        let file = root.join(path);
        let text = fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!(
                "the `# Callers` row for `{function}` cites {}, which cannot be \
                 read: {error}. A citation to a file that is gone is the claim \
                 itself going stale.",
                file.display()
            )
        });

        let total = text.lines().count();
        assert!(
            *line >= 1 && *line <= total,
            "the `# Callers` row for `{function}` cites {}:{line}, and that file \
             has {total} lines. The citation points outside the file, so a \
             reader following it lands nowhere.",
            file.display()
        );

        let (production, cut) = production_src(&text);
        let calls = call_lines(&production, function);
        assert!(
            !calls.is_empty(),
            "the `# Callers` row for `{function}` claims a live call site in {}, \
             and its production source — test modules {} — has none. Either the \
             last caller was removed and the header is false again, or the call \
             moved to another file and the row must move with it.",
            file.display(),
            if cut {
                "removed"
            } else {
                "not found, so none removed"
            }
        );
        // The locator is asserted exactly, with one measured exception: a row
        // citing a line in *this* file. The table sits above the calls it
        // cites, so every edit to the table moves those line numbers, and an
        // exact check there would measure the header's length rather than the
        // call's position — failing on correct code. Rows into other files are
        // unaffected by anything done here and are held to the line.
        if file != header_path {
            assert!(
                calls.contains(line),
                "the `# Callers` row for `{function}` cites {}:{line}, but the \
                 live call is at {}. This is not a failure of the claim — the \
                 function is wired — it is the locator drifting. Update the row \
                 to a line listed here so the next reader lands on the call.",
                file.display(),
                calls
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}

/// The exclusion that makes the check above mean what it says.
///
/// This test exists because the check above was measured passing while the
/// function it names had no caller left: the needle matched the `fn` line. A
/// helper that silently stopped excluding definitions would restore exactly
/// that, and nothing in the check above would notice — so the exclusion is
/// pinned here, on both the definition shapes this workspace uses.
#[test]
fn the_needle_does_not_match_a_definition() {
    let source = "\
fn bare(
pub fn public(
    pub(crate) fn restricted(
    let value = bare(a, b);
    other(bare(c));
    // see bare( for details
";
    let found = call_lines(source, "bare");
    assert_eq!(
        found,
        vec![4, 5],
        "`bare` is called on lines 4 and 5 and defined on line 1; the comment \
         mentioning it (line 6) is not a call. Found {found:?}. If this list \
         grew to include 1, the check above would pass on a function with no \
         caller — the defect this helper exists to prevent. If it grew to \
         include 6, a comment saying a function *stopped* being called would \
         keep it looking wired, which is the same defect through the other \
         door."
    );
    for (line, function) in [
        ("pub fn public(", "public"),
        ("    pub(crate) fn restricted(", "restricted"),
    ] {
        assert!(
            call_lines(line, function).is_empty(),
            "the definition {line:?} was counted as a call to {function}"
        );
    }
}

/// The other half of the check above: the test-module cut has to happen.
///
/// Every function the header cites also has a production call, so if
/// [`production_src`] silently stopped cutting, every row would still pass —
/// and the day the last production caller is deleted, the leftover test calls
/// would keep the row green. That is the same defect as the needle matching its
/// own definition, one level down. This pins the cut on a synthetic module,
/// where the answer is not in doubt.
#[test]
fn the_test_module_cut_removes_test_only_calls() {
    let source = "\
fn caller() {
    helper();
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        helper();
    }
}
";
    let (production, cut) = production_src(source);
    assert!(
        cut,
        "a `#[cfg(test)] mod` is present, so a cut must be reported; \
                  a helper that stops cutting is invisible to the check it feeds"
    );
    assert_eq!(
        call_lines(&production, "helper"),
        vec![2],
        "the only surviving call is the production one on line 2; the call \
         inside the cut module would make a dead function look wired"
    );

    // And the marker that decides *which* `#[cfg(test)]` to cut. A test-only
    // item — not a module — must not be cut at, because `installers.rs` has one
    // above the production code this file checks, and cutting there would
    // delete the code under test and fail for the wrong reason.
    let with_item = "\
#[cfg(test)]
thread_local! {
    static N: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn caller() {
    helper();
}
";
    let (production, cut) = production_src(with_item);
    assert!(
        !cut,
        "a `#[cfg(test)]` on a `thread_local!` is a test-only *item*, not a \
         module, and cutting at it would remove the production call below it"
    );
    assert_eq!(call_lines(&production, "helper"), vec![7]);
}

#[test]
fn the_installers_header_no_longer_claims_the_file_has_no_caller() {
    // The specific false sentences, asserted absent rather than merely
    // superseded: a rewrite that reinstated the paragraph while leaving the
    // table would otherwise satisfy the test above.
    let source = fs::read_to_string(repo_root().join("crates/core/src/installers.rs")).unwrap();
    for phrase in [
        "It has no *caller*",
        "have no call site outside their own tests",
    ] {
        assert!(
            !source.contains(phrase),
            "installers.rs still carries {phrase:?}, which was measured false: \
             all five functions that sentence named have production call sites \
             (`ARCH-05`)."
        );
    }
}
