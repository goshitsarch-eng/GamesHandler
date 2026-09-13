//! Every worker-to-UI send either goes through `report` or is one of the
//! three documented `let _ =` sends (BUG-28).
//!
//! The defect this guards was six `let _ =` sends on worker threads, two of
//! which carried *why* a launch failed: `unbounded_send` fails only once the
//! receiver has dropped — the window closed or the task was cancelled — so a
//! discarded send evaporates with no record at all, not even the `eprintln!`
//! the CLI reports through. The fix routes every outcome send through
//! `crate::report`, which retries the send and, on failure, leaves the message
//! on stderr.
//!
//! # The rule
//!
//! In production code under `src/`, `unbounded_send` may appear only:
//!
//!   * inside `report`'s own body — it is the sink; or
//!   * in a `let _ =` send constructing `Message::RunnerProgress`,
//!     `Message::EasyInstallProgress`, or `Message::Notify`.
//!
//! The three names are the exemption list, and it is deliberate rather than
//! short: the two progress sends fire once per archive chunk, so a dead
//! receiver logged once per chunk is spam rather than a record, and `Notify`
//! announces a window that is precisely what a gone receiver means is absent.
//! An outcome send — anything carrying a result or a failure text — has no
//! entry here and so must go through `report`.
//!
//! # Why a scanner and not a unit test
//!
//! `report`'s own contract is pinned in `main.rs`'s test module (delivery on a
//! live channel, no panic on a dead one). What that cannot see is a caller
//! reverting to `let _ =` — the regression this file exists for — because the
//! helper would still pass its own test while nobody uses it. This project
//! enforces that class of rule by reading the source (`dispatch_coverage.rs`,
//! `page_roster.rs`, `wiring_claims.rs`), and this is the same shape: the rule
//! is about *call sites*, so the call sites are what is scanned.
//!
//! # What it cannot see
//!
//! A send that swallows the error without `let _ =` — `if send.is_err() {}`,
//! `match send { _ => {} }` — is the same loss in a costume this guard does
//! not read. It also cannot see a `let _ =` send of an outcome variant that
//! was *also* given a whitelist-sounding name. Both err toward passing, which
//! is why the whitelist is three names rather than a pattern: a new outcome
//! variant cannot accidentally match it.

use std::fs;
use std::path::{Path, PathBuf};

/// The `let _ =`-sendable variants, with the reason each drop is correct.
///
/// An entry here is a hole in the guard, so it carries its justification
/// where a reader would find it. Three entries, and the list is meant to stay
/// short: adding one is the statement that a message may vanish without a
/// record, which for an outcome-carrying variant is the defect itself.
const DROPPABLE: [(&str, &str); 3] = [
    (
        "RunnerProgress",
        "one message per archive chunk; a dead receiver logged per chunk is spam",
    ),
    (
        "EasyInstallProgress",
        "one message per archive chunk; a dead receiver logged per chunk is spam",
    ),
    (
        "Notify",
        "announces the vendor wizard's window; a gone receiver is the window being gone",
    ),
];

/// The repository root, derived rather than hardcoded.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/app has a parent")
        .parent()
        .expect("crates has a parent")
        .to_path_buf()
}

/// Every `.rs` file under `src/`, recursively, sorted so the failure message
/// is stable.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in
            fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo_root().join("crates/app/src"), &mut out);
    out.sort();
    out
}

/// One source line with its line comments and string literals removed — the
/// same lexer `dispatch_coverage.rs` documents: a variant named in a comment
/// must not read as a send, and a brace inside a string must not move the
/// depth counters.
fn code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push_str("\"\"");
            }
            '/' if chars.peek() == Some(&'/') => break,
            _ => out.push(c),
        }
    }
    out
}

/// Net brace depth of one line, ignoring braces in comments and strings.
fn depth_delta(line: &str) -> i32 {
    let c = code(line);
    c.matches('{').count() as i32 - c.matches('}').count() as i32
}

/// The source's production lines: comments and strings stripped, everything
/// from the first top-level `#[cfg(test)]` removed.
///
/// Every module in this crate puts its tests in a `#[cfg(test)]` block, and
/// tests may construct sends to assert handlers behave — none of which is a
/// worker send. Cutting at the first `#[cfg(test)]` *at depth zero* is
/// `production_src`'s rule from `dispatch_coverage.rs`, because a module that
/// puts a lone `#[cfg(test)] fn` mid-file must not be truncated at it.
fn production_lines(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for line in src.lines() {
        if depth == 0 && code(line).trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        out.push(code(line));
        depth += depth_delta(line);
    }
    out
}

/// The name in `fn NAME`, `pub fn NAME`, `pub(crate) fn NAME` written at
/// column zero, or `None`. Used to know which function a send line sits
/// inside: `report` is the one place `unbounded_send` may appear unadorned.
fn top_level_fn(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = line
        .strip_prefix("fn ")
        .or_else(|| line.strip_prefix("pub fn "))
        .or_else(|| line.strip_prefix("pub(crate) fn "))?;
    rest.split(['(', '<', ' ']).next()
}

/// What one file's scan found.
struct Scan {
    /// `file:line` for each `unbounded_send` that is neither `report`'s own
    /// send nor a `let _ =` send of a [`DROPPABLE`] variant.
    violations: Vec<String>,
    /// `unbounded_send` occurrences inside `fn report` — must be at least one,
    /// or the sink is gone and every `report(...)` call site is unverifiable.
    sink_sends: usize,
    /// `let _ =` sends naming a [`DROPPABLE`] variant — must be at least one,
    /// or the classifier has stopped matching.
    dropped_sends: usize,
}

/// Run the rule over one source file. Separated from the directory walk so the
/// mutation test can feed a doctored `main.rs` through the real classifier
/// rather than re-expressing the rule as a second, weaker predicate.
fn scan_source(rel: &str, src: &str) -> Scan {
    let mut scan = Scan {
        violations: Vec::new(),
        sink_sends: 0,
        dropped_sends: 0,
    };
    let mut current_fn = String::new();
    for (index, line) in production_lines(src).iter().enumerate() {
        if let Some(name) = top_level_fn(line) {
            current_fn = name.to_string();
        }
        if !line.contains("unbounded_send") {
            continue;
        }
        if current_fn == "report" {
            scan.sink_sends += 1;
            continue;
        }
        let droppable = line.contains("let _ =")
            && DROPPABLE
                .iter()
                .any(|(variant, _)| line.contains(&format!("Message::{variant}")));
        if droppable {
            scan.dropped_sends += 1;
        } else {
            scan.violations.push(format!("{rel}:{}", index + 1));
        }
    }
    scan
}

#[test]
fn every_worker_send_reports_or_is_documented_droppable() {
    let mut violations = Vec::new();
    let mut sink_sends = 0;
    let mut dropped_sends = 0;
    for path in sources() {
        let rel = path
            .strip_prefix(repo_root())
            .unwrap_or(&path)
            .display()
            .to_string();
        let src = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let scan = scan_source(&rel, &src);
        violations.extend(scan.violations);
        sink_sends += scan.sink_sends;
        dropped_sends += scan.dropped_sends;
    }
    assert!(
        sink_sends >= 1,
        "no `unbounded_send` was found inside `fn report` — the sink is gone \
         or renamed, and this guard cannot tell a `report(...)` call site from \
         anything else without it"
    );
    assert!(
        dropped_sends >= 1,
        "no `let _ =` send of a DROPPABLE variant was found — either the \
         progress sends were rerouted (fine: delete this arm of the assert) or \
         the scan has stopped matching"
    );
    assert!(
        violations.is_empty(),
        "worker-to-UI sends that neither go through `report` nor name a \
         DROPPABLE variant:\n  {}\nAn outcome send dropped with `let _ =` is a \
         failure with no record (BUG-28). Route it through `report`, or — if \
         it is genuinely a progress-or-notice send — add the variant to \
         DROPPABLE with its justification.",
        violations.join("\n  ")
    );
}

/// The guard must fire on the defect it exists for, not merely pass on the
/// fixed tree.
///
/// The mutation is the send the finding names — `launch_and_watch`'s
/// `LaunchWatchFinished`, which carries why a launch died — reverted to the
/// bare `let _ =` it used to be, fed through the real classifier. A correct
/// guard names the line; a guard that only ever sees the fixed tree proves
/// nothing.
#[test]
fn the_guard_fires_on_a_reverted_send() {
    let main = repo_root().join("crates/app/src/main.rs");
    let src = fs::read_to_string(&main).expect("main.rs reads");
    let needle = "report(\n        sender,\n        Message::LaunchWatchFinished {";
    assert!(
        src.contains(needle),
        "`launch_and_watch` no longer sends `LaunchWatchFinished` through \
         `report` — if that call moved or changed shape, move this mutation's \
         needle with it rather than let this test pass against a tree it was \
         not written for"
    );
    // The pre-fix shape: `let _ = sender.unbounded_send(Message::
    // LaunchWatchFinished { … },);` — the trailing comma is the one
    // `report(sender, …)` left behind, and it is legal Rust, which is why the
    // splice is one substitution and not an edit.
    let mutated = src.replacen(
        needle,
        "let _ = sender.unbounded_send(Message::LaunchWatchFinished {",
        1,
    );
    let scan = scan_source("main.rs (mutated)", &mutated);
    assert_eq!(
        scan.violations.len(),
        1,
        "reverting `LaunchWatchFinished` to a bare `let _ =` must be exactly \
         one violation; got {:?}",
        scan.violations
    );
}
