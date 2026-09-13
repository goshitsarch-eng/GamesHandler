//! Every `Page` variant is in `Page::ALL` (`BUG-29`).
//!
//! `Page` and `Page::ALL` are two encodings of one list — the enum says which
//! pages exist, the constant says which pages the sidebar shows and in what
//! order. Rust cannot relate them: `Page::ALL: [Page; 6]` is checked against
//! the *number of elements written in the literal*, not against the number of
//! variants.
//!
//! # Which half the compiler covers, measured
//!
//! Both halves of this were run against the tree, because the first version of
//! this header asserted the weaker one and was wrong.
//!
//! **The bare mistake is caught by rustc.** Adding `Tools,` to the enum and
//! changing nothing else fails to build, with three `E0004` non-exhaustive
//! matches — `Page::label` (`crates/app/src/state.rs`), `page_icon` and
//! `Shell::view_body` (`crates/app/src/main.rs`). A variant that exists but is
//! never matched anywhere cannot happen.
//!
//! **The mistake that survives is the complete one**, and it is not exotic: it
//! is what the compiler's own help text walks a fixer into. rustc prints
//! ``help: ensure that all possible cases are being handled by adding a match
//! arm with a wildcard pattern or an explicit pattern``, so the natural repair
//! is to add a `Page::Tools =>` arm at each of the three sites — nothing tells
//! the fixer that `Page::ALL`, a fourth place with no compiler check on it,
//! also needs the variant. Measured: with those three arms added and
//! `Page::ALL` left at `[Page; 6]`, **the workspace compiles clean**, the
//! existing `every_page_has_a_distinct_label_and_the_order_is_the_nav_order`
//! test passes (it reads `ALL`, never the enum), and `Page::Tools` is absent
//! from the sidebar. `build_nav_model` iterates `ALL`, so no row carries the
//! variant, while `State::page` still accepts it — a handler naming it draws a
//! body under the previous page's highlighted row.
//!
//! That compiling mutation is what this test catches. It fails naming the
//! variant:
//!
//! ```text
//! `Page::ALL` does not contain ["Tools"]. `build_nav_model` iterates `ALL`, …
//!   enum Page  : ["Library", "Installers", "Runners", "Plugins", "Credits", "Settings", "Tools"]
//!   Page::ALL  : ["Library", "Installers", "Runners", "Plugins", "Credits", "Settings"]
//! ```
//!
//! # The guard that was there instead
//!
//! Before this file, the only thing standing there was `activate_page`'s
//! `.expect("every Page is in Page::ALL")`, which is not a guard: it is where
//! the divergence *surfaces*, as a panic, in release as well as debug (an
//! `expect` is not `debug_assert!`). The panic also ran **before**
//! `Shell::show_page`'s `debug_assert!(self.pages_agree())`, so the invariant
//! that doc-comment describes could never observe this case — and the
//! `debug_assert` is absent from a release build anyway. `activate_page` now
//! answers `false` for the release path, which is what `show_page` already did
//! with that `false`.
//!
//! # Why this reads the source
//!
//! `Page` has no runtime reflection, so "every variant" cannot be asked of the
//! type. The alternative — a hand-written list of six names here — is the same
//! defect one level up: a second list that can drift from the enum and report
//! agreement with it. The file is read as text and the variants are parsed out
//! of it, and the parse is asserted non-empty so a helper that silently stopped
//! matching cannot pass vacuously (this project's recurring defect class).
//!
//! `CARGO_MANIFEST_DIR` is `crates/app`, so `src/state.rs` is the file this
//! crate compiles.

use std::fs;
use std::path::{Path, PathBuf};

fn state_rs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/state.rs")
}

/// The variant names of `pub enum Page { … }`, in declaration order.
fn page_variants(source: &str) -> Vec<String> {
    let start = source
        .find("pub enum Page {")
        .expect("`pub enum Page` is the type this test is about; if it was renamed, this test must move with it rather than pass")
        + "pub enum Page {".len();
    let body = &source[start..];
    let end = body
        .find("\n}")
        .expect("an enum body ends with a `}` at column 0");
    body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                return None;
            }
            let name = line.strip_suffix(',').unwrap_or(line);
            // A variant is a bare identifier on its own line. A `#[…]`
            // attribute or a payload would not match, and neither appears on
            // this enum — if one is added, the count below drops and the
            // non-empty assertion is what catches it.
            (name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then(|| name.to_string())
        })
        .collect()
}

/// The `[Page; N]` length and the variants listed in `pub const ALL`.
fn all_of(source: &str) -> (usize, Vec<String>) {
    let anchor = source
        .find("pub const ALL: [Page; ")
        .expect("`Page::ALL` is the other half of this invariant");
    let rest = &source[anchor + "pub const ALL: [Page; ".len()..];
    let (digits, tail) = rest
        .split_once(']')
        .expect("the type is `[Page; N]`, so a `]` closes it");
    let declared: usize = digits
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("`[Page; {digits}]` is not a length: {error}"));

    let body = tail.split_once("= [").expect("the constant is `= [ … ]`").1;
    let body = body.split_once("];").expect("the literal ends with `];`").0;
    let listed = body
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("Page::")
                .map(|name| name.trim_end_matches(',').to_string())
        })
        .collect();
    (declared, listed)
}

#[test]
fn every_page_variant_is_in_page_all() {
    let path = state_rs();
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} should be readable: {error}", path.display()));

    let variants = page_variants(&source);
    let (declared, listed) = all_of(&source);

    // Without this, a helper that stopped parsing would compare two empty
    // lists, agree, and pass — the failure this file exists to prevent,
    // reproduced inside the fix for it.
    assert!(
        variants.len() >= 6,
        "parsed {} variants out of `Page`, which is fewer than the six this \
         enum has always had, so the parser is not reading the enum: {variants:?}",
        variants.len()
    );
    assert!(
        !listed.is_empty(),
        "parsed no entries out of `Page::ALL`, so the comparison below would be \
         vacuous: {listed:?}"
    );

    let missing: Vec<&String> = variants
        .iter()
        .filter(|variant| !listed.contains(variant))
        .collect();
    assert!(
        missing.is_empty(),
        "`Page::ALL` does not contain {missing:?}. `build_nav_model` iterates \
         `ALL`, so a variant missing from it has no sidebar row and cannot be \
         reached, while `State::page` will still accept the value and the body \
         will draw under the previous page's highlighted row. Add it to `ALL` \
         in nav-bar order.\n\n  enum Page  : {variants:?}\n  Page::ALL  : {listed:?}"
    );

    assert_eq!(
        declared,
        variants.len(),
        "`Page::ALL` is declared `[Page; {declared}]` and `Page` has {} \
         variants. The length is checked by the compiler against the number of \
         elements *written in the literal* and not against the enum, so these \
         two can disagree and still build. Variants: {variants:?}",
        variants.len()
    );
    assert_eq!(
        listed.len(),
        declared,
        "`Page::ALL` is declared `[Page; {declared}]` and lists {} entries: {listed:?}",
        listed.len()
    );

    let mut seen = listed.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        listed.len(),
        "`Page::ALL` lists a page twice, so the sidebar has two rows for it and \
         `activate_position` moves to the first: {listed:?}"
    );
}
