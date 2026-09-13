//! Which pages are still placeholders: pinned here, so a page cannot land
//! without this file being edited (task #32).
//!
//! T-08 landed the shell — sidebar, routing, toaster — and every page body with
//! it is `pending_page`, which renders "This page has not been ported yet
//! (<task>)." — `pending_page`, the single generator of every placeholder body.
//! That is six pages: the *entire* user-visible
//! surface of the application. Before this test, the same source with all six
//! replaced by real content and the same source with none of them replaced
//! produced identical results from `cargo test`, `verify.sh` and
//! `smoke-test.sh`: nothing anywhere asserted that a page had been ported.
//! Landing a page and not landing it were indistinguishable to the gate.
//!
//! This test makes them distinguishable by pinning the *set* below. It is
//! deliberately a list that must be edited by hand:
//!
//! - landing a page means deleting its name from [`PINNED_PENDING`], and until
//!   that happens the test fails **naming the page**;
//! - adding a page that is a placeholder fails too, naming the page that was
//!   not pinned;
//! - T-19's acceptance criterion is that this list is **empty**, which is one
//!   falsifiable line rather than a judgement call about whether the app looks
//!   finished.
//!
//! It is named for exactly what it asserts. That is the lesson of the
//! `cli-list` check next door (`smoke-test.sh`), which was called `cli-list`
//! while examining nothing but the exit status: a name that reads as stronger
//! than the check is worse than a weak check, because it retires the question.
//!
//! # What this does and does not establish
//!
//! It reads `crates/app/src/main.rs` — the file the Flatpak is compiled from —
//! and classifies each arm of the page dispatch by whether it calls
//! `pending_page`. So it pins the *mechanism*: a page is "ported" here if its
//! arm no longer calls `pending_page`. A page landed as a second placeholder
//! mechanism, under another name, would be classified as ported and this test
//! would not notice. Two things bound that: `pending_page` is the single
//! generator of every placeholder body, and
//! `scripts/verify.sh` asserts against the *shipped* binary that its
//! placeholder marker is present if and only if the source has pending call
//! sites, which is the artifact-side half of the same claim.
//!
//! # Why it reads the file rather than counting in it
//!
//! The obvious version of this test — `include_str!("main.rs")` plus a count of
//! the placeholder marker — is self-matching when the test lives in `main.rs`
//! (any literal written into the test contributes to the count it asserts), and
//! it is fooled by any *other* mention of the marker anywhere in the file or the
//! directory. This one never counts strings: it finds the page dispatch, walks
//! its arms, and asks what each arm calls. Comments cannot contribute an arm,
//! and a match arm that mentions `pending_page` in prose without calling it is
//! not an arm either.
//!
//! Two further guards, because a parser that silently finds nothing is the
//! defect class this whole file exists to catch:
//!
//! - the arms found are compared against `Page::ALL` in `state.rs`, so a parse
//!   that loses arms fails loudly instead of asserting over a shorter list;
//! - `file!()` is asserted not to end in `main.rs`, so moving this test into
//!   the file it scans is an error rather than a self-fulfilling pass.

use std::fs;
use std::path::{Path, PathBuf};

/// The pages whose body is still a placeholder, and nothing else.
///
/// **Edit this list when you land a page.** Deleting the name is the edit; the
/// test fails until you make it, and names the page when it does. This is the
/// only hand-maintained copy of the fact, and it is hand-maintained on purpose.
///
/// **Empty as of T-38**, which landed the last page (`Installers`) and the flow
/// behind it. The deletion was made by the landing commit itself, which is what
/// the paragraph above asks for; the test below reports "PORTED, but still
/// listed in PINNED_PENDING — delete the line" until it is.
///
/// The set this must equal is parsed out of the page dispatch in
/// `crates/app/src/main.rs` — `view_body`'s `match self.state.page` — so it is
/// not restated here. The page→task mapping lives in that file too, and is
/// deliberately not copied: an earlier version of this comment listed it, and
/// the list went stale while the test beside it stayed correct. A hand-copied
/// fact that no assertion reads has nothing to keep it honest.
const PINNED_PENDING: [&str; 0] = [];

/// The repository root, derived rather than hardcoded.
///
/// `CARGO_MANIFEST_DIR` is `<root>/crates/app` for this target, so the root is
/// two levels up. Using the build-time value (rather than the process's working
/// directory) means the test reads the same checkout cargo just built, whatever
/// directory it is invoked from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/app sits two levels below the repository root")
        .to_path_buf()
}

/// Assert that the checkout this test *reads* is the checkout cargo is
/// *testing*, and that it is a GameHandler checkout at all.
///
/// This test binary carries `CARGO_MANIFEST_DIR` from the moment it was
/// compiled, and — because `crates/app` has a `[[bin]]` and no library — an
/// integration test here does not depend on `src/main.rs` and is **not rebuilt
/// when `main.rs` changes**. Share a `target/` directory between two checkouts
/// and cargo will hand you the test binary compiled in the other one, which
/// then reads the other tree's `main.rs` and reports on it. That is not
/// hypothetical: it happened while the before/after evidence for this very test
/// was being produced, and the test passed against a tree whose Library page had
/// been ported, because it was reading the tree where it had not. A silent pass
/// is the one outcome this file exists to prevent, so it is a hard error here.
///
/// Cargo runs a test binary with the working directory set to the package root
/// it is testing, and that is decided at *run* time rather than baked in — so a
/// working directory that disagrees with the compiled-in path means exactly the
/// two-checkout case, and a working directory that has no `src/main.rs` (a
/// binary run by hand from somewhere else) is simply not evidence either way.
///
/// The three-path probe is the one `app_id_lockstep.rs` uses, for the same
/// reason: a tree that is *gone* and a tree that is a *different project* need
/// different advice, and `read()` below would otherwise report the second as
/// the first.
fn assert_right_tree(root: &Path) {
    let absent: Vec<&str> = ["Cargo.toml", "build-aux", "data"]
        .into_iter()
        .filter(|name| !root.join(name).exists())
        .collect();
    if !absent.is_empty() {
        panic!(
            "this test was COMPILED in {}, which is not the GameHandler checkout: \
             {} is not there. `CARGO_MANIFEST_DIR` is fixed when the test binary is \
             built, and a `target/` directory copied between checkouts (or shared \
             with CARGO_TARGET_DIR) can hand cargo a binary built somewhere else. \
             Run `cargo clean -p gamehandler` and re-run from the checkout you mean.",
            root.display(),
            absent.join(", ")
        );
    }

    // The package directory this binary was compiled for — `crates/app`, not the
    // repository root. It is what `current_dir` is compared against below, so
    // it has to be the same thing cargo sets the working directory to.
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR"));
    let running = std::env::current_dir().expect("the test process has a working directory");
    // `current_dir` is only evidence when it is a package directory — a binary
    // run by hand from elsewhere has some other directory and says nothing.
    if running.join("src/main.rs").is_file() && running != compiled {
        panic!(
            "this test binary was COMPILED for {}, but cargo is RUNNING it in {}. \
             It would read the first tree's pages and report on the second's. Two \
             checkouts sharing one `target/` directory is the cause; give each its \
             own, or run `cargo clean -p gamehandler` here.",
            compiled.display(),
            running.display()
        );
    }
}

/// One arm of the page dispatch: the `Page` variant, and the expression it
/// renders.
struct Arm {
    page: String,
    body: String,
}

impl Arm {
    /// Whether this arm renders the not-yet-ported placeholder.
    ///
    /// The whole point of the test: `pending_page` is the single generator of
    /// every placeholder body, so "does this arm call it" is the question.
    fn is_pending(&self) -> bool {
        self.body.starts_with("pending_page(")
    }

    /// The task id, when the arm is a placeholder and names one.
    fn task(&self) -> Option<&str> {
        self.body
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(task, _)| task)
    }
}

/// The arms of the page dispatch in `main.rs`, in source order.
///
/// Finds the match on `self.state.page` and takes the lines under it until the
/// indentation drops back out of the match. Indentation, not brace counting:
/// the arms are Rust and may contain braces, string literals with braces in
/// them, and nested `match`es, and a brace counter that does not also lex
/// strings would drift the moment a page landed — which is the edit this test
/// has to survive.
fn page_dispatch(source: &str) -> Vec<Arm> {
    let anchor = ["match self.state.page", "match self.page"]
        .into_iter()
        .find(|needle| source.contains(needle));
    let Some(anchor) = anchor else {
        panic!(
            "could not find the page dispatch (a `match self.state.page` or \
             `match self.page`) in crates/app/src/main.rs. This test parses that \
             match; until it is taught the new shape it would assert over an empty \
             list and pass for the wrong reason, so it fails here instead."
        );
    };

    let lines: Vec<&str> = source.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(anchor))
        .expect("the anchor was just found in the same text");
    let indent = |line: &str| line.len() - line.trim_start().len();
    let match_indent = indent(lines[start]);

    let mut arms = Vec::new();
    for line in &lines[start + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        // The `};` that closes the match is at the match's own indentation, and
        // so is the end of the function; anything less indented has left it.
        if indent(line) < match_indent {
            break;
        }
        if let Some(rest) = line.trim().strip_prefix("Page::") {
            let page: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            let Some((_, body)) = rest.split_once("=>") else {
                panic!("a `Page::{page}` line in the dispatch is not a match arm: {line}");
            };
            arms.push(Arm {
                page,
                body: body.trim().to_string(),
            });
        }
    }
    arms
}

/// The page variants `state.rs` declares, from `Page::ALL`.
///
/// Only used to check the parse above for completeness. `match` on `Page` is
/// exhaustive, so the dispatch has exactly one arm per variant and this list
/// must have the same length: if the parser loses an arm to a formatting change
/// the two disagree and the test fails, rather than quietly pinning a subset.
fn declared_pages(state_rs: &str) -> Vec<String> {
    let anchor = "pub const ALL: [Page;";
    let start = state_rs
        .find(anchor)
        .expect("state.rs no longer declares `pub const ALL: [Page; N]`")
        + anchor.len();
    let rest = &state_rs[start..];
    let body = &rest[..rest
        .find("];")
        .expect("`Page::ALL`'s array literal is not closed with `];`")];

    let mut pages: Vec<String> = Vec::new();
    let mut cursor = body;
    while let Some(at) = cursor.find("Page::") {
        cursor = &cursor[at + "Page::".len()..];
        let name: String = cursor.chars().take_while(|c| c.is_alphanumeric()).collect();
        if !name.is_empty() && !pages.contains(&name) {
            pages.push(name);
        }
    }
    assert!(
        !pages.is_empty(),
        "parsed `Page::ALL` as empty; the array's shape changed and this test's \
         completeness check is not checking anything"
    );
    pages
}

/// The checkout's `crates/app/src/main.rs` and `state.rs`.
fn sources() -> (String, String) {
    assert!(
        !file!().ends_with("main.rs"),
        "this test now lives in {} — the file it scans. Any literal written into \
         it would then be read back as if it were application code, which is the \
         self-matching defect this test is built to avoid. Move it back under \
         crates/app/tests/, or rewrite it to assert against a set built at runtime.",
        file!()
    );
    let root = repo_root();
    assert_right_tree(&root);
    let src = root.join("crates/app/src");
    let read = |name: &str| {
        let path = src.join(name);
        fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "cannot read {}: {err}. `assert_right_tree` has already established \
                 that the checkout is there and is the one cargo is testing, so this \
                 is the read itself failing — permissions, or the tree was changed \
                 under the running test.",
                path.display()
            )
        })
    };
    (read("main.rs"), read("state.rs"))
}

/// The parser finds one arm per page. A regression here means the test below
/// would be asserting over a list that lost entries.
#[test]
fn the_page_dispatch_has_one_arm_per_page() {
    let (main_rs, state_rs) = sources();
    let arms = page_dispatch(&main_rs);
    let found: Vec<String> = arms.iter().map(|arm| arm.page.clone()).collect();
    let declared = declared_pages(&state_rs);
    assert_eq!(
        found, declared,
        "the arms parsed out of the page dispatch in crates/app/src/main.rs and \
         the pages declared in `Page::ALL` disagree. Either the dispatch changed \
         shape and this test's parser needs updating — in which case it is not \
         currently checking anything and must not be left green — or a page was \
         added or removed."
    );
}

/// **The pin.** The pages that render a placeholder are exactly the ones named
/// in [`PINNED_PENDING`].
#[test]
fn pending_pages_match_the_pinned_set() {
    let (main_rs, _) = sources();
    let arms = page_dispatch(&main_rs);

    // At T-19 both sides of the comparison below are empty — `PINNED_PENDING`
    // is `[]` and no arm calls `pending_page` — so a set equality would be
    // `[] == []` and this test could never fail again. That is not a reason to
    // delete it (it is the pin that makes landing a page an edit), but the
    // vacuity has to be held off by something, and this is it: the parse must
    // have found the dispatch. With a non-empty `arms`, "no arm is pending" is
    // a statement about the arms that were read; without one it is a statement
    // about nothing, and reads identically.
    //
    // `the_page_dispatch_has_one_arm_per_page` above also fails on a parse that
    // loses arms, by comparing against `Page::ALL`. This is the same guard in
    // the test that needs it, so the pin stands on its own rather than on a
    // neighbour the next refactor may reorganise.
    assert!(
        !arms.is_empty(),
        "parsed no arms out of the page dispatch in crates/app/src/main.rs, so \
         the set comparison below is empty against empty and cannot fail. \
         `PINNED_PENDING` is {} and every page would read as ported off a parse \
         that found nothing.",
        PINNED_PENDING.len()
    );

    let mut actual: Vec<&str> = arms
        .iter()
        .filter(|arm| arm.is_pending())
        .map(|arm| arm.page.as_str())
        .collect();
    actual.sort_unstable();

    let mut pinned: Vec<&str> = PINNED_PENDING.to_vec();
    pinned.sort_unstable();

    if actual == pinned {
        return;
    }

    let mut report = String::new();
    let landed: Vec<&str> = pinned
        .iter()
        .copied()
        .filter(|p| !actual.contains(p))
        .collect();
    let new: Vec<&str> = actual
        .iter()
        .copied()
        .filter(|a| !pinned.contains(a))
        .collect();
    if !landed.is_empty() {
        report.push_str("\n  PORTED, but still listed in PINNED_PENDING — delete the line: ");
        for page in &landed {
            let body = arms
                .iter()
                .find(|arm| arm.page.as_str() == *page)
                .map(|arm| arm.body.as_str())
                .unwrap_or("<no arm in the dispatch>");
            report.push_str(&format!("\n    {page} => {body}"));
        }
    }
    if !new.is_empty() {
        report.push_str("\n  still a placeholder, but not in PINNED_PENDING — add the line: ");
        for page in &new {
            let task = arms
                .iter()
                .find(|arm| arm.page.as_str() == *page)
                .and_then(Arm::task)
                .unwrap_or("<no task recorded>");
            report.push_str(&format!(
                "\n    {page} (renders the placeholder for {task})"
            ));
        }
    }

    panic!(
        "the set of pages rendering a placeholder no longer matches PINNED_PENDING \
         in {}.\n\
         \n\
         pinned: {pinned:?}\n\
         actual: {actual:?}\n\
         {report}\n\
         \n\
         This list is edited by hand on purpose: landing a page means removing its \
         name from it, in the same commit. T-19 is the task that empties it. If you \
         are landing a page and this is the only thing still red, deleting the line \
         is the fix.",
        file!()
    );
}
