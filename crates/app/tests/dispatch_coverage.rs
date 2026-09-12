//! A view that draws a control must have somewhere for that control to land.
//!
//! Finding #65: `view/library.rs` emits `Message::OpenNewGameForm` from the
//! Library's empty state, `Shell::update` handled it with `{}`, and no view
//! rendered the form the message was supposed to open. The button was dead, and
//! `Library` had already left `PINNED_PENDING` — because that list answers *"does
//! this body say 'not ported'?"*, which is a different question from *"is this
//! page finished?"*. A placeholder is visible; a wired-looking control that
//! emits into an empty arm is not.
//!
//! So this test asks the second question directly, for every view module
//! `main.rs` renders:
//!
//! > every `Message` variant that module's production code constructs has a
//! > non-empty handler in `Shell::update`.
//!
//! "Every view module `main.rs` renders" is computed, not listed. The roots are
//! the view modules `main.rs`'s own production code names — the pages
//! `view_body` dispatches to a real body, and the overlay
//! `view_with_overlays` composes — and the closure adds every view module those
//! call, transitively. That is what puts the shared components (`widgets`,
//! `badge`, `cover`, `meta`, `metrics`) in scope: `main.rs` never names them,
//! but the Library's cards and the Runners' rows are built from them, so a
//! control there is as reachable as one drawn by the page. **#80**: before
//! this, the covered set was the dispatch roots alone, and `view/form.rs`'s
//! `Message::FetchCoverForForm` sat behind an empty arm at `main.rs:1339` that
//! the guard could not see.
//!
//! # Why dispatch, and not module existence
//!
//! A page still routed through `pending_page` is *correctly* deferred — it says
//! so on screen — so the guard does not fire on the messages its view module
//! builds, and no covered module calls it. When such an arm changes to a real
//! `view::<module>::view` body the guard starts covering that page with no edit
//! here; a per-page list would need remembering, and this does not. Reverting a
//! page to `pending_page` is therefore the escape hatch: a page that is not
//! finished should go back behind the placeholder, which is visible to a user,
//! rather than ship an inert control.
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
//!   * **A module the guard does not cover is invisible**, and this is the
//!     limit #80 was about. Coverage is
//!     computed ([`Guard::parse`]), not listed: the roots are the view modules
//!     `main.rs`'s production code names, and the closure adds every view module
//!     those call, transitively. A module no covered module calls — and that
//!     `main.rs` does not name — is not scanned at all. `view/installers.rs` is
//!     the live example, and it is *correct* that it is out of scope: its page
//!     is still behind `pending_page`, which is the deferral this guard's design
//!     is built around. The point is the mechanism, not that instance: the same
//!     reachability rule that correctly excludes a placeholder would equally
//!     exclude a finished module nothing happened to call yet. **#80.**
//!   * **An emission constructed in `main.rs` is not scanned.** Emissions are
//!     read from view modules; `main.rs` is read for arms. A `Message::`
//!     constructed in a rendered position in `main.rs` — a button built inline
//!     rather than in a view module — is checked by nothing here.
//!
//! So the limits above do not include the claim that none of them can make this
//! pass when it should fail — that sentence is deliberately not here. **Three of
//! the nine can**: a module outside the covered set (bullet 8), an emission
//! constructed in `main.rs` (bullet 9), and a `#[cfg(test)]` cut that removes
//! more than it should (bullet 2, and it is pinned for `runners` alone, not for
//! every module). The other six err toward reporting a *non*-defect, which is
//! the safe direction. The first two are the scope of the guard rather than
//! defects in it, and they are stated where a reader meets them.
//!
//! What the tests here establish is therefore narrower, and stated rather than
//! implied: the guard finds nothing on the real tree; it finds exactly one more
//! thing when an arm is re-emptied, both for a message a dispatched page emits
//! and for one a covered-only-by-call module emits; and the deferral machinery
//! matches on the value it claims to, with a bad key reported. A parser that has
//! stopped matching, a coverage set that has collapsed back to the dispatch
//! roots, a cut that has stopped cutting, and a guard that reports everything
//! each fail one of those.

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
/// # The key is `(variant, module)`, and the module is checked
///
/// It used to be `(variant, page)`, and that was correct — but it made the two
/// halves of the key different kinds of thing, one a `Message` variant and one a
/// **page** (`"Library"`) in a file where everything else is keyed by **module**
/// (`"library"`). Two reviewers read `p == page` as a module-to-page comparison
/// and concluded the list was inert; it was not — measured at `9940f07`, the
/// last revision carrying the entry, this guard printed `1 dead emissions
/// (1 deferred by KNOWN_DEAD)` and passed 3/3 by compiling that revision's own
/// copy of this file against that revision's sources. Had the comparison really
/// never matched, the `stale` assertion below would have fired, because an entry
/// that matches nothing in `dead` is exactly what it reports.
///
/// So the mechanism was never broken and the fix offered for it was wrong — but
/// the misreading was *available*, and that is a defect in the key. The module
/// is what identifies an emission ([`Guarded::module`] is what locates the file
/// and the call site); the page is a display name. Keying on the module makes
/// the component checkable, so [`Guard::uncovered`] now asserts every entry's
/// module is one the guard actually scans: a typo, or a module that has dropped
/// out of scope, fails loudly instead of becoming an entry that can never match.
/// That is the property the reviewers thought was missing, in the form that can
/// be held.
///
/// **That reject branch has fired in anger.** It was first demonstrated by hand
/// — a `("NavigateTo", "…", …)` entry added, both guard tests failing with the
/// message below, green restored on removal. Then it fired on real work: UX
/// landed #65's handler, `OpenNewGameForm` stopped being dead, and this list
/// went stale and failed the suite naming that entry. The message is the one
/// quoted here:
///
/// ```text
/// these KNOWN_DEAD entries no longer describe a dead emission: [("OpenNewGameForm",
/// "…")]. Either the arm was handled — in which case delete the
/// entry, and thank you, this list is supposed to shrink — or the page stopped
/// emitting the message, or the page moved back behind `pending_page`. An entry
/// nothing checks is how a deferral list turns into a place where problems are
/// forgotten.
/// ```
///
/// # What that run does not establish, and why `35e8085` is red
///
/// **A green run of this guard is evidence about the sources on disk, never
/// about the commit.** `35e8085` deleted the `OpenNewGameForm` entry on the
/// strength of a green run — and it was green, in a working tree that already
/// held the handler. The handler is not in that commit; it lands four commits
/// later at `2ce15e9`. So at its own sha the arm was still `{}`, the emission
/// was still dead, and the guard **failed**, naming it. Measured by compiling
/// that revision's own copy of this file against that revision's sources:
/// `2 passed; 1 failed`, `1 dead emissions (0 deferred by KNOWN_DEAD)`. The
/// guard was right and the commit message was wrong, which is D-45 applied to a
/// message: a commit message reporting a run is a claim about a sha, and only
/// the sha can confirm it. `32f5601`, `ce1cd7a` and `e27c4ea` are red for the
/// same reason; `2ce15e9` is green because it is the repair. Nothing was
/// reverted — the current tree is green and the entry is correctly gone.
/// Empty, and U5 emptied it: `FetchCoverForForm`'s arm is written and its
/// button ungated, and `FetchCover`'s arm is written behind U2's menu item —
/// so the staleness assertion below fired on both entries and they were
/// deleted, which is this list's own rule doing its job. The const stays (at
/// length zero) so the next genuine deferral has a list to join rather than a
/// mechanism to rebuild.
const KNOWN_DEAD: [(&str, &str, &str); 0] = [];

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

/// Every view module declared by `crates/app/src/view/mod.rs`.
///
/// The closure that builds [`Guard::covered`] must not follow a name out of the
/// view layer: `super::runners::RunnerManager` inside `form.rs` is
/// `gamehandler_core::runners`, a different crate's module with the same name,
/// and treating it as a view module would put a core file through the emission
/// scanner. Intersecting with this list is what bounds the closure to the view
/// layer — measured: without it, `settings.rs` "reaches" `runners` because of
/// `use gamehandler_core::runners::RunnerManager` on line 31.
fn view_modules(src: &str) -> BTreeSet<String> {
    let mods: BTreeSet<String> = src
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("pub mod ")
                .and_then(|rest| rest.strip_suffix(';'))
                .map(|name| name.trim().to_string())
        })
        .collect();
    assert!(
        mods.len() >= 10,
        "parsed only {} view modules out of crates/app/src/view/mod.rs: {mods:?}. That file \
         declares the view layer, and the coverage closure is bounded by it; a short list means \
         the parser has stopped matching and modules would silently drop out of scope.",
        mods.len()
    );
    mods
}

/// A view module the guard checks, and how it came to be in scope.
struct Covered {
    /// The page, when `view_body` dispatches one to this module. `None` for a
    /// module rendered without being a page: an overlay, or a shared component.
    page: Option<String>,
    /// The module name, which is what locates both the file to scan and the
    /// entry in [`KNOWN_DEAD`] that may defer its emissions.
    module: String,
    /// How this module is reached, for the failure message. Three shapes:
    /// a page dispatch, a name in `main.rs`, or a call from another module.
    reached: String,
}

impl Covered {
    /// How to name this module in a failure a reader has to act on: the page it
    /// is, when it is one, and always the file. The file is not redundant for a
    /// page — the page name and the module name are different strings, which is
    /// the trap [`KNOWN_DEAD`]'s key fell into.
    fn label(&self) -> String {
        match &self.page {
            Some(page) => format!("page {page} (view/{}.rs)", self.module),
            None => format!("view/{}.rs", self.module),
        }
    }
}

/// The view modules `main.rs`'s own production code names, and how.
///
/// Anchored on `view::<name>::` so a bare `runners::` from the core crate does
/// not count, and read through [`production_src`] so the test module's
/// references (a `crate::view::form::…` constant named only from `mod tests`)
/// do not make a module look rendered when only a test names it.
fn rendered_in_main(main_src: &str, modules: &BTreeSet<String>) -> Vec<String> {
    let (prod, _) = production_src(main_src);
    let mut out = BTreeSet::new();
    for line in prod.lines() {
        let code = code(line);
        let mut rest = code.as_str();
        while let Some(at) = rest.find("view::") {
            rest = &rest[at + "view::".len()..];
            let name = leading_ident(rest);
            if modules.contains(&name) {
                out.insert(name);
            }
        }
    }
    out.into_iter().collect()
}

/// The view modules `text`'s production code calls, bounded to `modules`.
///
/// Both spellings count: `crate::view::<m>::` from anywhere, and `super::<m>::`
/// from inside a view module. `use super::cover::{self, CoverSource};` at the
/// top of `widgets.rs` is a real dependency — every card in the Library is drawn
/// through it.
fn calls_in(text: &str, modules: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let code = code(line);
        for anchor in ["crate::view::", "super::"] {
            let mut rest = code.as_str();
            while let Some(at) = rest.find(anchor) {
                rest = &rest[at + anchor.len()..];
                let name = leading_ident(rest);
                if modules.contains(&name) {
                    out.insert(name);
                }
            }
        }
    }
    out
}

/// The parsed state the guard reasons over, so the red/green test below can run
/// the same reasoning over a mutated source instead of a second copy of it.
struct Guard {
    variants: BTreeSet<String>,
    handled: BTreeSet<String>,
    covered: Vec<Covered>,
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
        let modules = view_modules(&read("crates/app/src/view/mod.rs"));
        let dispatched: Vec<(String, String)> = page_dispatch(main_src)
            .into_iter()
            .filter_map(|d| d.module.map(|module| (d.page, module)))
            .collect();
        assert!(
            !dispatched.is_empty(),
            "no page in the dispatch renders a real body, so this guard would cover nothing and \
             pass trivially. Expected at least `view_body`'s non-`pending_page` arms."
        );

        // The roots are the modules `main.rs` names, whether `view_body`
        // dispatches to them as a page or `view_with_overlays` composes them.
        // Computed from `main.rs` rather than hardcoded, so a module rendered
        // without being a page is in scope the moment it is named.
        let named = rendered_in_main(main_src, &modules);
        let mut covered: Vec<Covered> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (page, module) in &dispatched {
            if seen.insert(module.clone()) {
                covered.push(Covered {
                    page: Some(page.clone()),
                    module: module.clone(),
                    reached: "dispatched by `view_body`".to_string(),
                });
            }
        }
        for module in &named {
            if seen.insert(module.clone()) {
                covered.push(Covered {
                    page: None,
                    module: module.clone(),
                    reached: "named in `main.rs`".to_string(),
                });
            }
        }
        assert!(
            !named.is_empty(),
            "found no `view::<module>::` reference in main.rs's production code, so no module \
             could be covered and every page would read as clean. The scan anchor has stopped \
             matching."
        );

        // The closure. A shared component is reached by being called, and
        // `main.rs` never names `widgets` — the Library's cards do. Bounded to
        // `modules` so a `super::runners::RunnerManager` (the core crate's, not
        // the view layer's) cannot walk the scan into another crate.
        let mut frontier: Vec<String> = covered.iter().map(|c| c.module.clone()).collect();
        while let Some(from) = frontier.pop() {
            let (src, _) = production_src(&read(&format!("crates/app/src/view/{from}.rs")));
            for module in calls_in(&src, &modules) {
                if seen.insert(module.clone()) {
                    covered.push(Covered {
                        page: None,
                        module: module.clone(),
                        reached: format!("called from `view/{from}.rs`"),
                    });
                    frontier.push(module);
                }
            }
        }
        covered.sort_by(|a, b| a.module.cmp(&b.module));

        // Anti-vacuity for the closure, both halves. If it silently did nothing
        // the guard would be back to the dispatch roots and would still pass --
        // the exact hole #80 is about -- so the reach is asserted, not assumed.
        let from_closure: Vec<&Covered> = covered
            .iter()
            .filter(|c| c.reached.starts_with("called from"))
            .collect();
        assert!(
            !from_closure.is_empty(),
            "the coverage closure added no module: every covered module is named directly by \
             main.rs, so nothing reached through a call is in scope. That is the #80 hole \
             restored, and it would pass every other assertion here. Covered: {:#?}",
            covered.iter().map(|c| &c.module).collect::<Vec<_>>()
        );
        for must in ["widgets", "badge"] {
            assert!(
                covered.iter().any(|c| c.module == must),
                "`view/{must}.rs` is not covered, but it is reached from a covered module and is \
                 what the Library's cards and the Runners' rows are built from. A control there \
                 is as reachable as one drawn by a page, which is the whole point of the \
                 closure. Covered: {:#?}",
                covered.iter().map(|c| &c.module).collect::<Vec<_>>()
            );
        }

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

    /// `(label, module, variant)` for every message a covered module emits that
    /// nothing handles, minus the deferrals in `known_dead`.
    ///
    /// The deferrals are checked here, in both directions:
    ///
    ///  * an entry whose module is not covered can never match, so it is inert
    ///    and fails — that is the check the module key exists to make possible;
    ///  * an entry that matches no dead emission has stopped describing one and
    ///    fails, which is what keeps the list from rotting.
    ///
    /// `known_dead` is a parameter rather than a read of [`KNOWN_DEAD`] so that
    /// both branches can be driven by a test over real parsed state. Until #78
    /// neither could be: the deferral path was exercised by nothing, which is
    /// how two reviewers came to believe it was inert while it was working.
    fn uncovered(&self, known_dead: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        let dead = self.dead_emissions();

        let covered: BTreeSet<&str> = self.covered.iter().map(|c| c.module.as_str()).collect();
        let out_of_scope: Vec<&(&str, &str, &str)> = known_dead
            .iter()
            .filter(|(_, module, _)| !covered.contains(*module))
            .collect();
        assert!(
            out_of_scope.is_empty(),
            "these KNOWN_DEAD entries name a module this guard does not cover, so they can never \
             match and are deferring nothing: {out_of_scope:?}. Covered modules are {covered:?}. \
             Either the module name is wrong — including a *page* name where a module belongs, \
             which reads as plausible because both are capitalised the same way at the call \
             site — or the module has dropped out of scope. Fix the name, or delete the entry if \
             the emission is gone with it."
        );

        let stale: Vec<&(&str, &str, &str)> = known_dead
            .iter()
            .filter(|(variant, module, _)| {
                !dead.iter().any(|(_, m, v)| v == variant && m == module)
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
            .filter(|(_, module, variant)| {
                !known_dead
                    .iter()
                    .any(|(v, m, _)| v == variant && m == module)
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
            // A module emitting nothing is *not* asserted against here, and that
            // is a correction rather than an omission. The first version asserted
            // it per page, on the reasoning that every covered page draws
            // controls — and it went red on `view/credits.rs` when Credits
            // landed: an about page whose only action is a URL link, rendered
            // disabled because the `Message` it would send does not exist yet.
            // Zero emissions is the honest state of that page. The widening to
            // shared components added four more such modules — `widgets`,
            // `badge`, `cover`, `meta` and `metrics` are layout helpers generic
            // over `M` and construct no message at all, which is what
            // `view/mod.rs` says the layer is for — so a per-module floor would
            // be wrong for five of the eleven. What emptiness cannot be
            // distinguished from is a scanner that stopped matching, so the
            // check for that is global: the floor below.
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
                out.push((page.label(), page.module.clone(), variant));
            }
        }

        // Anti-vacuity, global rather than per page (#32 / #43). A page may
        // legitimately emit nothing, so emptiness per page proves nothing; a
        // collapse across *all* of them means the scanner has stopped matching
        // and every module below is being called clean. Measured on the revision
        // the widening landed against — eleven modules, `#80` — library 7,
        // runners 7, form 6, settings 5, plugins 2, credits 0, and 0 from each of
        // `widgets`, `badge`, `cover`, `meta`, `metrics`: 27 in total, so the
        // floor is well clear of both a passing and a collapsing scan.
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        assert!(
            total >= 10,
            "the emission scan found only {total} `Message::` constructions across all {} \
             covered modules (per module: {counts:?}). That is a scanner that has stopped \
             matching, not a set of modules that emit nothing — a clean tree measures 27 here, \
             and the guard would report every module as covered-and-clean off a scan this short.",
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
fn every_message_a_covered_module_emits_has_a_handler() {
    let guard = Guard::parse(&read("crates/app/src/main.rs"));
    // Printed, not just asserted, because #65 was reported to the team as a
    // count ("N empty arms") and a count is only checkable if it is
    // reproducible. Run with `--nocapture` for the figures. They are measured
    // against whatever revision is checked out — the tree moves constantly
    // here, so a count without its commit means nothing (D-45), and a green run
    // here is evidence about the working tree and not about any commit.
    println!(
        "dispatch coverage: {} Message variants, {} match arms, {} covered modules \
         ({} reached by call rather than named in main.rs), {} dead emissions \
         ({} deferred by KNOWN_DEAD), {} exemptions in HANDLED_ELSEWHERE",
        guard.variants.len(),
        guard.arm_count,
        guard.covered.len(),
        guard
            .covered
            .iter()
            .filter(|c| c.reached.starts_with("called from"))
            .count(),
        guard.dead_emissions_len(),
        KNOWN_DEAD.len(),
        HANDLED_ELSEWHERE.len(),
    );
    // The list, not just the count: a count is checkable only if the membership
    // behind it is (D-45), and which modules are in scope is the whole subject
    // of #80. Run with `--nocapture`.
    for c in &guard.covered {
        println!("  covered: view/{}.rs — {}", c.module, c.reached);
    }
    let failures = guard.uncovered(&KNOWN_DEAD);
    assert!(
        failures.is_empty(),
        "{} message(s) a covered view module emits have an empty `Shell::update` arm. The \
         module is rendered — either as a page `view_body` dispatches to a real body, or \
         because a rendered module calls it — so the control is reachable and does \
         nothing:\n{}\n\n\
         Fix by handling the variant, or — if the page is not finished — put it back behind \
         `pending_page` so it says so on screen. `Message::Quit` is exempt: it is handled in \
         `App::update`, the layer that owns the window. The full exemption list is \
         {HANDLED_ELSEWHERE:?}.\n\n\
         This guard is not currently failing on {KNOWN_DEAD:?} — deferred in `KNOWN_DEAD`, each \
         with the task that owns it. It is still dead.",
        failures.len(),
        failures
            .iter()
            .map(|(label, _, variant)| format!("  {label}: Message::{variant}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The guard's reason for existing in the form `#80` is about: a control whose
/// message is constructed by a module that is **not** a page dispatch.
///
/// This is a second red run, and it is not redundant with the one below. That
/// one stages on `NavigateTo`, which is emitted by the Library — a dispatched
/// page. It would pass unchanged if the covered set were the dispatch roots
/// alone, which is exactly the hole #80 names: `view/form.rs` is composed by
/// `view_with_overlays`, not dispatched to, and `view/widgets.rs` is named by
/// nothing in `main.rs` at all. T-29's Play control lands in `widgets.rs`, so
/// the control that matters next is born in a module the old guard could not
/// see.
///
/// The target is chosen by *shape* — a variant every emitting module of which
/// is reached by call, not a dispatch root — so it keeps pointing at the hole
/// after these modules are renamed or added to.
#[test]
fn the_guard_covers_a_control_outside_the_page_dispatch() {
    let src = read("crates/app/src/main.rs");
    let guard = Guard::parse(&src);

    let roots: BTreeSet<&str> = guard
        .covered
        .iter()
        .filter(|c| c.page.is_some())
        .map(|c| c.module.as_str())
        .collect();
    assert!(
        !roots.is_empty(),
        "no covered module is a dispatched page, so this test cannot tell a root from a \
         non-root and would pass for the wrong reason."
    );
    let non_roots: Vec<&Covered> = guard.covered.iter().filter(|c| c.page.is_none()).collect();
    assert!(
        !non_roots.is_empty(),
        "every covered module is a dispatched page, so the closure over rendered modules is \
         adding nothing. That is #80 unfixed: the guard is back to the dispatch roots and \
         `view/form.rs` and the shared components are out of scope."
    );

    // Emit-from-non-root: a variant constructed by a covered module, where no
    // emitting module is a dispatch root. Such a variant is invisible to a
    // dispatch-only guard.
    let mut target: Option<String> = None;
    for module in &non_roots {
        let (prod, _) = production_src(&read(&format!("crates/app/src/view/{}.rs", module.module)));
        for variant in emissions(&prod) {
            if !guard.handled.contains(&variant) || !guard.variants.contains(&variant) {
                continue;
            }
            // Only if *nothing* that is a root emits it, or the dispatch-only
            // guard would have seen it and this test would prove nothing.
            let also_from_a_root = guard.covered.iter().filter(|c| c.page.is_some()).any(|c| {
                let (p, _) =
                    production_src(&read(&format!("crates/app/src/view/{}.rs", c.module)));
                emissions(&p).contains(&variant)
            });
            if !also_from_a_root {
                target = Some(variant);
                break;
            }
        }
        if target.is_some() {
            break;
        }
    }
    let target = target.unwrap_or_else(|| {
        panic!(
            "no `Message` variant is emitted exclusively by a non-dispatch covered module and \
             handled. Either the coverage closure has stopped reaching those modules, or every \
             such variant is unhandled — in which case the first test above is already red and \
             this one has nothing to stage. Covered modules: {:#?}",
            guard.covered.iter().map(|c| (&c.module, &c.reached)).collect::<Vec<_>>()
        )
    });

    assert!(
        !guard
            .uncovered(&KNOWN_DEAD)
            .iter()
            .any(|(_, _, v)| *v == target),
        "Message::{target} is already uncovered before any mutation, so this test cannot tell a \
         red run from the standing state."
    );

    let mutated = empty_arm(&src, &target);
    let after = Guard::parse(&mutated);
    let found = after.uncovered(&KNOWN_DEAD);
    let named: Vec<&(String, String, String)> =
        found.iter().filter(|(_, _, v)| *v == target).collect();
    assert!(
        !named.is_empty(),
        "Message::{target} is emitted by a view module that is not a page dispatch, its arm was \
         emptied, and the guard did not report it. Everything it reported: {found:?}. That is \
         #80: a control in `view/form.rs` or in a shared component can be dead and this guard \
         says nothing."
    );
    // The module it names must be the module that actually emits it, and must
    // not be a page root — otherwise the report is coming from somewhere else
    // and the assertion above passed for the wrong reason.
    for (label, module, _) in &named {
        assert!(
            non_roots.iter().any(|c| c.module == *module),
            "the guard reported Message::{target} against {label} (view/{module}.rs), which is \
             not one of the non-dispatch modules that emit it. Reported: {named:?}"
        );
    }
    println!(
        "closure red run: re-emptying Message::{target} was reported as {}",
        named
            .iter()
            .map(|(label, _, v)| format!("{label} / Message::{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// The deferral machinery, driven in both of its branches on real parsed state.
///
/// This is the test #78 says was missing. `uncovered`'s two assertions —
/// "this entry names a module the guard scans" and "this entry still describes
/// a dead emission" — were exercised by nothing, so a reader could conclude the
/// list was inert and the suite would not contradict them. Two reviewers did.
///
/// U5 emptied the deferral list by handling the last dead arms, so the tree no
/// longer offers a standing dead emission to work from — and the old revision
/// of this test panicked exactly there, prescribing deletion or staging. It is
/// staging: a handled, emitted arm is re-emptied in a mutated copy (the same
/// `empty_arm` `the_guard_notices_a_re_emptied_arm` uses) and both cases run
/// against that. Deletion would have dropped #78's coverage of the machinery
/// the next genuine deferral needs.
///
/// The second case is the one that matters: it passes a **page** name where a
/// module name belongs, which is the shape the old key invited, and requires the
/// guard to say so rather than silently deferring nothing.
#[test]
fn the_deferral_key_is_a_module_and_a_bad_key_is_reported() {
    let src = read("crates/app/src/main.rs");
    let live = Guard::parse(&src);
    // Any emission the tree handles, chosen by shape rather than by name —
    // variants get renamed, and a name-keyed pick would quietly fall through.
    let target = live
        .covered
        .iter()
        .flat_map(|page| {
            let (view_src, _) =
                production_src(&read(&format!("crates/app/src/view/{}.rs", page.module)));
            emissions(&view_src)
        })
        .find(|variant| live.handled.contains(variant))
        .unwrap_or_else(|| {
            panic!(
                "no `Message` variant is both emitted by a covered module and handled, so no \
                 dead emission can be staged. Either the guard's coverage has collapsed or \
                 every page has gone back behind `pending_page`."
            )
        });
    let mutated = empty_arm(&src, &target);
    assert_ne!(mutated, src, "empty_arm did not change the source");
    let guard = Guard::parse(&mutated);
    let dead = guard.dead_emissions();
    let covered: BTreeSet<&str> = guard.covered.iter().map(|c| c.module.as_str()).collect();

    // A real dead emission to work from. It has to exist rather than be
    // fabricated: the point is to drive the same code path the tree does.
    let (_, module, variant) = dead
        .first()
        .unwrap_or_else(|| {
            panic!(
                "re-emptying `{target}` produced no dead emission, so the deferral has nothing \
                 to run against. `empty_arm` changed the source but the guard does not read it \
                 as dead."
            )
        })
        .clone();

    // 1. The right key defers it.
    let deferral = [(variant.as_str(), module.as_str(), "test")];
    let left = guard.uncovered(&deferral);
    assert!(
        !left.iter().any(|(_, m, v)| *v == variant && *m == module),
        "an entry keyed on the module that emits Message::{variant} did not defer it. The \
         deferral list cannot do the one thing it exists for. Left uncovered: {left:?}"
    );
    assert_eq!(
        left.len(),
        dead.len() - 1,
        "deferring one emission changed the report by more than one entry."
    );

    // 2. A page name where a module belongs — the old key's shape, and the
    //    reading that made two reviewers call a working mechanism inert. It
    //    must be reported as out of scope, not accepted and not silently
    //    ignored.
    //
    //    The page name is read out of this tree's own dispatch rather than
    //    written here, and the case is unconditional. The first version of this
    //    test chose the module that owns the dead emission, found that `form` is
    //    not a page, and *skipped* — while still printing that a page key had
    //    been reported. A test whose message claims a thing it did not do is the
    //    defect it was written to catch.
    let root = guard
        .covered
        .iter()
        .find(|c| c.page.is_some())
        .expect("the dispatch roots are asserted non-empty by `Guard::parse`");
    let page_name = root.page.clone().expect("filtered on `page.is_some()`");
    assert_ne!(
        page_name, root.module,
        "the page name and the module name for `{}` are the same string, so the trap this case \
         exists to exercise is not available here — a key on one would be a key on the other. \
         Pick a root whose two names differ.",
        root.module
    );
    let bad = [(variant.as_str(), page_name.as_str(), "test")];
    let caught = std::panic::catch_unwind(|| guard.uncovered(&bad));
    let message = caught
        .err()
        .and_then(|e| {
            e.downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        })
        .unwrap_or_else(|| {
            panic!(
                "a KNOWN_DEAD entry keyed on the page name `{page_name}` (for view/{}.rs) rather \
                 than the module name was accepted. It can never match an emission, so it defers \
                 nothing — and a deferral that defers nothing is how this list turns into a place \
                 where problems are forgotten. The membership assertion is supposed to catch \
                 this shape.",
                root.module
            )
        });
    assert!(
        message.contains("can never match"),
        "the page-name entry `{page_name}` was rejected, but not for being out of scope. \
         Message: {message}"
    );

    // 3. And the module really is one the guard scans, so case 1 was not a
    //    coincidence of `uncovered` accepting anything.
    assert!(
        covered.contains(module.as_str()),
        "the module this test deferred from, `{module}`, is not in the covered set, yet case 1 \
         passed. The membership assertion is not doing its job."
    );

    println!(
        "deferral check: a module key on `{module}` defers Message::{variant}; a page key \
         (`{page_name}`, for view/{}.rs) is reported as one that can never match",
        root.module
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
    let live = guard.uncovered(&KNOWN_DEAD);
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
    let found = after.uncovered(&KNOWN_DEAD);
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
            .map(|(label, _, v)| format!("{label} / Message::{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
