//! The About & Credits page — `CreditsPage.qml` (142 lines).
//!
//! # The page is a reading surface
//!
//! Nothing here writes anything back. The reference's page has no controls: it
//! prints the application's identity, the acknowledgement paragraph, every
//! credited project grouped into sections, the "why one app" rationale, and a
//! footer — all of it read-only. So unlike [`super::library`] and
//! [`super::settings`], this page takes no `&State`: a parameter nothing reads
//! is the thing this project keeps writing comments about, and there is no
//! field of `State` that any line below consults.
//!
//! # Everything visible comes from `core::credits`
//!
//! The one way this page can be wrong is by *transcribing* the catalogue into
//! the view — a shorter hand-written list of sections, or a project named
//! inline — which would look right and be a second copy of the data. So the
//! sections, the entries and the rationale are reachable here **only** through
//! the four accessors below ([`credit_sections`], [`rationale`],
//! [`acknowledgement`], and the per-entry builders), and every one of them
//! delegates to [`gamehandler_core::credits`]. That is the shape T-26 arrived
//! at for the Install button: the value the view draws is produced by a named
//! function, so a literal is not merely discouraged, it is somewhere the tests
//! can see.
//!
//! That the accessors are *reached* is a separate claim from what they return,
//! and this module tests it rather than assuming it:
//! `the_page_draws_every_section_and_every_credit` lays the element out,
//! traverses it, and requires all five headings, all five summaries and all
//! twenty-five name-and-role pairs to be among the strings the framework was
//! handed. So a `view` that drew a hand-written subset — the way this page
//! could most plausibly go wrong — fails here even though every accessor above
//! is a one-line delegation.
//!
//! # The links, and the one thing missing to make them work
//!
//! `CreditsPage.qml:85-89` draws a `Kirigami.UrlButton` labelled "Visit" per
//! entry, and a "GameHandler on GitHub" button at the foot of the page. Both
//! are drawn here, at the reference's positions and labels, and **every URL is
//! the data layer's own**: the per-entry href is [`Credit::url`], reached
//! through [`credit_link`], and the footer's is [`repository_url`], which is
//! `Cargo.toml`'s `repository` field read with `env!` rather than retyped.
//! `the_footer_link_is_the_qmls_and_the_manifests` pins that value against
//! `CreditsPage.qml:138`, so the QML, the manifest and the page cannot drift
//! apart without a failure.
//!
//! **What they cannot do is open anything.** `button::link` with no `on_press`
//! renders disabled, and there is no `Message::OpenUrl` in this application to
//! give it — the same wall [`super::runners`] recorded for its two link
//! buttons, hit here twenty-six times over. Opening a URL needs a `Message`
//! variant and a handler, both of which live in `main.rs`, and this page is
//! landing in a window where `main.rs` is held by another task. So it is a
//! recorded interim rather than a silent one: see [`LINKS_OPEN`], which is the
//! value `the_links_are_still_unwired` reads, and which fails the day the
//! message lands so that this note cannot outlive the gap it describes.
//!
//! (`the_links_are_still_unwired` is a code span and not a link on purpose: it
//! is a `#[cfg(test)]` item, so rustdoc can never resolve it and writing it as
//! `[`…`]` produced a broken-intra-doc-link warning rather than a pointer. Same
//! for the test named in [`crate::view::form`], which lives in `main.rs`.)

use cosmic::iced::Length;
use cosmic::widget::{Column, Row, button, container, scrollable, text};
use cosmic::Element;
use gamehandler_core::credits::{self, Credit, CreditSection};
use gamehandler_core::{APP_NAME, VERSION};

use crate::Message;

/// The page's title. `CreditsPage.qml:10`, and `Main.qml:94` for the drawer.
///
/// The same string as [`Page::Credits`](crate::state::Page::Credits)'s label —
/// the reference uses one string in both places and so does this port;
/// `the_title_is_the_drawers_label_and_the_qmls` holds all three together.
pub const PAGE_TITLE: &str = "About & Credits";

/// The heading under the title. `CreditsPage.qml:17`.
pub const LEAD_HEADING: &str = "Standing on other people's work";

/// The attribution line under it, bold in the reference. `CreditsPage.qml:21`.
pub const MAKER_LINE: &str = "Made by Gosh.";

/// The rationale section's heading. `CreditsPage.qml:100`.
pub const WHY_HEADING: &str = "Why one app instead of assembling the stack yourself";

/// The sentence under that heading — *not* a [`credits::WHY_ALL_IN_ONE`] entry,
/// which are the five that follow it. `CreditsPage.qml:104`.
pub const WHY_LEAD: &str = "GameHandler replaces none of these projects. It removes the assembly work between them.";

/// The label on a credit's link button. `CreditsPage.qml:88`.
pub const VISIT_LABEL: &str = "Visit";

/// The footer link's label. `CreditsPage.qml:139`.
pub const GITHUB_LABEL: &str = "GameHandler on GitHub";

/// The prefix on the version line. `CreditsPage.qml:25`.
pub const VERSION_PREFIX: &str = "Version ";

/// The prefix on a credit's licence, when it has one, **newline included**.
///
/// `CreditsPage.qml:78` is `modelData.role + "\nLicense: " + modelData.license`
/// — the licence is on its own line in the reference, so the newline is part of
/// the literal rather than something this port adds at the join.
pub const LICENSE_PREFIX: &str = "\nLicense: ";

/// The footer's fixed tail, licence included. `CreditsPage.qml:131-132`.
///
/// Split from the leading `"{APP_NAME} {VERSION}"` because that half is this
/// build's own and this half is the reference's wording — see [`footer_line`].
pub const FOOTER_TAIL: &str = " — GPL-3.0-or-later. Proton and Wine builds are \
     downloaded from their maintainers at your request and remain under their own \
     licenses.";

/// Whether clicking a link on this page opens it.
///
/// # `false`, and it is a value rather than a comment on purpose
///
/// Every URL below is real and is the data layer's own; what is missing is the
/// application's ability to *act* on one. `button::link(...)` with no
/// `on_press` renders disabled — the widget is honest about it, and the
/// reference's affordance is in the reference's position — but the click needs
/// a `Message::OpenUrl`, which does not exist in this application yet.
///
/// This constant exists so that "the links do not open" is a fact a test reads
/// rather than a sentence a reader has to trust. `the_links_are_still_unwired`
/// asserts both that it is `false` **and** that `main.rs` still has no
/// `OpenUrl`, so the day the message lands the test fails and points at this
/// constant. Deleting the record is part of wiring the links, exactly as
/// deleting a `PINNED_PENDING` line is part of landing a page.
pub const LINKS_OPEN: bool = false;

// ---------------------------------------------------------------------------
// The data layer, reached through named functions
// ---------------------------------------------------------------------------

/// The credited projects, grouped, in the reference's order.
///
/// A named function rather than a direct `credits::sections()` call inside
/// [`view`] so that the page's source of truth is reachable by an assertion and
/// a substitution is reachable by a mutation: replacing this body with a
/// hand-written two-section list is a change `the_page_presents_every_credit_the_reference_declares`
/// fails on, which is the whole reason the accessor exists.
pub fn credit_sections() -> &'static [CreditSection] {
    credits::sections()
}

/// The five "why one app" pairs, `(heading, body)`. `bridge.py:1057-1060`.
pub fn rationale() -> &'static [(&'static str, &'static str)] {
    credits::WHY_ALL_IN_ONE
}

/// The paragraph the page leads with. `bridge.py:179`, `CreditsPage.qml:31`.
///
/// `bridge.py:1062-1069` also defines an `aboutText` that concatenates an
/// introduction with this paragraph. It is **not ported**: nothing reads it.
/// `grep aboutText gamehandler/` finds the definition and no caller, and no QML
/// file names it — so porting it would be porting a dead string, and this page
/// prints what the reference's page prints.
pub fn acknowledgement() -> &'static str {
    credits::ACKNOWLEDGEMENT
}

/// The project's own homepage, for the footer link.
///
/// Read from the manifest rather than written here: `Cargo.toml:25` is the
/// workspace's `repository`, the app crate inherits it
/// (`repository.workspace = true`), and `env!` bakes it in at compile time. The
/// alternative — the string typed into this file and again into
/// `data/com.goshapps.GameHandler.metainfo.xml` and again into the QML — is the
/// duplicate that drifts, which is what the coordinator's note on this page is
/// about.
pub fn repository_url() -> &'static str {
    env!("CARGO_PKG_REPOSITORY")
}

/// `"Version {VERSION}"`. `CreditsPage.qml:25`.
///
/// The version is this build's, from the same `CARGO_PKG_VERSION` the CLI's
/// `--version` prints ([`gamehandler_core::VERSION`]), so the page cannot claim
/// a version the binary is not.
pub fn version_line() -> String {
    format!("{VERSION_PREFIX}{VERSION}")
}

/// The footer: the application's name and version, then the reference's tail.
///
/// Built from [`APP_NAME`] and [`VERSION`] rather than the literal "GameHandler"
/// the QML writes, because `main.py` and the `.desktop` file both take the name
/// from one constant and a page that spelled it again would be a fourth copy.
pub fn footer_line() -> String {
    format!("{APP_NAME} {VERSION}{FOOTER_TAIL}")
}

/// The link a credit's "Visit" button points at, or `None` when it has none.
///
/// `CreditsPage.qml:86-89` is `visible: creditCard.modelData.url !== ""`, and
/// this is that test. Every credit in the reference carries a URL, so the
/// `None` arm is unreachable from [`credit_sections`] — which is exactly why it
/// is a named function: a branch that no datum reaches can only be kept honest
/// by being callable, and `a_credit_with_no_url_draws_no_link` calls it with a
/// synthetic credit to prove the test is not passing on an empty set.
///
/// # What the view does with the value
///
/// It uses it as the **gate** and does not render it. The reference's
/// `Kirigami.UrlButton` shows the word "Visit" and the URL is not visible text
/// anywhere on the page (`CreditsPage.qml:85-89`), so printing it would be a
/// visible departure from the reference. `libcosmic`'s `button::link` carries a
/// label and no href — an href is something the caller supplies through
/// `on_press` — so with no `Message::OpenUrl` there is nowhere for the URL to
/// go. It stays here, one call from the widget, and that is what makes the
/// wiring a one-line change rather than a rediscovery.
pub fn credit_link(credit: &Credit) -> Option<&'static str> {
    if credit.url.is_empty() {
        None
    } else {
        Some(credit.url)
    }
}

/// The line under a credit's name: its role, and its licence when it has one.
///
/// `CreditsPage.qml:77-79`, which is a ternary over the licence:
///
/// ```text
/// text: modelData.license
///     ? modelData.role + "\nLicense: " + modelData.license
///     : modelData.role
/// ```
///
/// The empty-licence arm is live here, unlike [`credit_link`]'s: five of the
/// twenty-five credits decline to guess an SPDX id (`core::credits`'s note on
/// [`Credit::license`] explains why a Proton build has none), so both shapes
/// are drawn on the page as it really is.
pub fn credit_role_line(credit: &Credit) -> String {
    if credit.license.is_empty() {
        credit.role.to_string()
    } else {
        format!("{}{LICENSE_PREFIX}{}", credit.role, credit.license)
    }
}

/// The page.
///
/// Holds nothing: see the module note on why there is no `&State` here. It
/// exists as a type rather than a bare `fn view()` because the dispatch arm
/// reads better as `view(CreditsPage)` beside its siblings, and because a
/// future `State`-dependent field (a "last checked" line, say) would otherwise
/// change every call site.
pub struct CreditsPage;

/// One credit's card: the name with its authors, the role and licence, and the
/// link.
///
/// The reference wraps each entry in a `Kirigami.AbstractCard`
/// (`CreditsPage.qml:60`), which is what `.class(Container::Card)` draws in this
/// toolkit.
fn credit_card(credit: &'static Credit) -> Element<'static, Message> {
    let details = Column::new()
        .spacing(2)
        .width(Length::Fill)
        .push(text::body(credit.label()))
        .push(text::caption(credit_role_line(credit)));

    let mut row = Row::new()
        .push(details)
        .spacing(12)
        .align_y(cosmic::iced::Alignment::Center)
        .width(Length::Fill);

    if credit_link(credit).is_some() {
        // The gate is `visible: url !== ""` (`CreditsPage.qml:86`) and the label
        // is the QML's "Visit"; see [`credit_link`] for where the URL itself
        // goes. A `link` with no `on_press` renders disabled — that is
        // [`LINKS_OPEN`]'s subject, not an accident of this call.
        row = row.push(button::link(VISIT_LABEL.to_string()));
    }

    container(row)
        .class(cosmic::theme::Container::Card)
        .padding(12)
        .width(Length::Fill)
        .into()
}

/// One section: its heading, its summary, then one card per entry.
fn section_block(section: &'static CreditSection) -> Element<'static, Message> {
    let mut block = Column::new()
        .spacing(6)
        .width(Length::Fill)
        .push(cosmic::widget::divider::horizontal::default())
        .push(text::title3(section.title))
        .push(text::caption(section.summary));

    for credit in section.entries {
        block = block.push(credit_card(credit));
    }

    block.into()
}

/// The page.
pub fn view(_page: CreditsPage) -> Element<'static, Message> {
    let mut body = Column::new()
        .spacing(12)
        .width(Length::Fill)
        .push(text::title3(LEAD_HEADING))
        .push(text::body(MAKER_LINE).font(cosmic::font::bold()))
        .push(text::caption(version_line()))
        .push(text::body(acknowledgement()));

    // Every section, in the reference's order. `credit_sections()` and not a
    // list written here; see the module note.
    for section in credit_sections() {
        body = body.push(section_block(section));
    }

    // ---- Why one app -------------------------------------------------------
    body = body
        .push(cosmic::widget::divider::horizontal::default())
        .push(text::title3(WHY_HEADING))
        .push(text::caption(WHY_LEAD));

    for (heading, text_body) in rationale() {
        body = body.push(
            Column::new()
                .spacing(2)
                .width(Length::Fill)
                .push(text::title4(*heading))
                .push(text::caption(*text_body)),
        );
    }

    // ---- Footer ------------------------------------------------------------
    // The footer's link is the manifest's own URL, reached through
    // [`repository_url`] — which, like a `Visit` button's href, has nowhere to
    // go on a `button::link` with no `on_press`. See [`LINKS_OPEN`].
    body = body
        .push(cosmic::widget::divider::horizontal::default())
        .push(text::caption(footer_line()))
        .push(button::link(GITHUB_LABEL.to_string()));

    container(scrollable(body)).padding(18).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A file from the repository root, read at test time.
    ///
    /// `CARGO_MANIFEST_DIR` is `crates/app`, so the root is two levels up.
    /// Reading the reference off disk is the point: a constant compared against
    /// a copy of itself cannot disagree with itself.
    fn repo_file(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/app sits two levels below the repository root")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()))
    }

    fn qml() -> String {
        repo_file("gamehandler/qml/CreditsPage.qml")
    }

    /// **The page presents every credit the reference declares.**
    ///
    /// The counts are taken from `credits.py` by counting its own constructors —
    /// `CreditSection(` and `Credit(` — rather than restated, so a section or a
    /// project that exists in the catalogue and not on the page fails here.
    /// That is the defect this test is for: a `view` that printed a hand-written
    /// subset of the catalogue would render, would look finished, and would be a
    /// second copy of the data.
    ///
    /// `"CreditSection("` does not contain `"Credit("` — the character after
    /// `Credit` is `S` — so the two counts are independent and the second needs
    /// no correction. That is worth stating because the tempting `- sections`
    /// would make this assert 20 against a catalogue of 25, which is the kind of
    /// arithmetic that passes review and eats a real entry.
    ///
    /// # What it does not check
    ///
    /// It reads [`credit_sections`], not the element [`view`] builds. A `view`
    /// that ignored the accessor and iterated a literal would pass. That check
    /// needs the drawn strings, and it belongs with the dispatch test in
    /// `main.rs`; see the module note.
    #[test]
    fn the_page_presents_every_credit_the_reference_declares() {
        let reference = repo_file("gamehandler/credits.py");
        let declared_sections = reference.matches("CreditSection(").count();
        let declared_credits = reference.matches("Credit(").count();

        let sections = credit_sections();
        let presented: Vec<&Credit> = sections.iter().flat_map(|s| s.entries.iter()).collect();

        assert_eq!(
            sections.len(),
            declared_sections,
            "the page draws {} sections where credits.py constructs {declared_sections}",
            sections.len()
        );
        assert_eq!(
            presented.len(),
            declared_credits,
            "the page draws {} credits where credits.py constructs {declared_credits}",
            presented.len()
        );

        // The same list, from the data layer's own accessor, so the two cannot
        // disagree about which credits they are — only about how many.
        assert_eq!(presented, credits::all_credits());

        // Neither count is zero: a parser that found nothing would make the two
        // assertions above compare nothing to nothing.
        assert!(declared_sections > 0 && declared_credits > 0);
    }

    /// Every credit's card is built from the credit's own fields, and the shape
    /// of the role line is the QML's.
    ///
    /// The `"License: "` prefix and the newline are read out of
    /// `CreditsPage.qml` rather than typed here, so a page that rendered
    /// `role — license` on one line would fail against the reference instead of
    /// against an opinion.
    #[test]
    fn every_role_line_is_the_credits_own_and_the_qmls_shape() {
        let qml = qml();

        // **The expected prefix is a literal in this test, not
        // [`LICENSE_PREFIX`].** The first version of this test built the
        // expected string with `format!("{}{LICENSE_PREFIX}{}", role, license)`
        // — the same constant `credit_role_line` uses — so it compared the
        // constant against itself, and dropping the newline from
        // `LICENSE_PREFIX` left it green. Measured: that mutation survived.
        //
        // The literal below is *not* a second place for the wording to drift,
        // because the line after it ties it to the QML's own source text: the
        // QML spells the newline as the two characters `\` `n` inside a string
        // literal, and the assert requires exactly that. So the chain is
        // QML source → this literal → the constant, and a break anywhere in it
        // fails here.
        const EXPECTED_PREFIX: &str = "\nLicense: ";
        assert!(
            qml.contains("\"\\nLicense: \""),
            "CreditsPage.qml no longer joins the role and the licence with a \
             newline and this prefix; the port's credit_role_line has to follow it"
        );
        assert_eq!(
            LICENSE_PREFIX, EXPECTED_PREFIX,
            "the port's licence prefix is not the one CreditsPage.qml:78 writes"
        );

        let mut with_license = 0;
        let mut without = 0;
        for section in credit_sections() {
            for credit in section.entries {
                let line = credit_role_line(credit);
                if credit.license.is_empty() {
                    without += 1;
                    assert_eq!(line, credit.role, "{} has no licence", credit.name);
                    assert!(
                        !line.contains(EXPECTED_PREFIX),
                        "{} has no licence, so the page must not print one",
                        credit.name
                    );
                } else {
                    with_license += 1;
                    assert_eq!(
                        line,
                        format!("{}{}{}", credit.role, EXPECTED_PREFIX, credit.license),
                        "{}: the licence must be on its own line, as it is in the QML",
                        credit.name
                    );
                }
                // The name line is the data layer's `label()`, authors and all.
                assert!(credit.label().starts_with(credit.name));
            }
        }

        // Both arms are exercised by the real catalogue, which is what makes
        // this a test of the page rather than of one branch.
        assert!(with_license > 0, "no credit carries a licence");
        assert!(without > 0, "every credit carries a licence; the empty arm is \
                              untested against real data");
    }

    /// **A credit with no URL draws no link**, and no real credit is in that
    /// state.
    ///
    /// The second half is what stops the first from being a test of an empty
    /// set: if every credit had an empty URL the loop below would pass while
    /// the page contradicted the reference, which draws a "Visit" button for
    /// all twenty-five.
    #[test]
    fn a_credit_with_no_url_draws_no_link() {
        let synthetic = Credit {
            name: "A project with no homepage",
            url: "",
            role: "Nothing in particular",
            license: "",
            authors: "",
        };
        assert_eq!(credit_link(&synthetic), None);

        for credit in credits::all_credits() {
            assert_eq!(
                credit_link(credit),
                Some(credit.url),
                "{} carries no URL, so the page would draw no Visit button where \
                 the reference draws one",
                credit.name
            );
            assert!(credit.url.starts_with("https://"), "{}", credit.name);
        }
    }

    /// The page's own strings are the QML's, read at test time.
    #[test]
    fn the_headings_and_labels_are_the_qmls() {
        let qml = qml();
        for (what, literal) in [
            ("the title", PAGE_TITLE),
            ("the lead heading", LEAD_HEADING),
            ("the maker line", MAKER_LINE),
            ("the rationale heading", WHY_HEADING),
            ("the rationale lead", WHY_LEAD),
            ("the visit label", VISIT_LABEL),
            ("the footer link label", GITHUB_LABEL),
        ] {
            assert!(
                qml.contains(&format!("\"{literal}\"")),
                "{what} ({literal:?}) is not a string in CreditsPage.qml"
            );
        }
    }

    /// **The title is one string in three places**: the page, the drawer, and
    /// the QML.
    ///
    /// The reference uses "[About & Credits]" for all three — `CreditsPage.qml:10`
    /// is the property, `Main.qml:94` is the drawer row — and finding #35 was
    /// this port saying "Credits" in the drawer while the QML said otherwise.
    /// The drawer's side is asserted in `main.rs`; this is the page's, and
    /// reading `Main.qml` here is what ties the third one on.
    #[test]
    fn the_title_is_the_drawers_label_and_the_qmls() {
        assert_eq!(PAGE_TITLE, "About & Credits");
        assert_eq!(
            crate::state::Page::Credits.label(),
            PAGE_TITLE,
            "the drawer and the page disagree about this page's name"
        );
        assert!(qml().contains(&format!("\"{PAGE_TITLE}\"")));
        assert!(
            repo_file("gamehandler/qml/Main.qml").contains(&format!("\"{PAGE_TITLE}\"")),
            "Main.qml no longer names the drawer row {PAGE_TITLE:?}"
        );
    }

    /// The footer is the application's own name and version, then the QML's
    /// wording — and the wording is read out of the QML, not retyped.
    ///
    /// `CreditsPage.qml:130-132` builds the same line by concatenation across
    /// three string literals. The assertion below takes those three fragments
    /// as they appear on disk and requires the port's line to end with their
    /// join, so a page that paraphrased the licence sentence fails.
    #[test]
    fn the_footer_is_the_references_wording() {
        let qml = qml();
        let fragments = [
            " — GPL-3.0-or-later. Proton and Wine builds are downloaded from ",
            "their maintainers at your request and remain under their own licenses.",
        ];
        for fragment in fragments {
            assert!(
                qml.contains(&format!("\"{fragment}\"")),
                "CreditsPage.qml no longer contains the footer fragment {fragment:?}"
            );
        }
        let joined = fragments.concat();
        assert_eq!(
            footer_line(),
            format!("{APP_NAME} {VERSION}{joined}"),
            "the port's footer is not the reference's sentence"
        );
        assert_eq!(FOOTER_TAIL, joined);
    }

    /// **The footer's link is the manifest's, and the manifest's is the QML's.**
    ///
    /// The GitHub URL is written down in three places in this repository —
    /// `Cargo.toml`, the metainfo XML, and `CreditsPage.qml` — and this is the
    /// tie between the one the page draws and the one the reference shows. The
    /// page does not retype it: [`repository_url`] is `env!("CARGO_PKG_REPOSITORY")`.
    #[test]
    fn the_footer_link_is_the_qmls_and_the_manifests() {
        let qml_url = "https://github.com/goshitsarch-eng/GamesHandler";
        assert!(
            qml().contains(&format!("\"{qml_url}\"")),
            "CreditsPage.qml:138 no longer points the footer link at {qml_url}"
        );
        assert_eq!(
            repository_url(),
            qml_url,
            "the manifest's repository field and the QML's footer link have drifted \
             apart; the page draws the manifest's"
        );
        assert!(
            repo_file("data/com.goshapps.GameHandler.metainfo.xml").contains(qml_url),
            "the metainfo file no longer carries the homepage"
        );
    }

    /// The rationale is the data layer's five pairs, and the lead above them is
    /// the QML's — the page does not fold the two together.
    #[test]
    fn the_rationale_is_the_data_layers_and_the_lead_is_the_qmls() {
        assert_eq!(rationale(), credits::WHY_ALL_IN_ONE);
        assert!(
            !rationale().is_empty(),
            "an empty rationale would make the comparison above vacuous"
        );
        for (heading, body) in rationale() {
            assert!(!heading.is_empty() && !body.is_empty());
        }
        assert!(!credits::WHY_ALL_IN_ONE.iter().any(|(h, _)| *h == WHY_LEAD));
    }

    /// The version lines name this build's version and the application's own
    /// name.
    #[test]
    fn the_version_lines_name_this_build() {
        assert_eq!(version_line(), format!("Version {VERSION}"));
        assert!(version_line().starts_with(VERSION_PREFIX));
        assert!(footer_line().starts_with(&format!("{APP_NAME} {VERSION}")));
        assert_eq!(APP_NAME, "GameHandler");
        assert!(
            qml().contains("\"Version \""),
            "CreditsPage.qml no longer prefixes the version with \"Version \""
        );
    }

    /// The acknowledgement is the data layer's paragraph, and it is the same
    /// one the README's credits section is built from.
    #[test]
    fn the_acknowledgement_is_the_data_layers() {
        assert_eq!(acknowledgement(), credits::ACKNOWLEDGEMENT);
        assert!(acknowledgement().len() > 40);
        assert!(credits::markdown().contains("independent"));
    }

    /// **The links do not open, and this fails the day they can.**
    ///
    /// [`LINKS_OPEN`] is `false` because the application has no
    /// `Message::OpenUrl` to give a `button::link`. This asserts both halves:
    /// the record, and the reason for it — read out of `main.rs`, which is
    /// where such a variant would be declared. Wiring the links means deleting
    /// the constant and this test together, which is what a self-invalidating
    /// record is for.
    #[test]
    fn the_links_are_still_unwired() {
        // `assert!(!LINKS_OPEN)` is a constant assertion and clippy rejects it
        // (`clippy::assertions_on_constants`), which is the right call — the
        // constant is the record and the compiler is already checking it. What
        // needs a test is the *reason*, below.
        let links_open: bool = LINKS_OPEN;
        assert!(!links_open, "the record says a link opens; it does not");
        let main_rs = repo_file("crates/app/src/main.rs");
        assert!(
            !main_rs.contains("OpenUrl"),
            "`main.rs` now has an `OpenUrl`, so the links on this page can be \
             wired: give the two `button::link` calls an `on_press`, delete \
             `LINKS_OPEN`, and delete this test."
        );
    }

    /// **The page draws every section and every credit.**
    ///
    /// This is the check the module note says the accessor tests cannot make:
    /// they read [`credit_sections`], and a [`view`] that ignored it and
    /// iterated its own literal would pass every one of them. Here the element
    /// is genuinely laid out and traversed, and the strings the framework was
    /// handed are compared against the catalogue — so the page and the data
    /// layer have to agree about all five sections and all twenty-five entries,
    /// heading by heading and name by name.
    ///
    /// # Why this is reachable here
    ///
    /// `iced` exposes no downcast, so the text a widget draws is readable only
    /// through `Widget::operate` — the same mechanism `crate::view::widgets`'s
    /// tests use, and the one `main.rs`'s `drawn_strings` uses. It is a real
    /// `layout` and a real `operate` over a real element with a real (software)
    /// renderer, so this needs no display and draws nothing.
    ///
    /// `text::body` and `text::caption` both report their string, so the
    /// headings, the summaries and the footer are seen too — see
    /// `the_static_copy_is_drawn_too`.
    #[test]
    fn the_page_draws_every_section_and_every_credit() {
        let drawn = drawn_strings(view(CreditsPage));

        for section in credit_sections() {
            assert!(
                drawn.iter().any(|text| text == section.title),
                "the section {:?} is not drawn; drawn: {drawn:?}",
                section.title
            );
            assert!(
                drawn.iter().any(|text| text == section.summary),
                "the summary of {:?} is not drawn",
                section.title
            );
            for credit in section.entries {
                let label = credit.label();
                assert!(
                    drawn.contains(&label),
                    "{} is in the catalogue but not on the page; drawn: {drawn:?}",
                    credit.name
                );
                assert!(
                    drawn.iter().any(|text| *text == credit_role_line(credit)),
                    "{}'s role line is not drawn",
                    credit.name
                );
                if credit_link(credit).is_some() {
                    assert!(
                        drawn.iter().any(|text| text == VISIT_LABEL),
                        "{} has a URL, so the page must offer {VISIT_LABEL:?}",
                        credit.name
                    );
                }
            }
        }

        // The traversal found something. A `view` that returned an empty
        // container would satisfy every loop above by drawing nothing at all
        // only if the catalogue were empty; this is the assertion that fails
        // first when it is not.
        assert!(
            drawn.len() >= credit_sections().len(),
            "the traversal saw {} strings, fewer than the {} sections",
            drawn.len(),
            credit_sections().len()
        );
    }

    /// The page's own copy — not the catalogue's — is on screen.
    ///
    /// The companion to the test above: that one proves the *data* is drawn,
    /// this one that the reading surface around it is. Both are needed because
    /// a `view` could draw the whole catalogue and none of the reference's
    /// prose, which would be a different page.
    ///
    /// # The one thing neither test can see, measured rather than assumed
    ///
    /// `credit_card` draws its `Visit` button only when [`credit_link`] answers
    /// `Some` — the reference's `visible: modelData.url !== ""`. **Removing
    /// that condition and drawing the button unconditionally leaves both tests
    /// green**, because every credit in the catalogue has a URL, so the drawn
    /// set is identical either way. Measured: that mutation survived.
    ///
    /// It is not fixable with the real data — the two implementations differ
    /// only on a credit that has none, and there is no such credit. So it is
    /// stated instead: [`a_credit_with_no_url_draws_no_link`] proves the *gate*
    /// is right, and this pair proves the page draws what the catalogue holds.
    /// What nothing here proves is that the second calls the first; only a
    /// catalogue with a URL-less entry could, and inventing one would be a page
    /// drawing data the reference does not have.
    #[test]
    fn the_static_copy_is_drawn_too() {
        let drawn = drawn_strings(view(CreditsPage));
        for (what, literal) in [
            ("the lead heading", LEAD_HEADING),
            ("the maker line", MAKER_LINE),
            ("the acknowledgement", acknowledgement()),
            ("the rationale heading", WHY_HEADING),
            ("the rationale lead", WHY_LEAD),
            ("the footer link label", GITHUB_LABEL),
        ] {
            assert!(
                drawn.iter().any(|text| text == literal),
                "{what} is not drawn; drawn: {drawn:?}"
            );
        }
        for (heading, body) in rationale() {
            assert!(
                drawn.iter().any(|text| text == heading) && drawn.iter().any(|text| text == body),
                "the rationale entry {heading:?} is not drawn in full"
            );
        }
        assert!(drawn.iter().any(|text| text == &version_line()));
        assert!(drawn.iter().any(|text| text == &footer_line()));
    }

    /// The strings a real element hands the operation traversal.
    ///
    /// The same mechanism `crate::view::widgets`'s tests and `main.rs`'s
    /// `drawn_strings` use: `iced` exposes no downcast, so the text a widget
    /// draws is reachable only through `Widget::operate`.
    fn drawn_strings(mut element: Element<'static, Message>) -> Vec<String> {
        use cosmic::iced::advanced::widget::{Operation, Tree};
        use cosmic::iced::advanced::{layout::Limits, Layout};
        use cosmic::iced::{Font, Pixels, Rectangle, Size};

        #[derive(Default)]
        struct Texts(Vec<String>);
        impl Operation for Texts {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn text(&mut self, _id: Option<&cosmic::widget::Id>, _bounds: Rectangle, text: &str) {
                self.0.push(text.to_string());
            }
        }

        // `layout` and `operate` take the renderer by shared reference; passing
        // it by `&mut` is `clippy::unnecessary_mut_passed`.
        let renderer = cosmic::Renderer::new(Font::default(), Pixels(16.0));
        let mut tree = Tree::new(element.as_widget());
        let limits = Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = element.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let mut texts = Texts::default();
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut texts);
        texts.0
    }
}
