//! The `Message`/`Task`/file-filter vocabulary the easy-install flow speaks.
//!
//! Everything here is a pure constructor — no `State`, no `self`, no worker —
//! which is what made it the seam `ARCH-11` cut: the flow half
//! ([`mod.rs`](self)) holds the install record, the download timeout and the
//! token a run is identified by, and this half only needs to name them. Split
//! so each file is one concern: `mod.rs` is *what the flow does*, this file is
//! *what it can say*.

use std::path::Path;
use std::time::Duration;

use cosmic::widget::toaster::ToastId;
use gamehandler_core::netpaths::as_local_path;

use crate::state::CoverHit;

use super::Message;

/// `done(result)` (`bridge.py:874-895`): the found branch finishes the install,
/// the other one stores it and asks the user for an executable.
///
/// The not-found branch leaves `easy_busy` **true**, which is the reference's
/// own asymmetry and not an oversight: the install is not abandoned, it is
/// waiting for the user, and `completeEasyInstall`/`cancelEasyInstall` are the
/// two ways it ends. A port that cleared the guard here would let a second
/// install start on top of a pending one — P-59's guard, defeated by the only
/// path that has something to lose.
/// The reference's `locateDialog` (`Main.qml:182-189`), as a portal chooser
/// task: `easyInstallNeedsExe`'s `FileDialog` with its two `nameFilters`,
/// accepted into [`Message::CompleteEasyInstall`] and rejected into
/// [`Message::CancelEasyInstall`].
///
/// Opened by [`easy_install_wizard_finished`]'s not-found branch, which is
/// what makes the busy state it leaves behind escapable: before this task
/// existed nothing produced either message, so a wizard that closed without
/// installing wedged the page forever (P-57).
///
/// # The start folder is not ported
///
/// `onEasyInstallNeedsExe` points the dialog at the prefix's `drive_c`
/// (`Main.qml:166`), and libcosmic's `open::Dialog` carries a `directory`
/// field for exactly that — but marks it `dead_code` because ashpd does not
/// expose it yet, and the portal request builder never sends it. So the
/// chooser opens wherever the portal opens, and this function does not take a
/// start folder it would silently drop.
///
/// # What is tested, and what is read
///
/// The filters ([`exe_file_filters`]) and the answer mapping
/// ([`locate_message`]) are pure and pinned below. The assembly — the title,
/// the `open_file` call itself — is read, not tested: `Dialog` keeps its
/// fields private and driving the portal needs a session bus. That is the
/// same wall `theme::apply`'s call sites stand behind, and it is named for
/// the same reason.
pub(crate) fn locate_exe_task(token: String, installer_name: String) -> cosmic::app::Task<Message> {
    use cosmic::dialog::file_chooser::open;
    cosmic::app::Task::perform(
        async move {
            let filters = exe_file_filters();
            let mut dialog = open::Dialog::new()
                .title(format!("Locate {installer_name}"))
                .current_filter(filters[0].clone());
            for filter in filters {
                dialog = dialog.filter(filter);
            }
            let answer = dialog
                .open_file()
                .await
                .map(|response| response.url().clone());
            locate_message(token, answer)
        },
        cosmic::Action::App,
    )
}

/// The reference's `nameFilters` (`Main.qml:185`): executables first, then
/// everything.
///
/// A named function rather than two literals at the call site so the patterns
/// are readable back — `Dialog` keeps its filters private, so this is the
/// half of the chooser a test can pin. The `*.EXE` second glob is the
/// reference's own case belt-and-braces, kept rather than assumed redundant.
pub(crate) fn exe_file_filters() -> Vec<cosmic::dialog::file_chooser::FileFilter> {
    use cosmic::dialog::file_chooser::FileFilter;
    vec![
        FileFilter::new("Windows executables")
            .glob("*.exe")
            .glob("*.EXE"),
        FileFilter::new("All files").glob("*"),
    ]
}

/// The pure half of the locate reply: the chooser's answer as the message the
/// shell already handles.
///
/// A chosen file becomes the path [`complete_easy_install`] finishes from —
/// `Url::to_file_path` is the `as_local_path` the reference applies on the
/// way (`bridge.py:926`), decoding the percent-escapes the portal leaves in.
/// A URL with no local path becomes the `None` that takes the cancel path,
/// exactly as `as_local_path` of nothing does there.
///
/// Any error — the user's cancel and the portal's own failure alike — becomes
/// [`Message::CancelEasyInstall`]. The two are indistinguishable for state
/// purposes: both must release the busy guard the not-found branch holds, and
/// the kept-prefix notice the cancel path toasts is honest in both cases,
/// because nothing deleted the prefix.
pub(crate) fn locate_message(
    token: String,
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    match answer {
        Ok(url) => {
            let path = url
                .to_file_path()
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
            Message::CompleteEasyInstall { token, path }
        }
        Err(_) => Message::CancelEasyInstall(token),
    }
}

/// `fetch_cover`'s own default (`covers.py:359`): twenty seconds for the
/// whole lookup, Steam and icon alike. One constant because the reference has
/// one default; a lookup that needs more patience than a store search is a
/// lookup that has already failed.
const COVER_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The worker half of both cover lookups: `fetch_cover` over the production
/// client, failing to the rendered string.
///
/// Called inside `Task::perform`'s future, like `start_prefix_tool` and
/// `open_prefix_folder` — the blocking client needs no async adaptation
/// because the future runs on a worker (D-48). The `Err` is a `String`
/// because `_async` turns whatever the work raised into one before handing it
/// to `fail` (`bridge.py:157`), and the default `fail` notifies it verbatim —
/// so the reply arms toast the string untouched.
pub(crate) fn cover_lookup(
    name: &str,
    game_id: &str,
    exe: Option<&Path>,
) -> Result<CoverHit, String> {
    gamehandler_core::covers::fetch_cover(
        &crate::http::UreqClient,
        name,
        game_id,
        exe,
        &gamehandler_core::paths::covers_dir(),
        COVER_FETCH_TIMEOUT,
    )
    .map_err(|error| error.to_string())
}

/// The cover a finished easy install carries:
/// `game.cover_path = str(save_exe_icon(exe_path, game_id))`, with `""` when
/// that raises (`bridge.py:907-911`) — a store launcher is not a Steam
/// product, so the executable the vendor just installed carries the right
/// artwork already.
///
/// # What is tested, and what is read
///
/// The `""` half is pinned below (an exe with no icon keeps the empty
/// string). The success half — bytes in, `.ico` path out — is `core`'s
/// `save_exe_icon_to`, tested there against PEs its own builders synthesise;
/// rebuilding that fixture here would duplicate a binary-format builder
/// across crates, so the three-line seam is read.
pub(crate) fn easy_install_cover(executable: &Path, game_id: &str) -> String {
    gamehandler_core::covers::save_exe_icon_to(
        executable,
        game_id,
        &gamehandler_core::paths::covers_dir(),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_default()
}

/// The reference's cover `nameFilters` (`GameFormPage.qml:352`): images only.
/// No "All files" row — the QML lists exactly one filter, and a cover that is
/// not an image is not a cover.
pub(crate) fn image_file_filters() -> Vec<cosmic::dialog::file_chooser::FileFilter> {
    use cosmic::dialog::file_chooser::FileFilter;
    vec![
        FileFilter::new("Images")
            .glob("*.png")
            .glob("*.jpg")
            .glob("*.jpeg")
            .glob("*.webp"),
    ]
}

/// The pure half of the exe reply: the chooser's answer as the local path
/// `urlToLocalFile` hands the form (`bridge.py:601-603`, P-21's browse half).
/// Cancel and portal failure alike become `None` — the reference's exe dialog
/// has no `onRejected` at all, so rejecting leaves the field as it was.
///
/// This runs the URL string through [`as_local_path`] rather than
/// `Url::to_file_path` (which is what the locate reply uses): the reference
/// runs the raw `selectedFile` through `as_local_path`, so a share URL keeps
/// its verbatim fallback instead of collapsing to a cancel.
pub(crate) fn exe_choice_message(
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    Message::ExeFileChosen(answer.ok().map(|url| as_local_path(url.as_str())))
}

/// The pure half of the cover reply: same URL handling as the exe choice —
/// the reference runs both through `as_local_path` (`importCustomCover` does
/// it on the way in, `bridge.py:593`) — and the same silent cancel.
pub(crate) fn cover_choice_message(
    answer: Result<url::Url, cosmic::dialog::file_chooser::Error>,
) -> Message {
    Message::CoverFileChosen(answer.ok().map(|url| as_local_path(url.as_str())))
}

/// The installed toast's Play action, as a value: `showPassiveNotification`'s
/// `function() { backend.playGame(gameId) }` (`Main.qml:159-161`).
///
/// A named function rather than an inline closure so the mapping is readable
/// back — `Toast` keeps its action private with no accessor, so a test cannot
/// drive the button; this pins the value the button *would* carry. What this
/// does not close: the arm could stop calling this and build the message
/// itself — the same call-site gap `remove_press`'s doc names.
pub(crate) fn installed_play_message(game_id: &str, toast_id: ToastId) -> Message {
    Message::PlayInstalled {
        game_id: game_id.to_string(),
        toast_id,
    }
}
