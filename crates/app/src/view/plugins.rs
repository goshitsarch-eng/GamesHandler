//! The Plugins page: five optional helpers, three states, and the install
//! action (P-64).
//!
//! Port of `PluginsPage.qml` (58 lines) and the plugin half of `bridge.py`
//! (`:949-1020`). The data is [`gamehandler_core::plugins`]; this module is the
//! page over it, and `main.rs` holds the cache the page reads.
//!
//! # What this page is, and what it is not
//!
//! Every helper is listed with the state the reference computes: **installed**
//! (on `PATH`), **missing** (installable from here), or **unavailable** (not
//! installable, either because the sandbox forbids it or because no package
//! mapping exists for this host). Only `missing` offers a button that acts —
//! `PluginsPage.qml:51` — which is the difference between a page and a page
//! with a dead control on it.
//!
//! # The install is a real one, and it is the reason T-28 existed
//!
//! [`Message::InstallPlugin`] runs the reference's own command through
//! `std::process::Command`: `privileged_command(install_command(plugin))`, with
//! the 300-second ceiling `install_plugin`'s default carries
//! (`plugins.py:172-175`). The spawn is here rather than in `core` on purpose —
//! `core` describes the command as data ([`install_command`],
//! [`privileged_command`], [`format_command`]) and this layer runs blocking
//! work off the UI thread, exactly as [`super::runners`] runs its downloads.
//! That split is what kept this task inside the app layer; `core`'s
//! `runners::launch` is a different seam (a game's launch) and was not touched.
//!
//! [`Message::InstallPlugin`]: crate::Message::InstallPlugin
//! [`install_command`]: gamehandler_core::plugins::install_command
//! [`privileged_command`]: gamehandler_core::plugins::privileged_command
//! [`format_command`]: gamehandler_core::plugins::format_command

use std::time::{Duration, Instant};

use cosmic::Element;
use cosmic::app::Task;
use cosmic::iced::Length;
use cosmic::widget::{Column, Row, button, container, scrollable, text};
use gamehandler_core::plugins::{self, Plugin, PluginEnv, PluginRow, PluginState, SystemPluginEnv};

use crate::Message;

/// The heading above the list (`PluginsPage.qml:17`).
pub const SECTION_HOST_PLUGINS: &str = "Host plugins";

/// `install_plugin`'s own default (`plugins.py:172`), which `installPlugin`
/// does not override — so this is the reference's effective ceiling rather than
/// a number chosen here.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// How often the child is polled while the ceiling is outstanding.
///
/// `std::process::Command` has no timeout, so the wait is a poll loop. 50 ms is
/// well below the shortest thing a package manager does (a `pkexec` prompt
/// appears in far less) and 6000 polls over the whole ceiling is not a cost
/// worth a second dependency to avoid. The reference does not have this number
/// at all — `subprocess.run` implements its timeout with a blocking wait and a
/// kill on a separate path — so it is an implementation constant, not a
/// reference value being transcribed.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Everything the page draws, computed by the caller.
///
/// Borrowed rather than owned, the shape [`super::settings::SettingsPage`]
/// uses: the rows live in `State` and the page must not copy them to render.
pub struct PluginsPage<'a> {
    /// `pluginsIntro` — the sentence above the list.
    pub intro: &'a str,
    /// `plugins` — one row per helper, in the catalogue's order.
    pub rows: &'a [PluginRow],
}

/// The button's label (`PluginsPage.qml:48-50`), a three-way ternary there and
/// a table here.
pub fn button_label(state: PluginState) -> &'static str {
    match state {
        PluginState::Installed => "Installed",
        PluginState::Missing => "Install",
        PluginState::Unavailable => "Unavailable",
    }
}

/// Whether the button acts. `enabled: state === "missing"` (`:51`).
///
/// The other two are shown but inert, which is the reference's decision: an
/// installed helper has nothing to do, and an unavailable one has no command
/// this page could honestly run.
pub fn button_enabled(state: PluginState) -> bool {
    state == PluginState::Missing
}

/// The message the button sends (`:52`), `installPlugin(modelData.pluginId)`.
///
/// A function rather than a closure in the builder for the reason finding #57
/// is about: a closure inside a builder cannot be reached by an assertion, and
/// the id is exactly the value that went wrong there.
///
/// It takes the **row**, not a `&str`, and that is the guard rather than a
/// style choice. With a `&str` the only call site was
/// `install_message(row.plugin_id)` written inline in [`card`], and an
/// assertion could only re-derive the same expression — so a `card` that
/// hardcoded a literal id would keep every test green, which is the defect
/// `the_button_sends_the_rows_own_id` says it prevents. Taking the row means
/// the id the button sends is the id of the row it was drawn from, by
/// construction: the compiler rejects the literal, and a wrong id would have to
/// be a whole fabricated [`PluginRow`] rather than one character.
pub fn install_message(row: &PluginRow) -> Message {
    Message::InstallPlugin(row.plugin_id.to_string())
}

/// What the button does when pressed: the message, or `None` for an inert one.
///
/// `enabled` and `onClicked` are two properties of one button in the QML
/// (`:51-52`), so they are one function here rather than two calls at the
/// builder. That matters for the same reason [`install_message`] takes a row:
/// `on_press_maybe(button_enabled(row.state).then(|| install_message(row)))`
/// is correct only if the reader pairs them correctly, and a builder that
/// swaps the two rows it is passed compiles. Here there is one row.
pub fn card_action(row: &PluginRow) -> Option<Message> {
    button_enabled(row.state).then(|| install_message(row))
}

/// The message shown while the install runs (`bridge.py:1009`).
pub fn installing_message(name: &str) -> String {
    format!("Installing {name}…")
}

/// The success message (`bridge.py:1016`).
pub fn installed_message(name: &str) -> String {
    format!("{name} is installed")
}

/// The message when the command ran but the helper is still not there
/// (`bridge.py:1018-1021`).
///
/// Two sentences, and the second is the one that matters: it tells the user the
/// command is visible on the page rather than leaving them with a failure and
/// no next step.
pub fn not_installed_message(name: &str) -> String {
    format!("{name} did not install. The command is shown on the Plugins page.")
}

/// The message when the command could not be run at all (`bridge.py:1024`),
/// which is `_async`'s `fail` path (`bridge.py:156-161`).
pub fn install_failed_message(name: &str, error: &str) -> String {
    format!("Could not install {name}: {error}")
}

/// `subprocess.TimeoutExpired`'s own text, as Python renders it.
///
/// `subprocess.run` raises it with `f"Command '{...}' timed out after {timeout}
/// seconds"`, and `_async` turns the exception into `str(exc)`, so this string
/// reaches the user through [`install_failed_message`]. Reproduced rather than
/// shortened because it is the only description of *what* timed out.
pub fn timeout_message(argv: &[String], seconds: u64) -> String {
    format!(
        "Command '{}' timed out after {seconds} seconds",
        plugins::format_command(argv)
    )
}

/// Whether an install succeeded: `result.returncode == 0 and
/// plugin.is_installed()` (`bridge.py:1015`).
///
/// A named function because the operator is the whole assertion: both halves
/// are load-bearing. A package manager that exits 0 without exposing the binary
/// (a `flatpak` install into a different installation, say) has not installed
/// anything the app can use, and a helper that was already present when a
/// failed command ran has not been installed by it either. Written as `||` or
/// with one half dropped, this page tells the user something untrue.
pub fn install_succeeded(exit_ok: bool, installed: bool) -> bool {
    exit_ok && installed
}

/// Run `argv` to completion, or kill it at `timeout`.
///
/// `Ok(true)` when it exited 0. `Err` carries the rendered reason, which is
/// either the spawn failure or [`timeout_message`] — the two things
/// `subprocess.run` can raise here that `_async` would surface.
pub fn run_to_completion(argv: &[String], timeout: Duration) -> Result<bool, String> {
    let Some((program, args)) = argv.split_first() else {
        // `subprocess.run([])` raises `IndexError`, which `_async` renders as
        // the class name. Unreachable from `install_command`, which always
        // names a program, and stated rather than panicking.
        return Err("IndexError".to_string());
    };
    let mut child = std::process::Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| error.to_string())?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => {
                if Instant::now() >= deadline {
                    // `subprocess.run` kills the child and then reaps it; the
                    // reaping is what stops this from leaving a zombie behind
                    // every time a package manager hangs.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(timeout_message(argv, timeout.as_secs()));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// The blocking half of the install: build the command, run it, check the
/// result.
///
/// Takes the environment so the command and the post-check agree about which
/// host they are describing — `is_installed` re-asks `which` here rather than
/// trusting the exit code, which is the reference's own arrangement.
pub fn run_install(plugin: &Plugin, env: &dyn PluginEnv) -> Result<bool, String> {
    let argv = plugins::install_command(plugin, None, env).map_err(|error| error.to_string())?;
    let argv = plugins::privileged_command(&argv, env);
    let exit_ok = run_to_completion(&argv, INSTALL_TIMEOUT)?;
    Ok(install_succeeded(exit_ok, plugin.is_installed(env)))
}

/// [`Message::InstallPlugin`]'s task half.
///
/// The plan is fixed **here**, on the UI thread, rather than inside the
/// spawned closure: an unknown id has to be a no-op that also pushes no toast,
/// and looking it up before the task exists is what makes that the same
/// decision the reference makes (`except KeyError: return`, `bridge.py:1007-1008`).
///
/// [`Message::InstallPlugin`]: crate::Message::InstallPlugin
pub fn install_plan(plugin_id: &str) -> Option<(String, Task<Message>)> {
    let plugin = plugins::plugin_by_id(plugin_id)?;
    let notice = installing_message(plugin.name);
    let task = Task::perform(
        async move {
            let env = SystemPluginEnv;
            let result = run_install(plugin, &env);
            Message::PluginInstallFinished {
                plugin_id: plugin.id.to_string(),
                result,
            }
        },
        // `cosmic::app::Task<Message>` is `iced::Task<Action<Message>>`, so a
        // task built here must wrap its output to reach the shell's `update`.
        cosmic::Action::App,
    );
    Some((notice, task))
}

/// One helper's card: the name, the subtitle, and the button.
fn card<'a>(row: &'a PluginRow) -> Element<'a, Message> {
    let details = Column::new()
        .spacing(2)
        .width(Length::Fill)
        .push(text::heading(row.name))
        .push(text::caption(row.subtitle.as_str()));
    let action = button::standard(button_label(row.state)).on_press_maybe(card_action(row));
    container(
        Row::new()
            .push(details)
            .push(action)
            .spacing(12)
            .align_y(cosmic::iced::Alignment::Center)
            .width(Length::Fill),
    )
    .padding(12)
    .width(Length::Fill)
    .into()
}

pub fn view<'a>(page: PluginsPage<'a>) -> Element<'a, Message> {
    let mut body = Column::new()
        .spacing(12)
        .width(Length::Fill)
        .push(text::title4(SECTION_HOST_PLUGINS))
        .push(text::caption(page.intro));

    for row in page.rows {
        body = body.push(card(row));
    }

    container(scrollable(body)).padding(18).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A file from the repository root, read at test time.
    ///
    /// `CARGO_MANIFEST_DIR` is `crates/app`, so the root is two levels up.
    /// Reading the reference off disk is the point: a constant compared against
    /// a copy of itself cannot disagree with itself.
    fn repo_file(relative: &str) -> String {
        let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
    }

    /// Python's implicit string concatenation, undone.
    ///
    /// `bridge.py` writes its notifications as adjacent literals across lines,
    /// and the joined text is what the user reads. Both a bare quote and a
    /// prefixed one (`f"…"`) can follow, which is what `probe`/`quote` scan.
    fn join_adjacent_literals(source: &str) -> String {
        fn is_prefix(character: char) -> bool {
            matches!(character, 'f' | 'F' | 'r' | 'R' | 'b' | 'B' | 'u' | 'U')
        }
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '"' {
                let mut probe = index + 1;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                let mut quote = probe;
                while quote < chars.len() && quote - probe < 2 && is_prefix(chars[quote]) {
                    quote += 1;
                }
                if probe > index + 1 && quote < chars.len() && chars[quote] == '"' {
                    index = quote + 1;
                    continue;
                }
            }
            out.push(chars[index]);
            index += 1;
        }
        out
    }

    #[test]
    fn the_button_is_the_qmls_three_labels_and_only_missing_acts() {
        // Both halves are read off `PluginsPage.qml` at test time. A page that
        // spelled a label differently, or that enabled the button for
        // `unavailable` — which would offer the user a command the page has
        // already decided it cannot run — fails here.
        let qml = repo_file("gamehandler/qml/PluginsPage.qml");
        for (state, label) in [
            (PluginState::Installed, "Installed"),
            (PluginState::Missing, "Install"),
            (PluginState::Unavailable, "Unavailable"),
        ] {
            assert_eq!(button_label(state), label);
            assert!(
                qml.contains(&format!("\"{label}\"")),
                "PluginsPage.qml no longer offers the button label {label:?}"
            );
        }
        assert!(button_enabled(PluginState::Missing));
        assert!(!button_enabled(PluginState::Installed));
        assert!(!button_enabled(PluginState::Unavailable));

        // The enabled test is the same expression the QML writes, so the two
        // cannot drift into disagreeing about which state acts.
        assert!(qml.contains("state === \"missing\""));
    }

    #[test]
    fn the_heading_is_the_qmls() {
        assert!(
            repo_file("gamehandler/qml/PluginsPage.qml")
                .contains(&format!("\"{SECTION_HOST_PLUGINS}\"")),
            "the page heading is no longer the QML's"
        );
    }

    #[test]
    fn the_button_sends_the_rows_own_id() {
        // `PluginsPage.qml:52` sends `modelData.pluginId`, and the row is built
        // from the catalogue — so the id that reaches the handler is the one
        // `plugin_by_id` will resolve.
        //
        // **This test was vacuous in its first version**, which is worth
        // recording because it is the exact defect it names. It asserted over
        // `install_message(row.plugin_id)` — the same expression `card` wrote
        // inline — so `card` could have hardcoded any literal and this stayed
        // green: it tested the function, never the wiring. `install_message`
        // now takes the row and `card` calls `card_action(row)`, so the pairing
        // this asserts *is* the pairing the widget builds. What is still not
        // reachable is `card`'s own use of `card_action`: iced keeps a button's
        // `on_press` behind the widget, not in the `Tree`, so no assertion can
        // read it back — the difference is that hardcoding one now requires a
        // fabricated `PluginRow` instead of a typo.
        let env = plugins::SystemPluginEnv;
        for plugin in plugins::plugins() {
            let row = plugins::plugin_row(plugin, &env);
            match card_action(&row) {
                Some(Message::InstallPlugin(sent)) => {
                    assert_eq!(sent, plugin.id);
                    assert!(
                        plugins::plugin_by_id(&sent).is_some(),
                        "the button would send {sent:?}, which does not resolve"
                    );
                }
                other => panic!("expected Some(InstallPlugin), got {other:?}"),
            }
        }
    }

    /// The three states, hand-built so the test does not depend on what this
    /// host happens to have installed: only the missing one offers a button,
    /// and the button it offers carries *that row's* id.
    #[test]
    fn only_a_missing_helper_offers_a_button_and_it_offers_that_rows_id() {
        let row = |id: &'static str, state| PluginRow {
            plugin_id: id,
            name: "A helper",
            subtitle: String::new(),
            state,
        };
        for state in [
            PluginState::Installed,
            PluginState::Missing,
            PluginState::Unavailable,
        ] {
            // An id that is deliberately **not** in the catalogue: a
            // `card_action` that reached for any real helper's id would still
            // disagree with this one.
            const ID: &str = "not-a-catalogue-entry";
            let row = row(ID, state);
            match card_action(&row) {
                None => assert_ne!(
                    state,
                    PluginState::Missing,
                    "a missing helper's button is inert, so the page offers an \
                     install the user cannot start"
                ),
                Some(Message::InstallPlugin(sent)) => {
                    assert_eq!(state, PluginState::Missing);
                    assert_eq!(sent, ID);
                }
                Some(other) => panic!("the button sends {other:?}"),
            }
        }
    }

    #[test]
    fn the_notifications_are_the_bridges_wording() {
        // `bridge.py:1009-1024`, joined and then compared word for word. The
        // reference builds each of these from `plugin.name`, so the name is
        // substituted here with a value that cannot appear in the source and
        // the surrounding text is checked against the reference's own literal.
        let bridge = join_adjacent_literals(&repo_file("gamehandler/bridge.py"));

        let installing = installing_message("MangoHud");
        assert_eq!(installing, "Installing MangoHud…");
        assert!(
            bridge.contains("Installing {plugin.name}…"),
            "the installing notice is not bridge.py's sentence"
        );

        assert_eq!(installed_message("MangoHud"), "MangoHud is installed");
        assert!(bridge.contains("{plugin.name} is installed"));

        let failed = not_installed_message("MangoHud");
        assert_eq!(
            failed,
            "MangoHud did not install. The command is shown on the Plugins page."
        );
        assert!(
            bridge.contains("The command is shown on the Plugins page."),
            "the join across the literal boundary is what makes this sentence one string"
        );

        assert_eq!(
            install_failed_message("MangoHud", "timed out"),
            "Could not install MangoHud: timed out"
        );
        assert!(bridge.contains("Could not install {plugin.name}: {message}"));
    }

    #[test]
    fn a_timeout_reads_as_pythons_timeoutexpired() {
        // `subprocess.TimeoutExpired`'s own rendering, which `_async` passes
        // through as `str(exc)`. Pinned as a literal because the argv is joined
        // with `format_command`, so this is also the assertion that the
        // timeout report names the command rather than just the delay.
        let argv: Vec<String> = ["pkexec", "apt-get", "install", "-y", "mangohud"]
            .iter()
            .map(|part| (*part).to_string())
            .collect();
        assert_eq!(
            timeout_message(&argv, 300),
            "Command 'pkexec apt-get install -y mangohud' timed out after 300 seconds"
        );
        assert_eq!(INSTALL_TIMEOUT.as_secs(), 300);
    }

    #[test]
    fn an_install_needs_both_the_exit_code_and_the_binary() {
        assert!(install_succeeded(true, true));
        assert!(!install_succeeded(true, false));
        assert!(!install_succeeded(false, true));
        assert!(!install_succeeded(false, false));
    }

    #[test]
    fn the_runner_kills_a_child_that_outlasts_its_budget() {
        // A real child, and a real kill: `/bin/sh` is the one program this can
        // assume, and the two cases are the two answers `subprocess.run` gives.
        let ok = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exit 0".to_string(),
        ];
        assert_eq!(run_to_completion(&ok, Duration::from_secs(10)), Ok(true));

        let bad = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exit 3".to_string(),
        ];
        assert_eq!(run_to_completion(&bad, Duration::from_secs(10)), Ok(false));

        let slow = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sleep 30".to_string(),
        ];
        let started = Instant::now();
        let outcome = run_to_completion(&slow, Duration::from_millis(300));
        assert_eq!(
            outcome,
            Err("Command '/bin/sh -c sleep 30' timed out after 0 seconds".to_string())
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the child was not killed at the deadline; it ran for {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_command_that_cannot_be_spawned_is_reported_and_not_panicked() {
        // `_async` catches whatever the work raised and renders it
        // (`bridge.py:156-161`), so an unresolvable program has to be an `Err`
        // rather than a crash in a worker thread.
        let argv = vec!["/nonexistent/gh-t06-no-such-binary".to_string()];
        let outcome = run_to_completion(&argv, Duration::from_secs(5));
        assert!(
            outcome.is_err(),
            "expected a spawn failure, got {outcome:?}"
        );

        // And the empty argv, which Python answers with an `IndexError`.
        assert_eq!(
            run_to_completion(&[], Duration::from_secs(5)),
            Err("IndexError".to_string())
        );
    }

    #[test]
    fn an_unknown_plugin_is_a_no_op_with_no_notice() {
        // `except KeyError: return` (`bridge.py:1007-1008`) — and the `return`
        // is before the notice, so an unknown id must not toast. Returning a
        // task here would be a message the user cannot act on.
        assert!(install_plan("no-such-plugin").is_none());
        // And every real id plans, so the button is never inert on a `missing`
        // row.
        for plugin in plugins::plugins() {
            assert!(
                install_plan(plugin.id).is_some(),
                "{} has no install plan",
                plugin.id
            );
        }
    }

    #[test]
    fn the_plan_names_the_helper_the_user_sees() {
        let (notice, _task) = install_plan("mangohud").expect("mangohud is in the catalogue");
        assert_eq!(notice, "Installing MangoHud…");
    }
}
