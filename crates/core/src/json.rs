//! JSON input and output that matches Python's `json` module.
//!
//! Two behaviours of CPython's `json` are load-bearing for compatibility and
//! are **not** `serde_json`'s defaults (see `docs/migration/oracle/FINDINGS.md`
//! and DECISIONS D-15/D-16):
//!
//! * **Reading** — `json.loads` accepts what strict JSON does not: the bare
//!   words `NaN`, `Infinity` and `-Infinity`, and numeric literals too large
//!   for a `f64` (`1e400`), which it saturates to `±inf`. It also accepts
//!   integers of arbitrary size. `serde_json` rejects all of these, and a
//!   rejection at this level discards the *entire* library file rather than one
//!   field — the exact failure D-06 exists to prevent. [`parse_lenient`]
//!   rewrites those literals into strict JSON that `serde_json` accepts, before
//!   parsing.
//!
//! * **Reading** — two tolerance cases where Python's failure is worse than
//!   its behaviour, both D-21. A leading UTF-8 BOM is stripped rather than
//!   read as a corrupt file (Python loses the library and overwrites it on the
//!   next save), and nesting is bounded by clamping rather than by a recursion
//!   limit ([`MAX_VALUE_DEPTH`]).
//!
//! * **Writing** — `json.dumps` defaults to `ensure_ascii=True`, so
//!   `"Pokémon"` is written `"Pok\u00e9mon"`. `serde_json` writes raw UTF-8.
//!   [`to_python_string`] reproduces Python's escaping and `indent=2` layout, so
//!   a file written here and one written by the Python app are diff-identical.

use std::borrow::Cow;
use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::ser::{CharEscape, Formatter, PrettyFormatter, Serializer};
use serde_json::Value;

/// The deepest value this parser will materialise (DECISIONS D-21).
///
/// `serde_json`'s own limit is 128, well below what CPython accepts: the oracle
/// pins a 1,000-deep document parsing in Python, and F-L measured the real
/// ceiling at **50,000**. Being stricter than Python is not a cosmetic
/// difference — a parse failure empties the library and the *next* save writes
/// that emptiness back over the user's file, so it means silently destroying a
/// library over a nested value the app never reads.
///
/// This bound is therefore far below Python's, and that is a deliberate trade
/// rather than an oversight. Values between 65 and 50,000 levels are read as
/// `null` where Python would materialise them, which is unobservable because of
/// what a value at that depth can be: all 31 `Game` fields are scalars, so a
/// field holding a 100-deep array is invalid to Python's own validators and to
/// D-18's coercion alike — both discard it, by different routes. A deep value
/// can otherwise only sit under an unknown key, which `from_dict` drops (F-D).
/// The depth at which a *legitimate* value lives is 3.
///
/// The obvious fix is to raise serde_json's limit, and that is the wrong one.
/// A recursive-descent parser uses stack proportional to nesting, so raising
/// the limit trades an empty library for something worse: an abort. Measured on
/// a 2 MiB stack (what a spawned thread gets), a 1,000-deep document is fine and
/// a 1,500-deep one overflows — and a stack overflow is `SIGABRT`, which no
/// caller can catch, in a process that also owns the user's window. The margin
/// is not theoretical either: enabling `serde_json/preserve_order` (which
/// `cosmic-theme` does, and Cargo feature-unifies into a workspace build) was
/// enough to push the same 1,000-deep fixture over.
///
/// So the document is not parsed deeper than this. [`clamp_depth`] replaces any
/// value nested past `MAX_VALUE_DEPTH` with `null` before the parser sees it,
/// which costs nothing: every field the app reads is a scalar at depth 3 or
/// less, and anything deeper is either inside an unknown key (discarded by
/// `from_dict` — F-D) or a wrong-typed value that D-18 coerces anyway. The
/// value is discarded either way; this way it is never built, so there is no
/// stack to overflow.
///
/// 64 is far above the 3 a real `Game` uses, and far below the 128 where
/// serde_json's own limit would step in — so the bound here is the one that
/// applies, and it is enforced by construction rather than by a check that can
/// be skipped.
const MAX_VALUE_DEPTH: usize = 64;

/// Parse JSON the way Python's `json.loads` does, plus a leading BOM.
///
/// Accepts everything strict JSON does, plus `NaN` / `Infinity` /
/// `-Infinity`, arbitrary-length integers, out-of-`f64`-range literals
/// (saturated to `±inf`), a UTF-8 byte-order mark, and documents of any
/// nesting depth (values past [`MAX_VALUE_DEPTH`] are read as `null`).
///
/// # The BOM (DECISIONS D-21)
///
/// `json.loads` does **not** strip a BOM — and because Python also lets the
/// resulting failure pass silently, the sequence is: a BOM-prefixed
/// `games.json` parses to nothing, the library comes up empty, and the next
/// save writes `[]` over the user's file. That is a hand-edited or
/// Windows-edited file being destroyed for the sake of one invisible character.
/// Stripping it is a deliberate divergence: it keeps the games.
///
/// # Depth (DECISIONS D-21)
///
/// Depth is handled by [`clamp_depth`], not by a recursion limit. See
/// [`MAX_VALUE_DEPTH`] for why, and note the consequence: a document deeper
/// than 64 levels parses *successfully*, with the over-deep values read as
/// `null`. That is not a compromise — those values are dropped downstream
/// regardless — but it does mean this function does not round-trip an
/// arbitrarily deep document, which is why it is the app's file reader rather
/// than a general-purpose parser.
pub fn parse_lenient(text: &str) -> Result<Value, serde_json::Error> {
    // `\u{feff}` is what a UTF-8 BOM decodes to, since the file is read as
    // UTF-8. Only the first character counts, as in `utf-8-sig`.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sanitized = sanitize(text);
    let clamped = clamp_depth(&sanitized);
    // `from_str` is left to apply serde_json's own 128-deep limit. It cannot
    // fire — `clamp_depth` has already bounded the document to
    // `MAX_VALUE_DEPTH` — and that redundancy is deliberate: if the clamp ever
    // regresses, the result is an error we can handle, not a stack overflow
    // that aborts the process.
    serde_json::from_str(&clamped)
}

/// Replace every value nested deeper than [`MAX_VALUE_DEPTH`] with `null`.
///
/// Non-recursive by construction, which is the entire point: it walks the text
/// with a bracket counter instead of descending into values, so it costs no
/// stack however deep the document is. Strings are skipped, so a `[` inside a
/// game name is not nesting — the same single-pass shape as [`sanitize`].
///
/// Returns a borrowed slice when nothing was too deep, which is every file the
/// Python app writes.
fn clamp_depth(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    // Everything before `copied` has been flushed into `out`.
    let mut copied = 0usize;
    let mut rewrote = false;
    // The number of containers we are currently inside.
    let mut depth = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = string_end(bytes, i),
            b'[' | b'{' => {
                if depth == MAX_VALUE_DEPTH {
                    // This container is the one that would nest too deep, so
                    // the whole value is replaced. `container_end` finds where
                    // it finishes by counting brackets, which is what keeps
                    // this pass off the stack.
                    let end = container_end(bytes, i);
                    out.push_str(&text[copied..i]);
                    out.push_str("null");
                    copied = end;
                    i = end;
                    rewrote = true;
                } else {
                    depth += 1;
                    i += 1;
                }
            }
            b']' | b'}' => {
                // Saturating: an unbalanced `]` is serde's error to report, not
                // ours, and this must not underflow on the way there.
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }

    if rewrote {
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// The index just past the container that opens at `start`.
///
/// `start` must be a `[` or `{`. An unbalanced document returns the end of the
/// input, leaving serde to report the syntax error — this pass only ever
/// decides how much text to drop, never whether the JSON is well formed.
fn container_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = string_end(bytes, i),
            b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b']' | b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Rewrite the literals `serde_json` cannot read into ones it can.
///
/// Returns a borrowed slice when nothing needed rewriting, which is the case
/// for every file the Python app itself writes.
///
/// The substitutions are observationally exact for the only fields that carry
/// numbers — `added` and `last_played`. Python's `Game.from_dict` treats a
/// non-finite or negative timestamp as *invalid* and regenerates it, and
/// `null` lands on that same path (D-14), so `1e400`, `NaN`, `Infinity` and
/// `null` are all equivalent. Integers that overflow `i64`/`u64` but stay
/// inside `f64` range are *not* invalid — Python converts them to the nearest
/// `f64` and keeps them — so those are rewritten to that exact `f64`.
pub fn sanitize(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    // Everything before `copied` has been flushed into `out`.
    let mut copied = 0usize;
    let mut rewrote = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let literal: (usize, usize, Cow<'_, str>) = match bytes[i] {
            b'"' => {
                i = string_end(bytes, i);
                continue;
            }
            b'N' if bytes[i..].starts_with(b"NaN") => (i, i + 3, Cow::Borrowed("null")),
            b'I' if bytes[i..].starts_with(b"Infinity") => (i, i + 8, Cow::Borrowed("null")),
            b'-' if bytes[i + 1..].starts_with(b"Infinity") => {
                (i, i + 9, Cow::Borrowed("null"))
            }
            b'-' | b'0'..=b'9' => {
                let end = number_end(bytes, i);
                match normalize_number(&text[i..end]) {
                    Some(replacement) => (i, end, Cow::Owned(replacement)),
                    None => {
                        i = end;
                        continue;
                    }
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };

        let (start, end, replacement) = literal;
        if !rewrote {
            rewrote = true;
            out.reserve(text.len() + 8);
        }
        out.push_str(&text[copied..start]);
        out.push_str(&replacement);
        copied = end;
        i = end;
    }

    if !rewrote {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// Index just past the closing quote of the string that starts at `start`.
///
/// Unterminated strings return `text.len()`; the result is handed to
/// `serde_json` either way, which reports the syntax error.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Index just past the number literal that starts at `start`.
fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < bytes.len() && matches!(bytes[j], b'+' | b'-') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// `Some(replacement)` when the literal must be rewritten for `serde_json`.
fn normalize_number(token: &str) -> Option<String> {
    // Python keeps a float literal's value; `serde_json` reads any in-range
    // float literal correctly, so only out-of-range ones need attention.
    let is_integer_literal = !token.bytes().any(|b| matches!(b, b'.' | b'e' | b'E'));
    if is_integer_literal && (token.parse::<i64>().is_ok() || token.parse::<u64>().is_ok()) {
        return None;
    }

    match token.parse::<f64>() {
        // Finite: an integer literal too big for `i64`/`u64` but inside `f64`
        // range. Python converts an arbitrary-precision `int` to the nearest
        // `f64`, which is what `str::parse::<f64>` already did — re-emit it in
        // a form `serde_json` accepts.
        Ok(value) if value.is_finite() => {
            if is_integer_literal {
                Some(Value::from(value).to_string())
            } else {
                None
            }
        }
        // `1e400` saturates to `inf`; `inf` and `nan` are both invalid
        // timestamps and normalize identically to `null` (D-14).
        _ => Some("null".to_string()),
    }
}

/// Serialise a value the way `json.dumps(value, indent=2)` does.
///
/// Infallible in practice: the writer is in memory, and every byte written is
/// already valid UTF-8.
pub fn to_python_string<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    let mut writer = Utf8Writer::default();
    let mut serializer = Serializer::with_formatter(&mut writer, PythonFormatter::new());
    value.serialize(&mut serializer)?;
    Ok(writer.0)
}

/// Write `value` to `path` as Python would, without ever leaving a partial file
/// behind.
///
/// `Library.save` and `Settings.save` both do this: create the parent
/// directory, write the bytes to `<name>.json.tmp` beside the target, then
/// rename it into place. A rename is atomic within a filesystem, so a crash
/// mid-write cannot truncate a library the user would then lose
/// (`docs/migration/architecture.md` §5).
pub fn write_python_file<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // In memory, so this cannot fail for any value we can actually build.
    let text = to_python_string(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text.as_bytes())?;
    std::fs::rename(&temporary, path)
}

/// An `io::Write` that appends to a `String` and refuses invalid UTF-8.
///
/// `serde_json`'s formatter trait is `io::Write`-based, but everything it
/// writes here is either ASCII from [`PythonFormatter`] or a whole `&str`, so
/// the error arm is unreachable — it exists so the failure is reported rather
/// than papered over with a lossy conversion.
#[derive(Default)]
struct Utf8Writer(String);

impl io::Write for Utf8Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(text) => {
                self.0.push_str(text);
                Ok(buf.len())
            }
            Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `serde_json`'s pretty printer with Python's string escaping.
struct PythonFormatter {
    pretty: PrettyFormatter<'static>,
}

impl PythonFormatter {
    fn new() -> Self {
        Self {
            pretty: PrettyFormatter::with_indent(b"  "),
        }
    }
}

/// `serde_json`'s `Formatter` has one method per JSON token; only string
/// writing differs from the pretty printer, so the rest are forwarded.
macro_rules! forward_to_pretty_printer {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(
            fn $name<W>(&mut self, writer: &mut W $(, $arg: $ty)*) -> io::Result<()>
            where
                W: ?Sized + io::Write,
            {
                self.pretty.$name(writer $(, $arg)*)
            }
        )*
    };
}

impl Formatter for PythonFormatter {
    /// Where `ensure_ascii=True` earns its keep.
    ///
    /// `serde_json` emits a string by splitting it at each character *it*
    /// escapes (`"`, `\`, the five short escapes, and the other C0 controls)
    /// and handing the runs between them to this method — see
    /// `format_escaped_str_contents` in `serde_json`'s `ser.rs`. So a fragment
    /// holds printable ASCII and any character `serde_json` is happy to write
    /// raw (DEL, everything above 0x7f, `é`, `✓`, emoji), and it is exactly
    /// those last ones Python refuses to write raw. Escaping them here
    /// reproduces `json.dumps` exactly.
    ///
    /// The characters `serde_json` handles itself arrive at
    /// [`Self::write_char_escape`] instead, which is left to the pretty
    /// printer: its output (`\b`, `\t`, `\n`, `\f`, `\r`, `\"`, `\\`, and
    /// lowercase `\u00xx` for the rest of the C0 range) is already
    /// character-for-character what Python writes.
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(escape_python_fragment(fragment).as_bytes())
    }

    forward_to_pretty_printer! {
        write_null();
        write_bool(value: bool);
        write_i8(value: i8);
        write_i16(value: i16);
        write_i32(value: i32);
        write_i64(value: i64);
        write_i128(value: i128);
        write_u8(value: u8);
        write_u16(value: u16);
        write_u32(value: u32);
        write_u64(value: u64);
        write_u128(value: u128);
        write_f32(value: f32);
        write_f64(value: f64);
        write_number_str(value: &str);
        begin_string();
        end_string();
        write_char_escape(char_escape: CharEscape);
        write_byte_array(value: &[u8]);
        begin_array();
        end_array();
        begin_array_value(first: bool);
        end_array_value();
        begin_object();
        end_object();
        begin_object_key(first: bool);
        end_object_key();
        begin_object_value();
        end_object_value();
        write_raw_fragment(fragment: &str);
    }
}

/// A JSON string literal exactly as `json.dumps` writes it with
/// `ensure_ascii=True`: `"`-quoted, with every character outside printable
/// ASCII replaced by a lowercase `\uXXXX` escape (a surrogate pair above the
/// BMP).
///
/// The writing path never needs the quoted form in one piece — `serde_json`
/// calls [`PythonFormatter::begin_string`] for the opening quote and
/// [`PythonFormatter::write_string_fragment`] for the contents — so this exists
/// as the executable statement of the contract the tests assert against.
#[cfg(test)]
fn escape_python_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    escape_python_fragment_into(value, &mut out);
    out.push('"');
    out
}

/// [`escape_python_string`] without the surrounding quotes.
///
/// Used for the runs between the escapes `serde_json` applies itself, where the
/// short escapes and the quote can no longer occur — but they are handled
/// identically here anyway, so there is one implementation of Python's escaping
/// rather than two that can drift.
fn escape_python_fragment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    escape_python_fragment_into(value, &mut out);
    out
}

fn escape_python_fragment_into(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Python escapes everything outside `0x20..=0x7e`, DEL included.
            character if !('\u{20}'..='\u{7e}').contains(&character) => {
                let code = character as u32;
                if code > 0xffff {
                    let offset = code - 0x10000;
                    push_escape(out, 0xd800 + (offset >> 10) as u16);
                    push_escape(out, 0xdc00 + (offset & 0x3ff) as u16);
                } else {
                    push_escape(out, code as u16);
                }
            }
            character => out.push(character),
        }
    }
}

/// Appends a `\uXXXX` escape with Python's lowercase hex.
fn push_escape(out: &mut String, code: u16) {
    use std::fmt::Write as _;
    // Writing to a `String` cannot fail.
    let _ = write!(out, "\\u{code:04x}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_leaves_ordinary_documents_untouched() {
        let text = r#"[{"id": "a", "name": "Half-Life", "added": 1700000000.5}]"#;
        let sanitized = sanitize(text);
        assert!(
            matches!(sanitized, Cow::Borrowed(_)),
            "a document with nothing to rewrite must not be reallocated"
        );
        assert_eq!(sanitized, text);
    }

    #[test]
    fn sanitize_ignores_literals_inside_strings() {
        // The word "NaN" here is data, not a number, and must survive verbatim.
        let text = r#"{"name": "NaN Infinity -Infinity", "exe_path": "1e400"}"#;
        assert!(matches!(sanitize(text), Cow::Borrowed(_)));
        assert_eq!(sanitize(text), text);
    }

    #[test]
    fn sanitize_maps_non_finite_words_to_null() {
        assert_eq!(sanitize("[NaN]"), "[null]");
        assert_eq!(sanitize("[Infinity]"), "[null]");
        assert_eq!(sanitize("[-Infinity]"), "[null]");
        assert_eq!(sanitize("[1, NaN, 2]"), "[1, null, 2]");
    }

    #[test]
    fn sanitize_saturates_out_of_range_floats() {
        // Python parses `1e400` to `inf`; `serde_json` rejects the literal.
        assert_eq!(sanitize("[1e400]"), "[null]");
        assert_eq!(sanitize("[-1e400]"), "[null]");
        // ...but an underflowing literal is a finite 0.0 in both languages, so
        // it is left exactly as written.
        assert_eq!(sanitize("[1e-400]"), "[1e-400]");
    }

    #[test]
    fn sanitize_keeps_arbitrary_precision_integers_as_f64() {
        // Python converts these to the nearest f64 rather than discarding them.
        // The rewritten text is compared numerically rather than character for
        // character: `Value::to_string` spells the exponent Rust's way
        // (`e+19`), which is valid JSON reading back as the same f64.
        assert_eq!(sanitize("[9223372036854775807]"), "[9223372036854775807]");

        for (literal, expected) in [
            ("18446744073709551616", 18446744073709551616_f64),
            ("123456789012345678901234567890", 123456789012345678901234567890_f64),
        ] {
            let text = format!("[{literal}]");
            let rewritten = sanitize(&text);
            assert!(
                !matches!(rewritten, Cow::Borrowed(_)),
                "{literal} is outside i64/u64 and must be rewritten"
            );
            assert_eq!(
                parse_lenient(&rewritten).unwrap()[0].as_f64(),
                Some(expected),
                "{literal} must become the nearest f64"
            );
        }
    }

    #[test]
    fn parse_lenient_reads_what_python_reads() {
        assert_eq!(parse_lenient("[NaN]").unwrap().to_string(), "[null]");
        assert_eq!(parse_lenient("[Infinity]").unwrap().to_string(), "[null]");
        assert_eq!(
            parse_lenient("[1e400]").unwrap().to_string(),
            "[null]",
            "1e400 must not take the file down"
        );
        let big = parse_lenient("[123456789012345678901234567890]").unwrap();
        assert_eq!(big[0].as_f64(), Some(1.2345678901234568e29));
    }

    #[test]
    fn parse_lenient_still_rejects_malformed_input() {
        assert!(parse_lenient("{not json at all").is_err());
        assert!(parse_lenient(r#"[{"a": 1}] trailing"#).is_err());
        assert!(parse_lenient("").is_err());
    }

    #[test]
    fn nesting_deeper_than_the_clamp_is_read_as_null() {
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        let clamped = clamp_depth(&deep);
        assert!(
            !matches!(clamped, Cow::Borrowed(_)),
            "a 200-deep document must be clamped"
        );
        // The clamp is textual, so the shape of what it produced is checked
        // rather than assumed: an over-deep value becomes `null` in place.
        assert_eq!(clamped.matches('[').count(), MAX_VALUE_DEPTH);
        assert_eq!(clamped.matches("null").count(), 1);
        assert!(parse_lenient(&deep).is_ok());
    }

    #[test]
    fn the_clamp_leaves_ordinary_documents_alone() {
        // The clamp must be invisible for anything the app actually writes: a
        // `Game` is three levels at its deepest.
        let text = r#"[{"id": "x", "name": "A [bracket] in a name", "tags": [[]]}]"#;
        assert!(matches!(clamp_depth(text), Cow::Borrowed(_)));
    }

    #[test]
    fn clamp_depth_ignores_brackets_inside_strings() {
        // A name full of brackets is not nesting. If `clamp_depth` counted
        // them, a long enough name would be silently truncated to `null`.
        let name = "[".repeat(500);
        let text = format!(r#"[{{"name": "{name}"}}]"#);
        assert!(matches!(clamp_depth(&text), Cow::Borrowed(_)));
        assert_eq!(
            parse_lenient(&text).unwrap()[0]["name"].as_str().unwrap().len(),
            500
        );
    }

    #[test]
    fn a_pathologically_deep_document_does_not_overflow_the_stack() {
        // The regression guard for the bug this clamp replaced. Raising
        // serde_json's 128-deep limit instead (`unbounded_depth` +
        // `disable_recursion_limit`) parses with stack proportional to nesting,
        // so a deep enough file aborts the whole process with `SIGABRT` —
        // uncatchable, and in a process that also owns the user's window. A
        // 20,000-deep document is 20x past where that aborted.
        //
        // This test *is* the assertion: if the clamp regresses to a recursive
        // parse, the harness dies here rather than reporting a failure. That is
        // the point — there is no way to catch it and check for it.
        let deep = format!("{}1{}", "[".repeat(20_000), "]".repeat(20_000));
        let parsed = parse_lenient(&deep).expect("depth must not be a parse error");
        // Walk down to the clamp boundary and confirm the rest is `null`
        // rather than a truncated or fabricated value.
        let mut node = &parsed;
        for _ in 1..MAX_VALUE_DEPTH {
            node = &node[0];
        }
        assert!(node[0].is_null(), "past the clamp the value is `null`");

        // And a deep document shaped like a real one keeps its games, which is
        // what D-21 is actually about.
        let junk = "[".repeat(20_000);
        let text = format!(
            r#"[{{"name": "Deep", "junk": {junk}{}}}, {{"name": "Sib"}}]"#,
            "]".repeat(20_000)
        );
        let value = parse_lenient(&text).expect("a deep junk key must not fail the parse");
        assert_eq!(value.as_array().unwrap().len(), 2);
        assert_eq!(value[0]["name"], "Deep");
        assert_eq!(value[1]["name"], "Sib");
    }

    #[test]
    fn python_escaping_matches_ensure_ascii() {
        assert_eq!(escape_python_string("Pokémon"), r#""Pok\u00e9mon""#);
        assert_eq!(escape_python_string("—"), r#""\u2014""#);
        assert_eq!(escape_python_string("日本語"), r#""\u65e5\u672c\u8a9e""#);
        assert_eq!(escape_python_string("plain"), r#""plain""#);
        // BMP-adjacent boundaries: DEL is escaped, `~` is not.
        assert_eq!(escape_python_string("\u{7f}"), r#""\u007f""#);
        assert_eq!(escape_python_string("~"), r#""~""#);
        // The five short escapes, and quote/backslash.
        assert_eq!(
            escape_python_string("\"\\\n\r\t\u{8}\u{c}"),
            r#""\"\\\n\r\t\b\f""#
        );
        assert_eq!(escape_python_string("\u{1}"), r#""\u0001""#);
    }

    #[test]
    fn python_escaping_uses_surrogate_pairs_above_the_bmp() {
        // Python: json.dumps("\U0001F600") == '"\\ud83d\\ude00"'
        assert_eq!(escape_python_string("\u{1f600}"), r#""\ud83d\ude00""#);
    }

    #[test]
    fn output_uses_python_layout_and_escaping() {
        // A struct, not a `json!` object: the field order under test is the
        // declaration order here, which is what the real writers rely on. A
        // `serde_json::Map`'s order is not a property of this crate at all —
        // `preserve_order` is off in `cargo test -p gamehandler-core` (BTreeMap,
        // alphabetical) and on in a workspace-wide `cargo test`, because
        // `cosmic-theme` enables it and Cargo unifies features. Asserting on a
        // map would make the test's result depend on who else is in the build.
        #[derive(serde::Serialize)]
        struct Sample {
            name: &'static str,
            n: i32,
            b: bool,
            empty: [i32; 0],
        }

        let value = Sample {
            name: "Pokémon — ✓",
            n: 1,
            b: true,
            empty: [],
        };
        let text = to_python_string(&value).unwrap();
        assert_eq!(
            text,
            concat!(
                "{\n",
                "  \"name\": \"Pok\\u00e9mon \\u2014 \\u2713\",\n",
                "  \"n\": 1,\n",
                "  \"b\": true,\n",
                "  \"empty\": []\n",
                "}"
            ),
            "indent=2, no trailing newline, empty array inline"
        );
    }

    #[test]
    fn realistic_timestamps_round_trip_bit_for_bit() {
        // The regression guard for `float_roundtrip` (see `crates/core/Cargo.toml`).
        //
        // `serde_json`'s default f64 reader is fast but not correctly rounded:
        // on values shaped like a real `time.time()` it returns a result one
        // ULP off roughly a fifth of the time. That is invisible in a UI and
        // fatal to D-15 — the port would read a timestamp Python wrote and
        // re-save different bytes, so every library file would churn on every
        // save. Removing the feature must make this test fail.
        //
        // The values are seeded so the test is deterministic, and compared by
        // bit pattern rather than by `==` so a 1-ULP error cannot pass as an
        // approximation. (`==` would already catch it — 1 ULP is a different
        // f64 — but bits make the intent unmistakable and catch `-0.0` too.)
        let mut state = 0x6772_6864_5f72_6f75u64;
        let mut expected = Vec::with_capacity(2_000);
        for _ in 0..2_000 {
            // A plain LCG: no dependency, and a fixed seed, so the corpus never
            // moves between runs.
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            // Uniform in [1.5e9, 2.0e9) — the shape of a timestamp from 2017
            // onwards, which is where `time.time()` actually lands.
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            expected.push(1.5e9 + unit * 5.0e8);
        }

        // Through the real writer and the real reader, not `to_string`.
        let text = to_python_string(&expected).unwrap();
        let Value::Array(parsed) = parse_lenient(&text).unwrap() else {
            panic!("a list of floats must parse back to a list");
        };
        assert_eq!(parsed.len(), expected.len());

        let mut mismatches = 0usize;
        for (index, (item, expected)) in parsed.iter().zip(&expected).enumerate() {
            let got = item
                .as_f64()
                .unwrap_or_else(|| panic!("entry {index} is not a number"));
            if got.to_bits() != expected.to_bits() {
                mismatches += 1;
                if mismatches == 1 {
                    eprintln!("first mismatch at {index}: wrote {expected}, read {got}");
                }
            }
        }
        assert_eq!(
            mismatches, 0,
            "{mismatches}/{} timestamps came back off by one ULP — is the \
             `float_roundtrip` feature still enabled on serde_json?",
            expected.len()
        );
    }

    #[test]
    fn output_of_an_empty_list_is_bare_brackets() {
        assert_eq!(to_python_string(&Vec::<u8>::new()).unwrap(), "[]");
        let empty: [u8; 0] = [];
        assert_eq!(to_python_string(&empty).unwrap(), "[]");
    }

    #[test]
    fn write_python_file_creates_parents_and_leaves_no_temporary() {
        let directory = std::env::temp_dir().join(format!("gh-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("nested").join("settings.json");

        write_python_file(&path, &json!({"name": "Pokémon"})).expect("write should succeed");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"name\": \"Pok\\u00e9mon\"\n}"
        );

        let files: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, ["settings.json"], "the .tmp file must be renamed away");

        let _ = std::fs::remove_dir_all(&directory);
    }
}
