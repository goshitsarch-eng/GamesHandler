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
//! and asks two questions of each arm of the page dispatch: does it call
//! `pending_page`, and does it call one of this crate's view modules.
//!
//! The first is the *pin*. It reads one spelling — `pending_page` was the single
//! generator of every placeholder body — and a page re-stubbed under another
//! name (`todo_page(Page::Library)`, `container(text::body("Coming soon"))`) has
//! no `pending_page(` in it. That is a claim that was true only for the spelling
//! the guard knew, which is BUG-17: an absence is a check only if the thing
//! absent cannot change its name. It *has* changed names once already — the
//! placeholder function is gone from `main.rs` entirely — and a property of the
//! present would have survived that.
//!
//! So the second question is the repair, and it is asked as a property of the
//! arm rather than as the absence of a spelling: every arm of the dispatch must
//! contain a `view::<module>::view(` call whose module `view/mod.rs` declares
//! ([`every_page_arm_draws_a_view_module`]). A placeholder is then nothing this
//! file has to recognise — it is anything that fails to be a page. The pin above
//! is kept beside it, because the two are different claims: the pin reports
//! *which* pages are placeholders against a hand-edited list, and the property
//! reports that none of them is.
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
//! not an arm either — and neither does prose count as a *call* inside an arm,
//! because the arm's body is blanked before it is searched ([`blank`]).
//!
//! Three further guards, because a parser that silently finds nothing is the
//! defect class this whole file exists to catch:
//!
//! - the arms found are compared against `Page::ALL` in `state.rs`, so a parse
//!   that loses arms fails loudly instead of asserting over a shorter list;
//! - `file!()` is asserted not to end in `main.rs`, so moving this test into
//!   the file it scans is an error rather than a self-fulfilling pass;
//! - an arm's body is the arm's *own* body — brace-matched, not the remainder
//!   of its first line and not everything up to the next arm — because five of
//!   the six arms bind their page into a local and call `view::…::view(page)`
//!   at the end of a block. Read from the first line, every one of those arms
//!   has the body `{`, and a placeholder one line further down is invisible.
//!   That is the half of BUG-17 the property check above cannot reach on its
//!   own: it would still find no `view::<module>::view(` and fail, but for the
//!   wrong reason and without naming the arm.

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
    /// The arm's whole body: everything after its `=>`, to the arm's own closing
    /// brace for a block arm and to the end of the line otherwise.
    ///
    /// It used to be the remainder of the `Page::X =>` **line**, which made
    /// every block arm's body the single character `{` — the shape BUG-17(a)
    /// records. The body is brace-matched now ([`arm_body`]) for the reason the
    /// module doc gives.
    body: String,
}

impl Arm {
    /// The arm's body with every comment, string and char literal blanked.
    ///
    /// Searches run over this rather than over [`Arm::body`] because a *mention*
    /// is not a call: the doc comment above `view_body` in `main.rs` quotes
    /// `pending_page`, and the same body contains `"Pending pages"`-style
    /// literals in places. Only blanked text can tell the two apart, and
    /// `view/form.rs` and `tests/dispatch_coverage.rs` keep lexers for the same
    /// reason.
    fn code(&self) -> String {
        blank(&self.body)
    }

    /// Whether this arm renders the not-yet-ported placeholder.
    ///
    /// The whole point of the test: `pending_page` was the single generator of
    /// every placeholder body, so "does this arm call it" is the question. It is
    /// searched for anywhere in the arm rather than at its start — see
    /// [`Arm::body`] — which is what makes a placeholder one line into a block
    /// arm visible.
    ///
    /// **This cannot see a placeholder under another name**, which is BUG-17(a)
    /// and is not something a wider search here could fix: an absence is a check
    /// only for the spellings it knows. [`every_page_arm_draws_a_view_module`]
    /// is the repair, and it is a separate test because it asks a separate
    /// question.
    fn is_pending(&self) -> bool {
        self.code().contains("pending_page(")
    }

    /// The module of the `view::<module>::view(` call this arm makes, if any.
    ///
    /// The whole arm is searched, not its head: five of the six arms bind their
    /// page into a local and call `view::…::view(page)` at the end of the block,
    /// so a check on the opening line finds nothing in any of them.
    fn rendered_module(&self) -> Option<String> {
        let code = self.code();
        let mut rest = code.as_str();
        while let Some(at) = rest.find("view::") {
            rest = &rest[at + "view::".len()..];
            let Some((module, _)) = rest.split_once("::view(") else {
                continue;
            };
            // The *last* path segment before `::view(`: `crate::view::library::view(`
            // must read as `library`, not as `view::library`.
            let module = module.rsplit("::").next().unwrap_or(module);
            if !module.is_empty() && module.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Some(module.to_string());
            }
        }
        None
    }

    /// The task id, when the arm is a placeholder and names one.
    ///
    /// Read out of the *raw* body at the placeholder call's own offset, because
    /// the id is a string literal and [`Arm::code`] has blanked those. The
    /// offsets agree: [`blank`] replaces a blanked character with as many spaces
    /// as it occupied, so a byte offset found in the blanked text indexes the
    /// same place in the original.
    fn task(&self) -> Option<&str> {
        let at = self.code().find("pending_page(")?;
        let rest = &self.body[at..];
        rest.split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(task, _)| task)
    }
}

/// The arms of the page dispatch in `main.rs`, in source order.
///
/// Finds the match on `self.state.page` and takes the arms under it until the
/// indentation drops back out of the match. Indentation, not brace counting, is
/// what finds the *arms*: the match may contain nested `match`es and blocks, and
/// a depth counter started at the wrong brace would drift. Each arm's *body* is
/// then brace-matched from its own `{` ([`arm_body`]), which is a bounded
/// problem — the arm's own braces are balanced — and is what makes a placeholder
/// anywhere inside a block arm visible.
///
/// Everything is done on a blanked copy of the source ([`blank`]), so a `{`
/// inside a string literal cannot end an arm early and a `pending_page(` quoted
/// in a comment cannot be read as a call. Byte offsets are preserved, so a range
/// found there is cut out of the original.
fn page_dispatch(source: &str) -> Vec<Arm> {
    let code = blank(source);
    let anchor = ["match self.state.page", "match self.page"]
        .into_iter()
        .find(|needle| code.contains(needle));
    let Some(anchor) = anchor else {
        panic!(
            "could not find the page dispatch (a `match self.state.page` or \
             `match self.page`) in crates/app/src/main.rs. This test parses that \
             match; until it is taught the new shape it would assert over an empty \
             list and pass for the wrong reason, so it fails here instead."
        );
    };

    let lines: Vec<&str> = code.lines().collect();
    // The byte offset each line starts at, found from the text rather than
    // assumed to be `line.len() + 1`: `str::lines` strips a `\r` it finds, and a
    // computed offset that is one short per line would drift into the middle of
    // the file by the bottom of the match.
    let mut starts = vec![0usize];
    for (index, byte) in code.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }

    let start = lines
        .iter()
        .position(|line| line.contains(anchor))
        .expect("the anchor was just found in the same text");
    let indent = |line: &str| line.len() - line.trim_start().len();
    let match_indent = indent(lines[start]);

    let mut arms = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
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
            if rest.split_once("=>").is_none() {
                panic!("a `Page::{page}` line in the dispatch is not a match arm: {line}");
            }
            let (from, to) = arm_body(&code, starts[index]);
            arms.push(Arm {
                page,
                body: source[from..to].trim().to_string(),
            });
        }
    }
    arms
}

/// The byte range, in `code`'s coordinates, of the body of the arm that begins
/// on the line at `line_start`: from just after the `=>` to and including the
/// arm's own closing brace, or to the end of the line for a body that is a
/// single expression.
///
/// `code` must be **blanked** ([`blank`]): the brace walk below counts every `{`
/// it sees, and a brace inside a string literal or a comment is not a brace.
/// That is the whole reason this is a walk over blanked text rather than a scan
/// of the raw arm.
fn arm_body(code: &str, line_start: usize) -> (usize, usize) {
    let rest = &code[line_start..];
    let arrow = rest
        .find("=>")
        .expect("`page_dispatch` only calls this for a line it has seen a `=>` on");
    let body_start = line_start + arrow + 2;

    // The end of the line, which is where a single-expression arm stops.
    let line_end = rest
        .find('\n')
        .map(|at| line_start + at)
        .unwrap_or(code.len());

    let bytes = code.as_bytes();
    let mut at = body_start;
    while at < code.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    if bytes.get(at) != Some(&b'{') {
        return (body_start, line_end);
    }

    let mut depth = 0i32;
    while at < code.len() {
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (body_start, at + 1);
                }
            }
            _ => {}
        }
        at += 1;
    }
    panic!(
        "the arm body starting at byte {body_start} of crates/app/src/main.rs is \
         not closed with a `}}`. An unterminated literal in the arm would do this: \
         `blank` leaves an unterminated string running to the end of the file."
    );
}

/// `source` with every comment, string literal and char literal blanked to
/// spaces. Newlines are kept, and **byte offsets are preserved** — a blanked
/// character becomes as many spaces as it occupied — so a range found in the
/// result indexes the original.
///
/// Two things here are load-bearing, and both are traps this file has to hold
/// off:
///
/// - **literals and comments are blanked** because arm bodies are found by
///   counting braces, and a `}` inside a string would end an arm early; and
///   because the prose in `main.rs` above the dispatch *names* both
///   `pending_page` and `view::…::view(`, so a scanner reading raw text finds
///   its own documentation first. `view/form.rs` and `tests/dispatch_coverage.rs`
///   each keep a lexer for the same reason.
/// - **lifetimes are not char literals.** Every view signature in this crate is
///   `Element<'a, Message>`; a lexer that takes every `'` for an opener blanks
///   the rest of the line and the parse silently loses arms.
///   `view/form.rs`'s `char_literal_at` is the same rule, and its doc records
///   the measured failure that put it there.
fn blank(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars[index] != '\n' {
                push_blank(&mut out, chars[index]);
                index += 1;
            }
        } else if current == '/' && chars.get(index + 1) == Some(&'*') {
            // The terminator is consumed with the body, so a `*/` that never
            // arrives leaves the scanner running to the end rather than
            // panicking.
            while index < chars.len()
                && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
            {
                push_blank(&mut out, chars[index]);
                index += 1;
            }
            for _ in 0..2 {
                if index < chars.len() {
                    push_blank(&mut out, chars[index]);
                    index += 1;
                }
            }
        } else if current == '"' {
            push_blank(&mut out, current);
            index += 1;
            while index < chars.len() && chars[index] != '"' {
                if chars[index] == '\\' {
                    push_blank(&mut out, chars[index]);
                    index += 1;
                }
                if index < chars.len() {
                    push_blank(&mut out, chars[index]);
                    index += 1;
                }
            }
            if index < chars.len() {
                push_blank(&mut out, chars[index]);
                index += 1;
            }
        } else if let Some(length) = char_literal_at(&chars, index) {
            for _ in 0..length {
                push_blank(&mut out, chars[index]);
                index += 1;
            }
        } else {
            out.push(current);
            index += 1;
        }
    }
    out
}

/// One blanked character: spaces of the same byte length, or the newline — a
/// newline has to survive or the line structure the caller walks is gone.
fn push_blank(out: &mut String, character: char) {
    if character == '\n' {
        out.push('\n');
    } else {
        for _ in 0..character.len_utf8() {
            out.push(' ');
        }
    }
}

/// The length of the char literal starting at `index`, or `None` when
/// `chars[index]` is not a quote or the `'` opens a lifetime (`&'a str`).
///
/// Transcription of `view/form.rs`'s function of the same name, which is where
/// the rule and its failure were worked out; the note there is the record.
fn char_literal_at(chars: &[char], index: usize) -> Option<usize> {
    if chars.get(index) != Some(&'\'') {
        return None;
    }
    match (
        chars.get(index + 1),
        chars.get(index + 2),
        chars.get(index + 3),
    ) {
        (Some('\\'), Some(_), Some('\'')) => Some(4),
        (Some(_), Some('\''), _) => Some(3),
        _ => None,
    }
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

/// The view modules `crates/app/src/view/mod.rs` declares.
///
/// Read rather than listed, for the reason `tests/dispatch_coverage.rs` gives
/// for its own copy of this parse: a hand-kept list of module names would have
/// to be edited by the same person who forgot to edit it, and the guard would
/// read a module that no longer exists as one that does.
fn view_modules(mod_rs: &str) -> Vec<String> {
    mod_rs
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("pub mod ")
                .and_then(|rest| rest.strip_suffix(';'))
                .map(str::to_string)
        })
        .collect()
}

/// The checkout's `crates/app/src/main.rs`, `state.rs` and `view/mod.rs`.
fn sources() -> (String, String, String) {
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
    let read = |relative: &str| {
        let path = src.join(relative);
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
    (read("main.rs"), read("state.rs"), read("view/mod.rs"))
}

/// The parser finds one arm per page. A regression here means the test below
/// would be asserting over a list that lost entries.
#[test]
fn the_page_dispatch_has_one_arm_per_page() {
    let (main_rs, state_rs, _) = sources();
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
    let (main_rs, _, _) = sources();
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

/// **Every page arm draws one of this crate's view modules.**
///
/// The *positive* half of the pin above, and BUG-17(a)'s repair.
/// [`Arm::is_pending`] asks whether an arm calls `pending_page`, which is an
/// absence — and an absence is a check only for the spellings the reader already
/// knows. `PINNED_PENDING` and `is_pending` between them knew one, and the same
/// page body written `todo_page(Page::Library)` or
/// `container(text::body("Coming soon"))` has no `pending_page(` in it, so the
/// pin classified it as **ported** and passed. The function itself is gone from
/// `main.rs` now, which does not retire the shape: a re-stubbed page has no
/// spelling to reuse and would be written as something new.
///
/// So this asks what an arm *does* render. Every arm of the dispatch must
/// contain a `view::<module>::view(` call whose module `view/mod.rs` declares,
/// which makes a placeholder nothing this test has to recognise — it is anything
/// that fails to be a page.
///
/// What it does not establish, stated rather than implied:
///
/// - an arm that calls a real view and *also* draws a placeholder passes here.
///   The pin above is what reads `pending_page`; this reads the property. They
///   are separate tests so neither can be read as the other.
/// - an arm that builds its page **inline**, without going through
///   `view::<module>::view`, fails here — a false positive. That is the
///   direction to fail in, and it is also the dispatch's own documented rule:
///   every arm draws a view module.
/// - a **name** is all that is checked. Whether `view::installers::view` is that
///   page's view, or is a second placeholder wearing the module's name, is not
///   something a text reader can settle. The rendered-body tests in `main.rs`
///   (`the_installers_page_draws_the_catalog_and_not_the_placeholder` and its
///   siblings) are what read that, one page at a time.
/// - the module has to be *declared*, not *reachable*: `view/mod.rs` gains a
///   `pub mod` for a file that does not exist and this stays green. The compiler
///   is what fails on that.
#[test]
fn every_page_arm_draws_a_view_module() {
    let (main_rs, _, view_mod_rs) = sources();
    let modules = view_modules(&view_mod_rs);

    // Both floors, for the reason the pin gives above: an empty list on either
    // side turns the loop below into a statement about nothing. The view
    // modules floor is the one that matters here — with `modules` empty, every
    // arm would fail with a message claiming the module is not declared, which
    // would send the reader to `main.rs` for a defect in `view/mod.rs`.
    assert!(
        !modules.is_empty(),
        "parsed no `pub mod` out of crates/app/src/view/mod.rs, so the check below \
         could not accept any arm even if it named a real module"
    );
    let arms = page_dispatch(&main_rs);
    assert!(
        !arms.is_empty(),
        "parsed no arms out of the page dispatch in crates/app/src/main.rs, so the \
         check below asserts nothing"
    );

    for arm in &arms {
        let Some(module) = arm.rendered_module() else {
            panic!(
                "{page} does not call `view::<module>::view(`, so it draws no page \
                 this crate has a module for. Every arm of the dispatch draws a real \
                 page — that is the dispatch's own rule, and it is asserted rather \
                 than the absence of one spelling because a placeholder under a new \
                 name has exactly this shape.\n\
                 \n\
                 arm: {page} => {body}\n\
                 \n\
                 modules declared by view/mod.rs: {modules:?}",
                page = arm.page,
                body = arm.body,
            );
        };
        assert!(
            modules.contains(&module),
            "{page} renders `view::{module}::view(`, but `view/mod.rs` declares no \
             module {module:?} — so the arm calls something that is not a view \
             module of this crate.\n\
             \n\
             arm: {page} => {body}\n\
             \n\
             modules declared by view/mod.rs: {modules:?}",
            page = arm.page,
            body = arm.body,
        );
    }
}
