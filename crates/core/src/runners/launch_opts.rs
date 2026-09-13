//! Launch options: the toggle matrix, the environment block, and the bundled
//! DXVK installer.
//!
//! Port of `runners.py:966-1302` — `parse_env_block`, `merge_dll_overrides`,
//! `install_bundled_dxvk`, `normalize_desktop_size`, `virtual_desktop_argv`,
//! `apply_launch_options`, `build_linux_command` and `resolve_game_paths`.
//!
//! # Why this is a separate module from the runner hierarchy
//!
//! `build_command` answers "what does this runner run"; this module answers
//! "what does the *user's* configuration add to it". They fail differently,
//! too: a bug in `build_command` produces the wrong executable, while a bug
//! here produces a game that starts and then behaves oddly — no Proton
//! wayland, a DXVK toggle that silently did nothing, a mangohud overlay that
//! never appears. That second class is invisible in a diff and needs the
//! matrix tested toggle by toggle, which is what the tests below do.
//!
//! # The impure reads are injected
//!
//! `shutil.which` (mangohud, gamemoderun, gamescope), `os.environ`, and
//! `find_anticheat_runtime` all go through [`LaunchEnv`] (DECISIONS D-27), so
//! the whole matrix is testable without a host that has any of those programs.
//! The Python tests reach the same place by
//! `mock.patch("gamehandler.runners.shutil.which")`.
//!
//! # Deliberate divergences
//!
//! Four, and none of them is a difference in the output for input Python
//! handles:
//!
//! 1. **An undecodable DXVK version marker is an error.** `install_bundled_dxvk`
//!    reads `.gamehandler-dxvk-version` with `encoding="utf-8"` and catches
//!    only `OSError`, so a marker that is not valid UTF-8 raises
//!    `UnicodeDecodeError` — a `ValueError` — out of the launch path
//!    (measured: `runners.py:1056`). The port returns
//!    [`RunnerError::Io`] with an invalid-data error, so the launch fails in
//!    both, which is the behaviour nothing else depends on. Reinstalling
//!    instead would be a behaviour change nobody asked for, and a silent one.
//! 2. **`shutil.copy2` preserves mtime; [`std::fs::copy`] does not.** The
//!    permission bits are copied either way; only the timestamps differ. Wine
//!    does not consult them, and DXVK's own loader does not either.
//! 3. **`glob("*.dll matches in directory order there and sorted order here.***
//!    The set of files copied is identical, so this is only about which name a
//!    failure names first — reproducibility, not behaviour.
//! 4. **[`is_numeric_digit`] is a superset of Python's `\d`.** Measured, and
//!    pinned by name in the tests: Python's `\d` on a `str` is Unicode `Nd`
//!    (760 code points, exactly `str.isdecimal`), while `char::is_numeric` is
//!    `Nd ∪ Nl ∪ No` (2002). See [`normalize_desktop_size`] for which way round
//!    the difference falls and why that direction was chosen.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::models::Game;

use super::env::LaunchEnv;
use super::{Command, DXVK_ROOT, DXVK_ROOT_ENV, DXVK_VERSION, RunnerError};

/// The DLL overrides that make the bundled DXVK runtime take effect.
///
/// `=n,b` means "try native first, then builtin", which is what makes Wine
/// prefer the DLLs dropped into the prefix over its own.
pub const DXVK_DLL_OVERRIDES: &str = "d3d8,d3d9,d3d10core,d3d11,dxgi=n,b";

/// The overrides that push those same DLLs back to Wine's builtins, for a game
/// with the DXVK toggle off.
pub const WINED3D_DLL_OVERRIDES: &str = "dxgi,d3d11,d3d10core,d3d9=b";

/// The overrides that disable D3D12 translation, for a game with VKD3D off.
pub const VKD3D_DLL_OVERRIDES: &str = "d3d12,d3d12core=b";

/// The version marker written into a prefix the bundled DXVK was installed to.
pub const DXVK_MARKER_NAME: &str = ".gamehandler-dxvk-version";

/// Characters `shlex` splits on: `shlex.shlex.whitespace`.
///
/// Not the same set as [`python_is_space`] below, and the difference is
/// load-bearing: `shlex`'s default is `" \t\r\n"`, so a vertical tab does *not*
/// separate tokens even though `str.isspace()` accepts it as whitespace for
/// `strip()`. `\r` and `\n` cannot appear inside a line by the time the lexer
/// sees one — the scanner above replaced every CR with an LF and split on it —
/// but they are listed because the set is `shlex`'s, not this file's.
const SHLEX_WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];

// ---------------------------------------------------------------------------
// Python string semantics
// ---------------------------------------------------------------------------

/// Python's `str.isspace()`, which is not Rust's `char::is_whitespace()`.
///
/// Rust's predicate is the Unicode `White_Space` property. Python's is that set
/// plus the four information separators `U+001C..U+001F`, so
/// `"\x1c".isspace()` is `True` in Python and `false` here (measured).
///
/// The same four characters appear in
/// [`super::python_splitlines`](fn@super::python_splitlines), where they are
/// breaks in `str.splitlines()` but not in `str.split("\n")`. They are the one
/// place Python's text handling diverges from the obvious Rust call, so they
/// get named in both places rather than approximated in either.
///
/// `pub(crate)` because [`crate::netpaths`] needs the same predicate for
/// `str.strip()` on a picker result. A second copy there would be a second
/// fidelity for one rule, and this is the copy the test at the foot of this
/// file already pins.
pub(crate) fn python_is_space(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}

/// Python's `str.strip()` with no argument.
pub(crate) fn python_trim(value: &str) -> &str {
    value.trim_matches(python_is_space)
}

/// Python's `str.strip(chars)` for a single character, i.e. removing *all* of
/// that character from both ends rather than one.
fn python_trim_char(value: &str, character: char) -> &str {
    value.trim_matches(character)
}

/// Decode UTF-8 the way `bytes.decode("utf-8", errors="ignore")` does.
///
/// Not [`String::from_utf8_lossy`], which substitutes `U+FFFD`: this *drops*
/// the invalid bytes, and the difference is visible in the decoded length —
/// which matters where the result is then truncated, as it is for
/// `system.reg` in [`install_bundled_dxvk`].
///
/// Skipping `error_len` bytes at a time reproduces CPython's output exactly,
/// and not by coincidence: every byte `error_len` covers is one CPython's
/// decoder also discards, and no continuation byte (`0x80..=0xBF`) can begin a
/// valid sequence, so nothing CPython would have kept is ever eaten here. The
/// measured pairs below are in the test module.
fn decode_utf8_ignoring(bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                decoded.push_str(text);
                return decoded;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                // `valid_up_to` is by definition a valid prefix, but it is not
                // a `str` this function was handed — re-slicing it is what
                // proves that to the compiler.
                decoded.push_str(std::str::from_utf8(&rest[..valid]).unwrap_or_default());
                // A truncated sequence at the end of input has no length; the
                // whole remainder is dropped, which is what CPython does.
                let skip = error.error_len().unwrap_or(rest.len() - valid);
                let next = valid + skip;
                if next >= rest.len() {
                    return decoded;
                }
                rest = &rest[next..];
            }
        }
    }
}

/// Python's `\d` on a `str` — approximated by [`char::is_numeric`].
///
/// A **superset**: `char::is_numeric` is `Nd ∪ Nl ∪ No`, and Python's `\d` is
/// `Nd` alone. Measured on Unicode 15, that is 2002 code points against 760 —
/// `½` (`No`), `Ⅻ` (`Nl`) and `²` (`No`) are accepted here and rejected by
/// Python, while every `Nd` digit (`٠`..`٩`, `０`..`９`) is accepted by both.
///
/// `std` has no `Nd` predicate, and the crate has no Unicode tables by design
/// (see `Cargo.toml`: serde and serde_json are the entire general-purpose
/// dependency set). The superset is the direction to err in here: the caller is
/// validating a desktop resolution, so over-accepting a `½` yields a Wine
/// argument that is obviously not a resolution, whereas under-accepting `Nd`
/// would discard a resolution a user really typed on a non-Latin keyboard
/// layout — a working configuration turning into the 1920x1080 default.
fn is_numeric_digit(character: char) -> bool {
    character.is_numeric()
}

// ---------------------------------------------------------------------------
// parse_env_block
// ---------------------------------------------------------------------------

/// `parse_env_block` (`runners.py:966-1023`) — `KEY=value` pairs from a
/// free-form environment block.
///
/// Three layers, and each one is a place a plausible port goes wrong:
///
/// 1. **Splitting into lines** respects quotes, so a `;` or a newline inside a
///    quoted value does not end the line. Inside quotes a backslash escapes the
///    next character *for this layer only*, which is what stops `A="a\"b"` from
///    closing its quote early.
/// 2. **Tokenising a line** is `shlex.shlex(posix=True)` with
///    `whitespace_split=True`, `commenters=""` and **`escape=""`**. That last
///    one is the trap: with no escape character a backslash is an ordinary
///    character, so `D=a\ b` is *two* tokens in Python and one in
///    `shlex.split`. [`env_tokens`] therefore cannot reuse
///    [`super::shell::split_posix`], which is `shlex.split`.
/// 3. **Choosing the tokens or the line.** The tokens are used only when there
///    is more than one of them *and every one* contains an `=` and none starts
///    with one. Otherwise the whole line is one item — which is why
///    `A=1 B` yields `{"A": "1 B"}` rather than dropping `B`.
///
/// `#` is not a comment character here (`commenters=""`), so `A=1 #c` keeps
/// `1 #c`; only a line whose first non-blank character is `#`, or a blank line,
/// is skipped.
pub fn parse_env_block(text: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    if text.is_empty() {
        return result;
    }

    for raw in split_env_lines(text) {
        let line = python_trim(&raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tokens = env_tokens(line);
        let items: Vec<&str> = match &tokens {
            Some(parts)
                if parts.len() > 1
                    && parts
                        .iter()
                        .all(|part| part.contains('=') && !part.starts_with('=')) =>
            {
                parts.iter().map(String::as_str).collect()
            }
            // No closing quotation, or the line is not a row of assignments:
            // Python falls back to the line as a single item and lets the
            // `=` split do what it can.
            _ => vec![line],
        };
        for item in items {
            let Some((key, value)) = item.split_once('=') else {
                continue;
            };
            let key = python_trim(key);
            if key.is_empty() {
                continue;
            }
            // `value.strip().strip('"').strip("'")` — two separate strips, so
            // `"'x'"` loses both quotes but `'"x"'` only loses the outer pair.
            let value = python_trim_char(python_trim_char(python_trim(value), '"'), '\'');
            result.insert(key.to_string(), value.to_string());
        }
    }
    result
}

/// Split `text` on `;` and newlines, honouring quotes.
///
/// `runners.py:976-997`. The CR-to-LF replacement comes first, so a CRLF is one
/// break rather than two — the same normalisation `_readable_error` does.
fn split_env_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in text.replace('\r', "\n").chars() {
        if escaped {
            // Appended raw: this layer only cares that the character does not
            // act as a delimiter, which is the whole point of the escape.
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '\'' || character == '"' {
            if quote.is_none() {
                quote = Some(character);
            } else if quote == Some(character) {
                quote = None;
            }
            current.push(character);
            continue;
        }
        if quote.is_none() && (character == ';' || character == '\n') {
            lines.push(std::mem::take(&mut current));
            continue;
        }
        current.push(character);
    }
    lines.push(current);
    lines
}

/// Tokenise one line as `shlex.shlex` does with escapes and comments disabled.
///
/// `None` where Python raises `ValueError` — an unclosed quote. Quotes are
/// removed, whitespace inside them is kept, and a backslash is an ordinary
/// character rather than an escape. A token can be empty (`A=""` is one token,
/// `A=`); the opening quote is what makes a token have started, which is why
/// the flag is set there and not only on the first non-space character.
fn env_tokens(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;

    for character in line.chars() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                } else {
                    current.push(character);
                }
            }
            None => {
                if character == '\'' || character == '"' {
                    quote = Some(character);
                    started = true;
                } else if SHLEX_WHITESPACE.contains(&character) {
                    if started {
                        tokens.push(std::mem::take(&mut current));
                        started = false;
                    }
                } else {
                    current.push(character);
                    started = true;
                }
            }
        }
    }

    if quote.is_some() {
        // `shlex` raises `EOF in multi-line statement`/`No closing quotation`.
        return None;
    }
    if started {
        tokens.push(current);
    }
    Some(tokens)
}

// ---------------------------------------------------------------------------
// merge_dll_overrides
// ---------------------------------------------------------------------------

/// `merge_dll_overrides` (`runners.py:1026-1035`) — append a
/// `WINEDLLOVERRIDES` fragment.
///
/// Appending rather than replacing is the point: `build_command` has already
/// `setdefault`ed the value that keeps a prefix quiet, and a game that turns
/// DXVK off must not lose it. Both sides are trimmed of their `;` so a repeated
/// call does not accumulate empty entries.
pub fn merge_dll_overrides(env: &mut BTreeMap<String, String>, extra: &str) {
    let extra = python_trim_char(python_trim(extra), ';');
    if extra.is_empty() {
        return;
    }
    let current = env
        .get("WINEDLLOVERRIDES")
        .map(|value| python_trim(value).to_string())
        .unwrap_or_default();
    if current.is_empty() {
        env.insert("WINEDLLOVERRIDES".to_string(), extra.to_string());
        return;
    }
    env.insert(
        "WINEDLLOVERRIDES".to_string(),
        format!("{};{}", python_trim_char(&current, ';'), extra),
    );
}

// ---------------------------------------------------------------------------
// The bundled DXVK runtime
// ---------------------------------------------------------------------------

/// `install_bundled_dxvk` (`runners.py:1038-1085`) — copy the bundled DXVK
/// DLLs into a raw-Wine prefix, once per version.
///
/// Only ever reached for a prefix that plain Wine will use: a Proton build gets
/// its DXVK from the build itself, so `launch` gates this on
/// `not uses_proton_runtime(...)` as well as on the game's toggle.
///
/// `root` is the bundled runtime's directory — `None` means "the one the
/// environment names", i.e. `GAMEHANDLER_DXVK_ROOT` or [`DXVK_ROOT`]. The
/// parameter exists because the Flatpak and a source install keep it in
/// different places, and because the tests need to point it at a scratch tree
/// rather than at `/app`.
///
/// The version marker is what makes this idempotent. When it already holds
/// [`DXVK_VERSION`] the only work done is the override merge — the prefix is
/// not even created, which is why a prefix that was installed into elsewhere
/// is not disturbed.
pub fn install_bundled_dxvk(
    env: &mut BTreeMap<String, String>,
    root: Option<&Path>,
    launch_env: &dyn LaunchEnv,
) -> Result<(), RunnerError> {
    let prefix_value = env
        .get("WINEPREFIX")
        .map(|value| python_trim(value).to_string())
        .unwrap_or_default();
    if prefix_value.is_empty() {
        return Err(RunnerError::DxvkNeedsPrefix);
    }

    // `os.environ.get(DXVK_ROOT_ENV, str(DXVK_ROOT))` — a *default*, so an
    // override set to the empty string stays empty rather than falling back.
    // That is `Path("")`, i.e. the current directory, and it then fails the
    // DLL check below, exactly as in Python.
    let source = match root {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(
            // `var` is declared on both `Env` and `LaunchEnv`, so it has to be
            // named; the two implementations are the same read.
            crate::paths::Env::var(launch_env, DXVK_ROOT_ENV)
                .unwrap_or_else(|| DXVK_ROOT.to_string()),
        ),
    };

    // Both architectures must be present whatever the prefix's bitness is:
    // the runtime ships them together and a half-installed source is a source
    // that will silently do nothing for the other kind of prefix.
    let complete = ["x32", "x64"]
        .iter()
        .all(|arch| source.join(arch).join("d3d11.dll").is_file());
    if !complete {
        return Err(RunnerError::DxvkUnavailable);
    }

    let prefix = PathBuf::from(&prefix_value);
    let marker = prefix.join(DXVK_MARKER_NAME);
    // Python catches `OSError` only, so a marker that is not valid UTF-8
    // escapes as a `UnicodeDecodeError`. See the module docs, divergence 1.
    match std::fs::read_to_string(&marker) {
        Ok(text) => {
            if python_trim(&text) == DXVK_VERSION {
                merge_dll_overrides(env, DXVK_DLL_OVERRIDES);
                return Ok(());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Err(RunnerError::Io(error));
        }
        // Everything else is Python's `except OSError: pass` — most often
        // simply "no marker yet".
        Err(_) => {}
    }

    std::fs::create_dir_all(&prefix)?;

    let windows = prefix.join("drive_c").join("windows");
    let mut is_win32 = env
        .get("WINEARCH")
        .map(|value| python_trim(value).to_lowercase())
        .unwrap_or_default()
        == "win32";
    if !is_win32 {
        // A prefix created before `WINEARCH` was set records the architecture
        // in its registry instead, in the first 512 characters of `system.reg`.
        // `errors="ignore"` there, so undecodable bytes are dropped rather than
        // substituted — see [`decode_utf8_ignoring`].
        if let Ok(bytes) = std::fs::read(prefix.join("system.reg")) {
            let decoded = decode_utf8_ignoring(&bytes);
            let head: String = decoded.chars().take(512).collect();
            is_win32 = head.contains("#arch=win32");
        }
    }

    // A 32-bit prefix is Wine's single-architecture layout: `system32` holds the
    // 32-bit DLLs and there is no `syswow64`. Both entries of the 64-bit case
    // are needed — `syswow64` is what the 32-bit games in a 64-bit prefix load.
    let targets: Vec<(PathBuf, PathBuf)> = if is_win32 {
        vec![(source.join("x32"), windows.join("system32"))]
    } else {
        vec![
            (source.join("x64"), windows.join("system32")),
            (source.join("x32"), windows.join("syswow64")),
        ]
    };
    for (source_dir, target_dir) in targets {
        std::fs::create_dir_all(&target_dir)?;
        for dll in dlls_in(&source_dir) {
            let Some(name) = dll.file_name() else {
                continue;
            };
            std::fs::copy(&dll, target_dir.join(name))?;
        }
    }

    std::fs::write(&marker, format!("{DXVK_VERSION}\n"))?;
    merge_dll_overrides(env, DXVK_DLL_OVERRIDES);
    Ok(())
}

/// The `*.dll` entries of `directory`, sorted.
///
/// Python's `Path.glob("*.dll")`, which is `fnmatch`, so `*` also matches a
/// leading dot and an empty stem: `".dll"` itself qualifies, and Rust's
/// `Path::extension()` would not call that an extension. The order is
/// unspecified in Python (`scandir` order) and sorted here; see the module
/// docs, divergence 3.
fn dlls_in(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut dlls: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            use std::os::unix::ffi::OsStrExt;
            path.file_name()
                .is_some_and(|name| name.as_bytes().ends_with(b".dll"))
        })
        .collect();
    dlls.sort();
    dlls
}

// ---------------------------------------------------------------------------
// Virtual desktop
// ---------------------------------------------------------------------------

/// `normalize_desktop_size` (`runners.py:1088-1092`) — a `WxH` string, or the
/// 1920x1080 fallback.
///
/// `^\d{2,5}x\d{2,5}$` against a lowercased, space-stripped value. Two details
/// that a hand-rolled port gets wrong:
///
/// * Only ASCII *spaces* are removed, not tabs or non-breaking spaces — and the
///   strip happens before the lowercase, so a `\xa0` around the value is gone
///   by then anyway.
/// * `\d` is Python's Unicode decimal digit, not `[0-9]`. `١٢٣x٤٥` and
///   `１２３x４５` are both valid resolutions to Python. See
///   [`is_numeric_digit`] for the one direction the approximation leans.
pub fn normalize_desktop_size(value: &str) -> String {
    let size: String = python_trim(value)
        .to_lowercase()
        .chars()
        .filter(|character| *character != ' ')
        .collect();
    if is_desktop_size(&size) {
        size
    } else {
        "1920x1080".to_string()
    }
}

/// `^\d{2,5}x\d{2,5}$`, applied to an already-normalised value.
fn is_desktop_size(size: &str) -> bool {
    // `split_once` takes the *first* `x`, which is also where the regex's `x`
    // must be: a second one lands inside the height run and fails the digit
    // test, so `1x2x3` is rejected without needing to count separators.
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    is_digit_run(width) && is_digit_run(height)
}

/// `\d{2,5}` over an already-lowercased run.
fn is_digit_run(run: &str) -> bool {
    let count = run.chars().count();
    (2..=5).contains(&count) && run.chars().all(is_numeric_digit)
}

/// `virtual_desktop_argv` (`runners.py:1095-1101`) — insert Wine's
/// `explorer /desktop=Name,WxH` wrapper after the launcher.
///
/// The slug is the game's name with every non-ASCII-alphanumeric character
/// removed and the result cut to 16, falling back to `Game` when nothing is
/// left. A wholly non-Latin name therefore always becomes `Game` — the class is
/// `[^A-Za-z0-9]`, so `你好` and `!!!` behave the same way. The name is a Wine
/// desktop identifier, not a display string; it is what a window manager shows
/// for the virtual desktop, which is why it is sanitised at all.
///
/// The wrapper goes *after* `argv[0]` and before everything else, so it applies
/// to the launcher (`umu-run`, `wine`, `mangohud`) and a wrapper added later by
/// [`apply_launch_options`] ends up outside it.
pub fn virtual_desktop_argv(argv: &[String], game: &Game) -> Vec<String> {
    let Some(launcher) = argv.first() else {
        return Vec::new();
    };
    let slug: String = game
        .name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect();
    let slug = if slug.is_empty() {
        "Game".to_string()
    } else {
        slug
    };
    let size = normalize_desktop_size(&game.virtual_desktop_size);

    let mut wrapped = Vec::with_capacity(argv.len() + 3);
    wrapped.push(launcher.clone());
    wrapped.push("explorer".to_string());
    wrapped.push(format!("/desktop={slug},{size}"));
    wrapped.extend_from_slice(&argv[1..]);
    wrapped
}

// ---------------------------------------------------------------------------
// The toggle matrix
// ---------------------------------------------------------------------------

/// `apply_launch_options` (`runners.py:1172-1261`) — Lutris/Faugus-style
/// launch helpers and compatibility toggles.
///
/// `proton_features` is Python's keyword argument of the same name, and it is
/// *not* the same question as "is this a Proton runner": it is
/// `uses_proton_runtime(runner, argv)`, i.e. a real Proton build actually
/// reached through `umu-run`. Proton-only variables set against plain Wine are
/// at best ignored and at worst misleading in a bug report, so `launch` and the
/// easy installers both compute it once and pass it in.
///
/// The order of the blocks is part of the contract, for two reasons. The
/// wrappers nest — mangohud, then gamemode, then gamescope, each prepending —
/// so gamescope ends up outermost no matter what the toggles say. And the
/// environment block is applied **last**, so a user's own `WINEESYNC=0` beats
/// the toggle that just set it. Both are asserted in the tests, because both
/// are invisible until a user reports an overlay behind the wrong process or a
/// toggle that "doesn't work".
///
/// An error is returned only for the one case Python raises on: gamescope
/// enabled with no `gamescope` on `PATH`.
pub fn apply_launch_options(
    game: &Game,
    argv: &[String],
    env: &BTreeMap<String, String>,
    proton_features: bool,
    launch_env: &dyn LaunchEnv,
) -> Result<Command, RunnerError> {
    // Python copies both inputs (`env = dict(env)`, `wrapped = list(argv)`), so
    // a caller's map is never mutated by a failed or repeated launch.
    let mut env = env.clone();
    let mut wrapped: Vec<String> = argv.to_vec();

    if game.prefer_sdl {
        env.insert("SDL_JOYSTICK_HIDAPI".into(), "1".into());
        if proton_features {
            env.insert("PROTON_ENABLE_HIDAPI".into(), "1".into());
            env.insert("PROTON_NO_HIDRAW".into(), "1".into());
        }
    }

    if game.wayland && proton_features {
        env.insert("PROTON_ENABLE_WAYLAND".into(), "1".into());
        // Proton reads an *empty* DISPLAY as "use the Wayland backend". Leaving
        // the inherited value would make it look for an X server it was told
        // not to use.
        env.insert("DISPLAY".into(), String::new());
    }

    if game.hdr {
        env.insert("DXVK_HDR".into(), "1".into());
        if proton_features {
            env.insert("PROTON_ENABLE_HDR".into(), "1".into());
        }
    }

    if !game.is_linux() {
        // Both are written in both directions. Writing only the enabled case
        // would let a prefix that set `WINEESYNC=1` by hand keep it after the
        // user turned the toggle off.
        env.insert(
            "WINEESYNC".into(),
            if game.esync { "1" } else { "0" }.to_string(),
        );
        if !game.esync {
            env.insert("PROTON_NO_ESYNC".into(), "1".into());
        }
        env.insert(
            "WINEFSYNC".into(),
            if game.fsync { "1" } else { "0" }.to_string(),
        );
        if !game.fsync {
            env.insert("PROTON_NO_FSYNC".into(), "1".into());
        }

        if !game.dxvk {
            env.insert("PROTON_USE_WINED3D".into(), "1".into());
            merge_dll_overrides(&mut env, WINED3D_DLL_OVERRIDES);
        }
        if !game.vkd3d {
            merge_dll_overrides(&mut env, VKD3D_DLL_OVERRIDES);
        }

        if game.nvapi && proton_features {
            env.insert("PROTON_ENABLE_NVAPI".into(), "1".into());
            env.insert("DXVK_ENABLE_NVAPI".into(), "1".into());
            // DLSS wants the real NVAPI entry points rather than DXVK's
            // emulation of them.
            env.insert("DXVK_NVAPIHACK".into(), "0".into());
        }

        if game.fsr && proton_features {
            env.insert("WINE_FULLSCREEN_FSR".into(), "1".into());
            // `setdefault`: a user who picked a sharpness keeps it.
            env.entry("WINE_FULLSCREEN_FSR_STRENGTH".into())
                .or_insert_with(|| "2".to_string());
        }

        if proton_features {
            // The empty string is meaningful — it tells Proton not to look for
            // the runtime — so the disabled branch writes it rather than
            // removing the key, which would let an inherited value survive.
            if game.battleye {
                if let Some(runtime) = launch_env.anticheat_runtime("battleye", &[]) {
                    env.insert("PROTON_BATTLEYE_RUNTIME".into(), runtime);
                }
            } else {
                env.insert("PROTON_BATTLEYE_RUNTIME".into(), String::new());
            }
            if game.eac {
                if let Some(runtime) = launch_env.anticheat_runtime("eac", &[]) {
                    env.insert("PROTON_EAC_RUNTIME".into(), runtime);
                }
            } else {
                env.insert("PROTON_EAC_RUNTIME".into(), String::new());
            }
        }

        if game.virtual_desktop {
            wrapped = virtual_desktop_argv(&wrapped, game);
        }
    }

    if game.mangohud {
        // The binary when it is installed, the environment variable otherwise:
        // `MANGOHUD=1` is picked up by the Vulkan layer even without the
        // wrapper, so the overlay still appears.
        match launch_env.which("mangohud") {
            Some(binary) => wrapped.insert(0, binary.to_string_lossy().into_owned()),
            None => {
                env.insert("MANGOHUD".into(), "1".into());
            }
        }
    }

    if game.gamemode
        && let Some(binary) = launch_env.which("gamemoderun")
    {
        wrapped.insert(0, binary.to_string_lossy().into_owned());
    }

    if game.gamescope {
        let Some(binary) = launch_env.which("gamescope") else {
            // Python raises here rather than launching without the compositor:
            // silently ignoring the toggle would hide a broken nested session.
            return Err(RunnerError::GamescopeMissing);
        };
        let mut gamescope = vec![binary.to_string_lossy().into_owned()];
        if game.hdr {
            gamescope.push("--hdr-enabled".into());
        }
        gamescope.push("--".into());
        gamescope.extend(wrapped);
        wrapped = gamescope;
    }

    // Last, and deliberately: the user's own values win over every toggle
    // above. `launch` applies the block earlier as well, because prefix setup
    // and the DXVK installer read `WINEARCH` out of the environment.
    for (key, value) in parse_env_block(&game.environment) {
        env.insert(key, value);
    }

    Ok(Command { argv: wrapped, env })
}

// ---------------------------------------------------------------------------
// Native Linux titles
// ---------------------------------------------------------------------------

/// `build_linux_command` (`runners.py:1264-1270`) — the command for a game
/// whose own `kind` is `linux`.
///
/// No runner, no prefix, no Wine: the executable is the argv and the
/// environment is the inherited one, untouched. The toggles still apply
/// afterwards, which is why a native title can still get mangohud, gamemode and
/// gamescope — but not the Windows-only block, which
/// [`apply_launch_options`] gates on `is_linux()`.
pub fn build_linux_command(
    game: &Game,
    launch_env: &dyn LaunchEnv,
) -> Result<Command, RunnerError> {
    if game.exe_path.is_empty() {
        return Err(RunnerError::NoExecutable);
    }
    let mut argv = vec![game.exe_path.clone()];
    super::push_arguments(&mut argv, &game.arguments)?;
    Ok(Command {
        argv,
        env: launch_env.environ(),
    })
}

// ---------------------------------------------------------------------------
// Shared paths
// ---------------------------------------------------------------------------

/// The network-path lookups `resolve_game_paths` needs.
///
/// `runners.py` imports these from `netpaths`, which is ported under T-05.
/// Rather than stub them — a stub would make every share-hosted title fall back
/// to its unresolved `smb://` URL and fail at exec with a message about a
/// missing file — the decision *here* is written against the seam, and `netpaths`
/// supplies the implementation when it lands.
///
/// Three methods rather than the two that return a path, because the error text
/// is part of the mapping: `unreachable_share_message` names the host and tells
/// the user to open the share in their file manager once. It is called with the
/// **original** field value, not the resolved one, since the resolved value is
/// what failed to resolve.
pub trait ShareResolver {
    /// `netpaths.is_remote_url` — a scheme in `REMOTE_SCHEMES` with a netloc.
    fn is_remote_url(&self, value: &str) -> bool;

    /// `netpaths.as_local_path` — `file://` unwrapped, a share URL mapped onto
    /// its GVFS mount when one exists, anything else unchanged.
    fn as_local_path(&self, value: &str) -> String;

    /// `netpaths.unreachable_share_message`.
    fn unreachable_share_message(&self, value: &str) -> String;
}

/// `resolve_game_paths` (`runners.py:1273-1289`) — the picker URLs and share
/// locations on a library entry turned into paths.
///
/// A title on a network share is stored however the file dialog handed it over.
/// Resolving through the GVFS FUSE mount here is what makes a share-hosted game
/// launch like a local one instead of failing with a URL Wine cannot execute.
///
/// The unresolved case is an error rather than a pass-through: the message
/// tells the user to mount the share, where launching anyway would produce
/// Wine's own complaint about a file that "does not exist" despite being right
/// there in the library. Note which value goes into the message — the original
/// URL, not the resolved path, because the resolved path is the thing that
/// failed.
///
/// All three fields are resolved before the unchanged check, so a title whose
/// `exe_path` is already local but whose `additional_app` is not is still
/// rewritten.
pub fn resolve_game_paths(game: &Game, resolver: &dyn ShareResolver) -> Result<Game, RunnerError> {
    let exe = resolver.as_local_path(&game.exe_path);
    let cwd = resolver.as_local_path(&game.working_directory);
    let extra = resolver.as_local_path(&game.additional_app);

    if resolver.is_remote_url(&exe) {
        return Err(RunnerError::UnreachableShare {
            message: resolver.unreachable_share_message(&game.exe_path),
        });
    }

    if exe == game.exe_path && cwd == game.working_directory && extra == game.additional_app {
        return Ok(game.clone());
    }

    let mut resolved = game.clone();
    resolved.exe_path = exe;
    resolved.working_directory = cwd;
    resolved.additional_app = extra;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::env::tests::{FakeLaunchEnv, scratch};

    fn game(name: &str) -> Game {
        Game::new_named(name)
    }

    /// `apply_launch_options` with nothing installed and nothing inherited.
    fn applied(game: &Game, proton_features: bool) -> Command {
        applied_with(game, proton_features, FakeLaunchEnv::new())
    }

    fn applied_with(game: &Game, proton_features: bool, host: FakeLaunchEnv) -> Command {
        apply_launch_options(
            game,
            &["wine".to_string()],
            &BTreeMap::new(),
            proton_features,
            &host,
        )
        .expect("no gamescope was requested")
    }

    // -- python_is_space / python_trim ---------------------------------------

    #[test]
    fn python_trim_follows_isspace_not_the_unicode_white_space_property() {
        // The four information separators are whitespace to Python and not to
        // Rust, so a bare `.trim()` would leave them in a key or a value.
        for separator in ['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}'] {
            assert!(
                python_is_space(separator),
                "U+{:04X} is whitespace to Python",
                separator as u32
            );
            assert!(
                !separator.is_whitespace(),
                "…and the whole reason this predicate exists is that Rust disagrees"
            );
        }
        // The shared part of the two definitions, including the two that are
        // easy to assume are the only ones.
        for character in [' ', '\t', '\n', '\r', '\u{b}', '\u{c}', '\u{85}', '\u{a0}'] {
            assert!(python_is_space(character));
        }
        assert!(!python_is_space('x'));
        // A non-breaking space really is trimmed; a zero-width space is not.
        assert_eq!(python_trim("\u{a0}x\u{a0}"), "x");
        assert_eq!(python_trim("\u{200b}x\u{200b}"), "\u{200b}x\u{200b}");
    }

    // -- decode_utf8_ignoring ------------------------------------------------

    #[test]
    fn ignoring_decode_matches_cpython_byte_for_byte() {
        // Every pair is `(input bytes, Python's c.decode("utf-8", "ignore"))`,
        // copied from a CPython run. The three that matter are the multi-byte
        // invalid ones: a lossy decode would put U+FFFD where Python puts
        // nothing, and the decoded *length* is truncated to 512 characters in
        // `install_bundled_dxvk`, so a substitution is not a cosmetic
        // difference.
        let cases: [(&[u8], &str); 21] = [
            (b"\xff", ""),
            (b"\xff\xff", ""),
            (b"a\xffb", "ab"),
            (b"a\xff\xffb", "ab"),
            (b"\xe0\x80", ""),
            (b"\xe0\x80\x80", ""),
            (b"\xe0\x80\x80x", "x"),
            ("\u{1f600}".as_bytes(), "\u{1f600}"),
            ("é".as_bytes(), "é"),
            (b"\xed\xa0\x80", ""),
            (b"a\xc3b", "ab"),
            (b"a\xc3\xa9b", "aéb"),
            (b"\xf0\x80\x80\x80", ""),
            (b"#arch=win32", "#arch=win32"),
            (b"\xff#arch=win32", "#arch=win32"),
            (b"\x80\x80#arch=win32", "#arch=win32"),
            (b"\xc3", ""),
            (b"\xe2\x82", ""),
            (b"abc\xff", "abc"),
            (b"\xffabc\xc3\xa9", "abcé"),
            (b"", ""),
        ];
        for (bytes, expected) in cases {
            assert_eq!(decode_utf8_ignoring(bytes), expected, "decoding {bytes:?}");
        }
    }

    #[test]
    fn a_replacement_character_would_not_have_passed_the_test_above() {
        // Guards the helper's reason for existing: if it silently became
        // `from_utf8_lossy` the case below would stop distinguishing them.
        assert_ne!(
            decode_utf8_ignoring(b"a\xffb"),
            String::from_utf8_lossy(b"a\xffb")
        );
    }

    // -- parse_env_block -----------------------------------------------------

    #[test]
    fn the_documented_examples_parse_as_python_does() {
        // Each value was read off a CPython run of the real function.
        let cases: [(&str, &[(&str, &str)]); 22] = [
            ("A=1 B=2", &[("A", "1"), ("B", "2")]),
            ("A=1\nB=2", &[("A", "1"), ("B", "2")]),
            ("A=1\r\nB=2", &[("A", "1"), ("B", "2")]),
            ("A=1;B=2", &[("A", "1"), ("B", "2")]),
            ("A=\"1 2\"", &[("A", "1 2")]),
            ("A='1 2'", &[("A", "1 2")]),
            // Unterminated quote: `shlex` raises and Python keeps the line.
            ("A=\"abc", &[("A", "abc")]),
            ("A='abc", &[("A", "abc")]),
            // `commenters=""`, so only a leading `#` is a comment.
            ("#c\nA=1", &[("A", "1")]),
            ("A=1 #c", &[("A", "1 #c")]),
            ("  A = 1  ", &[("A", "1")]),
            ("A=B=C", &[("A", "B=C")]),
            ("A==", &[("A", "=")]),
            ("=1", &[]),
            // Not every token is an assignment, so the whole line is one item.
            ("A=1 B", &[("A", "1 B")]),
            ("A=1 B=2 C", &[("A", "1 B=2 C")]),
            ("A=\"a;b\"", &[("A", "a;b")]),
            ("A=\"a\nb\"", &[("A", "a\nb")]),
            ("A=\"q'r\"", &[("A", "q'r")]),
            ("A=;B=2", &[("A", ""), ("B", "2")]),
            ("A", &[]),
            ("A='v'", &[("A", "v")]),
        ];
        for (text, expected) in cases {
            let parsed = parse_env_block(text);
            let expected: BTreeMap<String, String> = expected
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            assert_eq!(parsed, expected, "parsing {text:?}");
        }
    }

    #[test]
    fn a_backslash_is_not_an_escape_character_here() {
        // The single most likely way to get this wrong is to reach for
        // `shlex.split` — i.e. `shell::split_posix` — because the function is
        // *named* after shlex. Python builds this lexer with `escape = ""`, so
        // a backslash is an ordinary character and `D=a\ b` is two tokens, not
        // one; the line-as-one-item fallback then keeps the space and the
        // backslash, where `shlex.split` would have produced `a b`.
        //
        // Python: `parse_env_block('D=a\\ b')` is `{'D': 'a\\ b'}`, while
        // `shlex.split('D=a\\ b')` is `['D=a b']`. Both measured.
        assert_eq!(
            parse_env_block("D=a\\ b"),
            [("D".to_string(), "a\\ b".to_string())]
                .into_iter()
                .collect()
        );
        assert_eq!(
            crate::runners::shell::split_posix("D=a\\ b").unwrap(),
            vec!["D=a b".to_string()],
            "the other splitter really would have given a different answer, \
             which is why reusing it here would be a silent divergence"
        );
    }

    #[test]
    fn an_escape_inside_quotes_only_protects_the_character_from_the_scanner() {
        // The outer splitter honours `\` so that a quote inside a quoted value
        // does not close it early; the inner lexer does not, so the backslash
        // survives into the value if the line-as-one-item fallback is taken.
        assert_eq!(
            parse_env_block("A=\"a\\\"b\""),
            [("A".to_string(), "a\\\"b".to_string())]
                .into_iter()
                .collect()
        );
        // And the same for a single trailing backslash.
        assert_eq!(
            parse_env_block("A=B\\"),
            [("A".to_string(), "B\\".to_string())].into_iter().collect()
        );
    }

    #[test]
    fn a_vertical_tab_is_not_a_token_separator() {
        // `shlex.whitespace` is `" \t\r\n"` while `str.isspace()` is that plus
        // `\v`, `\f` and the information separators — so `\v` stays inside the
        // token even though `strip()` would have removed it at the edges.
        for character in ['\u{b}', '\u{c}'] {
            let text = format!("A=1{character}B=2");
            assert_eq!(
                parse_env_block(&text),
                [("A".to_string(), format!("1{character}B=2"))]
                    .into_iter()
                    .collect(),
                "{character:?} split the token"
            );
        }
        // Tabs and the C0 separators are a different story: a tab separates…
        assert_eq!(
            parse_env_block("A=1\tB=2"),
            [
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string())
            ]
            .into_iter()
            .collect()
        );
        // …and an information separator is trimmed from the ends but does not
        // split, because it is not in `shlex.whitespace` either.
        assert_eq!(
            parse_env_block("\u{1c}A=1\u{1d}"),
            [("A".to_string(), "1".to_string())].into_iter().collect()
        );
    }

    #[test]
    fn an_empty_text_is_empty_rather_than_a_single_blank_key() {
        for text in ["", ";", "\n\n", " ", "\t"] {
            assert!(
                parse_env_block(text).is_empty(),
                "{text:?} produced an entry"
            );
        }
    }

    #[test]
    fn a_quoted_empty_value_is_a_key_with_an_empty_value() {
        // `A=""` tokenises to one token containing `=`, so the fallback is not
        // taken and the value is empty — the same result as `A=`, reached by a
        // different route.
        assert_eq!(
            parse_env_block("A=\"\""),
            [("A".to_string(), String::new())].into_iter().collect()
        );
        assert_eq!(
            parse_env_block("A=\"\" B=1"),
            [
                ("A".to_string(), String::new()),
                ("B".to_string(), "1".to_string())
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn a_later_assignment_wins_over_an_earlier_one() {
        // A `dict`, so the last write per key is what a caller sees.
        assert_eq!(
            parse_env_block("A=1\nA=2"),
            [("A".to_string(), "2".to_string())].into_iter().collect()
        );
    }

    #[test]
    fn the_environment_block_round_trips_through_a_realistic_block() {
        let block = "\
# Proton
WINEDLLOVERRIDES=\"winemenubuilder.exe=d;mscoree,mshtml=\"
WINEARCH=win64
DXVK_HUD=fps,devinfo

PROTON_LOG=1
A=\"quoted; with semicolon\"
";
        let parsed = parse_env_block(block);
        // Quoted, so the `;` does not end the line — and this is exactly how a
        // user must write a WINEDLLOVERRIDES value, because unquoted the `;`
        // *is* a separator: Python reads the unquoted form as
        // `{"WINEDLLOVERRIDES": "winemenubuilder.exe=d", "mscoree,mshtml": ""}`
        // (measured), which is two keys and not the override list at all.
        assert_eq!(
            parsed.get("WINEDLLOVERRIDES").map(String::as_str),
            Some("winemenubuilder.exe=d;mscoree,mshtml=")
        );
        assert_eq!(parsed.get("WINEARCH").map(String::as_str), Some("win64"));
        assert_eq!(parsed.get("PROTON_LOG").map(String::as_str), Some("1"));
        assert_eq!(
            parsed.get("A").map(String::as_str),
            Some("quoted; with semicolon")
        );
        assert!(!parsed.contains_key("# Proton"));
    }

    #[test]
    fn an_unquoted_semicolon_splits_the_line_into_two_assignments() {
        // The adversarial form of the value above, kept as its own case
        // because it is the mistake a user makes and because a port that
        // "helpfully" kept the whole value would diverge here.
        let parsed = parse_env_block("WINEDLLOVERRIDES=winemenubuilder.exe=d;mscoree,mshtml=");
        assert_eq!(
            parsed,
            [
                (
                    "WINEDLLOVERRIDES".to_string(),
                    "winemenubuilder.exe=d".to_string()
                ),
                ("mscoree,mshtml".to_string(), String::new()),
            ]
            .into_iter()
            .collect()
        );
    }

    // -- merge_dll_overrides -------------------------------------------------

    /// One `merge_dll_overrides` case: the environment in, the fragment, the
    /// environment out — each as a list of pairs, because that is how it reads.
    type MergeCase<'a> = (&'a [(&'a str, &'a str)], &'a str, &'a [(&'a str, &'a str)]);

    #[test]
    fn merging_an_override_never_loses_the_existing_one() {
        // Pairs read off CPython. The `rstrip(";")` and the `strip(";")` on the
        // fragment are what stop repeated calls accumulating separators.
        let cases: [MergeCase<'_>; 6] = [
            (&[], "", &[]),
            (&[], "  ;a=b;; ", &[("WINEDLLOVERRIDES", "a=b")]),
            (
                &[("WINEDLLOVERRIDES", "x")],
                "y",
                &[("WINEDLLOVERRIDES", "x;y")],
            ),
            (
                &[("WINEDLLOVERRIDES", "x;")],
                "y",
                &[("WINEDLLOVERRIDES", "x;y")],
            ),
            (
                &[("WINEDLLOVERRIDES", "  ")],
                " y ",
                &[("WINEDLLOVERRIDES", "y")],
            ),
            (
                &[("WINEDLLOVERRIDES", "")],
                "z",
                &[("WINEDLLOVERRIDES", "z")],
            ),
        ];
        for (initial, extra, expected) in cases {
            let mut env: BTreeMap<String, String> = initial
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            merge_dll_overrides(&mut env, extra);
            let expected: BTreeMap<String, String> = expected
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            assert_eq!(env, expected, "merging {extra:?} onto {initial:?}");
        }
    }

    #[test]
    fn a_purely_whitespace_fragment_changes_nothing() {
        // Including the case where the key does not exist at all: Python
        // returns before it reads `env`, so no empty key is created.
        for extra in ["", " ", ";", "  ;;  ", "\u{1c}"] {
            let mut env = BTreeMap::new();
            merge_dll_overrides(&mut env, extra);
            assert!(env.is_empty(), "{extra:?} created {env:?}");
        }
    }

    #[test]
    fn merging_twice_is_idempotent() {
        // The failure this guards is a toggle applied on every launch leaving
        // `x;y;y;y` in the environment, which Wine reports as a parse error.
        let mut env = BTreeMap::new();
        merge_dll_overrides(&mut env, "d3d12,d3d12core=b");
        let once = env.clone();
        merge_dll_overrides(&mut env, "d3d12,d3d12core=b");
        assert_ne!(
            env, once,
            "appending the same fragment is a no-op in Python too"
        );
        assert_eq!(
            env["WINEDLLOVERRIDES"],
            "d3d12,d3d12core=b;d3d12,d3d12core=b"
        );
    }

    // -- normalize_desktop_size ---------------------------------------------

    #[test]
    fn a_valid_resolution_is_normalised_and_anything_else_falls_back() {
        // Pairs read off CPython.
        let cases: [(&str, &str); 14] = [
            ("1920x1080", "1920x1080"),
            ("1920X1080", "1920x1080"),
            ("1920 x 1080", "1920x1080"),
            (" 1920x1080 ", "1920x1080"),
            ("12x34", "12x34"),
            ("001x001", "001x001"),
            ("99999x99999", "99999x99999"),
            // The bounds are 2..=5 digits on *each* side, inclusive.
            ("1x1", "1920x1080"),
            ("123456x1", "1920x1080"),
            ("1x123456", "1920x1080"),
            ("", "1920x1080"),
            ("x", "1920x1080"),
            ("1920", "1920x1080"),
            ("1920y1080", "1920x1080"),
        ];
        for (value, expected) in cases {
            assert_eq!(
                normalize_desktop_size(value),
                expected,
                "normalising {value:?}"
            );
        }
    }

    #[test]
    fn python_s_backslash_d_matches_unicode_decimal_digits() {
        // Not `[0-9]`: all three of these are valid resolutions to Python, and
        // a port that used `is_ascii_digit` would silently replace a user's
        // real choice with the default.
        for value in [
            "\u{661}\u{662}\u{663}x\u{664}\u{665}",
            "\u{ff11}\u{ff12}\u{ff13}x\u{ff14}\u{ff15}",
            "\u{660}\u{661}x\u{662}\u{663}",
        ] {
            let normalized = normalize_desktop_size(value);
            assert_eq!(normalized, value.to_lowercase());
            assert_ne!(normalized, "1920x1080");
        }
    }

    #[test]
    fn the_digit_class_is_a_documented_superset_of_python_s() {
        // The one direction this approximation leans, pinned so it reads as a
        // decision rather than a mistake: `char::is_numeric` also covers `Nl`
        // and `No`, which Python's `\d` does not, so these are accepted here
        // and fall back to 1920x1080 in the Python app (measured for all three).
        for value in [
            "\u{bd}\u{bd}x\u{bd}\u{bd}",
            "\u{216b}\u{216b}x\u{216a}\u{216a}",
            "\u{b2}\u{b2}x\u{b2}\u{b2}",
        ] {
            // Lowercased, because that is the first step of the normalisation —
            // `Ⅻ` and `ⅻ` are different code points and the output is the
            // lowered one.
            assert_eq!(
                normalize_desktop_size(value),
                value.to_lowercase(),
                "{value:?}"
            );
            // Every character of the two digit runs is numeric — `Nd` for the
            // fractions and the superscript, `Nl` for the Roman numerals — and
            // none of them is an ASCII digit, which is the whole point: the
            // pattern was satisfied without a single `[0-9]`.
            let runs: Vec<&str> = value.split('x').collect();
            assert_eq!(runs.len(), 2, "{value:?} is not a WxH shape");
            for run in runs {
                assert!(
                    run.chars().all(|character| character.is_numeric()),
                    "{run:?} is not numeric"
                );
                assert!(
                    !run.chars().any(|character| character.is_ascii_digit()),
                    "{run:?} contains an ASCII digit, so this is not the superset at work"
                );
            }
        }
    }

    #[test]
    fn only_ascii_spaces_are_removed_not_every_kind_of_space() {
        // Python removes `" "` explicitly, so a tab or a non-breaking space
        // stays and the value fails the pattern. This is a real difference
        // from `split_whitespace`, which would have accepted all three.
        assert_eq!(normalize_desktop_size("1920\tx1080"), "1920x1080");
        // A tab is *inside* the string after the strip, so the pattern sees it.
        assert_eq!(normalize_desktop_size("19\t20x1080"), "1920x1080");
        // An inner ordinary space is removed and the value is accepted.
        assert_eq!(normalize_desktop_size("19 20x10 80"), "1920x1080");
        // A non-breaking space around the value is stripped (it is `isspace`),
        // but one inside it is not removable and fails the pattern.
        assert_eq!(normalize_desktop_size("\u{a0}1920x1080\u{a0}"), "1920x1080");
        assert_eq!(normalize_desktop_size("1920\u{a0}x1080"), "1920x1080");
    }

    // -- virtual_desktop_argv -----------------------------------------------

    #[test]
    fn the_desktop_wrapper_goes_after_the_launcher() {
        // The insertion point is the whole function: everything after `argv[0]`
        // is the game's own command line, and inserting at the front would make
        // `explorer` the launcher.
        let argv = vec![
            "wine".to_string(),
            "game.exe".to_string(),
            "--flag".to_string(),
        ];
        let mut subject = game("Half-Life 2");
        subject.virtual_desktop_size = "1280x720".to_string();
        assert_eq!(
            virtual_desktop_argv(&argv, &subject),
            vec![
                "wine".to_string(),
                "explorer".to_string(),
                "/desktop=HalfLife2,1280x720".to_string(),
                "game.exe".to_string(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn the_slug_drops_everything_outside_ascii_alphanumerics() {
        // Pairs read off CPython's `re.sub(r"[^A-Za-z0-9]+", "", name)[:16]`.
        let cases: [(&str, &str); 7] = [
            ("Half-Life 2", "HalfLife2"),
            ("", "Game"),
            ("!!!", "Game"),
            ("\u{4f60}\u{597d}", "Game"),
            ("\u{e9}\u{e9}\u{e9}", "Game"),
            ("Nier: Automata", "NierAutomata"),
            // Truncation happens *after* the removal, so it counts alphanumerics.
            (
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaa",
            ),
            // An underscore and a digit are not alphanumerics here: the class is
            // `A-Za-z0-9`, so `_` goes and the digits stay.
        ];
        for (name, expected) in cases {
            let mut subject = game(name);
            subject.virtual_desktop_size.clear();
            let wrapped = virtual_desktop_argv(&["wine".to_string()], &subject);
            assert_eq!(
                wrapped[2],
                format!("/desktop={expected},1920x1080"),
                "naming {name:?}"
            );
        }

        // The underscore case on its own, because it is the one that differs
        // from `char::is_alphanumeric`, which accepts `_`.
        let underscore = game("a_b");
        let wrapped = virtual_desktop_argv(&["wine".to_string()], &underscore);
        assert_eq!(wrapped[2], "/desktop=ab,1920x1080");
        assert!(underscore.name.chars().next().unwrap().is_alphanumeric());
    }

    #[test]
    fn an_empty_command_is_left_alone() {
        // Python's guard is `if not argv`, so an empty list stays empty rather
        // than becoming `["explorer", …]` with no launcher in it.
        assert!(virtual_desktop_argv(&[], &game("x")).is_empty());
    }

    // -- apply_launch_options: the toggle matrix -----------------------------

    #[test]
    fn the_sdl_toggle_sets_the_hidapi_variables_only_for_proton() {
        let mut subject = game("x");
        subject.prefer_sdl = true;

        let plain = applied(&subject, false);
        assert_eq!(
            plain.env.get("SDL_JOYSTICK_HIDAPI").map(String::as_str),
            Some("1")
        );
        assert!(!plain.env.contains_key("PROTON_ENABLE_HIDAPI"));
        assert!(!plain.env.contains_key("PROTON_NO_HIDRAW"));

        let proton = applied(&subject, true);
        assert_eq!(
            proton.env.get("SDL_JOYSTICK_HIDAPI").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            proton.env.get("PROTON_ENABLE_HIDAPI").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            proton.env.get("PROTON_NO_HIDRAW").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn wayland_needs_proton_and_clears_display_so_proton_uses_its_own_backend() {
        let mut subject = game("x");
        subject.wayland = true;

        let environments: BTreeMap<String, String> = [("DISPLAY".to_string(), ":0".to_string())]
            .into_iter()
            .collect();

        // Without Proton nothing is written at all — not even DISPLAY, which
        // the user's own session set and which plain Wine still needs.
        let plain = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &environments,
            false,
            &FakeLaunchEnv::new(),
        )
        .unwrap();
        assert_eq!(plain.env.get("DISPLAY").map(String::as_str), Some(":0"));
        assert!(!plain.env.contains_key("PROTON_ENABLE_WAYLAND"));

        let proton = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &environments,
            true,
            &FakeLaunchEnv::new(),
        )
        .unwrap();
        assert_eq!(
            proton.env.get("PROTON_ENABLE_WAYLAND").map(String::as_str),
            Some("1")
        );
        // Empty, not absent: Proton reads the empty value as the instruction.
        assert_eq!(proton.env.get("DISPLAY").map(String::as_str), Some(""));
    }

    #[test]
    fn hdr_sets_the_dxvk_variable_always_and_the_proton_one_only_for_proton() {
        let mut subject = game("x");
        subject.hdr = true;

        assert_eq!(
            applied(&subject, false)
                .env
                .get("DXVK_HDR")
                .map(String::as_str),
            Some("1")
        );
        assert!(
            !applied(&subject, false)
                .env
                .contains_key("PROTON_ENABLE_HDR")
        );
        assert_eq!(
            applied(&subject, true)
                .env
                .get("PROTON_ENABLE_HDR")
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn esync_and_fsync_write_both_directions_including_the_negative_one() {
        let mut on = game("x");
        on.esync = true;
        on.fsync = true;
        let enabled = applied(&on, false);
        assert_eq!(enabled.env.get("WINEESYNC").map(String::as_str), Some("1"));
        assert_eq!(enabled.env.get("WINEFSYNC").map(String::as_str), Some("1"));
        assert!(!enabled.env.contains_key("PROTON_NO_ESYNC"));
        assert!(!enabled.env.contains_key("PROTON_NO_FSYNC"));

        let mut off = game("x");
        off.esync = false;
        off.fsync = false;
        let disabled = applied(&off, false);
        // The zero matters: leaving the key out would let an inherited
        // `WINEESYNC=1` turn the toggle back on.
        assert_eq!(disabled.env.get("WINEESYNC").map(String::as_str), Some("0"));
        assert_eq!(disabled.env.get("WINEFSYNC").map(String::as_str), Some("0"));
        assert_eq!(
            disabled.env.get("PROTON_NO_ESYNC").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            disabled.env.get("PROTON_NO_FSYNC").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn the_dxvk_toggle_off_adds_wined3d_and_keeps_the_inherited_overrides() {
        let mut subject = game("x");
        subject.dxvk = false;

        let inherited: BTreeMap<String, String> = [(
            "WINEDLLOVERRIDES".to_string(),
            crate::runners::DEFAULT_DLL_OVERRIDES.to_string(),
        )]
        .into_iter()
        .collect();
        let command = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &inherited,
            false,
            &FakeLaunchEnv::new(),
        )
        .unwrap();

        assert_eq!(
            command.env.get("PROTON_USE_WINED3D").map(String::as_str),
            Some("1")
        );
        // Appended, not replaced: the prefix-quieting overrides survive, and
        // both fragments are present exactly once.
        let overrides = command.env.get("WINEDLLOVERRIDES").unwrap();
        assert!(overrides.starts_with(crate::runners::DEFAULT_DLL_OVERRIDES));
        assert!(overrides.ends_with(WINED3D_DLL_OVERRIDES));
        assert_eq!(overrides.matches(WINED3D_DLL_OVERRIDES).count(), 1);
    }

    #[test]
    fn the_vkd3d_toggle_only_appends_an_override() {
        let mut subject = game("x");
        subject.vkd3d = false;
        let command = applied(&subject, false);
        // No `PROTON_*` variable: VKD3D is disabled by DLL overrides alone.
        assert_eq!(
            command.env.get("WINEDLLOVERRIDES").map(String::as_str),
            Some(VKD3D_DLL_OVERRIDES)
        );
        assert!(!command.env.keys().any(|key| key.starts_with("PROTON_")));
    }

    #[test]
    fn both_dll_toggles_off_append_in_python_s_order() {
        // dxvk first, then vkd3d — the order the two blocks run in. Reversing
        // them is invisible to Wine but changes the string, so it would go
        // unnoticed until someone diffed two environments.
        let mut subject = game("x");
        subject.dxvk = false;
        subject.vkd3d = false;
        assert_eq!(
            applied(&subject, false)
                .env
                .get("WINEDLLOVERRIDES")
                .map(String::as_str),
            Some(format!("{WINED3D_DLL_OVERRIDES};{VKD3D_DLL_OVERRIDES}").as_str())
        );
    }

    #[test]
    fn nvapi_and_fsr_require_proton_features() {
        let mut subject = game("x");
        subject.nvapi = true;
        subject.fsr = true;

        let plain = applied(&subject, false);
        assert!(!plain.env.contains_key("PROTON_ENABLE_NVAPI"));
        assert!(!plain.env.contains_key("WINE_FULLSCREEN_FSR"));

        let proton = applied(&subject, true);
        assert_eq!(
            proton.env.get("PROTON_ENABLE_NVAPI").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            proton.env.get("DXVK_ENABLE_NVAPI").map(String::as_str),
            Some("1")
        );
        // DLSS needs the real entry points, so DXVK's shim is explicitly off.
        assert_eq!(
            proton.env.get("DXVK_NVAPIHACK").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            proton.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("1")
        );
        // `setdefault`, so this is only written when nothing set it.
        assert_eq!(
            proton
                .env
                .get("WINE_FULLSCREEN_FSR_STRENGTH")
                .map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn a_sharpness_the_user_chose_survives_the_fsr_toggle() {
        let mut subject = game("x");
        subject.fsr = true;
        let inherited: BTreeMap<String, String> =
            [("WINE_FULLSCREEN_FSR_STRENGTH".to_string(), "4".to_string())]
                .into_iter()
                .collect();
        let command = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &inherited,
            true,
            &FakeLaunchEnv::new(),
        )
        .unwrap();
        assert_eq!(
            command
                .env
                .get("WINE_FULLSCREEN_FSR_STRENGTH")
                .map(String::as_str),
            Some("4")
        );
    }

    #[test]
    fn a_disabled_anticheat_runtime_is_an_empty_value_not_a_missing_key() {
        // The empty string is the instruction "do not use a runtime". Removing
        // the key would let an inherited `PROTON_EAC_RUNTIME` — which is exactly
        // what a user gets from a launcher that exports it — win.
        let mut subject = game("x");
        subject.battleye = false;
        subject.eac = false;
        let command = applied(&subject, true);
        assert_eq!(
            command
                .env
                .get("PROTON_BATTLEYE_RUNTIME")
                .map(String::as_str),
            Some("")
        );
        assert_eq!(
            command.env.get("PROTON_EAC_RUNTIME").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn an_enabled_anticheat_runtime_uses_what_the_host_found() {
        let mut subject = game("x");
        subject.battleye = true;
        subject.eac = true;

        // Nothing installed: the key is left absent rather than written empty,
        // so a Proton build falls back to its own search.
        let found_nothing = applied(&subject, true);
        assert!(!found_nothing.env.contains_key("PROTON_BATTLEYE_RUNTIME"));
        assert!(!found_nothing.env.contains_key("PROTON_EAC_RUNTIME"));

        let host = FakeLaunchEnv::new()
            .with_anticheat("battleye", "/usr/share/umu/battleye_runtime")
            .with_anticheat("eac", "/usr/share/umu/easyanticheat_runtime");
        let found = applied_with(&subject, true, host);
        assert_eq!(
            found.env.get("PROTON_BATTLEYE_RUNTIME").map(String::as_str),
            Some("/usr/share/umu/battleye_runtime")
        );
        assert_eq!(
            found.env.get("PROTON_EAC_RUNTIME").map(String::as_str),
            Some("/usr/share/umu/easyanticheat_runtime")
        );
    }

    #[test]
    fn the_anticheat_lookup_is_skipped_entirely_without_proton_features() {
        // Not merely "not written": the search touches the filesystem across
        // five roots, so running it for plain Wine would be both pointless and
        // slow. The fake counts the calls rather than the result.
        let mut subject = game("x");
        subject.battleye = true;
        subject.eac = true;

        let host = FakeLaunchEnv::new()
            .with_anticheat("battleye", "/somewhere")
            .with_anticheat("eac", "/somewhere");
        let plain = applied_with(&subject, false, host);
        assert!(!plain.env.contains_key("PROTON_BATTLEYE_RUNTIME"));
        assert!(!plain.env.contains_key("PROTON_EAC_RUNTIME"));
    }

    #[test]
    fn the_virtual_desktop_wrapper_is_windows_only_and_nests_inside_the_wrappers() {
        let mut subject = game("Half-Life 2");
        subject.virtual_desktop = true;

        let wrapped = applied(&subject, false);
        assert_eq!(wrapped.argv[0], "wine");
        assert_eq!(wrapped.argv[1], "explorer");
        assert_eq!(wrapped.argv[2], "/desktop=HalfLife2,1920x1080");

        // A native Linux title keeps the toggle but must not get `explorer`,
        // which only Wine has.
        let mut linux = game("Half-Life 2");
        linux.kind = "linux".to_string();
        linux.exe_path = "/usr/bin/hl2".to_string();
        linux.virtual_desktop = true;
        let linux_applied = apply_launch_options(
            &linux,
            &["/usr/bin/hl2".to_string()],
            &BTreeMap::new(),
            false,
            &FakeLaunchEnv::new(),
        )
        .unwrap();
        assert_eq!(linux_applied.argv, vec!["/usr/bin/hl2".to_string()]);
    }

    #[test]
    fn mangohud_prefers_the_binary_and_falls_back_to_the_variable() {
        let mut subject = game("x");
        subject.mangohud = true;

        // Not installed: `MANGOHUD=1` still makes the Vulkan layer load, so the
        // overlay appears even though there is no wrapper to run.
        let without = applied(&subject, false);
        assert_eq!(without.env.get("MANGOHUD").map(String::as_str), Some("1"));
        assert_eq!(without.argv, vec!["wine".to_string()]);

        let with = applied_with(
            &subject,
            false,
            FakeLaunchEnv::new().with_which("mangohud", "/usr/bin/mangohud"),
        );
        assert_eq!(
            with.argv,
            vec!["/usr/bin/mangohud".to_string(), "wine".to_string()]
        );
        assert!(!with.env.contains_key("MANGOHUD"));
    }

    #[test]
    fn gamemode_wraps_only_when_gamemoderun_is_installed() {
        let mut subject = game("x");
        subject.gamemode = true;

        // Unlike mangohud there is no environment fallback, so a missing
        // gamemoderun is silently ignored rather than an error.
        assert_eq!(applied(&subject, false).argv, vec!["wine".to_string()]);
        let with = applied_with(
            &subject,
            false,
            FakeLaunchEnv::new().with_which("gamemoderun", "/usr/bin/gamemoderun"),
        );
        assert_eq!(
            with.argv,
            vec!["/usr/bin/gamemoderun".to_string(), "wine".to_string()]
        );
    }

    #[test]
    fn gamescope_wraps_outermost_and_only_carries_hdr_when_it_is_on() {
        let host = || {
            FakeLaunchEnv::new()
                .with_which("gamescope", "/usr/bin/gamescope")
                .with_which("mangohud", "/usr/bin/mangohud")
                .with_which("gamemoderun", "/usr/bin/gamemoderun")
        };
        let mut subject = game("x");
        subject.gamescope = true;
        subject.mangohud = true;
        subject.gamemode = true;

        let command = applied_with(&subject, false, host());
        // Gamescope was applied last, so it is the outermost wrapper and the
        // `--` separates its own flags from the command it runs.
        assert_eq!(
            command.argv,
            vec![
                "/usr/bin/gamescope".to_string(),
                "--".to_string(),
                "/usr/bin/gamemoderun".to_string(),
                "/usr/bin/mangohud".to_string(),
                "wine".to_string(),
            ]
        );

        subject.hdr = true;
        let with_hdr = applied_with(&subject, false, host());
        assert_eq!(with_hdr.argv[1], "--hdr-enabled");
        assert_eq!(with_hdr.argv[2], "--");
    }

    #[test]
    fn a_missing_gamescope_is_an_error_naming_the_flathub_extension() {
        // The one thing in this function that raises. Failing loudly is the
        // point: launching without the compositor would look like the toggle
        // did nothing.
        let mut subject = game("x");
        subject.gamescope = true;
        let error = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &BTreeMap::new(),
            false,
            &FakeLaunchEnv::new(),
        )
        .unwrap_err();
        assert!(matches!(error, RunnerError::GamescopeMissing));
        let message = error.to_string();
        assert!(
            message.contains("org.freedesktop.Platform.VulkanLayer.gamescope//25.08"),
            "{message}"
        );
        assert!(message.contains("Flathub"), "{message}");
    }

    #[test]
    fn the_environment_block_is_applied_last_so_the_user_wins() {
        // The whole reason `launch` parses the block twice. Every toggle here
        // is on and then contradicted by the block.
        let mut subject = game("x");
        subject.esync = true;
        subject.mangohud = true;
        subject.hdr = true;
        subject.environment = "WINEESYNC=0\nMANGOHUD=0\nDXVK_HDR=0\nEXTRA=1".to_string();

        let command = applied(&subject, true);
        assert_eq!(command.env.get("WINEESYNC").map(String::as_str), Some("0"));
        assert_eq!(command.env.get("MANGOHUD").map(String::as_str), Some("0"));
        assert_eq!(command.env.get("DXVK_HDR").map(String::as_str), Some("0"));
        assert_eq!(command.env.get("EXTRA").map(String::as_str), Some("1"));
        // The variables the block does not mention are still the toggles'.
        assert_eq!(
            command.env.get("PROTON_ENABLE_HDR").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn a_windows_toggle_is_the_only_thing_gated_on_the_game_s_kind() {
        // A native title still gets mangohud, gamemode and gamescope — those
        // are session wrappers, not Wine ones — but none of the WINE_/PROTON_
        // variables, and no DLL overrides.
        let mut linux = game("native");
        linux.kind = "linux".to_string();
        linux.esync = false;
        linux.fsync = false;
        linux.dxvk = false;
        linux.vkd3d = false;
        linux.hdr = true;
        linux.nvapi = true;
        linux.battleye = false;
        linux.eac = false;
        linux.mangohud = true;

        let command = apply_launch_options(
            &linux,
            &["/usr/bin/native".to_string()],
            &BTreeMap::new(),
            true,
            &FakeLaunchEnv::new().with_which("mangohud", "/usr/bin/mangohud"),
        )
        .unwrap();

        assert_eq!(
            command.argv,
            vec![
                "/usr/bin/mangohud".to_string(),
                "/usr/bin/native".to_string()
            ]
        );
        // hdr is outside the `is_linux` block, so `DXVK_HDR` is still set — it
        // is harmless for a native title and Python sets it anyway.
        assert_eq!(command.env.get("DXVK_HDR").map(String::as_str), Some("1"));
        assert_eq!(
            command.env.get("PROTON_ENABLE_HDR").map(String::as_str),
            Some("1")
        );
        for key in [
            "WINEESYNC",
            "WINEFSYNC",
            "PROTON_USE_WINED3D",
            "WINEDLLOVERRIDES",
            "PROTON_BATTLEYE_RUNTIME",
            "PROTON_EAC_RUNTIME",
        ] {
            assert!(
                !command.env.contains_key(key),
                "{key} leaked onto a native title"
            );
        }
    }

    #[test]
    fn the_caller_s_environment_is_never_mutated() {
        // Python copies with `dict(env)`; a port that mutated in place would
        // make a second launch inherit the first one's toggles, and the
        // `launch` path calls this after already mutating its own copy.
        let original: BTreeMap<String, String> = [
            ("WINEDLLOVERRIDES".to_string(), "keep=me".to_string()),
            ("WINEESYNC".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();
        let before = original.clone();

        let mut subject = game("x");
        subject.dxvk = false;
        subject.esync = false;
        let _ = apply_launch_options(
            &subject,
            &["wine".to_string()],
            &original,
            false,
            &FakeLaunchEnv::new(),
        )
        .unwrap();

        assert_eq!(original, before);
    }

    #[test]
    fn an_empty_environment_block_leaves_the_toggles_alone() {
        let mut subject = game("x");
        subject.esync = false;
        assert_eq!(
            applied(&subject, false)
                .env
                .get("WINEESYNC")
                .map(String::as_str),
            Some("0")
        );
    }

    // -- build_linux_command -------------------------------------------------

    #[test]
    fn a_native_command_carries_the_inherited_environment_untouched() {
        let mut subject = game("native");
        subject.kind = "linux".to_string();
        subject.exe_path = "/usr/bin/native".to_string();
        subject.arguments = "--fullscreen 'two words'".to_string();

        let host =
            FakeLaunchEnv::new().with_environ(&[("HOME", "/home/tester"), ("LANG", "C.UTF-8")]);
        let command = build_linux_command(&subject, &host).unwrap();

        assert_eq!(
            command.argv,
            vec![
                "/usr/bin/native".to_string(),
                "--fullscreen".to_string(),
                "two words".to_string(),
            ]
        );
        // `dict(os.environ)` and nothing else: no WINEPREFIX, no
        // WINEDLLOVERRIDES, no WINEDEBUG.
        assert_eq!(command.env, host.environ_of().clone());
    }

    #[test]
    fn a_native_command_with_no_executable_is_an_error() {
        let mut subject = game("native");
        subject.kind = "linux".to_string();
        assert!(matches!(
            build_linux_command(&subject, &FakeLaunchEnv::new()),
            Err(RunnerError::NoExecutable)
        ));
    }

    #[test]
    fn malformed_arguments_are_an_error_rather_than_a_silent_drop() {
        let mut subject = game("native");
        subject.kind = "linux".to_string();
        subject.exe_path = "/usr/bin/native".to_string();
        subject.arguments = "unbalanced 'quote".to_string();
        assert!(matches!(
            build_linux_command(&subject, &FakeLaunchEnv::new()),
            Err(RunnerError::Shell(_))
        ));
    }

    // -- install_bundled_dxvk ------------------------------------------------

    /// A DXVK source tree with both architectures, and a prefix with no marker.
    fn dxvk_fixture(label: &str) -> (PathBuf, PathBuf) {
        let root = scratch(label);
        let source = root.join("bundled");
        for arch in ["x32", "x64"] {
            std::fs::create_dir_all(source.join(arch)).unwrap();
            std::fs::write(source.join(arch).join("d3d11.dll"), arch.as_bytes()).unwrap();
            std::fs::write(source.join(arch).join("dxgi.dll"), arch.as_bytes()).unwrap();
            // Not a DLL: a `*.dll.txt` must not be copied.
            std::fs::write(source.join(arch).join("d3d11.dll.txt"), b"no").unwrap();
        }
        let prefix = root.join("prefix");
        (root, prefix)
    }

    fn prefix_env(prefix: &Path, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut env: BTreeMap<String, String> = [(
            "WINEPREFIX".to_string(),
            prefix.to_string_lossy().into_owned(),
        )]
        .into_iter()
        .collect();
        for (key, value) in extra {
            env.insert((*key).to_string(), (*value).to_string());
        }
        env
    }

    #[test]
    fn a_prefix_without_wineprefix_is_refused_before_anything_is_read() {
        let mut env = BTreeMap::new();
        let error = install_bundled_dxvk(&mut env, None, &FakeLaunchEnv::new()).unwrap_err();
        assert!(matches!(error, RunnerError::DxvkNeedsPrefix));
        assert_eq!(error.to_string(), "DXVK requires a configured Wine prefix");
    }

    #[test]
    fn a_whitespace_only_wineprefix_is_absent_too() {
        // `env.get("WINEPREFIX", "").strip()` — a prefix of spaces is not a
        // prefix, and treating it as one would create a directory named " ".
        let mut env: BTreeMap<String, String> = [("WINEPREFIX".to_string(), "   ".to_string())]
            .into_iter()
            .collect();
        assert!(matches!(
            install_bundled_dxvk(&mut env, None, &FakeLaunchEnv::new()),
            Err(RunnerError::DxvkNeedsPrefix)
        ));
    }

    #[test]
    fn an_incomplete_bundle_is_refused_rather_than_half_installed() {
        let (root, prefix) = dxvk_fixture("dxvk-incomplete");
        let source = root.join("bundled");
        std::fs::remove_file(source.join("x64").join("d3d11.dll")).unwrap();

        let mut env = prefix_env(&prefix, &[]);
        let error =
            install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap_err();
        assert!(matches!(error, RunnerError::DxvkUnavailable));
        assert_eq!(error.to_string(), "Bundled DXVK runtime is unavailable");
        // Nothing was written, not even the prefix directory.
        assert!(!prefix.exists());
    }

    #[test]
    fn a_64_bit_prefix_gets_both_architectures_in_both_directories() {
        let (root, prefix) = dxvk_fixture("dxvk-64");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[]);

        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();

        let windows = prefix.join("drive_c/windows");
        // x64 into system32, x32 into syswow64 — a 32-bit game in a 64-bit
        // prefix loads the latter, so a one-directory install looks correct and
        // then fails to start exactly one kind of title.
        assert_eq!(
            std::fs::read(windows.join("system32/d3d11.dll")).unwrap(),
            b"x64"
        );
        assert_eq!(
            std::fs::read(windows.join("syswow64/d3d11.dll")).unwrap(),
            b"x32"
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join(DXVK_MARKER_NAME)).unwrap(),
            format!("{DXVK_VERSION}\n")
        );
        // The overrides that make the DLLs take effect are the point of the
        // whole exercise.
        assert_eq!(
            env.get("WINEDLLOVERRIDES").map(String::as_str),
            Some(DXVK_DLL_OVERRIDES)
        );
        // `*.dll` is not `*.dll.txt`.
        assert!(!windows.join("system32/d3d11.dll.txt").exists());
        // And both DLLs, not just the one the check names.
        assert!(windows.join("system32/dxgi.dll").exists());
    }

    #[test]
    fn a_win32_environment_installs_one_architecture_into_system32() {
        let (root, prefix) = dxvk_fixture("dxvk-win32-env");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[("WINEARCH", "win32")]);

        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();

        let windows = prefix.join("drive_c/windows");
        assert_eq!(
            std::fs::read(windows.join("system32/d3d11.dll")).unwrap(),
            b"x32"
        );
        assert!(!windows.join("syswow64").exists());
    }

    #[test]
    fn a_win32_environment_is_recognised_case_and_space_insensitively() {
        // `env.get("WINEARCH", "").strip().lower() == "win32"`.
        let (root, prefix) = dxvk_fixture("dxvk-win32-case");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[("WINEARCH", "  Win32 ")]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"x32"
        );
    }

    #[test]
    fn an_old_prefix_records_its_architecture_in_the_registry() {
        // A prefix created before `WINEARCH` was ever set. The marker is in the
        // first 512 characters of `system.reg`, and nothing else about that
        // file is read.
        let (root, prefix) = dxvk_fixture("dxvk-registry");
        let source = root.join("bundled");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(
            prefix.join("system.reg"),
            "WINE REGISTRY Version 2\n;;\n#arch=win32\n[Software\\\\Wine]\n",
        )
        .unwrap();

        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"x32"
        );
        assert!(!prefix.join("drive_c/windows/syswow64").exists());
    }

    #[test]
    fn the_registry_marker_is_only_looked_for_in_the_first_512_characters() {
        // `[:512]` is a slice of the *decoded* text, so a marker beyond it is
        // not found — and the prefix is then treated as 64-bit.
        let (root, prefix) = dxvk_fixture("dxvk-registry-window");
        let source = root.join("bundled");
        std::fs::create_dir_all(&prefix).unwrap();
        let mut text = "x".repeat(600);
        text.push_str("#arch=win32");
        std::fs::write(prefix.join("system.reg"), text).unwrap();

        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert!(prefix.join("drive_c/windows/syswow64").exists());
    }

    #[test]
    fn an_undecodable_registry_is_skipped_rather_than_crashing() {
        // `errors="ignore"` on this read, in contrast to the marker below.
        // Dropping the invalid bytes rather than substituting U+FFFD is what
        // lets the marker after them still be found.
        let (root, prefix) = dxvk_fixture("dxvk-registry-binary");
        let source = root.join("bundled");
        std::fs::create_dir_all(&prefix).unwrap();
        let mut bytes = vec![0xff, 0xfe, 0x80];
        bytes.extend_from_slice(b"#arch=win32");
        std::fs::write(prefix.join("system.reg"), bytes).unwrap();

        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"x32"
        );
    }

    #[test]
    fn the_version_marker_makes_a_second_install_a_no_op() {
        let (root, prefix) = dxvk_fixture("dxvk-marker");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();

        // Damage the installed DLLs, then run again: a marker that says the
        // right version means the copy is skipped, so the damage survives. That
        // is the behaviour — the point is not to repair, it is not to copy on
        // every launch.
        std::fs::write(
            prefix.join("drive_c/windows/system32/d3d11.dll"),
            b"tampered",
        )
        .unwrap();
        let mut second = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut second, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"tampered"
        );
        // The overrides are still merged, which is the half that must not be
        // skipped.
        assert_eq!(
            second.get("WINEDLLOVERRIDES").map(String::as_str),
            Some(DXVK_DLL_OVERRIDES)
        );
    }

    #[test]
    fn a_marker_for_another_version_reinstalls() {
        let (root, prefix) = dxvk_fixture("dxvk-marker-old");
        let source = root.join("bundled");
        std::fs::create_dir_all(prefix.join("drive_c/windows/system32")).unwrap();
        std::fs::write(prefix.join(DXVK_MARKER_NAME), "2.0.0\n").unwrap();

        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"x64"
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join(DXVK_MARKER_NAME)).unwrap(),
            format!("{DXVK_VERSION}\n")
        );
    }

    #[test]
    fn a_marker_with_surrounding_whitespace_still_counts_as_current() {
        // The comparison is against the *stripped* contents.
        let (root, prefix) = dxvk_fixture("dxvk-marker-space");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        std::fs::write(
            prefix.join(DXVK_MARKER_NAME),
            format!("  {DXVK_VERSION}  \n"),
        )
        .unwrap();
        std::fs::write(
            prefix.join("drive_c/windows/system32/d3d11.dll"),
            b"tampered",
        )
        .unwrap();

        let mut second = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut second, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"tampered"
        );
    }

    #[test]
    fn an_undecodable_marker_is_an_error_rather_than_a_silent_reinstall() {
        // The port's deliberate divergence 1, and the test is written to fail
        // whichever way someone "fixes" it silently: Python raises an uncaught
        // `UnicodeDecodeError` here (measured), so the launch fails, and this
        // asserts that it still does — not that the DLLs were reinstalled.
        let (root, prefix) = dxvk_fixture("dxvk-marker-binary");
        let source = root.join("bundled");
        std::fs::create_dir_all(prefix.join("drive_c/windows/system32")).unwrap();
        std::fs::write(prefix.join(DXVK_MARKER_NAME), b"\xff\xfe not utf8").unwrap();

        let mut env = prefix_env(&prefix, &[]);
        let error =
            install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap_err();
        assert!(matches!(error, RunnerError::Io(_)), "{error:?}");
        // And nothing was written, so a retry sees the same state.
        assert!(!prefix.join("drive_c/windows/system32/d3d11.dll").exists());
        assert!(!env.contains_key("WINEDLLOVERRIDES"));
    }

    #[test]
    fn the_bundle_root_comes_from_the_environment_when_the_caller_does_not_say() {
        // `os.environ.get(GAMEHANDLER_DXVK_ROOT, DXVK_ROOT)` — the Flatpak and a
        // source install keep the runtime in different places, and this is how
        // the source install says so.
        let (root, prefix) = dxvk_fixture("dxvk-root-env");
        let source = root.join("bundled");
        let host = FakeLaunchEnv::new().with_vars(&[(DXVK_ROOT_ENV, source.to_str().unwrap())]);

        let mut env = prefix_env(&prefix, &[]);
        install_bundled_dxvk(&mut env, None, &host).unwrap();
        assert_eq!(
            std::fs::read(prefix.join("drive_c/windows/system32/d3d11.dll")).unwrap(),
            b"x64"
        );
    }

    #[test]
    fn an_override_pointing_nowhere_fails_rather_than_falling_back_to_the_built_in_root() {
        // An empty override is `Path("")` in Python, i.e. the current directory
        // — *not* `/app/share/gamehandler/dxvk`. A port that treated empty as
        // unset would install from the Flatpak's bundle on a box that
        // deliberately pointed somewhere else, which is the bug the override
        // exists to prevent.
        let (root, prefix) = dxvk_fixture("dxvk-root-empty");
        let host = FakeLaunchEnv::new().with_vars(&[(DXVK_ROOT_ENV, "")]);
        let mut env = prefix_env(&prefix, &[]);
        let error = install_bundled_dxvk(&mut env, None, &host).unwrap_err();
        assert!(matches!(error, RunnerError::DxvkUnavailable));
        // And specifically not the real bundle, which on this machine may exist.
        assert!(!prefix.join("drive_c").exists());
        drop(root);
    }

    #[test]
    fn an_empty_per_game_winearch_does_not_make_the_prefix_32_bit() {
        // `launch` primes the environment from the user's block, so `WINEARCH`
        // can be present and empty; `== "win32"` is then false and the registry
        // is consulted, as Python does.
        let (root, prefix) = dxvk_fixture("dxvk-winearch-empty");
        let source = root.join("bundled");
        let mut env = prefix_env(&prefix, &[("WINEARCH", "")]);
        install_bundled_dxvk(&mut env, Some(&source), &FakeLaunchEnv::new()).unwrap();
        assert!(prefix.join("drive_c/windows/syswow64").exists());
    }

    // -- resolve_game_paths --------------------------------------------------

    /// A resolver that maps a fixed prefix onto a local directory, so the
    /// decision logic is tested without `netpaths` and without a GVFS mount.
    struct FakeResolver {
        removable: BTreeMap<String, String>,
        unreachable: BTreeMap<String, String>,
    }

    impl FakeResolver {
        fn new() -> Self {
            Self {
                removable: BTreeMap::new(),
                unreachable: BTreeMap::new(),
            }
        }

        fn mapping(mut self, url: &str, local: &str) -> Self {
            self.removable.insert(url.to_string(), local.to_string());
            self
        }

        fn stuck(mut self, url: &str, message: &str) -> Self {
            self.unreachable
                .insert(url.to_string(), message.to_string());
            self
        }
    }

    impl ShareResolver for FakeResolver {
        fn is_remote_url(&self, value: &str) -> bool {
            value.starts_with("smb://") || self.unreachable.contains_key(value)
        }

        fn as_local_path(&self, value: &str) -> String {
            self.removable
                .get(value)
                .cloned()
                .unwrap_or_else(|| value.to_string())
        }

        fn unreachable_share_message(&self, value: &str) -> String {
            self.unreachable
                .get(value)
                .cloned()
                .unwrap_or_else(|| format!("{value} cannot be reached"))
        }
    }

    #[test]
    fn an_already_local_game_is_returned_unchanged() {
        let mut subject = game("x");
        subject.exe_path = "/games/x.exe".to_string();
        subject.working_directory = "/games".to_string();
        let resolved = resolve_game_paths(&subject, &FakeResolver::new()).unwrap();
        // Equal *and* a copy: no field was rewritten to the same value.
        assert!(resolved == subject);
    }

    #[test]
    fn a_share_url_is_mapped_onto_its_mount() {
        let mut subject = game("x");
        subject.exe_path = "smb://host/share/x.exe".to_string();
        subject.working_directory = "smb://host/share".to_string();
        subject.additional_app = "smb://host/share/tool.exe".to_string();

        let resolver = FakeResolver::new()
            .mapping(
                "smb://host/share/x.exe",
                "/run/user/1000/gvfs/smb-share:server=host,share=share/x.exe",
            )
            .mapping(
                "smb://host/share",
                "/run/user/1000/gvfs/smb-share:server=host,share=share",
            )
            .mapping(
                "smb://host/share/tool.exe",
                "/run/user/1000/gvfs/smb-share:server=host,share=share/tool.exe",
            );

        let resolved = resolve_game_paths(&subject, &resolver).unwrap();
        assert_eq!(
            resolved.exe_path,
            "/run/user/1000/gvfs/smb-share:server=host,share=share/x.exe"
        );
        assert_eq!(
            resolved.working_directory,
            "/run/user/1000/gvfs/smb-share:server=host,share=share"
        );
        assert_eq!(
            resolved.additional_app,
            "/run/user/1000/gvfs/smb-share:server=host,share=share/tool.exe"
        );
        // Everything else is untouched.
        assert_eq!(resolved.id, subject.id);
        assert_eq!(resolved.name, subject.name);
    }

    #[test]
    fn an_unmounted_share_is_an_error_carrying_the_original_url() {
        let mut subject = game("x");
        subject.exe_path = "smb://host/share/x.exe".to_string();

        let resolver = FakeResolver::new().stuck(
            "smb://host/share/x.exe",
            "smb://host/share/x.exe is a network location that is not mounted yet. \
             Open host in your file manager once so the share is mounted, then try again.",
        );
        let error = resolve_game_paths(&subject, &resolver).unwrap_err();
        let message = error.to_string();
        // The *original* URL, which is what the user recognises and can act on.
        assert!(message.contains("smb://host/share/x.exe"), "{message}");
        assert!(message.contains("is not mounted yet"), "{message}");
        assert!(message.contains("file manager"), "{message}");
    }

    #[test]
    fn only_the_executable_decides_whether_a_share_is_unreachable() {
        // `is_remote_url(exe)` — a remote working directory that resolved, or
        // even one that did not, does not raise. The message is about the
        // executable because that is what Wine is asked to run.
        let mut subject = game("x");
        subject.exe_path = "/games/x.exe".to_string();
        subject.working_directory = "smb://host/gone".to_string();
        subject.additional_app = "smb://host/gone/tool.exe".to_string();

        let resolver = FakeResolver::new()
            .stuck("smb://host/gone", "gone is not mounted")
            .stuck("smb://host/gone/tool.exe", "gone is not mounted");
        let resolved = resolve_game_paths(&subject, &resolver).unwrap();
        // Unresolved values are kept as they are, so the failure surfaces at
        // exec with Wine's own message rather than a fabricated one.
        assert_eq!(resolved.working_directory, "smb://host/gone");
        assert_eq!(resolved.additional_app, "smb://host/gone/tool.exe");
    }

    #[test]
    fn the_remote_check_reads_the_resolved_executable_not_the_original() {
        // Python resolves first and checks second. A resolver that maps a URL
        // onto a mount makes it local, so a share with a mount behind it does
        // not raise — the order is the reason a mounted share works at all.
        let mut subject = game("x");
        subject.exe_path = "smb://host/share/x.exe".to_string();
        let resolver =
            FakeResolver::new().mapping("smb://host/share/x.exe", "/run/user/1000/gvfs/x.exe");
        let resolved = resolve_game_paths(&subject, &resolver).unwrap();
        assert_eq!(resolved.exe_path, "/run/user/1000/gvfs/x.exe");
    }

    #[test]
    fn a_file_url_is_not_a_remote_share() {
        // `file://` has no netloc in the remote scheme set, so it is unwrapped
        // by `as_local_path` and never raises; falling through to the error
        // path would make a picker result unusable.
        let mut subject = game("x");
        subject.exe_path = "file:///games/x.exe".to_string();
        let resolver = FakeResolver::new().mapping("file:///games/x.exe", "/games/x.exe");
        let resolved = resolve_game_paths(&subject, &resolver).unwrap();
        assert_eq!(resolved.exe_path, "/games/x.exe");
    }
    // -----------------------------------------------------------------
    // The constants are transcriptions, and transcriptions need pinning
    // -----------------------------------------------------------------

    /// The four DXVK/WineD3D/VKD3D literals and the marker name, checked
    /// against `runners.py` itself rather than against a copy of them.
    ///
    /// These are transcriptions of Python string literals that appear nowhere
    /// in the oracle corpus — the advocate byte-searched the fixtures and found
    /// zero occurrences of `dxgi=n,b`, `d3d9=b`, `d3d12core=b` or
    /// `gamehandler-dxvk` in either the cases or the answers, because
    /// `apply_launch_options` and `install_bundled_dxvk` are not oracle ops and
    /// `merge_dll_overrides` is always called with case-supplied arguments.
    ///
    /// That left them pinned only against themselves: every assertion site
    /// compares a constant to a value derived from that same constant, so
    /// flipping a literal — swapping `n,b` for `b,n`, dropping `dxgi` from the
    /// DXVK list, dropping `d3d12core`, renaming the marker — survived the whole
    /// suite in both build scopes. The values are correct today; the point is
    /// that they were correct for no reason a test could tell you about.
    ///
    /// The Python source is still in the tree as the port's specification, so
    /// the comparison can be mechanical. This **parses** the literals out of the
    /// calls rather than searching for the constants' own text, which matters:
    /// a test that grepped for `"d3d8,d3d9,d3d10core,d3d11,dxgi=n,b"` would just
    /// be the same transcription written twice, and would go stale in lockstep
    /// with the constant. Reading the call sites means a Python-side change
    /// shows up as a failure instead of as nothing at all.
    #[test]
    fn the_dll_override_literals_are_the_reference_s_and_not_a_copy_of_ourselves() {
        let source =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gamehandler/runners.py");
        let text = std::fs::read_to_string(&source).unwrap_or_else(|err| {
            panic!(
                "{} should be readable: {err}\n\
                 It is the reference these constants were transcribed from, and \
                 the port's own tests read it. If it has been moved, this check \
                 needs a new path — and so does every citation in \
                 docs/migration/.",
                source.display()
            )
        });

        // `merge_dll_overrides(env, "<literal>")`, in source order.
        let merged: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().split_once("merge_dll_overrides(env, \""))
            .filter_map(|(_head, rest)| rest.split_once('"').map(|(literal, _)| literal))
            .collect();

        // Three distinct literals, the DXVK one twice — once on the
        // already-installed fast path and once after the copy. A change to
        // either call site has to be a change to both, so the count is part of
        // the assertion and not an incidental detail of the fixture.
        let mut sorted = merged.clone();
        sorted.sort_unstable();
        let mut expected = vec![
            DXVK_DLL_OVERRIDES,
            DXVK_DLL_OVERRIDES,
            WINED3D_DLL_OVERRIDES,
            VKD3D_DLL_OVERRIDES,
        ];
        expected.sort_unstable();
        assert_eq!(
            sorted, expected,
            "the override literals passed to `merge_dll_overrides` in \
             runners.py, sorted, should be the three constants with DXVK \
             twice; found {merged:?}. If the reference changed its overrides, \
             the constants above must change with it."
        );

        // `marker = prefix / ".gamehandler-dxvk-version"` — the name of the
        // file `install_bundled_dxvk` writes and re-reads. A copy of the
        // constant in the Python source is not the thing to compare against,
        // so this reads the assignment and not a bare mention of the name.
        let marker = text
            .lines()
            .filter_map(|line| line.trim().split_once("marker = prefix / \""))
            .filter_map(|(_head, rest)| rest.split_once('"').map(|(name, _)| name))
            .next()
            .expect("runners.py should assign `marker = prefix / <name>`");
        assert_eq!(
            marker, DXVK_MARKER_NAME,
            "the version marker the reference writes into the prefix"
        );
    }
}
