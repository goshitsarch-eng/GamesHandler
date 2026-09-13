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
//! For every `| `[`function`]` | `path`, in `container` |` row of the header's
//! `# Callers` table:
//!
//! * the cited path exists and is readable;
//! * the cited file's **production** source — test modules cut, see
//!   [`production_src`] — contains a *call-shaped* occurrence of the function,
//!   `function(`. This is the claim: it is called from live code.
//! * that call is inside `container`, the function the row names as its
//!   enclosing scope — resolved by [`enclosing_function`].
//!
//! The enclosing function is the **locator**, not the claim. The assertion that
//! matters is the second one, and it does not drift when unrelated code above
//! it is edited.
//!
//! The five functions the old sentence named are additionally required to still
//! appear as rows, so the table cannot shrink to nothing and pass vacuously.
//!
//! # What is deliberately not asserted, and why
//!
//! **The exact line number.** The table cited `path:line` for one revision and
//! this test asserted the line exactly, and that is the version that failed
//! *inside the hour*: concurrent edits to `main.rs` moved `easy_install_worker`
//! from `:3160` to `:3269`, every `main.rs` citation stopped landing, and the
//! check failed on correct code for a reason that is not the claim. That is
//! `ARCH-16`'s defect (*a pointer that no longer lands is worse than no
//! pointer, because it is trusted*) arriving in the fix for `ARCH-05`. A symbol
//! survives an edit; a line number does not. So a row names the enclosing
//! function and [`enclosing_function`] resolves it at check time.
//!
//! That is **stronger than the line was**, not merely steadier. "Some line in
//! this file contains a call" is satisfied by a call in a dead branch, by an
//! unrelated function beside it, or by a test helper in a test block this
//! reader does not recognise as one; "this call is inside the function the
//! header names" is the wiring claim itself. It also makes both rows that cite
//! `installers.rs` subject to the same rule as the rest — the previous version
//! had to exempt them, because the table sits above the calls it cites there
//! and every edit to the table moved their line numbers.
//!
//! **That the occurrence is a call rather than a mention.** `function(` is
//! matched textually, so a comment reading `download_installer(` would satisfy
//! it. Resolving this properly means parsing Rust, which is a heavier thing
//! than the claim needs: the failure this guards against is a function that
//! lost its last caller, and that removes the text too. The bound is stated
//! here rather than left for a reader to find. [`enclosing_function`] has the
//! same bound from the other direction — it scans backwards for `fn ` rather
//! than parsing — and is exact for the shapes the files here contain.

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
    /// The function the call must be inside. A symbol, not a line, so the row
    /// survives an edit above it — see the module docs.
    container: String,
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
        // `| [`name`] | `path`, in `container` |`
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
        // Both halves are code-spanned in the table — `` `path`, in
        // `container` `` — so the backticks are stripped rather than trimmed:
        // `trim_matches` would leave the one between the path and the comma,
        // and the path would carry a trailing backtick into the filesystem.
        let citation: String = cells[1].chars().filter(|c| *c != '`').collect();
        let citation = citation.trim();
        let Some((path, container)) = citation.split_once(", in ") else {
            panic!(
                "the `# Callers` row for `{name}` cites {citation:?}, which is \
                 not a `path`, in `container` citation, so this test cannot \
                 check it and would pass without reading anything. A row that \
                 names only a file is the shape that failed before: a file has \
                 thousands of lines and naming it asserts nothing."
            );
        };
        if path.is_empty() || container.is_empty() {
            panic!(
                "the `# Callers` row for `{name}` cites {citation:?}, which has \
                 an empty path or an empty container, so there is nothing to \
                 resolve."
            );
        }
        rows.push(Row {
            function: name.to_string(),
            path: PathBuf::from(path),
            container: container.to_string(),
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

/// The name of the innermost `fn` whose body contains `line` (1-based), from
/// `src`'s production source.
///
/// The scan is textual and backwards from the call: the first line above it
/// that declares a function is the one the call sits in, because Rust has no
/// forward-referencing items inside a body. It is exact for the shapes these
/// files contain and the bound is the same one [`call_lines`] states — a `fn `
/// inside a string literal read as a definition would resolve the container
/// wrongly, and none appears on these paths.
///
/// `//`-commented lines are skipped, for the same reason [`call_lines`] skips
/// them: a commented-out definition is not a scope, and a stale comment naming
/// a function is exactly what this file exists to catch.
///
/// **A call outside every function resolves to `None` and fails the check**,
/// which is the safe direction: the row's claim is that live code calls the
/// function, and a free-standing call in a module body is not that.
fn enclosing_function(src: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    for index in (0..line.min(lines.len())).rev() {
        let text = lines[index];
        if text.trim_start().starts_with("//") {
            continue;
        }
        let Some((_, rest)) = text.split_once("fn ") else {
            continue;
        };
        // The identifier after `fn `, up to the first character that cannot be
        // part of one. A signature that wraps has the name and then `(` or
        // nothing, and an empty capture means the `fn ` was not a definition.
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        return Some(name);
    }
    None
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
            container,
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

        // The locator: every call of `function` in this file must be inside the
        // container the row names. `any`, not `all` — a function may have more
        // than one production call site and the row names the one it names —
        // but the messages below distinguish the two ways this fails, because
        // "no call is in X" and "there are no calls at all" have different
        // fixes: move the row, or say in the header that the wiring is gone.
        let containers: Vec<Option<String>> = calls
            .iter()
            .map(|line| enclosing_function(&production, *line))
            .collect();
        assert!(
            containers
                .iter()
                .any(|found| found.as_deref() == Some(container.as_str())),
            "the `# Callers` row for `{function}` says the call is in \
             `{container}`, in {}, and it is not. The calls this row's file has \
             are at {} — inside {}. This is not a failure of the claim (the \
             function is wired) unless the list is empty of `{container}` \
             entirely, in which case the header is naming a function that no \
             longer calls it. Update the row to a container listed here.",
            file.display(),
            calls
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            containers
                .iter()
                .map(|found| found
                    .as_deref()
                    .map_or("no function at all".to_string(), |name| format!("`{name}`")))
                .collect::<Vec<_>>()
                .join(", ")
        );
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
fn the_container_is_the_innermost_function_and_survives_an_edit_above_it() {
    // The locator this test replaces was a line number, and it went stale
    // within the hour: `easy_install_worker` moved from `:3160` to `:3269`
    // under concurrent edits and every `main.rs` row stopped landing. The
    // assertion below is the half that matters — the *same source* with three
    // lines inserted above the call must resolve to the same container, which
    // is exactly what a line number cannot do.
    let source = "\
fn outer() {
    let a = 1;
    let b = 2;
    call_me(a, b);
}

fn other() {
    call_me(0, 0);
}
";
    assert_eq!(
        enclosing_function(source, 4).as_deref(),
        Some("outer"),
        "the call on line 4 is inside `outer`"
    );
    assert_eq!(
        enclosing_function(source, 8).as_deref(),
        Some("other"),
        "the call on line 8 is inside `other` — the scan must take the \
         *nearest* `fn` above the call, not the first in the file. A helper \
         that returned `outer` for every call would make the container claim \
         unfalsifiable, which is the defect this file exists to catch."
    );

    let shifted = format!("// a new line\n// and another\n// and a third\n{source}");
    assert_eq!(
        enclosing_function(&shifted, 11).as_deref(),
        Some("other"),
        "the same call, four lines lower, must resolve to the same container. \
         This is the property the line-number locator did not have."
    );

    // A call outside every function has no container, and `None` is what makes
    // the row fail rather than pass vacuously.
    assert_eq!(enclosing_function("call_me(1);\n", 1), None);
}

#[test]
fn a_definition_is_not_its_own_container() {
    // `enclosing_function` scans backwards from the call, so a definition below
    // the call would be wrong to pick up. This pins the direction: the call on
    // line 1 is inside `body`, not inside `later`.
    let source = "\
fn body() {
    call_me(1);
}

fn later() {
    call_me(2);
}
";
    assert_eq!(
        enclosing_function(source, 2).as_deref(),
        Some("body"),
        "a function defined *after* the call must not be reported as its \
         container — that would name a scope the call is not in, and the header \
         row would be checked against the wrong function."
    );
}

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
