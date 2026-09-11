//! A page that draws a control must have somewhere for that control to land.
//!
//! Finding #65: `view/library.rs` emits `Message::OpenNewGameForm` from the
//! Library's empty state, `Shell::update` handled it with `{}`, and no view
//! rendered the form the message was supposed to open. The button was dead, and
//! `Library` had already left `PINNED_PENDING` — because that list answers *"does
//! this body say 'not ported'?"*, which is a different question from *"is this
//! page finished?"*. A placeholder is visible; a wired-looking control that
//! emits into an empty arm is not.
//!
//! So this test asks the second question directly, for each page `main.rs`
//! **dispatches to a real body**:
//!
//! > every `Message` variant that page's view module constructs has a non-empty
//! > handler in `Shell::update`.
//!
//! # Why dispatch, and not module existence
//!
//! The guard keys on the page dispatch, not on which view modules exist. A page
//! still routed through `pending_page` is *correctly* deferred — it says so on
//! screen — so the guard does not fire on the messages its view module builds.
//! When such an arm changes to a real `view::<module>::view` body the guard
//! starts covering that page with no edit here; a per-page list would need
//! remembering, and this does not.
//!
//! That is also the guard's escape hatch: a page that is not finished should go
//! back behind `pending_page`, which is visible to a user, rather than ship an
//! inert control.
//!
//! # What it cannot see, stated rather than implied
//!
//! This is a text parser over source, not a compiler pass. Its limits, all of
//! them measured against this revision rather than assumed, and each one found
//! by the guard reporting something that was not a defect:
//!
//!   * **Comments and string literals are stripped before any scan.** Without
//!     that, `view/runners.rs`'s `// TODO(T-11): … until a \`Message::OpenUrl\`
//!     exists` reads as an emission of a variant that does not exist — and it
//!     did, on the first working version.
//!   * **`#[cfg(test)]` modules are cut, at brace depth zero.** Tests in these
//!     files construct messages precisely to assert the page *declines* them;
//!     `runners.rs`'s `a_message_this_page_does_not_own_is_declined_rather_than_swallowed`
//!     constructs `Message::LaunchWatchTick` and is not page code. Two of the
//!     first three failures this guard reported were that decoy and
//!     `library.rs`'s `Message::SetCategoryFilter` match pattern. See
//!     [`production_src`].
//!   * **A construction cannot be told from a match pattern** that survives the
//!     test cut, since a page that destructures a `Message` in its own code
//!     names the variant the same way a constructor does. This errs toward
//!     reporting a *non*-defect, which is the safe direction, and the failure
//!     names the file so a reader can check the shape themselves.
//!   * **A page may legitimately emit nothing**, so the guard does not require
//!     emissions per page. `view/credits.rs` emits none: it is an about page
//!     whose only action is a URL link, drawn disabled until the `Message` it
//!     would send exists. An earlier version asserted per-page non-emptiness and
//!     went red on exactly that, which was the guard being wrong, not the page.
//!     The anti-vacuity check for a scanner that has stopped matching is a floor
//!     on the *total* instead — see `dead_emissions`.
//!   * **A multi-variant arm is one arm.** `Shell::update` has an or-pattern
//!     binding seven variants at once; a parser that took only the first name in
//!     each arm read that as one variant and lost six, reporting 45 arms for 51
//!     variants. [`Guard::parse`] asserts the arm patterns and the enum agree
//!     exactly, so that shape fails loudly instead.
//!   * **"Handled" means "the arm's body is not `{}`".** An arm whose body is a
//!     comment, or a `Task::none()` that does nothing, reads as handled. The
//!     guard covers the `=> {}` case, which is the one this project writes for
//!     "not yet", and it is matched by the `TODO(T-xx)` markers beside them.
//!   * **Char literals containing braces are not lexed.** No such literal
//!     appears in the files this parses at the time of writing; a `'{'` in one
//!     would skew the depth counter. If that happens the counts stop matching
//!     and the guard fails loudly rather than quietly.
//!
//! None of those limits can make this pass when it should fail: every one makes
//! it *more* likely to report a problem. The two tests here pin it from both
//! sides — the first must find nothing on the real tree, the second must find
//! exactly one more thing when an arm is re-emptied — so a parser that has
//! stopped matching, or one that reports everything, fails one of them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Variants that are deliberately `{}` in `Shell::update`, with the reason.
///
/// One entry. An entry here is a hole in the guard, so it carries its
/// justification in the same place a reader would find the exemption.
const HANDLED_ELSEWHERE: [(&str, &str); 1] = [(
    "Quit",
    "handled by `App::update`'s `if matches!(&message, Message::Quit)`, the layer that owns \
     the window; `Shell` has no window to close, so its arm is `{}` by design",
)];

/// Emissions that are dead on this tree today, with the task that owns each fix
/// — a **deferral, not an exemption**, and the difference is asserted rather
/// than promised: an entry that stops being dead fails the test and has to be
/// deleted (see `uncovered`). So the list cannot rot into a place where problems
/// go to be forgotten, which is the failure mode a list like this normally has.
///
/// It exists so this guard can land without blocking three other agents on a
/// file that is not mine to fix. Each entry is one line of debt, named on screen
/// in the failure message of everything the guard *does* catch.
///
/// Adding an entry is a decision, not housekeeping: it is the statement that a
/// user-reachable control may sit inert. Prefer fixing the arm.
///
/// **That reject branch has been observed, not just written.** No test below
/// covers it — it is an inline `assert!` that holds on a clean tree — so it was
/// run by hand: with a `("NavigateTo", "Library", …)` entry added, both guard
/// tests failed with
///
/// ```text
/// these KNOWN_DEAD entries no longer describe a dead emission: [("NavigateTo",
/// "Library", "…")]. Either the arm was handled — in which case delete the
/// entry, and thank you, this list is supposed to shrink — or the page stopped
/// emitting the message, or the page moved back behind `pending_page`. An entry
/// nothing checks is how a deferral list turns into a place where problems are
/// forgotten.
/// ```
///
/// and removing the entry restored green. Recorded here because when #65 is
/// fixed and this list has to shrink, the reader meeting that failure should
/// know it is the list working rather than the guard regressing.
const KNOWN_DEAD: [(&str, &str, &str); 1] = [(
    // variant, page, why — the task that owns it
    "OpenNewGameForm",
    "Library",
    "#65 / T-09. The Library's empty state draws `Add your first game` \
     (view/library.rs:309) and `Shell::update`'s arm is `{}` (main.rs:1043); no view renders the \
     form the button asks for. Reachable the moment the library is empty, and it does nothing.",
)];

/// The repository root, derived rather than hardcoded.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/app has a parent")
        .parent()
        .expect("crates has a parent")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// One source line with its line comments and string literals removed.
///
/// Both matter and both were measured: a comment naming a variant would
/// otherwise count as an emission, and a brace inside a string literal would
/// otherwise move the depth counters this file uses to find arm boundaries.
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
                // Kept as an empty literal rather than dropped, so `=> ""` and
                // `=> {}` stay distinguishable when emptiness is decided.
                out.push_str("\"\"");
            }
            '/' if chars.peek() == Some(&'/') => break,
            _ => out.push(c),
        }
    }
    out
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Net brace depth of one line, ignoring braces in comments and strings.
fn depth_delta(line: &str) -> i32 {
    let c = code(line);
    c.matches('{').count() as i32 - c.matches('}').count() as i32
}

/// The leading identifier of a trimmed line, if it starts with one.
fn leading_ident(line: &str) -> String {
    line.trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Every variant name declared in `pub enum <name>` in `src`.
fn enum_variants(src: &str, name: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&format!("pub enum {name}")))
        .unwrap_or_else(|| {
            panic!(
                "found no `pub enum {name}` in crates/app/src/main.rs. This test reads that \
                 declaration; until it is taught the new shape it would assert over an empty \
                 variant list and pass for the wrong reason, so it fails here instead."
            )
        });

    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut opened = false;
    for line in &lines[start..] {
        if line.contains('{') {
            opened = true;
        }
        if opened && depth == 1 {
            let ident = leading_ident(&code(line));
            if ident.chars().next().is_some_and(char::is_uppercase) {
                out.push(ident);
            }
        }
        depth += depth_delta(line);
        if opened && depth == 0 {
            break;
        }
    }
    out
}

/// One arm of `Shell::update`'s match.
struct Arm {
    /// Every variant this arm's pattern matches.
    ///
    /// More than one for an or-pattern arm. `Shell::update` has one, which binds
    /// seven variants at once:
    ///
    /// ```ignore
    /// @ (Message::FetchReleases { .. }
    /// | Message::ReleasesFetchFinished { .. }
    /// | … ) => { … }
    /// ```
    ///
    /// A parser that took only the first `Message::` in the arm read that as one
    /// arm for `FetchReleases` and silently lost the other six — measured on
    /// this revision before the change: 45 arms against 51 variants, with the
    /// six missing ones all appearing inside that one arm's text.
    variants: Vec<String>,
    /// Whether the body after `=>` is exactly `{}`.
    empty: bool,
    /// First and last line of the arm, both inclusive, so the red/green test
    /// below can rewrite exactly this arm and nothing else.
    first_line: usize,
    last_line: usize,
}

/// Every `Message::<Variant>` named in `text`, in order, duplicates kept.
fn message_idents(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("Message::") {
        rest = &rest[at + "Message::".len()..];
        let name = leading_ident(rest);
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn arm_from(body: &str, first_line: usize, last_line: usize) -> Arm {
    let (pattern, after) = body.split_once("=>").unwrap_or_else(|| {
        panic!(
            "an arm of `Shell::update` was collected with no `=>` in it — the boundary finder \
             has drifted into the surrounding code. Body was:\n{body}"
        )
    });
    let variants = message_idents(pattern);
    assert!(
        !variants.is_empty(),
        "an arm of `Shell::update` has a pattern naming no `Message::` variant. The boundary \
         finder has drifted. Body was:\n{body}"
    );
    let normalised: String = after.chars().filter(|c| !c.is_whitespace()).collect();
    Arm {
        variants,
        empty: normalised == "{}" || normalised.is_empty() || normalised == ",",
        first_line,
        last_line,
    }
}

/// The arms of the `match message` in `Shell::update`, in source order.
///
/// Boundaries are found by brace depth, not by indentation. Indentation was the
/// first attempt and it is wrong here: a struct-pattern arm such as
/// `Message::FetchCoverForForm { … } => {}` closes its `}` at the arm's own
/// indentation, so an indent-based walk treats that line as the start of the
/// next arm and attaches the following arm's body to this one. Measured on this
/// revision before the change: four arms came out with no `=>` in them at all.
fn update_arms(src: &str) -> Vec<Arm> {
    let lines: Vec<&str> = src.lines().collect();
    let fn_line = lines
        .iter()
        .position(|l| l.contains("fn update(&mut self, message: Message)"))
        .unwrap_or_else(|| {
            panic!(
                "found no `fn update(&mut self, message: Message)` in crates/app/src/main.rs. \
                 This test parses that function's match; until it is taught the new shape it \
                 would assert over an empty arm list and pass for the wrong reason."
            )
        });
    let match_line = (fn_line..lines.len())
        .find(|&i| lines[i].trim_start().starts_with("match message"))
        .unwrap_or_else(|| panic!("no `match message` inside the fn update body at line {fn_line}"));

    let base = indent(lines[match_line]);
    let mut arms = Vec::new();
    let mut i = match_line + 1;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        if indent(lines[i]) <= base {
            break;
        }
        let mut body = String::new();
        let mut depth = 0i32;
        let mut seen_arrow = false;
        let mut j = i;
        while j < lines.len() {
            let c = code(lines[j]);
            depth += depth_delta(lines[j]);
            if c.contains("=>") {
                seen_arrow = true;
            }
            body.push_str(&c);
            body.push('\n');
            if seen_arrow && depth <= 0 {
                break;
            }
            j += 1;
        }
        arms.push(arm_from(&body, i, j));
        i = j + 1;
    }
    arms
}

/// One arm of the page dispatch in `view_body`.
struct Dispatch {
    page: String,
    /// The `view::<name>::view(…)` module, when the arm renders a real body.
    /// `None` for an arm that still calls `pending_page`.
    module: Option<String>,
}

/// The arms of the page dispatch in `main.rs`.
///
/// The anchor is the statement, matched from its start: the string
/// `match self.state.page` also appears in a doc comment above the function,
/// and a `contains` search finds that comment first and parses nothing.
fn page_dispatch(src: &str) -> Vec<Dispatch> {
    let lines: Vec<&str> = src.lines().collect();
    let anchor = lines
        .iter()
        .position(|l| l.trim_start().starts_with("match self.state.page"))
        .unwrap_or_else(|| {
            panic!(
                "found no `match self.state.page` statement in crates/app/src/main.rs. The page \
                 dispatch is what this test keys coverage on; without it the guard would cover \
                 no pages and pass for the wrong reason."
            )
        });
    let base = indent(lines[anchor]);
    let arm_indent = base + 4;

    let mut out = Vec::new();
    let mut i = anchor + 1;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        if indent(lines[i]) <= base {
            break;
        }
        if indent(lines[i]) == arm_indent && lines[i].trim_start().starts_with("Page::") {
            // The arm's own line is taken unconditionally: it sits *at*
            // `arm_indent`, so a "deeper than the arm" test rejects it and
            // every arm comes out with an empty body — which reads as "no page
            // renders a real body", the vacuous pass the assertion below the
            // loop exists to catch. It caught exactly that.
            let mut body = String::from(lines[i]);
            body.push('\n');
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || indent(lines[j]) > arm_indent)
            {
                body.push_str(lines[j]);
                body.push('\n');
                j += 1;
            }
            let page = leading_ident(&lines[i].trim_start()["Page::".len()..]);
            let module = body.split_once("view::").and_then(|(_, rest)| {
                let name = leading_ident(rest);
                if body.contains(&format!("view::{name}::view(")) {
                    Some(name)
                } else {
                    None
                }
            });
            out.push(Dispatch { page, module });
            // `max(i + 1)`: a one-line arm leaves `j == i`, and `i = j` would
            // then re-read the same arm forever.
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

/// `src` up to its first top-level `#[cfg(test)]` item, and whether one was
/// found.
///
/// Every view module puts its tests in one `#[cfg(test)] mod tests` as the last
/// item, and those tests are full of decoys: `runners.rs`'s
/// `a_message_this_page_does_not_own_is_declined_rather_than_swallowed`
/// constructs `Message::LaunchWatchTick` precisely to assert the page *declines*
/// it, and `library.rs`'s `category_of` matches a `Message::SetCategoryFilter`
/// pattern. Both scan as emissions, and both are not page code — measured: they
/// were two of the three failures the first working version of this guard
/// reported, and neither is a defect.
///
/// Cutting at the first `#[cfg(test)]` **at brace depth zero** rather than at
/// the first occurrence anywhere is what keeps this honest: the attribute also
/// appears attached to a single `#[cfg(test)] fn` mid-file (in `main.rs`, among
/// others), and cutting there would throw away real code below it. If the depth
/// check never finds a top-level one, nothing is cut — and then the decoys
/// reappear and the guard fails loudly rather than passing on a short read.
fn production_src(src: &str) -> (String, bool) {
    let lines: Vec<&str> = src.lines().collect();
    let mut depth = 0i32;
    for (i, line) in lines.iter().enumerate() {
        if depth == 0 && code(line).trim_start().starts_with("#[cfg(test)]") {
            let mut cut = lines[..i].join("\n");
            if !cut.is_empty() {
                cut.push('\n');
            }
            return (cut, true);
        }
        depth += depth_delta(line);
    }
    (src.to_string(), false)
}

/// Every `Message::<Variant>` in `text`, as written — no cutting.
///
/// Comments and string literals are stripped, so a message named in prose is
/// not counted. Kept separate from [`emissions`] so that the cut can be tested
/// against an uncut scan of the same bytes; folding the two together is what
/// made the first version of that test compare a cut scan with itself.
fn scan(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        out.extend(message_idents(&code(line)));
    }
    out
}

/// Every `Message::<Variant>` constructed in a view module's production code.
///
/// Comments and string literals are stripped; the test module is cut. What
/// remains still cannot distinguish a construction from a match pattern, so a
/// view module that destructures a `Message` in its own `update` reports that
/// variant as emitted. That errs toward a false *failure*, which is the
/// direction to err in, and the failure message names the file so a reader can
/// see the shape for themselves.
fn emissions(src: &str) -> BTreeSet<String> {
    scan(&production_src(src).0)
}

/// A page that `view_body` dispatches to a real `view::<module>::view` body.
struct CoveredPage {
    page: String,
    module: String,
}

/// The parsed state the guard reasons over, so the red/green test below can run
/// the same reasoning over a mutated source instead of a second copy of it.
struct Guard {
    variants: BTreeSet<String>,
    handled: BTreeSet<String>,
    covered: Vec<CoveredPage>,
    /// Number of arms in `Shell::update`'s match. Fewer than `variants.len()`
    /// when an or-pattern binds several at once, which is exactly why it is
    /// recorded rather than recomputed by a reader.
    arm_count: usize,
}

impl Guard {
    /// Parse, asserting at every step that the parse found something.
    ///
    /// The anti-vacuity assertions live here rather than in the test so that
    /// the mutation test below cannot pass by running a parser that has quietly
    /// stopped matching (#32 / #43): a source-reading guard that finds nothing
    /// reports every page clean, and that failure mode is invisible in its
    /// output.
    fn parse(main_src: &str) -> Guard {
        let variants = enum_variants(main_src, "Message");
        let arms = update_arms(main_src);

        assert!(
            variants.len() >= 40,
            "parsed only {} variants out of `pub enum Message` — the parser has stopped \
             matching. A guard that found no variants passes everything.",
            variants.len()
        );
        let variant_set: BTreeSet<String> = variants.iter().cloned().collect();
        assert!(
            arms.len() >= 40,
            "parsed only {} arms out of `Shell::update` — the boundary finder has stopped \
             matching. A guard that found no arms calls every variant handled.",
            arms.len()
        );

        // The match is exhaustive and has no wildcard, so its arms' patterns
        // must together name every variant exactly once. `exactly` and not `at
        // least`: a variant in two arms means an arm boundary was found in the
        // wrong place — the shape that hid six variants behind the first name of
        // an or-pattern.
        let named: Vec<String> = arms.iter().flat_map(|a| a.variants.clone()).collect();
        let named_set: BTreeSet<String> = named.iter().cloned().collect();
        assert_eq!(
            named_set, variant_set,
            "the arm parser and the enum parser disagree about which variants exist. One of them \
             is matching the wrong lines."
        );
        let mut seen = BTreeSet::new();
        let duplicated: Vec<&String> = named
            .iter()
            .filter(|v| !seen.insert((*v).clone()))
            .collect();
        assert!(
            duplicated.is_empty(),
            "these variants are named by more than one arm of `Shell::update`: {duplicated:?}. \
             An exhaustive match cannot do that, so an arm boundary was found in the wrong \
             place — most likely a nested `match` inside an arm's body was read as the next arm."
        );

        let handled: BTreeSet<String> = arms
            .iter()
            .filter(|a| !a.empty)
            .flat_map(|a| a.variants.clone())
            .chain(HANDLED_ELSEWHERE.iter().map(|(v, _)| (*v).to_string()))
            .collect();

        // A page still routed through `pending_page` is correctly deferred and
        // is deliberately not covered; see the module header.
        let covered: Vec<CoveredPage> = page_dispatch(main_src)
            .into_iter()
            .filter_map(|d| {
                d.module.map(|module| CoveredPage {
                    page: d.page,
                    module,
                })
            })
            .collect();
        assert!(
            !covered.is_empty(),
            "no page in the dispatch renders a real body, so this guard would cover nothing and \
             pass trivially. Expected at least `view_body`'s non-`pending_page` arms."
        );

        Guard {
            variants: variant_set,
            handled,
            covered,
            arm_count: arms.len(),
        }
    }

    /// How many dead emissions exist, deferred ones included. Used for the
    /// figures this prints; the check itself uses [`Self::uncovered`].
    fn dead_emissions_len(&self) -> usize {
        self.dead_emissions().len()
    }

    /// `(page, module, variant)` for every message a covered page emits that
    /// nothing handles, minus the [`KNOWN_DEAD`] deferrals.
    ///
    /// The deferrals are checked in the same pass: an entry that no longer
    /// describes a dead emission fails here. That is what keeps the list
    /// honest, and it is the same rule the `PINNED_PENDING` header states — a
    /// hand-copied fact that no assertion reads has nothing to keep it true.
    fn uncovered(&self) -> Vec<(String, String, String)> {
        let dead = self.dead_emissions();
        let stale: Vec<&(&str, &str, &str)> = KNOWN_DEAD
            .iter()
            .filter(|(variant, page, _)| {
                !dead
                    .iter()
                    .any(|(p, _, v)| v == variant && p == page)
            })
            .collect();
        assert!(
            stale.is_empty(),
            "these KNOWN_DEAD entries no longer describe a dead emission: {stale:?}. Either the \
             arm was handled — in which case delete the entry, and thank you, this list is \
             supposed to shrink — or the page stopped emitting the message, or the page moved \
             back behind `pending_page`. An entry nothing checks is how a deferral list turns \
             into a place where problems are forgotten."
        );
        dead.into_iter()
            .filter(|(page, _, variant)| {
                !KNOWN_DEAD
                    .iter()
                    .any(|(v, p, _)| v == variant && p == page)
            })
            .collect()
    }

    /// Every dead emission, including the deferred ones.
    fn dead_emissions(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let mut counts: Vec<(String, usize)> = Vec::new();
        for page in &self.covered {
            let raw = read(&format!("crates/app/src/view/{}.rs", page.module));
            let (src, had_tests) = production_src(&raw);
            if had_tests {
                assert!(
                    src.len() < raw.len(),
                    "view/{}.rs has a `#[cfg(test)]` item but cutting at it did not shorten the \
                     source, so the cut is not doing what this guard assumes. The decoys in \
                     that module's tests would then scan as page code and report defects that \
                     are not there.",
                    page.module
                );
            }
            let found = emissions(&src);
            // A page emitting nothing is *not* asserted against here, and that
            // is a correction rather than an omission. The first version did
            // assert it, on the reasoning that every covered page draws
            // controls — and it went red on `view/credits.rs` when Credits
            // landed: an about page whose only action is a URL link, rendered
            // disabled because the `Message` it would send does not exist yet.
            // Zero emissions is the honest state of that page. What emptiness
            // cannot be distinguished from is a scanner that stopped matching,
            // so the check for that is global — see the floor below — where it
            // is a claim the guard can actually support.
            counts.push((page.module.clone(), found.len()));
            for variant in found {
                if self.handled.contains(&variant) {
                    continue;
                }
                // Not a variant of our `Message`: another type's, or a name
                // that reached here through a path this parser does not model.
                if !self.variants.contains(&variant) {
                    continue;
                }
                out.push((page.page.clone(), page.module.clone(), variant));
            }
        }

        // Anti-vacuity, global rather than per page (#32 / #43). A page may
        // legitimately emit nothing, so emptiness per page proves nothing; a
        // collapse across *all* of them means the scanner has stopped matching
        // and every page below is being called clean. Measured on the revision
        // this landed against: library 7, runners 7, settings 5, plugins 2,
        // credits 0 — 21 in total, so the floor is well clear of both a passing
        // and a collapsing scan.
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        assert!(
            total >= 10,
            "the emission scan found only {total} `Message::` constructions across all {} \
             covered pages (per page: {counts:?}). That is a scanner that has stopped matching, \
             not a set of pages that emit nothing — a clean tree measures 21 here, and the \
             guard would report every page as covered-and-clean off a scan this short.",
            counts.len()
        );
        out
    }
}

/// Rewrite the arm of `Shell::update` that handles `variant` so its body is
/// `{}`, the way a developer would leave it as a TODO.
///
/// The result is never compiled — the guard reads source as text — but it is
/// written in the shape the real file would take, so what the red run below
/// proves is that the guard reacts to the edit a developer actually makes.
fn empty_arm(src: &str, variant: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let arms = update_arms(src);
    let arm = arms
        .iter()
        .find(|a| a.variants.iter().any(|v| v == variant))
        .unwrap_or_else(|| {
            panic!(
                "`Shell::update` has no arm for Message::{variant}, so this test cannot re-empty \
                 it. Pick another variant — the guard's sensitivity is what is under test here, \
                 not this particular message."
            )
        });
    assert!(
        !arm.empty,
        "Message::{variant}'s arm is already `{{}}`. Re-emptying it would prove nothing: the \
         red run has to start from a handled arm."
    );
    let joined = lines[arm.first_line..=arm.last_line].join("\n");
    let pattern = joined
        .split_once("=>")
        .expect("arm_from accepted the arm, so it has a `=>`")
        .0;

    let mut out: Vec<String> = lines[..arm.first_line].iter().map(|l| (*l).to_string()).collect();
    out.push(format!("{} => {{}}", pattern.trim_end()));
    out.extend(lines[arm.last_line + 1..].iter().map(|l| (*l).to_string()));
    let mut mutated = out.join("\n");
    if src.ends_with('\n') {
        mutated.push('\n');
    }
    mutated
}

#[test]
fn every_message_a_dispatched_page_emits_has_a_handler() {
    let guard = Guard::parse(&read("crates/app/src/main.rs"));
    // Printed, not just asserted, because #65 was reported to the team as a
    // count ("N empty arms") and a count is only checkable if it is
    // reproducible. Run with `--nocapture` for the figures. They are measured
    // against whatever revision is checked out — the tree moves constantly
    // here, so a count without its commit means nothing (D-45).
    println!(
        "dispatch coverage: {} Message variants, {} match arms, {} covered pages, \
         {} dead emissions ({} deferred by KNOWN_DEAD), {} exemptions in HANDLED_ELSEWHERE",
        guard.variants.len(),
        guard.arm_count,
        guard.covered.len(),
        guard.dead_emissions_len(),
        KNOWN_DEAD.len(),
        HANDLED_ELSEWHERE.len(),
    );
    let failures = guard.uncovered();
    assert!(
        failures.is_empty(),
        "{} message(s) a dispatched page emits have an empty `Shell::update` arm. The page is \
         dispatched to a real body, so the control is reachable and does nothing:\n{}\n\n\
         Fix by handling the variant, or — if the page is not finished — put it back behind \
         `pending_page` so it says so on screen. `Message::Quit` is exempt: it is handled in \
         `App::update`, the layer that owns the window. The full exemption list is \
         {HANDLED_ELSEWHERE:?}.\n\n\
         This guard is not currently failing on {KNOWN_DEAD:?} — that is the standing debt, \
         deferred in `KNOWN_DEAD` so the guard could land without blocking the tree. It is \
         still dead.",
        failures.len(),
        failures
            .iter()
            .map(|(page, module, variant)| format!(
                "  page {page}, view/{module}.rs: Message::{variant}"
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The `#[cfg(test)]` cut, checked rather than assumed.
///
/// This is the assertion that makes the guard's greenness mean something. Its
/// three false positives so far all came from reading test code as page code,
/// and two of them — `LaunchWatchTick` and `SetInstallerSearch` — were reported
/// to this tree as findings by another agent who ran an earlier build without
/// the cut. If the cut silently stopped working, the guard would go back to
/// exactly that, and it would be *reporting*, not passing quietly, which is at
/// least visible. The dangerous direction is the other one: a cut that removes
/// too much makes pages look clean.
///
/// So both directions are pinned here, by name, on the module with real decoys:
/// the tests' decoys must be absent from the cut, and the page's own messages
/// must survive it.
#[test]
fn the_test_module_cut_removes_the_decoys_and_keeps_the_page() {
    let module = "runners";
    let raw = read(&format!("crates/app/src/view/{module}.rs"));
    let (production, had_tests) = production_src(&raw);
    assert!(
        had_tests,
        "view/{module}.rs no longer has a top-level `#[cfg(test)]` module, so this test cannot \
         check the cut. If its tests moved, point this at a module that still has them; if the \
         tests are gone, delete this test rather than leaving it asserting nothing."
    );

    // `scan` on the raw bytes, not `emissions` — `emissions` cuts, and comparing
    // it with the cut scan compares a thing with itself, which is how the first
    // version of this test passed over an empty decoy list.
    let uncut = scan(&raw);
    let cut = emissions(&raw);
    assert_eq!(
        scan(&production),
        cut,
        "scanning the cut source and scanning the raw source through `emissions` disagree, so \
         `emissions` is not `scan ∘ production_src` and this test's comparison is meaningless."
    );

    // Present in the file's text, absent from its production code. Each of
    // these is constructed by a test asserting the page *declines* it, which is
    // the opposite of an emission.
    for decoy in ["LaunchWatchTick", "SetInstallerSearch", "RefreshPlugins"] {
        assert!(
            uncut.contains(decoy),
            "view/{module}.rs no longer constructs `Message::{decoy}` anywhere, including its \
             tests, so it is no longer a decoy and this test is not checking what it claims. \
             Update the list to a message that module's tests do construct."
        );
        assert!(
            !cut.contains(decoy),
            "the `#[cfg(test)]` cut did NOT remove `Message::{decoy}` from view/{module}.rs, \
             which is only constructed in its tests. The guard is reading test code as page \
             code — that is the exact false positive this cut exists to prevent, and the two \
             variants reported against this tree from a build without the cut were \
             LaunchWatchTick and SetInstallerSearch."
        );
    }

    // And the page's own messages are still there: a cut that took the whole
    // file would satisfy every assertion above.
    for real in ["FetchReleases", "InstallRunner"] {
        assert!(
            cut.contains(real),
            "view/{module}.rs's production code no longer constructs `Message::{real}` after \
             the cut, and it is supposed to. A cut that removes too much reports pages as clean \
             — the failure mode this guards against."
        );
    }

    println!(
        "cut check: view/{module}.rs keeps {} production emissions and drops {} test-only ones",
        cut.len(),
        uncut.len() - cut.len()
    );
}

/// The guard in the test above is only worth having if it goes red on the edit
/// it exists to catch (#26). This re-empties one arm that is handled today and
/// checks the guard names that variant and the page that emits it — then checks
/// the unmutated source does *not*, in the same run, so the two halves of the
/// comparison come from one build.
///
/// It runs the same `Guard::parse` and `Guard::uncovered` as the real check, so
/// it fails if the guard is weakened into something that no longer notices a
/// re-emptied arm — including the way that would be quietest: a parser that
/// stops finding the emission at all (#32 / #43).
#[test]
fn the_guard_notices_a_re_emptied_arm() {
    let src = read("crates/app/src/main.rs");
    let guard = Guard::parse(&src);

    // Stage the red run on a *button*, not on any old message: #65 is a control
    // a user presses that does nothing, so a guard that only reacts to, say, a
    // timer tick would be proving the wrong sensitivity. Chosen by shape — a
    // `Message::` inside an `on_press(` — rather than by name, because these
    // variants get renamed (`NavigateToPage` became `NavigateTo` while this test
    // was being written) and a name-keyed test would quietly fall through to
    // something weaker.
    let live = guard.uncovered();
    let mut button_emissions: BTreeSet<String> = BTreeSet::new();
    let mut any_emission: BTreeSet<String> = BTreeSet::new();
    for p in &guard.covered {
        let (src, _) =
            production_src(&read(&format!("crates/app/src/view/{}.rs", p.module)));
        let found = emissions(&src);
        any_emission.extend(found.iter().cloned());
        for line in src.lines() {
            if line.contains("on_press(") {
                button_emissions.extend(message_idents(&code(line)));
            }
        }
    }
    let target = ["NavigateTo", "NavigateToPage"]
        .into_iter()
        .find(|t| button_emissions.contains(*t) && guard.handled.contains(*t))
        .or_else(|| {
            button_emissions
                .iter()
                .find(|v| guard.handled.contains(*v))
                .map(|s| s.as_str())
        })
        .or_else(|| {
            any_emission
                .iter()
                .find(|v| guard.handled.contains(*v))
                .map(|s| s.as_str())
        })
        .unwrap_or_else(|| {
            panic!(
                "no `Message` variant is both emitted by a covered page and handled in \
                 `Shell::update`, so this test cannot stage a red run. Either the guard's page \
                 coverage has collapsed to nothing, or every page has gone back behind \
                 `pending_page`."
            )
        });

    assert!(
        !live.iter().any(|(_, _, v)| v == target),
        "Message::{target} is already uncovered on the unmutated source, so this test cannot \
         tell a red run from the standing state. The guard is reporting: {live:?}"
    );

    let mutated = empty_arm(&src, target);
    assert_ne!(mutated, src, "empty_arm did not change the source");

    let after = Guard::parse(&mutated);
    let found = after.uncovered();
    let named: Vec<&(String, String, String)> =
        found.iter().filter(|(_, _, v)| v == target).collect();
    assert!(
        !named.is_empty(),
        "`Message::{target}`'s arm was emptied and the guard did not report it. Everything it \
         did report: {found:?}. The guard is not sensitive to a re-emptied arm, which is the \
         one thing it exists to catch."
    );
    for (page, module, _) in &named {
        assert!(
            !page.is_empty() && !module.is_empty(),
            "the guard reported Message::{target} without naming the page or module that emits \
             it. A report a reader cannot act on is not a guard."
        );
    }

    // A control against the opposite failure: a guard that reports *everything*
    // would pass the assertions above. The mutated report must be exactly the
    // unmutated report plus the entries the mutation created — nothing else
    // moved.
    for entry in &live {
        assert!(
            found.contains(entry),
            "emptying Message::{target}'s arm made the guard stop reporting {entry:?}, which was \
             dead before the mutation. The guard's report is not a function of the source it \
             was given."
        );
    }
    assert_eq!(
        found.len(),
        live.len() + named.len(),
        "emptying one arm changed the report by more than the variants it emptied. Before: \
         {live:?}. After: {found:?}. A guard whose report moves like this is reporting \
         something other than what is dead."
    );
    println!(
        "red run: re-emptying Message::{target} was reported as {}",
        named
            .iter()
            .map(|(page, module, v)| format!("page {page} / view/{module}.rs / Message::{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
