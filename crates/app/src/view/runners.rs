//! The Runners page — `RunnersPage.qml`, `ux.md` §5, PLAN P-32…P-39.
//!
//! # The split, and where it differs from [`super::widgets`]
//!
//! [`super::widgets`] is generic over the message type and reads no state; this
//! module fixes `Message` and reads [`State`], because a *page* is where the
//! two meet and pretending otherwise would put the binding somewhere less
//! visible. What it keeps from that module is the part that matters: every
//! decision a test can make is a **pure function over data** in the first half
//! of this file, and [`view`] is thin enough to be nothing but arrangement.
//!
//! ```text
//! installed_rows   → who is installed, and whether each can be removed
//! release_rows     → which builds are offered, and which are already here
//! status_line      → the tri-state sentence under "Available versions"
//! family_note      → the two-line description under the family selector
//! guide_subtitle   → "proton · maintained by GloriousEggroll"
//! release_detail   → "Proton-GE · GE-Proton9-5.tar.gz · 412 MB"
//! progress_fraction → whether the bar is drawn, and how full
//! uninstall_line   → what a removal's toast says, either way
//! ```
//!
//! Every one of those is asserted without a renderer.
//!
//! # What is *not* here yet, and why the shape is a parameter rather than a read
//!
//! [`view`] takes a [`RunnersView`] — a borrowed bundle of the page's inputs —
//! rather than reaching into [`State`] itself. Two of those inputs are the
//! reason the shape is what it is:
//!
//! * **the installed rows**, because building them runs `wine --version`
//!   ([`WineRunner::version`]), which spawns a process with a fifteen-second
//!   bound. The reference evaluates `installedRunners` as a QML `Property`, so
//!   it runs on every read; a libcosmic `view()` is called on every frame, and
//!   spawning there would be a hang with a friendly face. The rows are
//!   therefore computed off the render path and **held** — `State::installed`
//!   and `State::release_rows`, written by [`refresh`] and by nothing else.
//! * **the family selector's current value**, which the reference keeps in a
//!   QML-local `property string selectedFamilyId: "proton-ge"` and which
//!   Python's `_releases_family` mirrors. It is held in `State` for the same
//!   reason a QML property cannot be used here: `view` is handed data and reads
//!   no globals, so the value the dropdown shows has to outlive the frame.
//!
//! Borrowing them rather than reading `State` inside [`view`] is what makes
//! [`view`] callable from a test with no `State` at all — which is how the
//! render-level assertions in the module's tests run. Where the two are *held*
//! is a `State` question and is documented there.
//!
//! # What is handled, and what is *not*
//!
//! [`update`] handles every one of this page's messages. Two of them reach the
//! network — `FetchReleases` and `InstallRunner` — and both run `core`'s work
//! through the concrete [`HttpClient`] this crate injects (DECISIONS D-26):
//! `crate::http` is that client, and it is the reason these arms can return a
//! task rather than a promise of one.
//!
//! Neither returns `None`, and that is now a statement about the *page* rather
//! than about this module: `None` means *not mine*, so an arm that declined one
//! of the page's own messages would fall through the shell's dispatcher and do
//! nothing at all. The two arms that used to decline — both of them were
//! declined while the work behind them was missing — now do the work, and the
//! messages they *do* drop (a tag the page is not offering, a click while a
//! download is running) return `Some(Task::none())`, which is the different
//! statement: handled, with nothing to do.
//!
//! [`WineRunner::version`]: gamehandler_core::runners::WineRunner::version
//! [`HttpClient`]: gamehandler_core::runners::proton::HttpClient

use cosmic::Element;
use cosmic::app::Task;
use cosmic::iced::{Alignment, Background, Border, Length};
use cosmic::widget::{Column, Row, Space, button, container, divider, icon, progress_bar, text};
use gamehandler_core::runners::families::{
    ReleaseInfo, RunnerFamily, RunnerGuide, families, runner_guide_details,
};
use gamehandler_core::runners::proton;
use gamehandler_core::runners::proton::HttpClient;
use gamehandler_core::runners::{ProtonRunner, Runner, RunnerError, SYSTEM_WINE, SystemLaunchEnv};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `StreamExt::map`, for turning the install's report channel into a task.
use cosmic::iced::futures::StreamExt;

use crate::Message;
use crate::state::{ReleasesStatus, State};

use super::badge::badge;

// ---------------------------------------------------------------------------
// The decisions, as data
// ---------------------------------------------------------------------------

/// One row of the "Installed" list. `bridge.py:621-646`.
///
/// `removable` is System Wine's row being the only one that is not: the
/// reference sets it `False` there because there is nothing of GameHandler's to
/// delete — and the id is [`SYSTEM_WINE`], the same constant Python puts on the
/// row (`runners.py:39,414`), not a word invented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRow {
    /// The runner id the remove action names.
    pub runner_id: String,
    /// The display name.
    pub name: String,
    /// The status line under it — a version, "Not installed on this system", or
    /// the family label.
    pub detail: String,
    /// Whether the build can actually be launched with. Drives the status icon:
    /// `emblem-checked` when true, `data-warning` when false.
    pub available: bool,
    /// Whether the delete button is drawn at all.
    pub removable: bool,
}

/// One row of "Available versions". `bridge.py:672-681`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRow {
    /// The release tag, which the install action names.
    pub tag: String,
    /// `"<family> · <asset> · <n> MB"`.
    pub detail: String,
    /// Whether a build for this tag is already on disk, which swaps the Install
    /// button for the badge.
    pub installed: bool,
}

/// One row of the guide. `bridge.py:663-670`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideRow {
    pub title: String,
    pub subtitle: String,
    pub advice: String,
    /// Empty means the "Visit project" button is not drawn
    /// (`RunnersPage.qml:233`).
    pub homepage: String,
}

/// The rows the "Installed" section shows, System Wine first.
///
/// Split from [`view`] because building it is the expensive part: the system
/// runner's `detail` is `wine --version`, which is a subprocess.
pub fn installed_rows(system: &dyn Runner, protons: &[ProtonRunner]) -> Vec<InstalledRow> {
    let available = system.is_available();
    let mut rows = vec![InstalledRow {
        runner_id: SYSTEM_WINE.to_string(),
        name: "System Wine".to_string(),
        detail: if available {
            system.version()
        } else {
            "Not installed on this system".to_string()
        },
        available,
        removable: false,
    }];
    rows.extend(protons.iter().map(|proton| InstalledRow {
        runner_id: proton.id().to_string(),
        name: proton.name().to_string(),
        detail: proton.family_label(),
        available: true,
        removable: true,
    }));
    rows
}

/// The rows "Available versions" shows for the current family.
///
/// `runners_directory` is what decides `installed`: Python asks the manager,
/// which is a filesystem question, and the answer changes when a build is
/// installed or removed — so it is read here rather than cached in the release
/// list, which is refetched far less often.
pub fn release_rows(releases: &[ReleaseInfo], runners_directory: &Path) -> Vec<ReleaseRow> {
    releases
        .iter()
        .map(|release| ReleaseRow {
            tag: release.tag.clone(),
            detail: release_detail(release),
            installed: proton::is_release_installed(runners_directory, release),
        })
        .collect()
}

/// `"<family name> · <asset name> · <n> MB"`, the line under a release's tag.
///
/// `size_mb` is `round`ed, as `bridge.py:677` does — a build shown as
/// "412.6 MB" would be a number the reference never displays. The family's name
/// comes from the catalogue rather than from the release, which carries only
/// its id; an id naming no family is shown as-is rather than as an empty gap,
/// because that is what a lookup miss should look like to a user.
pub fn release_detail(release: &ReleaseInfo) -> String {
    let family = families()
        .iter()
        .find(|family| family.id == release.family_id)
        .map_or(release.family_id.as_str(), |family| family.name);
    format!(
        "{} · {} · {} MB",
        family,
        release.name,
        release.size_mb().round() as i64
    )
}

/// The sentence under "Available versions", or `None` when the list speaks for
/// itself.
///
/// `None` covers exactly the case the reference hides the label in
/// (`RunnersPage.qml:158`): a ready, non-empty list. Every other status is a
/// string, and the error one carries the message verbatim — Python's
/// `releasesStatus` is `"error: <message>"` and the QML strips the seven
/// characters itself, so the prefix is part of the wire format and the strip is
/// here rather than at the call site.
pub fn status_line(status: &ReleasesStatus, release_count: usize) -> Option<String> {
    match status {
        ReleasesStatus::Ready if release_count > 0 => None,
        ReleasesStatus::Loading => Some("Fetching the latest builds…".to_string()),
        ReleasesStatus::Error(message) => Some(format!("Could not fetch builds — {message}")),
        ReleasesStatus::Ready | ReleasesStatus::Idle => {
            Some("No builds found for this family.".to_string())
        }
    }
}

/// The two lines under the family selector: the description, then who
/// maintains it when anyone does. `RunnersPage.qml:131-139`.
pub fn family_note(family: &RunnerFamily) -> String {
    if family.maintainer.is_empty() {
        family.description.to_string()
    } else {
        format!(
            "{}\nMaintained by {}",
            family.description, family.maintainer
        )
    }
}

/// `"proton · maintained by GloriousEggroll"`, or just the kind.
///
/// The kind is capitalised, which `bridge.py:666` does on the Python side
/// (`.capitalize()`) — so the capital is part of the data the page is handed,
/// not a rendering choice made here.
pub fn guide_subtitle(guide: &RunnerGuide) -> String {
    if guide.maintainer.is_empty() {
        guide.kind.clone()
    } else {
        format!("{} · maintained by {}", guide.kind, guide.maintainer)
    }
}

/// The guide rows, ready to draw.
pub fn guide_rows() -> Vec<GuideRow> {
    runner_guide_details()
        .into_iter()
        .map(|guide| GuideRow {
            subtitle: guide_subtitle(&guide),
            title: guide.title,
            advice: guide.advice,
            homepage: guide.homepage,
        })
        .collect()
}

/// The progress bar's fraction, or `None` when it is not drawn.
///
/// `RunnersPage.qml:33-38` shows the bar when `backend.busy` is true and
/// `backend.progress` is at least zero, clamped up by `Math.max(0, progress)`
/// over a `0..1` range. `busy` is *either* long job ([`State::busy`]), so the
/// easy-install path's progress draws this page's bar too — which is the
/// reference's own coupling and is preserved rather than tidied, because a bar
/// that vanished when the other job started would be a regression a user sees.
///
/// The sign test is the `-1.0` sentinel: [`State::progress`] is `None` when
/// idle, so both spellings collapse to the same `None` here.
pub fn progress_fraction(state: &State) -> Option<f32> {
    if !state.busy() {
        return None;
    }
    state
        .progress
        .filter(|value| *value >= 0.0)
        .map(|value| value.max(0.0))
}

/// The index of `family_id` in the catalogue, for the selector.
pub fn family_index(family_id: &str) -> Option<usize> {
    families().iter().position(|family| family.id == family_id)
}

/// The family the page starts on before the user has chosen one.
///
/// The reference hardcodes the string in the page (`RunnersPage.qml:14`); this
/// reads the catalogue's first entry, which is the same family and cannot drift
/// from the selector's own ordering — [`family_index`] is a position in that
/// same list, so a second literal here would be a value that has to agree with
/// a table it is not derived from.
pub fn default_family() -> &'static str {
    families()[0].id
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// Everything the page draws, borrowed.
///
/// A struct rather than six arguments because the two `&[..]` and the two
/// `&str` are easy to transpose and impossible to read at a call site.
pub struct RunnersView<'a> {
    /// From [`installed_rows`], computed off the render path.
    pub installed: &'a [InstalledRow],
    /// The family the release list belongs to.
    pub selected_family: &'a str,
    /// The fetch state.
    pub status: &'a ReleasesStatus,
    /// From [`release_rows`].
    pub releases: &'a [ReleaseRow],
    /// From [`progress_fraction`].
    pub progress: Option<f32>,
}

/// Recompute the two bundles [`State`] holds for this page, as a task.
///
/// **Neither is computed in `view_body`**, which runs once per frame: both reach
/// the filesystem, and a frame is the wrong rate for it. This is the only
/// function that asks for them, so there is one place to call and one place to
/// get wrong.
///
/// It is called from the page's entry point and from every message that changes
/// its inputs; the arms below say which and why. Nothing calls it on a timer.
///
/// # Why the *work* is a task and not a call
///
/// The module's header already says the rows must be computed off the render
/// path, because building them spawns a process. That reasoning is about the
/// spawn, and the spawn is not in `view` — it is in [`installed_rows`], which
/// asks the system row for [`Runner::version`], i.e. `wine --version`. So
/// *calling* this function is the spawn, and a caller on the update thread
/// would freeze the window for as long as Wine takes to answer — up to the
/// fifteen-second bound `run_version` sets — every time the user opens the
/// page or finishes a download.
///
/// [`Task::perform`] runs its future on a tokio worker (D-48 measured the
/// backend), so the spawn happens off the update thread and the finished rows
/// come back as [`Message::RunnersRefreshed`].
///
/// # What is read here, and what is read in the future
///
/// The three cheap inputs — the `PATH` scan, the runners directory listing, and
/// the release list — are read *here*, on the calling thread, synchronously
/// with the request. That is deliberate and it is the part worth not
/// "tidying": the whole point of each call site is that the rows describe the
/// disk **as of that message**, so the question `RunnerInstallFinished` asks is
/// "what is on disk now that the download has stopped". Deferring these reads
/// into the future would answer that question some unspecified time later and
/// silently reorder it against the next request. Only the spawn is deferred.
///
/// # The token, and why there is one
///
/// Two refreshes may be in flight at once — a download finishing while an
/// uninstall runs, say — and they complete in whatever order Wine answers in.
/// The reference cannot have this problem because it has no in-flight state at
/// all: `installedRunners` is a QML `Property`, evaluated on read, so it is
/// always as fresh as the render that asked for it.
///
/// Held state reintroduces the ordering question, and the expensive call makes
/// a late reply genuinely possible rather than theoretical — the *earlier*
/// request's `wine --version` can finish *last*, and would then overwrite the
/// newer rows with older ones, resurrecting a build the user just removed. The
/// token is the same shape the family guard uses and the same shape
/// `form_cover_token` uses for cover lookups: a reply whose token is not the
/// current one is stale and is dropped.
pub fn refresh(state: &mut State) -> Task<Message> {
    // Read on the calling thread. See the note above on which half is which.
    let system = state.runners.system_wine(&SystemLaunchEnv);
    let protons = state.runners.installed_protons();
    let releases = state.releases.clone();
    let runners_directory = state.runners.runners_directory().to_path_buf();

    state.runner_rows_token = state.runner_rows_token.wrapping_add(1);
    let token = state.runner_rows_token;

    Task::perform(
        async move {
            (
                installed_rows(&system, &protons),
                release_rows(&releases, &runners_directory),
            )
        },
        move |(installed, release_rows)| {
            cosmic::Action::App(Message::RunnersRefreshed {
                token,
                installed,
                release_rows,
            })
        },
    )
}

/// The page.
pub fn view<'a>(page: RunnersView<'a>) -> Element<'a, Message> {
    let mut body = Column::new().spacing(12).width(Length::Fill);

    if let Some(fraction) = page.progress {
        body = body.push(progress_bar::determinate_linear(fraction).width(Length::Fill));
    }

    // ---- Installed ---------------------------------------------------------
    body = body.push(text::title3("Installed")).push(
        text::body("Available for launching and for the per-game runner picker.")
            .width(Length::Fill),
    );

    for row in page.installed {
        body = body.push(installed_card(row));
    }

    if page.installed.len() <= 1 {
        body = body.push(text::body(
            "No downloaded runners yet. Install a Proton or Wine build below, \
             then assign it to a game.",
        ));
    }

    body = body.push(divider::horizontal::default());

    // ---- Download a build --------------------------------------------------
    body = body.push(text::title3("Download a build")).push(text::body(
        "GameHandler fetches these archives from each maintainer's own release \
             page, the same upstream sources ProtonPlus uses. Nothing is bundled or \
             re-hosted here.",
    ));

    let names: Vec<String> = families().iter().map(|f| f.name.to_string()).collect();
    let selected = family_index(page.selected_family);
    body = body.push(
        Row::new()
            .push(text::body("Family:"))
            .push(cosmic::widget::dropdown(names, selected, move |index| {
                Message::FetchReleases {
                    family: families()[index].id.to_string(),
                }
            }))
            .spacing(8)
            .align_y(Alignment::Center),
    );

    if let Some(family) = selected.and_then(|index| families().get(index)) {
        body = body.push(text::body(family_note(family)));
        // The reference's `Kirigami.UrlButton` (`RunnersPage.qml:144-150`) opens
        // the maintainer's page itself; `button::link` carries no href, so the
        // URL travels as the press message — built by [`family_press`], which
        // the test below reads because a built button cannot be.
        let url = family.homepage();
        body = body.push(button::link(url.clone()).on_press(family_press(family)));
    }

    // ---- Available versions ------------------------------------------------
    body = body.push(text::title3("Available versions"));

    if let Some(line) = status_line(page.status, page.releases.len()) {
        body = body.push(text::body(line));
    }

    for release in page.releases {
        body = body.push(release_card(release));
    }

    body = body.push(divider::horizontal::default());

    // ---- The guide ---------------------------------------------------------
    body = body
        .push(text::title3("Which runner should I use?"))
        .push(text::body(
            "Proton builds are the usual choice for Windows games; standalone Wine \
             is lighter and better for some older titles. Each entry links to the \
             project that maintains it.",
        ));

    for guide in guide_rows() {
        body = body.push(guide_card(&guide));
    }

    cosmic::widget::scrollable(body).into()
}

/// A row of the Installed list: status icon, name, detail, and a delete button
/// on everything removable.
fn installed_card(row: &InstalledRow) -> Element<'_, Message> {
    let status_icon = if row.available {
        crate::icons::Icon::Checked
    } else {
        crate::icons::Icon::Warning
    };

    let mut line = Row::new()
        .push(icon::icon(crate::icons::handle(status_icon)).size(24))
        .push(
            Column::new()
                .push(text::heading(row.name.clone()))
                .push(text::caption(row.detail.clone()))
                .spacing(2)
                .width(Length::Fill),
        )
        .spacing(12)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if row.removable {
        // The site is **unobservable**, and naming it here is the whole
        // mitigation. [`remove_press`] mutated to name a constant —
        // `"system"`, one keystroke from `SYSTEM_WINE` — survives every test
        // in this file.
        //
        // There is no reader to close it with, and this is established by
        // reading the vendored sources rather than by assuming:
        // `Operation`'s seven arms carry no message
        // (`iced/core/src/widget/operation.rs:21-68`), cosmic `Button::operate`
        // reports only `container` and `focusable`
        // (`src/widget/button/widget.rs:338-357`), and `Button.on_press` is a
        // private field holding an opaque `Box<dyn Fn(..) -> Message>`
        // (`widget.rs:48`) rather than a comparable value.
        //
        // [`remove_press`] exists because the *value* should still be a readable
        // thing rather than a literal buried in a builder — but it does not
        // close this line. A test can only call the helper directly, and a site
        // that routed around it would survive that test unchanged. Moving the
        // value is not the same as testing this call; both statements are needed
        // and neither substitutes for the other.
        //
        // So the value is checked where it can be, in
        // `the_system_row_names_python_s_runner_id_and_not_the_family_id` (the
        // field `installed_rows` sets) and
        // `installers.rs`'s `install_press` (the same decision on the other
        // page, where the guard is at least a value). This line is checked by
        // reading it.
        line = line.push(
            button::icon(crate::icons::handle(crate::icons::Icon::Delete))
                .on_press(remove_press(row)),
        );
    }

    card(line.into())
}

/// A release row: tag, detail, and either the badge or the Install button.
fn release_card(row: &ReleaseRow) -> Element<'_, Message> {
    let mut line = Row::new()
        .push(
            Column::new()
                .push(text::heading(row.tag.clone()))
                .push(text::caption(row.detail.clone()))
                .spacing(2)
                .width(Length::Fill),
        )
        .spacing(12)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if row.installed {
        line = line.push(badge("Installed"));
    } else {
        line = line.push(
            button::standard("Install")
                .leading_icon(crate::icons::handle(crate::icons::Icon::Download))
                .on_press(Message::InstallRunner {
                    tag: row.tag.clone(),
                }),
        );
    }

    card(line.into())
}

/// A guide row: title, a "Visit project" link when there is one, the
/// kind-and-maintainer line, and the advice.
fn guide_card(guide: &GuideRow) -> Element<'static, Message> {
    let mut heading = Row::new()
        .push(text::title4(guide.title.clone()))
        .push(Space::new().width(Length::Fill))
        .width(Length::Fill);

    // The reference's `Kirigami.UrlButton` (`RunnersPage.qml:233-237`)
    // carries `url:` and opens it on click; `button::link` carries only a
    // label, so the URL travels as the press message — built by
    // [`guide_press`], which the test below reads because a built button
    // cannot be. P-33. The `if let` is also the `visible: homepage !== ""`
    // gate: no homepage, no button.
    if let Some(press) = guide_press(guide) {
        heading = heading.push(button::link("Visit project".to_string()).on_press(press));
    }

    card(
        Column::new()
            .push(heading)
            .push(text::caption(guide.subtitle.clone()))
            .push(text::body(guide.advice.clone()))
            .spacing(2)
            .width(Length::Fill)
            .into(),
    )
}

/// The message the family selector's project link sends. P-33.
///
/// A named function rather than an inline `Message::OpenUrl(family.homepage())`
/// for the same reason `credits.rs`'s [`credit_press`](super::credits::credit_press)
/// is one: the message a `button::link` holds cannot be read back out of the
/// built widget (no downcast; the sources are cited at [`installed_card`]), so
/// the press is produced here and the view merely calls this. A call site that
/// routed around it — or a `button::link` that lost its `on_press` — would render
/// disabled (`libcosmic/src/widget/button/link.rs` → `button/widget.rs:148`),
/// which no string assertion can see.
pub fn family_press(family: &RunnerFamily) -> Message {
    Message::OpenUrl(family.homepage())
}

/// The message a guide row's "Visit project" link sends, or `None` when the row
/// has no homepage. P-33.
///
/// `None` and not a fallback for the same reason as
/// [`credit_press`](super::credits::credit_press)'s: the reference draws no
/// button at all on an empty url (`RunnersPage.qml:234`), so there is no message
/// to send and inventing one would be a control the QML does not have.
pub fn guide_press(guide: &GuideRow) -> Option<Message> {
    if guide.homepage.is_empty() {
        None
    } else {
        Some(Message::OpenUrl(guide.homepage.clone()))
    }
}

/// The card surface, matching `super::widgets`' `card_style`.
///
/// Repeated rather than shared because that one is a private function of a
/// module over `&Game`, and T-14 owns it. If a third page needs it, it moves;
/// two is not yet a pattern worth a module of its own, and a card whose surface
/// came from somewhere else would be the second visual idiom.
fn card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(12)
        .style(card_style)
        .into()
}

fn card_style(theme: &cosmic::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(
            theme.cosmic().background(false).base.into(),
        )),
        border: Border {
            radius: 14.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The handler
// ---------------------------------------------------------------------------

/// The line a removal ends in, either way.
///
/// A pure function rather than a `format!` inside the arm, and the reason is
/// testability rather than taste: a removal's toast is its **only** observable.
/// `Toasts` keeps its queue in a private `SlotMap` and exposes no iterator —
/// `push`, `remove` and nothing else (`src/widget/toaster/mod.rs:155-204`) — so
/// no test can read back the string that was pushed. A line built inline in the
/// arm is therefore untestable by construction, and an error text dropped there
/// is invisible: the arm still returns a task and still clears
/// [`State::releases_status`], so a test that only drove `update` would stay
/// green while the user was told nothing.
///
/// Extracted for the same reason [`release_detail`] and [`status_line`] are —
/// one visible decision, one function, one test — and generic over the error so
/// both halves can be driven without a runner directory to fail against.
///
/// # What this extraction does and does not close — measured, not argued
///
/// **The line is now readable; the call site is not.** The extracted body is
/// covered: mutating `Err(error) => format!("Could not remove {runner_id}:
/// {error}")` to drop the error tail fails
/// `a_failed_removal_carries_the_error_text_and_the_runner_id` — a code span
/// rather than a link, because that test is a `#[cfg(test)]` item
/// (`runners.rs:1915`) and rustdoc does not document those, so the bracket form
/// resolved to nothing — and mutating it to `Err(_) => String::new()` fails it
/// too.
///
/// Two call-site mutations **survive**, and are recorded rather than papered
/// over:
///
/// ```text
/// A   the arm ignores this function and formats success itself   SURVIVES
/// A2  the call site passes a constant runner id, not `runner_id` SURVIVES
/// ```
///
/// Both are unobservable for the reason above: the toast is the only output of
/// this path and `Toasts` has no reader, so no test can see what the arm
/// actually pushed. Closing them needs a `Toasts` accessor that libcosmic does
/// not expose — the same wall `DismissToast` runs into, already measured and
/// accepted for this project. A test double would be satisfied by the very
/// thing it cannot distinguish, so there is deliberately none.
fn uninstall_line<E: std::fmt::Display>(runner_id: &str, result: Result<(), E>) -> String {
    match result {
        Ok(()) => format!("Removed {runner_id}"),
        Err(error) => format!("Could not remove {runner_id}: {error}"),
    }
}

/// The message this helper builds for the remove button — **the value is
/// readable here; whether [`installed_card`] calls it is not, see below.**
///
/// The first line says "this helper" rather than "the remove button" for the
/// reason the whole extraction is documented the way it is: the button carries
/// whatever `installed_card` builds, and nothing can read a built `Button`'s
/// message back out (the sources are cited at the call site). Naming the helper
/// keeps the claim inside what the measurement supports.
///
/// The message is the confirm pair's first half, not the removal: the
/// reference's delete button opens `removeRunnerDialog` (`RunnersPage.qml:84`)
/// and the dialog's Remove is what sends `runnerId` to `uninstallRunner`
/// (`:267-271`). It carries the row's display name too, because the dialog
/// titles `"Remove {name}?"` (`:259`) and the button is the only place that
/// knows both halves. P-37.
///
/// This is [`InstalledRow::runner_id`]'s remaining consumer. The field's other
/// half — the value [`installed_rows`] puts there — is pinned by
/// `the_system_row_names_python_s_runner_id_and_not_the_family_id` (a code span:
/// a `#[cfg(test)]` item at `runners.rs:1084`, which rustdoc does not document);
/// this makes the value the button *would* carry readable, so `"system"` cannot
/// replace it in the helper.
///
/// What this does **not** close: `installed_card` could stop calling this and
/// build the message itself, and that mutation survives — the site is
/// unobservable for the reason given there. This is the third such gap, beside
/// `uninstall_line`'s call site (A) and `install_press`'s (B).
fn remove_press(row: &InstalledRow) -> Message {
    Message::ConfirmRemoveRunner {
        runner_id: row.runner_id.clone(),
        name: row.name.clone(),
    }
}

/// The Runners page's half of the dispatcher.
///
/// `None` means *this page does not handle that message*, which is a different
/// statement from `Some(Task::none())` — see this module's header for which
/// messages are which and why `FetchReleases`/`InstallRunner` are declined
/// rather than half-written.
///
/// Each arm below is one of `bridge.py`'s async `done`/`fail` closures, ported
/// as the state transition they are.
/// Python's `limit=12` (`bridge.py:702`), applied after parsing by
/// [`proton::fetch_available`], so these are twelve *usable* releases.
const RELEASES_LIMIT: usize = 12;

/// The fetch's ceiling.
///
/// `fetch_available`'s own default (`runners.py:818`), applied to the request
/// at `urlopen(req, timeout=timeout)` (`runners.py:830`), with `fetchReleases`
/// passing nothing to override it (`bridge.py:702`) — so 30 s is the
/// **reference's** bound rather than a number chosen here. [`HttpClient::get`]
/// takes its timeout as an argument and has no default, and this constant is
/// what carries the reference's value into it; the port adds no second bound.
///
/// **An earlier version of this comment said the opposite**, and the reversal
/// is worth one line because the comment is what a reader trusts instead of
/// re-reading the Python: it claimed Python passed no timeout, inherited
/// `requests`', and left a worker parked forever. All three are false —
/// `gamehandler/` imports `urllib.request` and never `requests` (`runners.py`
/// `:33`), the timeout is explicit, and a stalled connection fails at 30 s
/// there exactly as it does here.
const RELEASES_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetch one family's releases through the injected client (D-26).
///
/// Split out from the task below so it can be tested with a fake client: the
/// limit, the family that reaches the client, and the error that comes back are
/// all decisions, and none of them is observable through a `Task`.
pub fn fetch_releases(
    client: &dyn HttpClient,
    family: &str,
) -> Result<Vec<ReleaseInfo>, RunnerError> {
    proton::fetch_available(client, Some(family), RELEASES_LIMIT, RELEASES_TIMEOUT)
}

/// `str(exc) or exc.__class__.__name__` (`bridge.py:157`) — the one place an
/// error becomes the text a user reads.
///
/// The fallback is not defensive padding. `_async` renders whatever the
/// background work raised, and on the failures that are commonest in the field
/// `str()` is **empty** — `ConnectionResetError`, `TimeoutError` and
/// `http.client.RemoteDisconnected` all render as `''`, measured and recorded
/// in `VERIFY-FINDINGS` §5. Without this the status line shows
/// `Could not fetch builds — ` with a dangling dash, which is what
/// [`status_line`] builds when the message is empty.
///
/// [`RunnerError::class_name`] is the analogue of `__class__`; it is asked only
/// when the rendered message is empty, so the ordinary case is unchanged.
pub fn rendered_message(error: &RunnerError) -> String {
    let text = error.to_string();
    if text.is_empty() {
        error.class_name().to_string()
    } else {
        text
    }
}

/// The task half of [`Message::FetchReleases`].
///
/// `Task::perform` runs this off the UI thread, so the blocking client inside
/// [`fetch_releases`] needs no async adaptation and no second hop — iced's
/// native executor is the tokio runtime, which spawns onto a worker
/// (`iced/futures/src/backend/native/tokio.rs`). That is the same shape as the
/// reference, which runs the work on a `threading.Thread` and dispatches the
/// result back (`bridge.py:152-166`).
fn fetch_releases_task(family: String) -> Task<Message> {
    Task::perform(
        async move {
            // `str(exc)` — `_async` turns whatever the work raised into a
            // string and hands it to `fail` (`bridge.py:157`), so the variant
            // carries the rendered message rather than the error type. The
            // status line prefixes the sentence for display; Python keeps the
            // `"error: "` prefix on the wire and strips seven characters in the
            // QML (`RunnersPage.qml:164`), which [`status_line`] does not need
            // to reproduce.
            let result = fetch_releases(&crate::http::UreqClient, &family)
                .map_err(|error| rendered_message(&error));
            Message::ReleasesFetchFinished { family, result }
        },
        // `cosmic::app::Task<Message>` is `iced::Task<Action<Message>>`, so a
        // task built here must wrap its output in `Action::App` to reach the
        // shell's `update`. This is the idiomatic libcosmic mapping.
        cosmic::Action::App,
    )
}

/// The install's ceiling. `ProtonManager.install`'s own default
/// (`runners.py:868`), and `installRelease` passes no timeout (`bridge.py:750`),
/// so 60 s is the reference's effective value rather than a choice made here.
///
/// It is deliberately not [`RELEASES_TIMEOUT`]. The two are separate numbers in
/// the reference and a build is ~100 MB where a releases listing is a few KB, so
/// sharing one constant would silently couple the listing's patience to the
/// download's or the reverse.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// [`Message::InstallRunner`]'s task: a blocking install that reports progress.
///
/// # Why this is a stream and not `Task::perform`
///
/// The fetch handler returns one value when it finishes, so `Task::perform` is
/// enough. This one reports *while* it runs — `install` takes a
/// `&dyn Fn(f32)` progress callback, which the reference wires to
/// `_progress_cb` and which reaches the page as [`Message::RunnerProgress`].
/// `Task::perform`'s mapper is `FnOnce`, so it cannot yield a message more than
/// once; `Task::stream` over a channel can, and the channel is what lets the
/// callback — which is synchronous and takes no `&mut State` — reach the
/// update loop at all.
///
/// # Why a thread, when the executor is already off the UI thread
///
/// `Task::stream`'s future runs on the executor, which is where the blocking
/// `install` would block a worker for the whole download. The blocking work is
/// therefore moved to its own thread, exactly as Python's `_async` does
/// (`bridge.py:152-166`), and the stream only forwards what that thread sends.
/// The sender is dropped when the thread ends, which ends the stream; there is
/// no separate "done" signal to get wrong.
///
/// **The order is load-bearing**: the progress reports and the final
/// [`Message::RunnerInstallFinished`] go through one channel, so the result
/// cannot overtake the last progress tick and leave the bar drawn above a
/// finished install.
///
/// # What ends the stream, and the one case where nothing does
///
/// The sender is moved into the thread and dropped when it ends, so the stream
/// finishes on its own — including when the install returns `Err`, which is the
/// ordinary path. The exception is a **panic**: the thread unwinds, the sender
/// drops with the panic message on stderr, and the stream ends without a
/// [`Message::RunnerInstallFinished`], leaving the page busy. That is left
/// unguarded deliberately. Python wraps the work in `try/except` because an
/// exception is its ordinary error channel; here the ordinary channel is
/// `Result`, which [`proton::install`] uses throughout, so a panic is a bug in
/// this port rather than a network condition — and converting it into a toast
/// would hide it. A `catch_unwind` would inherit [`Task::perform`]'s lack of one
/// in the fetch handler, so the two would be asymmetric for no gain.
fn install_runner_task(release: ReleaseInfo, runners_directory: PathBuf) -> Task<Message> {
    // `unbounded` because the callback is synchronous and cannot await: a bounded
    // send from inside it would have to drop reports, and a dropped report is a
    // progress bar that stalls. The volume is one message per archive chunk.
    let (sender, receiver) = cosmic::iced::futures::channel::mpsc::unbounded::<Message>();

    std::thread::spawn(move || {
        let tag = release.tag.clone();
        let progress = |fraction: f32| {
            // A send fails only when the receiver is gone, i.e. the page was
            // navigated away from and the task dropped. Nothing to report to.
            let _ = sender.unbounded_send(Message::RunnerProgress(fraction));
        };
        let result = proton::install(
            &crate::http::UreqClient,
            &runners_directory,
            &release,
            &progress,
            INSTALL_TIMEOUT,
        )
        .map(|_installed_to| ())
        .map_err(|error| rendered_message(&error));
        // The tag travels with both arms: `fail` names the release it could not
        // install (`bridge.py:760`), so it cannot be recovered from a `Err`.
        let _ = sender.unbounded_send(Message::RunnerInstallFinished { tag, result });
    });

    // Each report becomes `Action::App(message)` because a task built in a page
    // is `iced::Task<Action<Message>>` — the same mapping `fetch_releases_task`
    // explains at its `Task::perform` call.
    Task::stream(receiver.map(cosmic::Action::App))
}

pub fn update(state: &mut State, message: &Message) -> Option<Task<Message>> {
    match message {
        // `fetchReleases` clears the list and marks the fetch in flight before
        // the work starts (`bridge.py:697-700`), so a stale list is never shown
        // under a loading label.
        Message::FetchReleases { family } => {
            state.releases_family = family.clone();
            state.releases_status = ReleasesStatus::Loading;
            state.releases.clear();
            // The cleared list is one of the two things `refresh` derives, so
            // the rows are recomputed alongside the fetch rather than after it.
            let rows = refresh(state);
            Some(Task::batch([rows, fetch_releases_task(family.clone())]))
        }
        // `done`/`fail` in one arm, because the guard is the same for both:
        // a reply for a family the user has navigated away from is dropped
        // (`bridge.py:705-706`).
        Message::ReleasesFetchFinished { family, result } => {
            if *family != state.releases_family {
                return Some(Task::none());
            }
            let rows = match result {
                Ok(found) => {
                    state.releases = found.clone();
                    state.releases_status = ReleasesStatus::Ready;
                    // The new list changes both the rows and, for each tag that
                    // is already on disk, the Install/badge decision.
                    refresh(state)
                }
                Err(message) => {
                    state.releases_status = ReleasesStatus::Error(message.clone());
                    // Nothing on disk changed and the list is not drawn, so the
                    // rows already held are still the ones this reply would
                    // produce. Recomputing them here would spend a
                    // `wine --version` on a message the rows do not depend on.
                    Task::none()
                }
            };
            Some(rows)
        }
        // `installRelease` (`bridge.py:738-769`).
        //
        // The two early returns are the reference's, and both are silent: a
        // click while a download is already running is dropped rather than
        // queued (`if self._runner_busy: return`, `bridge.py:739-740`), and so
        // is a tag that is not in the list the page is showing
        // (`if release is None: return`, `bridge.py:742-743`) —
        // which a stale page can still send after a family change, since the
        // button was drawn from the list that has since been replaced.
        //
        // The state is written **before** the work starts, so the bar and the
        // disabled buttons are up while the download is in flight; that is
        // Python's order too (`busyChanged` before `_async`).
        Message::InstallRunner { tag } => {
            if state.runner_busy {
                return Some(Task::none());
            }
            let Some(release) = state.releases.iter().find(|item| item.tag == *tag).cloned() else {
                return Some(Task::none());
            };
            state.runner_busy = true;
            state.progress = Some(0.0);
            // No `refresh` here, deliberately: nothing has changed on disk yet,
            // and the progress messages that follow arrive many times a second.
            // The two arms that end a download do refresh.

            // The toast and the download are one task, so the page cannot end up
            // busy with no explanation of why.
            Some(Task::batch([
                push_toast(state, format!("Downloading {tag}…")),
                install_runner_task(
                    release.clone(),
                    state.runners.runners_directory().to_path_buf(),
                ),
            ]))
        }
        // `_progress_cb` (`bridge.py:733-735`).
        Message::RunnerProgress(fraction) => {
            state.progress = Some(*fraction);
            Some(Task::none())
        }
        // `done` and `fail` both clear the guard and the bar before they
        // differ, so they are written once and the message is the only
        // difference (`bridge.py:753-767`).
        Message::RunnerInstallFinished { tag, result } => {
            state.runner_busy = false;
            state.progress = None;
            // A build may now be on disk, so both the Installed list and every
            // release row's badge are stale until this runs — and it runs on the
            // failure path too, where nothing changed. Python's `fail` path
            // emits `notify` alone (`bridge.py:763-767`), and that is not a
            // counter-argument: its rows are a QML `Property` read at render
            // time, so they are recomputed after a failure there whether or not
            // anything was emitted. Held rows have to be recomputed explicitly
            // or they go stale exactly when the user is looking for the reason.
            // The cost is one `PATH` scan, one readdir and one `wine --version`
            // per *finished download*, not per frame.
            let rows = refresh(state);
            // Both arms name the release (`bridge.py:756-758`, `:767`), which is
            // why the tag travels on the message rather than only on success.
            let line = match result {
                Ok(()) => {
                    format!("Installed {tag}. You can now choose it when adding or editing a game.")
                }
                Err(message) => format!("Failed to install {tag}: {message}"),
            };
            Some(Task::batch([rows, push_toast(state, line)]))
        }
        // Synchronous in the reference too (`bridge.py:772`), and its two
        // messages are the whole observable.
        //
        // Nothing else is touched, and that is Python's shape rather than an
        // omission: `uninstallRunner` assigns no field of the bridge at all, it
        // only notifies (`bridge.py:772-782`). The success path's extra
        // `releasesChanged.emit()` re-reads `installed` on every row, and this
        // port gets that for nothing because [`release_rows`] calls
        // `is_release_installed` while it builds the rows — the badge is
        // recomputed on the next render, so no state change is needed to ask for
        // it. An earlier revision set [`ReleasesStatus::Idle`] here to stand in
        // for that emit, which was wrong twice over: the reference's status is
        // written by `fetchReleases` and its callbacks and nowhere else
        // (`bridge.py:697,708,714`), and `Idle` with a non-empty list is a state
        // the reference cannot reach — it renders "No builds found for this
        // family." directly above the list, because [`status_line`]'s `Idle` arm
        // is the same sentence as its empty-`Ready` one.
        // The delete button on an installed row (`RunnersPage.qml:82-85`): the
        // reference opens `removeRunnerDialog` rather than deleting, and the
        // dialog's Remove sends the row's `runnerId` to `uninstallRunner`. The
        // pending removal is the page-local `pendingRemove` (`:15`) — the id
        // the removal will name and the display name the dialog titles — held
        // on `State` because `view` is handed data and reads no globals. P-37.
        Message::ConfirmRemoveRunner { runner_id, name } => {
            state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
                runner_id: runner_id.clone(),
                name: name.clone(),
            });
            // One modal layer: opening this dialog closes a pending game
            // removal, as `ConfirmDeleteGame` closes this one.
            state.confirm_delete = None;
            Some(Task::none())
        }
        // The dialog's Remove (`RunnersPage.qml:267-271`): the pending removal
        // is cleared first so the dialog closes whether or not the removal
        // succeeds, and the removal itself is the same one `UninstallRunner`
        // names below — reached through [`remove_runner`] so the two cannot
        // diverge.
        Message::RemoveRunnerConfirmed(runner_id) => {
            state.confirm_remove_runner = None;
            Some(remove_runner(state, runner_id))
        }
        Message::UninstallRunner(runner_id) => Some(remove_runner(state, runner_id)),
        // The rows [`refresh`] computed, back from the worker thread that
        // spawned Wine.
        //
        // The token check is the stale-reply guard `refresh`'s doc explains;
        // there is no Python line for it because Python holds no rows between
        // renders. A superseded reply is dropped and the state is left alone —
        // returning the newer rows unchanged, not clearing them.
        Message::RunnersRefreshed {
            token,
            installed,
            release_rows,
        } => {
            if *token != state.runner_rows_token {
                return Some(Task::none());
            }
            state.installed = installed.clone();
            state.release_rows = release_rows.clone();
            Some(Task::none())
        }
        _ => None,
    }
}

/// The removal `uninstallRunner` names (`bridge.py:772-782`), shared by the
/// dialog's confirmation and the direct route so the two cannot diverge.
///
/// A build is gone from disk, so the Installed list and the badges go with it
/// — whether the removal succeeded or not, for the reason the arm below used
/// to spell out: `uninstallRunner` assigns no field of the bridge at all, it
/// only notifies, and the held rows have to be recomputed explicitly or they
/// go stale exactly when the user is looking for the reason.
///
/// The order is the whole function: [`refresh`] snapshots
/// `installed_protons()` synchronously and ships the snapshot into the reply
/// task, so refreshing *before* the uninstall resurrects the removed row when
/// the reply lands — the Installed list then names a build that is gone from
/// disk, with a working delete button for it. T-19 watched that happen live;
/// `the_removal_refreshes_after_the_uninstall_not_before` pins the order.
fn remove_runner(state: &mut State, runner_id: &str) -> Task<Message> {
    let line = uninstall_line(
        runner_id,
        proton::uninstall(state.runners.runners_directory(), runner_id),
    );
    let rows = refresh(state);
    Task::batch([rows, push_toast(state, line)])
}

/// `notify` (`bridge.py`), which every path above ends in.
fn push_toast(state: &mut State, line: String) -> Task<Message> {
    state
        .toasts
        .push(cosmic::widget::toaster::Toast::new(line))
        .map(cosmic::Action::App)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamehandler_core::runners::proton::ResponseHead;
    use gamehandler_core::runners::{RunnerManager, WineRunner};
    use std::cell::RefCell;

    fn released(tag: &str, name: &str, size: i64) -> ReleaseInfo {
        ReleaseInfo::new(tag, name, "https://example.invalid/a.tar.gz", size)
    }

    // ---- installed_rows ---------------------------------------------------

    /// System Wine is first, always, and it is the row that is not removable.
    /// The reference makes the same pair of claims in one place
    /// (`bridge.py:621-632`), and a reordering would move the row a user
    /// reaches for first.
    #[test]
    fn system_wine_is_first_and_is_the_only_row_that_cannot_be_removed() {
        let system = WineRunner::with_binary(None);
        let rows = installed_rows(&system, &[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "System Wine");
        assert!(!rows[0].removable);
        assert!(
            !rows[0].available,
            "a WineRunner with no binary is not available"
        );
        assert_eq!(rows[0].detail, "Not installed on this system");
    }

    /// The system row's id is [`SYSTEM_WINE`], pinned to Python's own value.
    ///
    /// No widget draws this field — the row is not removable, so no button
    /// carries it — which is exactly why it is worth a test: `"system"` is a
    /// plausible-looking id that reads correctly in every render and names no
    /// runner at all. `"system"` is in fact the **family** id
    /// (`runners.py:416`, the family the plain-Wine entry declares), so the two
    /// are one keystroke apart and the row would
    /// look right in every visual check. `bridge.py:624` puts `SYSTEM_WINE` on
    /// the row and `uninstall` matches on that value (`proton.rs:1085`), so the
    /// id is load-bearing the moment anything reaches for it.
    ///
    /// The second assertion is the one that pins the *value* rather than the
    /// constant: `assert_eq!(rows[0].runner_id, SYSTEM_WINE)` alone would pass
    /// for any pair of equal strings, including a second constant that shadowed
    /// the first.
    #[test]
    fn the_system_row_names_python_s_runner_id_and_not_the_family_id() {
        let rows = installed_rows(&WineRunner::with_binary(None), &[]);
        assert_eq!(rows[0].runner_id, SYSTEM_WINE);
        assert_eq!(
            rows[0].runner_id, "wine-system",
            "the id is Python's value, not a second spelling of it"
        );
        assert_ne!(
            rows[0].runner_id, "system",
            "\"system\" is the family id, and a row carrying it names no runner"
        );
    }

    /// A downloaded build is always removable and always available — the
    /// reference hard-codes both (`bridge.py:637-640`), because a build that
    /// was discovered on disk is by construction launchable and by
    /// construction GameHandler's to delete.
    #[test]
    fn a_downloaded_build_is_removable_and_its_detail_is_its_family() {
        let system = WineRunner::with_binary(None);
        let protons = vec![ProtonRunner::new(
            "/runners/GE-Proton9-5",
            "proton-ge",
            "GE-Proton9-5",
        )];

        let rows = installed_rows(&system, &protons);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].runner_id, "GE-Proton9-5");
        assert_eq!(rows[1].detail, "Proton-GE");
        assert!(rows[1].removable);
        assert!(rows[1].available);
    }

    /// The system row's detail is the version probe when there is a binary, and
    /// that is a subprocess — which is why `installed_rows` is not called from
    /// [`view`]. The test asserts the *branch*, using a binary that answers.
    #[test]
    fn an_available_system_wine_reports_its_version_rather_than_absence() {
        let root = std::env::temp_dir().join(format!("gh-runners-wine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let wine = root.join("wine");
        std::fs::write(&wine, "#!/bin/sh\necho 'wine-9.0'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wine, std::fs::Permissions::from_mode(0o755)).unwrap();

        let system = WineRunner::with_binary(Some(wine));
        let rows = installed_rows(&system, &[]);
        assert!(rows[0].available);
        assert_eq!(rows[0].detail, "wine-9.0");
        assert_ne!(rows[0].detail, "Not installed on this system");

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- release_rows -----------------------------------------------------

    /// `installed` is true exactly when the release's own directory is on disk,
    /// and the rest of the row comes from the release rather than from the disk.
    ///
    /// This is the one decision in the module's table that had nothing asserting
    /// it, which is worth stating because the header claimed otherwise. It is
    /// also the field that decides whether a card offers Install or shows the
    /// "Installed" badge, so a `bool` that was inverted or answered about the
    /// wrong release is a wrong button on every card — and would pass a render
    /// review, since a plausible mix of the two states is exactly what the
    /// reference shows.
    ///
    /// The fixture's directory name comes from `families::install_id_for`, the
    /// same helper `is_installed` uses. That is deliberate rather than circular:
    /// what is under test is the *mapping* — that the question is asked about
    /// this release's tag and family, that `tag` and `detail` survive it, and
    /// that the rows keep the list's order. The name itself is `proton.rs`'s
    /// claim and is asserted there.
    #[test]
    fn a_release_is_installed_exactly_when_its_own_directory_is_on_disk() {
        use gamehandler_core::runners::families::install_id_for;

        let root = std::env::temp_dir().join(format!("gh-releases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let here = released("GE-Proton9-5", "GE-Proton9-5.tar.gz", 1024 * 1024);
        let absent = released("GE-Proton9-4", "GE-Proton9-4.tar.gz", 1024 * 1024);

        // `proton_entry_exists` follows a symlink and is true for a plain file,
        // so a file is enough — nothing here needs the entry to be executable.
        let dir = root.join(install_id_for(&here.tag, &here.family_id).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("proton"), "#!/bin/sh\n").unwrap();

        let rows = release_rows(&[here.clone(), absent.clone()], &root);

        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].installed,
            "{} carries a proton entry, so this build is here",
            dir.display()
        );
        assert!(
            !rows[1].installed,
            "no directory was made for {}, so it is offered",
            absent.tag
        );
        // The row's own fields are the release's, not the disk's, and the order
        // is the list's — a `sorted` or a set here would reorder the cards.
        assert_eq!(rows[0].tag, "GE-Proton9-5");
        assert_eq!(rows[1].tag, "GE-Proton9-4");
        assert_eq!(rows[0].detail, release_detail(&here));
        assert_eq!(rows[1].detail, release_detail(&absent));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// An empty directory is not an installed build. The difference between
    /// "the directory exists" and "the directory holds a runner" is what
    /// `proton_entry_exists` exists to draw, and a port that asked
    /// `Path::exists` about the directory itself would call every half-removed
    /// install present.
    #[test]
    fn a_directory_without_a_proton_entry_is_not_an_installed_build() {
        use gamehandler_core::runners::families::install_id_for;

        let root = std::env::temp_dir().join(format!("gh-releases-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let release = released("GE-Proton9-5", "a.tar.gz", 1024 * 1024);
        std::fs::create_dir_all(
            root.join(install_id_for(&release.tag, &release.family_id).unwrap()),
        )
        .unwrap();

        let rows = release_rows(&[release], &root);
        assert!(
            !rows[0].installed,
            "the directory is there and the runner is not, which is not installed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- release_detail ---------------------------------------------------

    /// The three parts, in the reference's order and with the size rounded.
    #[test]
    fn a_release_detail_names_the_family_the_asset_and_the_whole_megabytes() {
        // 432_000_000 bytes is 412.0 MiB; 412.6 rounds to 413.
        let release = released("GE-Proton9-5", "GE-Proton9-5.tar.gz", 432_600_000);
        assert_eq!(
            release_detail(&release),
            "Proton-GE · GE-Proton9-5.tar.gz · 413 MB"
        );
    }

    /// `round()` is half-away-from-zero on a `.5`, and the reference writes
    /// `round(release.size_mb)` — a detail that reads "412 MB" where Python
    /// says "413" is a parity break in the only place a user sees the number.
    #[test]
    fn a_half_megabyte_rounds_up_as_python_rounds_it() {
        let exact = 413.5 * 1024.0 * 1024.0;
        let release = released("t", "a.tar.gz", exact as i64);
        assert_eq!(release_detail(&release), "Proton-GE · a.tar.gz · 414 MB");
    }

    /// An id naming no family is shown as itself. A lookup miss must not render
    /// as a blank, which is what an `unwrap_or_default` here would produce.
    #[test]
    fn an_unknown_family_id_is_shown_rather_than_dropped() {
        let mut release = released("t", "a.tar.gz", 1024 * 1024);
        release.family_id = "proton-nonesuch".to_string();
        assert_eq!(
            release_detail(&release),
            "proton-nonesuch · a.tar.gz · 1 MB"
        );
    }

    // ---- status_line ------------------------------------------------------

    /// The one case the label is hidden in, and the three it is not.
    #[test]
    fn the_status_line_is_absent_exactly_when_a_ready_list_speaks_for_itself() {
        assert_eq!(status_line(&ReleasesStatus::Ready, 3), None);
        assert_eq!(
            status_line(&ReleasesStatus::Ready, 0).as_deref(),
            Some("No builds found for this family."),
            "a ready but empty list still needs the sentence"
        );
        assert_eq!(
            status_line(&ReleasesStatus::Loading, 0).as_deref(),
            Some("Fetching the latest builds…")
        );
    }

    /// The error message is carried verbatim behind the reference's own words.
    /// `bridge.py` stores `f"error: {message}"` and the QML strips seven
    /// characters; the port strips it at the source, so the prefix cannot leak
    /// into the sentence a user reads.
    #[test]
    fn a_fetch_error_is_shown_behind_the_reference_s_own_words() {
        let status = ReleasesStatus::Error("rate limit exceeded".to_string());
        assert_eq!(
            status_line(&status, 0).as_deref(),
            Some("Could not fetch builds — rate limit exceeded")
        );
    }

    /// `Idle` is "nothing asked for yet", which a user meets as an empty list —
    /// the same sentence as a ready-and-empty one, because they are the same
    /// thing to look at.
    #[test]
    fn idle_reads_as_empty_rather_than_as_a_blank_line() {
        assert_eq!(
            status_line(&ReleasesStatus::Idle, 0).as_deref(),
            Some("No builds found for this family.")
        );
    }

    // ---- family_note ------------------------------------------------------

    #[test]
    fn a_family_note_appends_the_maintainer_only_when_there_is_one() {
        let family = &families()[0];
        assert!(family_note(family).contains("Maintained by"));

        let mut orphan = families()[0].clone();
        orphan.maintainer = "";
        assert_eq!(family_note(&orphan), orphan.description);
        assert!(!family_note(&orphan).contains("Maintained by"));
    }

    /// The note is two lines when there is a maintainer — the QML joins them
    /// with a newline (`RunnersPage.qml:136`), and a port that used a space
    /// would be one line of run-on text.
    #[test]
    fn the_family_note_breaks_between_the_description_and_the_maintainer() {
        let family = &families()[0];
        let note = family_note(family);
        assert_eq!(note.lines().count(), 2);
        assert!(note.lines().nth(1).unwrap().starts_with("Maintained by"));
    }

    // ---- guide_subtitle ---------------------------------------------------

    #[test]
    fn a_guide_subtitle_is_the_kind_and_the_maintainer_or_just_the_kind() {
        let rows = guide_rows();
        assert!(rows[0].subtitle.contains("maintained by WineHQ"));

        let mut orphan = rows[0].clone();
        orphan.subtitle = guide_subtitle(&RunnerGuide {
            title: "x".to_string(),
            kind: "proton".to_string(),
            advice: String::new(),
            maintainer: String::new(),
            homepage: String::new(),
        });
        assert_eq!(orphan.subtitle, "proton");
    }

    /// The guide is the system row plus one per family — nine, which is P-33's
    /// acceptance criterion, and it is asserted against the catalogue rather
    /// than against the literal so a family added to `families.rs` moves it.
    #[test]
    fn the_guide_has_one_row_per_family_plus_system_wine() {
        let rows = guide_rows();
        assert_eq!(rows.len(), families().len() + 1);
        assert_eq!(rows[0].title, "System Wine");
        assert_eq!(rows.len(), 9);
    }

    /// `System Wine`'s homepage is set, so its "Visit project" button is
    /// drawn; the reference hides the button only on an empty url.
    ///
    /// Named for exactly what it measures, because the previous name —
    /// `every_guide_row_that_has_a_homepage_names_one_a_button_can_open` —
    /// was the defect class: it asserted only the `https://` prefix while its
    /// own name claimed the button could open it, and the two dead links stayed
    /// green behind it. The press clause below is what closes the name's claim:
    /// every non-empty homepage must also be the URL the row's link sends.
    #[test]
    fn every_guide_row_that_has_a_homepage_names_one() {
        for row in guide_rows() {
            assert!(
                row.homepage.starts_with("https://"),
                "{} has no usable homepage: {:?}",
                row.title,
                row.homepage
            );
        }
    }

    /// Every drawn "Visit project" button carries its own row's URL. P-33.
    ///
    /// Read from [`guide_press`], which is the same function the view calls to
    /// build the link's `on_press` — not a copy of its logic. A button whose
    /// press is dropped renders disabled, which no string assertion can see;
    /// producing the message in a named function is what leaves the view with
    /// nothing to get wrong except calling it. The empty-homepage arm is
    /// covered by the `None` case below rather than by fixture luck: no row in
    /// the catalogue has an empty homepage, so a test that only iterated the
    /// catalogue would pass without ever exercising the arm the reference's
    /// `visible: homepage !== ""` names.
    #[test]
    fn every_guide_links_press_opens_its_own_homepage() {
        for row in guide_rows() {
            assert!(
                !row.homepage.is_empty(),
                "{} has no homepage, so its press is untestable here: {:?}",
                row.title,
                row.homepage
            );
            match guide_press(&row) {
                Some(Message::OpenUrl(url)) => {
                    assert_eq!(
                        url, row.homepage,
                        "the press carries the wrong URL for {}",
                        row.title
                    );
                }
                other => panic!(
                    "a guide row's press must be `OpenUrl`, got {other:?} for {}",
                    row.title
                ),
            }
        }

        // `Message` is `Clone + Debug` without `PartialEq` (see `main.rs`'s
        // `every_message` on why), so the `None` arm is matched rather than
        // compared — the same route `credits.rs` takes for its presses.
        let mut no_home = guide_rows()[0].clone();
        no_home.homepage.clear();
        match guide_press(&no_home) {
            None => {}
            Some(press) => panic!(
                "an empty homepage draws no button, so there is no press to send — got {press:?}"
            ),
        }
    }

    /// The family selector's project link carries the selected family's URL.
    /// P-33.
    ///
    /// Same named-function route as the guide rows: [`family_press`] is what
    /// the view's `on_press` calls, and a link that lost it would render
    /// disabled rather than wrong.
    #[test]
    fn the_family_links_press_opens_the_familys_homepage() {
        for family in families() {
            match family_press(family) {
                Message::OpenUrl(url) => {
                    assert_eq!(
                        url,
                        family.homepage(),
                        "the press carries the wrong URL for {}",
                        family.name
                    );
                    assert!(
                        url.starts_with("https://"),
                        "{} has no usable homepage: {url:?}",
                        family.name
                    );
                }
                other => panic!(
                    "a family link's press must be `OpenUrl`, got {other:?} for {}",
                    family.name
                ),
            }
        }
    }

    // ---- link wiring ------------------------------------------------------

    /// Every `button::link` the app draws must carry an `on_press`. P-33.
    ///
    /// A link without one renders **disabled**
    /// (`libcosmic/src/widget/button/link.rs` → `button/widget.rs:148`), and
    /// nothing can read the press back out of the built widget — `Operation`'s
    /// seven arms carry no message, `Button::operate` reports only `container`
    /// and `focusable`, and `on_press` is a private opaque closure (`widget.rs`)
    /// — so a dropped press is invisible to every render-level test. This is
    /// the tripwire on the call site that the value-level tests above cannot
    /// see: it scans the production sources for `button::link(` without a
    /// `.on_press(` before the statement ends, the same device `form.rs` uses
    /// for the dropdown index rule (`no_dropdown_callback_turns_its_index_into_the_payload`).
    ///
    /// What it cannot see is a press that names the *wrong* message — a link
    /// wired to `Message::CloseDialog` compiles and passes here, and that
    /// mutation was run and survives. That half is pinned by the value tests
    /// above reading [`guide_press`] and [`family_press`] directly: the scanner
    /// proves a press exists, the value tests prove it is the right one, and
    /// neither claim stands without the other.
    #[test]
    fn no_link_button_is_drawn_without_a_press() {
        let sources = link_button_sources();
        let mut findings = Vec::new();
        let mut links = 0usize;
        for (name, source) in &sources {
            let found = link_without_press_findings(source);
            links += link_button_count(source);
            findings.extend(
                found
                    .into_iter()
                    .map(|finding| format!("{name}: {finding}")),
            );
        }

        assert!(
            findings.is_empty(),
            "a `button::link` without `.on_press` renders disabled — the P-33 \
             defect:\n  {}",
            findings.join("\n  ")
        );
        assert!(
            links >= 3,
            "the scan found only {links} `button::link` calls over {} files, so \
             its silence above is not evidence",
            sources.len()
        );
    }

    /// The instrument, driven on the defect it was written for and on the fix.
    ///
    /// A guard that reports nothing on the real tree is only worth anything if
    /// it reports something on the tree it was written to reject, so both the
    /// P-33 defect and the fixed shape are run through the same entry point the
    /// scan uses.
    #[test]
    fn the_link_guard_reports_a_pressless_link() {
        // The P-33 defect, verbatim: a label and no press.
        let defect = "heading = heading.push(button::link(\"Visit project\".to_string()));";
        assert_eq!(
            link_without_press_findings(defect).len(),
            1,
            "the instrument is silent on the defect it was written for"
        );

        // Every shape that is right, and must not be reported.
        for correct in [
            "heading = heading.push(button::link(\"Visit project\".to_string()).on_press(press));",
            "row = row.push(button::link(VISIT_LABEL.to_string()).on_press(press));",
            "body = body.push(button::link(url.clone()).on_press(family_press(family)));",
        ] {
            assert!(
                link_without_press_findings(correct).is_empty(),
                "the instrument reported a finding in the fixed shape `{correct}` \
                 — a guard that fires on the fix is a guard somebody deletes"
            );
        }
    }

    /// Every `.rs` file the view directory holds, plus `main.rs`, with comments
    /// blanked and the `#[cfg(test)]` modules cut — because this module's own
    /// defect sample is a string that contains a pressless link, and a scanner
    /// that read it would report itself. The same cut `form.rs` makes for its
    /// dropdown rule.
    fn link_button_sources() -> Vec<(String, String)> {
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(crate_dir.join("src/view"))
            .expect("`src/view` is where this crate keeps its pages")
            .map(|entry| entry.expect("a readable directory entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        paths.push(crate_dir.join("src/main.rs"));
        paths.sort();

        paths
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
                let name = path
                    .strip_prefix(crate_dir)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                (name, link_button_production_source(&text))
            })
            .collect()
    }

    /// One file's production source: everything from the first `#[cfg(test)]`
    /// to the end of the file is cut (test modules live at the end of every
    /// file in this tree, and this module's own defect sample is a string that
    /// contains a pressless link), and `//`/`/* */` comments are blanked with
    /// offsets kept so a finding can name a line. String literals are kept:
    /// the rule matches on the call shape, not on their contents.
    ///
    /// Cutting at the first `#[cfg(test)]` rather than brace-matching each
    /// test module is what keeps a brace inside a doc comment from ending the
    /// cut early — the failure mode that blanked this file's own view code
    /// during development and left the guard green on the defect it was
    /// written for. Nothing production lives below a test module here, so the
    /// coarser cut loses nothing.
    fn link_button_production_source(text: &str) -> String {
        let cut_at = text.find("#[cfg(test)]").unwrap_or(text.len());
        let (production, _) = text.split_at(cut_at);
        let chars: Vec<char> = production.chars().collect();
        let mut out = chars.clone();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
                while index < chars.len() && chars[index] != '\n' {
                    out[index] = ' ';
                    index += 1;
                }
            } else if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                out[index] = ' ';
                out[index + 1] = ' ';
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    if chars[index] != '\n' {
                        out[index] = ' ';
                    }
                    index += 1;
                }
                for _ in 0..2 {
                    if index < chars.len() {
                        out[index] = ' ';
                        index += 1;
                    }
                }
            } else {
                index += 1;
            }
        }
        out.into_iter().collect()
    }

    /// The `button::link(` sites in `src` whose statement carries no
    /// `.on_press(`.
    ///
    /// A "statement" is the text from the call to the first `;` — string
    /// literals blanked first, so a `;` inside `"Visit project"` cannot end it
    /// early, and parens tracked so one inside an argument cannot either. A
    /// link whose press arrives in a later statement (bound first, pressed
    /// after) would be reported, and that is deliberate: every link in this
    /// tree presses in the same statement, and a second spelling is a thing
    /// the scan should be taught rather than silently accept.
    fn link_without_press_findings(src: &str) -> Vec<String> {
        // Blank string and char literals: their contents (labels, `"…"`) are
        // not code, and a `;` or paren inside one must not move the scan.
        let chars: Vec<char> = src.chars().collect();
        let mut code = chars.clone();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '"' {
                code[index] = ' ';
                index += 1;
                while index < chars.len() && chars[index] != '"' {
                    if chars[index] == '\\' {
                        code[index] = ' ';
                        index += 1;
                    }
                    if index < chars.len() {
                        if chars[index] != '\n' {
                            code[index] = ' ';
                        }
                        index += 1;
                    }
                }
                if index < chars.len() {
                    code[index] = ' ';
                    index += 1;
                }
            } else if chars[index] == '\'' && chars.get(index + 2) == Some(&'\'')
                || chars[index] == '\''
                    && chars.get(index + 1) == Some(&'\\')
                    && chars.get(index + 3) == Some(&'\'')
            {
                let width = if chars.get(index + 1) == Some(&'\\') {
                    4
                } else {
                    3
                };
                for offset in 0..width {
                    if index + offset < code.len() && chars[index + offset] != '\n' {
                        code[index + offset] = ' ';
                    }
                }
                index += width;
            } else {
                index += 1;
            }
        }

        let needle: Vec<char> = "button::link(".chars().collect();
        let mut findings = Vec::new();
        let mut from = 0;
        while from + needle.len() <= code.len() {
            let Some(found) = find_chars(&code[from..], &needle) else {
                break;
            };
            let call = from + found;
            let line = 1 + code[..call].iter().filter(|c| **c == '\n').count();
            // The statement runs to the first `;` after the call's own `(` has
            // closed — i.e. depth has returned to 0 *and* at least one paren
            // has been seen. Depth going negative (a `)` that closes an outer
            // expression the call sits inside) must not end the scan early,
            // and a `;` before the call's parens close is inside an argument.
            let mut depth = 0i32;
            let mut seen_open = false;
            let mut end = None;
            for (offset, character) in code[call..].iter().enumerate() {
                match character {
                    '(' => {
                        depth += 1;
                        seen_open = true;
                    }
                    ')' => depth -= 1,
                    ';' if seen_open && depth <= 0 => {
                        end = Some(call + offset);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(stop) = end else {
                break;
            };
            let stmt: String = code[call..stop].iter().collect();
            if !stmt.contains(".on_press(") {
                let snippet: String = stmt
                    .chars()
                    .take(120)
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                findings.push(format!(
                    "line {line}: `{snippet}…` — no `.on_press` before the statement ends"
                ));
            }
            from = stop + 1;
        }
        findings
    }

    /// How many `button::link(` calls `src` holds — the non-vacuity count.
    fn link_button_count(src: &str) -> usize {
        let chars: Vec<char> = src.chars().collect();
        let needle: Vec<char> = "button::link(".chars().collect();
        let mut count = 0;
        let mut from = 0;
        while from + needle.len() <= chars.len() {
            let Some(found) = find_chars(&chars[from..], &needle) else {
                break;
            };
            count += 1;
            from += found + needle.len();
        }
        count
    }

    /// `needle` in `haystack`, as an offset — `str::find` on char slices.
    fn find_chars(haystack: &[char], needle: &[char]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (0..=haystack.len() - needle.len()).find(|at| haystack[*at..].starts_with(needle))
    }

    // ---- progress_fraction ------------------------------------------------

    fn state() -> State {
        State::new(
            gamehandler_core::models::Library::new(None),
            gamehandler_core::settings::Settings::load(None),
            RunnerManager::at("/nonexistent"),
        )
    }

    /// Idle: no bar, which is the `-1.0` sentinel and the `busy` guard together.
    #[test]
    fn no_job_means_no_progress_bar() {
        assert_eq!(progress_fraction(&state()), None);
    }

    /// Busy with no fraction yet: still no bar, because `progress >= 0` is
    /// false — the reference distinguishes "working, progress unknown" from
    /// "working, 40% done", and only the second draws a determinate bar.
    #[test]
    fn a_busy_job_with_no_fraction_yet_draws_no_bar() {
        let mut state = state();
        state.runner_busy = true;
        assert_eq!(progress_fraction(&state), None);
        assert!(state.busy());
    }

    /// Busy with a fraction: the bar is drawn at that fraction, and the
    /// fraction is clamped at zero rather than allowed to go negative.
    #[test]
    fn a_busy_job_with_a_fraction_draws_the_bar_at_that_fraction() {
        let mut state = state();
        state.runner_busy = true;
        state.progress = Some(0.4);
        assert_eq!(progress_fraction(&state), Some(0.4));

        state.progress = Some(-0.25);
        assert_eq!(
            progress_fraction(&state),
            None,
            "a negative fraction is the idle sentinel, not a bar before the start"
        );
    }

    /// `busy` is *either* job, so the easy-install path moves this page's bar
    /// too. That coupling is the reference's (`RunnersPage.qml:35` reads
    /// `backend.busy`) and is preserved rather than tidied.
    #[test]
    fn the_easy_install_job_moves_this_pages_bar_as_well() {
        let mut state = state();
        state.easy_busy = true;
        state.progress = Some(0.6);
        assert_eq!(progress_fraction(&state), Some(0.6));
    }

    // ---- family_index -----------------------------------------------------

    /// The selector's current value is an id from the catalogue, and an id that
    /// names nothing is `None` — which the view renders as "no selection"
    /// rather than silently as the first entry.
    #[test]
    fn the_selected_family_is_looked_up_by_id_and_an_unknown_one_is_no_selection() {
        assert_eq!(family_index("proton-ge"), Some(0));
        assert_eq!(
            family_index("proton-cachyos"),
            families().iter().position(|f| f.id == "proton-cachyos")
        );
        assert_eq!(family_index("proton-nonesuch"), None);
        assert_eq!(
            family_index(""),
            None,
            "the initial state is empty, not a family"
        );
    }

    // ---- fetch_releases ---------------------------------------------------

    /// Records the URL it was asked for and answers with one canned body.
    ///
    /// The trait's failure is a message rather than a distinct type, so a body
    /// that cannot be parsed *is* the failure case and no second double is
    /// needed — which is the shape `core`'s own doubles have too.
    struct FakeClient {
        body: String,
        seen: RefCell<Vec<String>>,
    }

    impl FakeClient {
        fn body(body: String) -> Self {
            Self {
                body,
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl HttpClient for FakeClient {
        fn get(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            self.seen.borrow_mut().push(url.to_string());
            on_head(&ResponseHead {
                content_length: None,
                // This fake never redirects, so the request URL *is* the final
                // URL — the field is `""` only for a client that cannot say.
                final_url: url.to_string(),
            })?;
            sink(self.body.as_bytes())
        }
    }

    /// A GitHub releases payload of `count` *usable* releases — each with an
    /// asset `pick_asset` accepts for `proton-ge`.
    ///
    /// Written with `format!` rather than `serde_json`, because `crates/app`
    /// has no JSON dependency and adding one for a fixture would be the tail
    /// wagging the dog.
    fn payload(count: usize) -> String {
        let releases: Vec<String> = (0..count)
            .map(|i| {
                format!(
                    "{{\"tag_name\":\"GE-Proton9-{i}\",\"assets\":[\
                     {{\"name\":\"GE-Proton9-{i}.tar.gz\",\
                     \"browser_download_url\":\"https://example.invalid/{i}.tar.gz\",\
                     \"size\":1024}}]}}"
                )
            })
            .collect();
        format!("[{}]", releases.join(","))
    }

    /// The family reaches the client and becomes *its* URL — not a constant,
    /// and not the first family in the list.
    #[test]
    fn a_fetch_asks_for_the_family_it_was_given() {
        let client = FakeClient::body(payload(0));
        fetch_releases(&client, "proton-cachyos").unwrap();
        let seen = client.seen.borrow();
        assert_eq!(seen.len(), 1, "one fetch is one request");
        assert!(
            seen[0].contains("cachyos"),
            "the URL must be the given family's, got {}",
            seen[0]
        );
    }

    /// Python's `limit=12` (`bridge.py:702`), applied after parsing. Twenty
    /// usable releases in, twelve out: a port that passed a different limit, or
    /// none at all, is caught here rather than by a user reading a longer list
    /// than the reference shows.
    #[test]
    fn a_fetch_returns_at_most_pythons_twelve_releases() {
        let client = FakeClient::body(payload(20));
        let found = fetch_releases(&client, "proton-ge").unwrap();
        assert_eq!(found.len(), RELEASES_LIMIT);
        assert_eq!(found.len(), 12, "and the limit is Python's twelve");
    }

    /// A payload that is not a JSON array is an error rather than an empty
    /// list, so the failure reaches the status line. Returning `Ok(vec![])`
    /// here is the plausible wrong version, and it renders "No builds found for
    /// this family." — a negative result the app never obtained.
    #[test]
    fn a_fetch_that_cannot_be_parsed_is_an_error_not_an_empty_list() {
        let client = FakeClient::body("{\"message\":\"Not Found\"}".to_string());
        let error = fetch_releases(&client, "proton-ge").unwrap_err();
        assert_eq!(error.to_string(), "Unexpected GitHub releases response");
    }

    // ---- rendered_message -------------------------------------------------

    /// **The message a failure renders is never empty**, which is the invariant
    /// `VERIFY-FINDINGS` §5 names and the reason `bridge.py:157` has a fallback
    /// at all.
    ///
    /// All four variants whose `Display` can be empty are driven, not just the
    /// one the fetch path happens to produce: a test on `Http` alone would pass
    /// for an implementation that special-cased it. The premise is asserted
    /// first — each of these really does render as `''` — because otherwise the
    /// test would pass for an error that was never empty to begin with, which is
    /// the defect the fallback exists to catch.
    ///
    /// The ordinary case is asserted too: the fallback must not rewrite a
    /// message that already has one.
    #[test]
    fn a_failure_never_renders_an_empty_message() {
        let empty: Vec<(&str, RunnerError)> = vec![
            (
                "Http",
                RunnerError::Http {
                    message: String::new(),
                },
            ),
            (
                "UnreachableShare",
                RunnerError::UnreachableShare {
                    message: String::new(),
                },
            ),
            ("Io", RunnerError::Io(std::io::Error::other(""))),
            (
                "Archive",
                RunnerError::Archive(gamehandler_core::runners::ArchiveError::Io(
                    std::io::Error::other(""),
                )),
            ),
        ];

        for (name, error) in empty {
            assert_eq!(
                error.to_string(),
                "",
                "the premise: {name} renders empty, so the fallback is what is under test"
            );
            assert_eq!(rendered_message(&error), name, "the class name stands in");

            // The point of the invariant: what the user actually reads.
            let line = status_line(&ReleasesStatus::Error(rendered_message(&error)), 0);
            assert_eq!(
                line.as_deref(),
                Some(format!("Could not fetch builds — {name}").as_str()),
                "a dangling dash is the defect this prevents"
            );
        }

        let message = RunnerError::Http {
            message: "rate limit exceeded".to_string(),
        };
        assert_eq!(
            rendered_message(&message),
            "rate limit exceeded",
            "a real message is passed through untouched"
        );
    }

    // ---- update -----------------------------------------------------------

    /// A fetch request clears the previous family's list *and* marks the fetch
    /// in flight, before any work starts. A port that only set the status would
    /// show the old family's builds under the new family's name.
    #[test]
    fn requesting_a_family_clears_the_previous_list_and_marks_it_loading() {
        let mut state = state();
        state.releases = vec![released("old", "old.tar.gz", 1024)];
        state.releases_status = ReleasesStatus::Ready;

        let task = update(
            &mut state,
            &Message::FetchReleases {
                family: "proton-ge".to_string(),
            },
        );
        assert!(task.is_some(), "FetchReleases is this page's message");
        assert_eq!(state.releases_family, "proton-ge");
        assert_eq!(state.releases_status, ReleasesStatus::Loading);
        assert!(
            state.releases.is_empty(),
            "the previous family's list must go"
        );
    }

    /// The stale guard, in both directions. This is the one that matters: a
    /// slow reply for a family the user has left must not overwrite the list
    /// they are looking at.
    #[test]
    fn a_reply_for_a_family_the_user_has_left_is_dropped() {
        let mut state = state();
        state.releases_family = "proton-cachyos".to_string();
        state.releases_status = ReleasesStatus::Loading;
        let fresh = vec![released("cachy", "a.tar.gz", 1024)];

        // A reply for the family the user is on gets stored.
        update(
            &mut state,
            &Message::ReleasesFetchFinished {
                family: "proton-cachyos".to_string(),
                result: Ok(fresh.clone()),
            },
        );
        assert_eq!(state.releases, fresh);
        assert_eq!(state.releases_status, ReleasesStatus::Ready);

        // A late reply for the family they left does not, and does not touch
        // the status either.
        update(
            &mut state,
            &Message::ReleasesFetchFinished {
                family: "proton-ge".to_string(),
                result: Ok(vec![released("stale", "s.tar.gz", 2048)]),
            },
        );
        assert_eq!(
            state.releases, fresh,
            "the stale reply overwrote the live list"
        );
        assert_eq!(state.releases_status, ReleasesStatus::Ready);
    }

    /// **A row reply for a superseded request is dropped, and the current one
    /// is written.**
    ///
    /// Both arms are here on purpose (D-47's clause 1): the second assertion
    /// alone would pass for a guard that dropped *every* reply, which is a page
    /// whose rows never arrive — indistinguishable from a correct guard by any
    /// single-arm test. The stale arm is the one that matters, because the rows
    /// are computed on a worker thread and an *earlier* request's
    /// `wine --version` can finish last; without the guard that reply overwrites
    /// newer rows and resurrects a build the user has just removed.
    #[test]
    fn a_row_reply_for_a_superseded_request_is_dropped() {
        let mut state = state();
        state.runner_rows_token = 7;
        let rows = |tag: &str| {
            vec![ReleaseRow {
                tag: tag.to_string(),
                detail: "Proton-GE · v1.0.tar.gz · 1 MB".to_string(),
                installed: false,
            }]
        };

        // The control: the token the state is currently on. It must land.
        update(
            &mut state,
            &Message::RunnersRefreshed {
                token: 7,
                installed: Vec::new(),
                release_rows: rows("current"),
            },
        );
        assert_eq!(
            state.release_rows.first().map(|row| row.tag.as_str()),
            Some("current"),
            "the current reply was dropped, so the guard is not a guard but a wall"
        );

        // The stale one: an older token than the state is on.
        update(
            &mut state,
            &Message::RunnersRefreshed {
                token: 6,
                installed: Vec::new(),
                release_rows: rows("stale"),
            },
        );
        assert_eq!(
            state.release_rows.first().map(|row| row.tag.as_str()),
            Some("current"),
            "the superseded reply overwrote the live rows"
        );
    }

    /// A failure carries its message into the status, which is the only place
    /// the user can read it.
    #[test]
    fn a_failed_fetch_stores_its_message_for_the_status_line() {
        let mut state = state();
        state.releases_family = "proton-ge".to_string();
        update(
            &mut state,
            &Message::ReleasesFetchFinished {
                family: "proton-ge".to_string(),
                result: Err("rate limit exceeded".to_string()),
            },
        );
        assert_eq!(
            state.releases_status,
            ReleasesStatus::Error("rate limit exceeded".to_string())
        );
        assert_eq!(
            status_line(&state.releases_status, state.releases.len()).as_deref(),
            Some("Could not fetch builds — rate limit exceeded")
        );
    }

    /// An install finishing clears the guard and the bar whichever way it went.
    /// The reference writes that pair before the success/failure branch
    /// (`bridge.py:753-755` and `:764-766`), so a port that only cleared them on
    /// success would
    /// leave the page permanently busy after one failure.
    #[test]
    fn an_install_finishing_clears_the_guard_on_success_and_on_failure() {
        for result in [Ok(()), Err("checksum mismatch".to_string())] {
            let mut state = state();
            state.runner_busy = true;
            state.progress = Some(0.5);

            let task = update(
                &mut state,
                &Message::RunnerInstallFinished {
                    tag: "GE-Proton9-5".to_string(),
                    result,
                },
            );
            assert!(!state.runner_busy, "the guard must clear either way");
            assert_eq!(state.progress, None, "the bar must clear either way");
            assert!(
                task.is_some(),
                "the toast is the observable, and it is a task"
            );
        }
    }

    /// Progress is stored as given, and the bar's own rule decides what to do
    /// with it — so a progress tick for a job that has not started cannot make
    /// the bar appear.
    #[test]
    fn a_progress_tick_is_stored_and_the_bar_rule_decides() {
        let mut state = state();
        update(&mut state, &Message::RunnerProgress(0.25));
        assert_eq!(state.progress, Some(0.25));
        assert_eq!(
            progress_fraction(&state),
            None,
            "not busy, so no bar, however much progress was reported"
        );
    }

    /// The messages this page does not own are declined rather than swallowed,
    /// which is what lets the caller leave them as TODOs without this page
    /// claiming them.
    #[test]
    fn a_message_this_page_does_not_own_is_declined_rather_than_swallowed() {
        let mut state = state();
        for message in [
            Message::SetInstallerSearch("x".to_string()),
            Message::RefreshPlugins,
            Message::LaunchWatchTick,
        ] {
            assert!(
                update(&mut state, &message).is_none(),
                "{message:?} is not the Runners page's"
            );
        }
    }

    /// **Every handler is now written** — the two that used to be declined are
    /// handled, and the page has no arm left that returns `None` for one of its
    /// own messages.
    ///
    /// This test used to be its own opposite: it asserted `InstallRunner` and
    /// `FetchReleases` were *declined, not half-written*, because an arm that
    /// set `runner_busy` without doing the work would render as a busy page that
    /// never finishes and would look implemented. Both now do the work, so an
    /// assertion that they were declined would be asserting the opposite of the
    /// truth. What replaces it is the pair below: a tag the page is not
    /// offering is dropped, and a click while a download is already running is
    /// dropped — the reference's two silent early returns, which are now the
    /// only way this arm returns without work.
    ///
    /// Neither dropping case leaves state behind, which is the half a "returns
    /// something" assertion would miss.
    #[test]
    fn a_tag_the_page_is_not_offering_is_dropped() {
        let mut state = state();
        state.releases = vec![released("GE-Proton9-5", "a.tar.gz", 1024)];

        let task = update(
            &mut state,
            &Message::InstallRunner {
                tag: "not-offered".to_string(),
            },
        );
        assert!(
            task.is_some(),
            "the message is this page's — it is dropped, not declined"
        );
        assert!(
            !state.runner_busy,
            "a dropped install must not mark the page busy"
        );
        assert_eq!(state.progress, None, "and must not draw a bar");
    }

    /// A second click while a download is in flight is dropped rather than
    /// queued — `if self._runner_busy: return` (`bridge.py:739-740`).
    ///
    /// Without it the second click would start a second task against the same
    /// directory, which the install would refuse as already-installed once the
    /// first rename lands, and the user would see a failure toast for a download
    /// that succeeded.
    #[test]
    fn a_click_while_an_install_is_running_is_dropped() {
        let mut state = state();
        state.releases = vec![released("GE-Proton9-5", "a.tar.gz", 1024)];
        state.runner_busy = true;
        state.progress = Some(0.4);

        let task = update(
            &mut state,
            &Message::InstallRunner {
                tag: "GE-Proton9-5".to_string(),
            },
        );
        assert!(task.is_some(), "dropped, not declined");
        assert!(state.runner_busy, "the guard is unchanged");
        assert_eq!(
            state.progress,
            Some(0.4),
            "and the running download's bar is not reset to zero"
        );
    }

    /// The accepting path writes both halves of the busy state *before* any work
    /// starts, which is what makes the bar appear and the buttons disable
    /// immediately rather than one frame later.
    ///
    /// The task is asserted to be a task and not inspected: it spawns a real
    /// download, so driving it would reach the network. What is checkable
    /// without a network is the transition, and that is what this asserts. The
    /// task's own shape — the channel, the thread, the order of the reports — is
    /// covered by [`install_runner_task`]'s note and by
    /// `an_install_finishing_clears_the_guard_on_success_and_on_failure` from the
    /// receiving end.
    #[test]
    fn installing_marks_the_page_busy_at_zero_progress_before_the_work_starts() {
        let mut state = state();
        state.releases = vec![released("GE-Proton9-5", "a.tar.gz", 1024)];

        let task = update(
            &mut state,
            &Message::InstallRunner {
                tag: "GE-Proton9-5".to_string(),
            },
        );
        assert!(task.is_some(), "the install is this page's message");
        assert!(state.runner_busy, "the guard is up before the download");
        assert_eq!(
            state.progress,
            Some(0.0),
            "and the bar starts at zero, not at the previous download's value"
        );
        assert_eq!(progress_fraction(&state), Some(0.0));
    }

    // ---- uninstall_line ----------------------------------------------------

    /// [`remove_press`] asks first — and names the row's own id and name, never a
    /// constant — so the dialog titles the right build and the removal deletes
    /// it.
    ///
    /// `"system"` is the mutation this exists for: the family id is one
    /// keystroke from `SYSTEM_WINE`, it reads as correct, and removing a
    /// Windows game's runner by asking to remove the *family* is a refusal at
    /// best. Asserted as an equality against `row.runner_id` so the helper and
    /// the row cannot diverge, plus the literal so a second constant with the
    /// same value cannot shadow the first. The name is asserted likewise: an
    /// id without it would open a dialog titled for the wrong build.
    ///
    /// The `UninstallRunner` rejection is deliberate and load-bearing, not
    /// fallout of the retarget: the button must not delete on click, so a
    /// helper that still builds the removal directly is the defect P-37 exists
    /// to end, and this test fails on it rather than merely not covering the
    /// new message.
    ///
    /// **The name says `helper` because that is the only thing this measures.**
    /// It cannot see the button: `installed_card` could build the message
    /// itself and this stays green — that mutation survives, and the call site
    /// carries the comment saying so. A name promising "the remove button names
    /// its row" would assert a fact about a widget this test never reaches,
    /// which is the D-44 defect in miniature.
    ///
    /// The system row has no button at all — `removable: false` — so the value
    /// is asserted for a downloaded build, the only row that draws one.
    #[test]
    fn the_remove_press_helper_names_the_rows_own_id() {
        let protons = vec![ProtonRunner::new(
            "/runners/GE-Proton9-5",
            "proton-ge",
            "GE-Proton9-5",
        )];
        let rows = installed_rows(&WineRunner::with_binary(None), &protons);

        match remove_press(&rows[1]) {
            Message::ConfirmRemoveRunner { runner_id, name } => {
                assert_eq!(runner_id, rows[1].runner_id, "the helper names its own row");
                assert_eq!(runner_id, "GE-Proton9-5");
                assert_ne!(
                    runner_id, "system",
                    "\"system\" is the family id, and removing it names no runner"
                );
                assert_eq!(name, rows[1].name, "the dialog titles the same row");
                assert_eq!(name, "GE-Proton9-5");
            }
            other => panic!("the remove helper must ask first, got {other:?}"),
        }

        assert!(
            rows[1].removable,
            "a downloaded build is the row that draws the button"
        );
        assert!(
            !rows[0].removable,
            "and the system row does not, so this value is never built for it"
        );
    }

    /// The ask sets the pending removal; the dialog draws the title it names.
    ///
    /// Two mutations this exists for, separated because they are in two files:
    /// an arm that answers without writing `confirm_remove_runner` — the
    /// button clicks and no dialog opens — and an arm that writes a constant
    /// instead of the message's own id, which opens the dialog for the wrong
    /// build. Both leave `is_handled` green, which is why this asserts the
    /// field rather than the effect.
    #[test]
    fn asking_sets_the_pending_removal_the_dialog_draws() {
        let mut state = state();
        let task = update(
            &mut state,
            &Message::ConfirmRemoveRunner {
                runner_id: "GE-Proton9-5".to_string(),
                name: "GE-Proton9-5".to_string(),
            },
        );

        assert!(task.is_some(), "the ask is this page's to handle");
        assert_eq!(
            state.confirm_remove_runner,
            Some(crate::state::PendingRunnerRemoval {
                runner_id: "GE-Proton9-5".to_string(),
                name: "GE-Proton9-5".to_string(),
            }),
            "the pending removal is the message's own halves"
        );
        assert_eq!(
            state.confirm_remove_runner.as_ref().unwrap().title(),
            "Remove GE-Proton9-5?",
            "the dialog titles the row the button was pressed on"
        );
    }

    /// The confirmation clears the pending removal and runs the removal —
    /// through the refused id, so this needs no fixture.
    ///
    /// The clear is asserted, not just the toast: an arm that removed without
    /// clearing would leave the dialog open over the list it just changed.
    /// `RemoveRunnerConfirmed` reaching `proton::uninstall` is covered from the
    /// other side by `a_refused_removal_is_handled_and_leaves_the_release_status_alone`
    /// for the direct route; both routes call [`remove_runner`], so one
    /// refusal covers the shared half.
    #[test]
    fn confirming_clears_the_pending_removal_and_removes() {
        let mut state = state();
        state.confirm_remove_runner = Some(crate::state::PendingRunnerRemoval {
            runner_id: SYSTEM_WINE.to_string(),
            name: "System Wine".to_string(),
        });

        let task = update(
            &mut state,
            &Message::RemoveRunnerConfirmed(SYSTEM_WINE.to_string()),
        );

        assert!(task.is_some(), "the confirmation is this page's to handle");
        assert!(
            state.confirm_remove_runner.is_none(),
            "the dialog must close on confirmation"
        );
        assert!(!state.runner_busy, "a removal is not a busy job");
    }

    /// Whether `remove_runner`'s body uninstalls before it refreshes — `None`
    /// when the body no longer has the shape this reads, so a refactor that
    /// moves the calls fails the guard instead of silently passing it.
    fn removal_refreshes_after_uninstall(body: &str) -> Option<bool> {
        let uninstall = body.find("proton::uninstall")?;
        let refresh = body.find("refresh(state)")?;
        Some(uninstall < refresh)
    }

    /// The instrument, driven on the defect it was written for and on the fix.
    ///
    /// `remove_runner` at `f37ca49`, verbatim: the refresh snapshots
    /// `installed_protons()` before the uninstall deletes the build, so the
    /// reply resurrects the row. T-19 watched the removed runner stay listed.
    #[test]
    fn the_removal_order_guard_reports_the_defect_it_was_written_for() {
        let defect = "fn remove_runner(state: &mut State, runner_id: &str) -> Task<Message> {\n    let rows = refresh(state);\n    let line = uninstall_line(\n        runner_id,\n        proton::uninstall(state.runners.runners_directory(), runner_id),\n    );\n    Task::batch([rows, push_toast(state, line)])\n}";
        assert_eq!(
            removal_refreshes_after_uninstall(defect),
            Some(false),
            "the instrument must report the refresh-before-uninstall order"
        );

        let fixed = "fn remove_runner(state: &mut State, runner_id: &str) -> Task<Message> {\n    let line = uninstall_line(\n        runner_id,\n        proton::uninstall(state.runners.runners_directory(), runner_id),\n    );\n    let rows = refresh(state);\n    Task::batch([rows, push_toast(state, line)])\n}";
        assert_eq!(
            removal_refreshes_after_uninstall(fixed),
            Some(true),
            "the instrument must stay silent on the uninstall-before-refresh order"
        );
    }

    /// And the instrument on this tree: `remove_runner` uninstalls first.
    ///
    /// A test cannot drive the refresh reply — the rows arrive inside an opaque
    /// `Task` — so this is a source scan in the shape of
    /// `no_dropdown_callback_turns_its_index_into_the_payload`, with the live
    /// walk as its behavioral half: remove a runner and the row goes with it.
    #[test]
    fn the_removal_refreshes_after_the_uninstall_not_before() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/view/runners.rs");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
        let start = text
            .find("fn remove_runner(")
            .expect("`remove_runner` is where a removal refreshes");
        let body = &text[start..];
        let end = body
            .find("\n}\n")
            .expect("`remove_runner` ends where its closing brace is");
        assert_eq!(
            removal_refreshes_after_uninstall(&body[..end]),
            Some(true),
            "`remove_runner` must uninstall before it refreshes, or the reply \
             resurrects the removed row"
        );
    }

    /// `CloseDialog` clears a pending runner removal, like the form and the
    /// pending game delete — but that arm lives in `main.rs`, so it is
    /// asserted there rather than here. This test pins the page's half of the
    /// contract: the field the dialog reads is the field the ask writes, and
    /// clearing it is what closes the dialog.
    #[test]
    fn the_dialog_is_open_exactly_while_a_removal_is_pending() {
        let mut state = state();
        assert!(
            state.confirm_remove_runner.is_none(),
            "a fresh page has no dialog open"
        );

        let _ = update(
            &mut state,
            &Message::ConfirmRemoveRunner {
                runner_id: "GE-Proton9-5".to_string(),
                name: "GE-Proton9-5".to_string(),
            },
        );
        assert!(
            state.confirm_remove_runner.is_some(),
            "the ask opens the dialog"
        );

        let _ = update(
            &mut state,
            &Message::RemoveRunnerConfirmed("GE-Proton9-5".to_string()),
        );
        assert!(
            state.confirm_remove_runner.is_none(),
            "the confirmation closes it"
        );
    }

    /// A failed removal says *what* went wrong and *which* runner it was about.
    ///
    /// `uninstallRunner`'s `except` (`bridge.py:775-777`) is where this line
    /// comes from, and the runner id is the half that says which build is still
    /// on disk. The `contains` is a second, deliberately different claim from
    /// the `assert_eq!`: it is the one that survives a line that keeps its shape
    /// but drops the error tail — `Could not remove GE-Proton9-5` is a sentence
    /// a user reads as a refusal with no reason, and it would pass an equality
    /// test written against whatever the port produced.
    #[test]
    fn a_failed_removal_carries_the_error_text_and_the_runner_id() {
        let line = uninstall_line("GE-Proton9-5", Err("permission denied".to_string()));
        assert_eq!(line, "Could not remove GE-Proton9-5: permission denied");
        assert!(
            line.contains("permission denied"),
            "the runner's own error is the only part that says what to fix: {line:?}"
        );
        assert!(
            line.contains("GE-Proton9-5"),
            "and the id is the only part that says which build survived: {line:?}"
        );
    }

    /// The other half, which must not acquire the error wording. The two arms
    /// are one `match` apart and a port that shared one `format!` would tell a
    /// user their removal failed when it succeeded.
    #[test]
    fn a_removal_that_worked_says_so_without_an_error_tail() {
        assert_eq!(
            uninstall_line::<String>("GE-Proton9-5", Ok(())),
            "Removed GE-Proton9-5"
        );
    }

    /// The arm itself, driven for real through a failure it can reach without a
    /// filesystem: removing System Wine is refused by `proton::uninstall` before
    /// it looks at the directory (`proton.rs:1085`), so this needs no fixture.
    ///
    /// What this does **not** cover, and cannot: the *text* of the toast. The
    /// toast is the only observable of this arm and `Toasts` exposes no reader
    /// (`toaster/mod.rs:155-204` — `push` and `remove` are the whole surface),
    /// so the line is covered instead by
    /// [`a_failed_removal_carries_the_error_text_and_the_runner_id`] calling
    /// [`uninstall_line`] directly. The arm is one call away from it, which is
    /// as much as the observable allows.
    ///
    /// The status claim is the interesting half: `_releases_status` is written
    /// by `fetchReleases` and its two callbacks and **nowhere else**
    /// (`bridge.py:697,708,714`; the only other write is the constructor's
    /// `"idle"`, `bridge.py:136`), and `uninstallRunner` assigns nothing at all
    /// (`bridge.py:772-782`). Asserted over every state rather than one, because
    /// the defect this guards against is not a wrong *value* — it is an arm that
    /// writes the field at all. An earlier revision of this arm set `Idle` here
    /// to stand in for the reference's `releasesChanged.emit()`; that is a state
    /// the reference cannot reach with a non-empty list, and it renders
    /// "No builds found for this family." above the builds.
    #[test]
    fn a_refused_removal_is_handled_and_leaves_the_release_status_alone() {
        for before in [
            ReleasesStatus::Idle,
            ReleasesStatus::Loading,
            ReleasesStatus::Ready,
            ReleasesStatus::Error("rate limit exceeded".to_string()),
        ] {
            let mut state = state();
            state.releases_status = before.clone();

            let task = update(
                &mut state,
                &Message::UninstallRunner(SYSTEM_WINE.to_string()),
            );

            assert!(task.is_some(), "a removal is this page's to handle");
            assert_eq!(
                state.releases_status, before,
                "only fetchReleases writes the release status"
            );
            assert!(!state.runner_busy, "a removal is not a busy job");
        }
    }
}
