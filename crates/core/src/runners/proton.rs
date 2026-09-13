//! Proton and Wine build management: listing, installing and removing builds
//! published on GitHub. Port of `runners.py:776-936` (the `ProtonManager`
//! class), `_write_metadata` (`runners.py:709`) and `_rename_noreplace`
//! (`runners.py:663`).
//!
//! # What this slice covers
//!
//! Both halves. The *pure* layer — the GitHub payload parser, the header
//! parser, the installed-build probe, and the injected [`HttpClient`] of D-26 —
//! is what can be exercised without a network or a live install, and it was
//! pinned first, so the staging logic below is written against a parser already
//! known to agree with Python. The *filesystem* half is [`install`],
//! [`uninstall`], [`resolve_staged`], [`write_metadata`] and
//! [`rename_noreplace`].
//!
//! The install is a security boundary and its order is the property, so it is
//! stated once here: the archive lands in a private `0700` staging directory
//! **inside** the runners directory, is extracted there, the *extracted tree*
//! is validated (not the archive), metadata is written into the validated tree,
//! and only then is the tree renamed into place under a name that cannot
//! already exist. Nothing attacker-influenced is reachable under its final name
//! until every check has passed, and no partial state is ever visible there.
//!
//! # The two `proton` predicates, which are NOT the same predicate
//!
//! `runners.py` spells "is this a runner?" two ways and the difference is
//! observable, so both are reproduced rather than unified:
//! [`proton_entry_exists`] (`.exists()`, `runners.py:843`) and
//! [`is_staged_runner`] (`.is_file()`, `runners.py:855`, `:936`, `:946`,
//! `:659`). A tree whose `proton` is a **directory** is installed to one and
//! unresolvable to the other. It looks like an oversight in the reference and
//! may be one; it is on the path that decides whether the UI offers to install
//! a build that is already on disk, so it is kept and pinned rather than
//! tidied.
//!
//! # Deliberate divergences
//!
//! Five, and each is pinned by a test rather than left to a reader:
//!
//! 1. **`int()` accepts ASCII digits only.** Python's `int()` accepts any
//!    Unicode `Nd` decimal digit — measured, `int("٣")` is `3` and
//!    `int("１２")` is `12`. Rust's `char::to_digit` is ASCII-only, and the
//!    crate carries no Unicode tables by design (`Cargo.toml`: serde and
//!    serde_json are the entire general-purpose dependency set). So
//!    [`python_int`] rejects non-ASCII digits where Python accepts them. This
//!    is the one place the port leans **stricter**, and it matters only for a
//!    `Content-Length` or an asset `size` that a malicious or exotic server
//!    spelled in Arabic-Indic digits; every real server sends ASCII. Note this
//!    is the *opposite* direction from [`super::launch_opts`]'s documented
//!    `\d` superset, and deliberately so — see [`python_int`].
//! 2. **`size` is saturated to `i64`.** Python's `int` is unbounded, and a
//!    JSON `1e30` reaches `int(1e30)` = 1000000000000000019884624838656, which
//!    no `i64` holds. `ReleaseInfo::size` is `i64`, so the port saturates.
//!    Unobservable except in `size_mb()`, a display value for a release whose
//!    declared size is nonsense to begin with; every *decision* taken on
//!    `size` compares against a 2 GiB cap, which saturation cannot move.
//! 3. **A non-string `tag` is coerced.** `tag = release.get("tag_name") or
//!    release.get("name") or ""` has no `str()` around it, unlike the two
//!    fields beside it, so Python carries a JSON number or list into a
//!    `str`-annotated dataclass field and fails later, at whichever call site
//!    happens to use a string method. The port coerces at the boundary with
//!    [`python_str`], which is what the annotation claims.
//! 4. **An undecodable release body is an error, not a crash.** Python's
//!    `resp.read().decode("utf-8")` raises an uncaught `UnicodeDecodeError`
//!    out of `fetch_available`. The port returns [`RunnerError::Http`]. Same
//!    class as the DXVK-marker divergence in `launch_opts`: Python's failure is
//!    a traceback, ours is a message the caller can render.
//! 5. **The staging name is drawn here, not by `tempfile`.** Python uses
//!    `TemporaryDirectory(prefix=".install-")`; the port draws the same
//!    `.install-<8 chars>` shape from the same `[a-z0-9_]` alphabet, from the
//!    process id and a clock, with no new dependency. The name is transient and
//!    never observed — it is not read back, matched on, or shown to a user — so
//!    the entropy source is not part of the contract. What *is* kept is that a
//!    collision advances to the next name rather than clearing the occupied one:
//!    the parent directory holds the user's installed runners, and an
//!    unexpectedly-occupied name may not be ours to delete.

use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;
use serde_json::Value;

use super::archive::{
    MAX_RUNNER_ARCHIVE_BYTES, METADATA_NAME, safe_install_id, sanitise_release_tag,
};
use super::families::{
    ReleaseInfo, RunnerFamily, family_by_id, find_wine_binary, is_truthy, pick_asset, python_str,
};
use super::{RunnerError, SYSTEM_WINE, USER_AGENT, read_metadata};

/// The default family, matching Python's `family_by_id("proton-ge")` default
/// on every entry point that takes a family. A release with no family named is
/// a Proton-GE release, never an unknown one.
pub const DEFAULT_FAMILY_ID: &str = "proton-ge";

/// The header GitHub wants for its releases API, and the one the download
/// wants. Kept as constants so the two call sites cannot drift.
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

// ---------------------------------------------------------------------------
// The injected HTTP client (DECISIONS D-26)
// ---------------------------------------------------------------------------

/// What a completed [`HttpClient::get`] call learned about the response.
///
/// `content_length` is the **raw header text**, not a number, and that is
/// deliberate: Python parses it with `int()` inside a `try`/`except` that turns
/// a bad value into a specific `RuntimeError`, so the parse is part of the
/// behaviour being ported and belongs in [`parse_content_length`] where it is
/// tested — not hidden inside a client implementation whose only job is
/// transport. A client that pre-parsed would make that error path
/// unreachable, and D-26 exists precisely so the error paths stay reachable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseHead {
    /// The `Content-Length` header verbatim, or `None` if it was absent.
    pub content_length: Option<String>,
    /// The URL the response actually came from, **after every redirect**.
    ///
    /// Required rather than optional, and empty is not a valid answer: the one
    /// caller that reads it is an allowlist check
    /// ([`crate::installers`]'s download origin validation), and a field that
    /// could be `None` would need a rule for what `None` means. The only two
    /// candidates are "trust the request URL instead" — which turns a redirect
    /// to an attacker's host into an accepted download — and "reject", which is
    /// the same as an empty string here. Making it a plain `String` removes the
    /// question: a client that cannot report the final URL reports `""`, and
    /// `""` is not in any installer's `allowed_hosts`, so it fails closed.
    ///
    /// [`Default`] gives `""`, which is the same fail-closed direction.
    pub final_url: String,
}

/// A blocking HTTP GET, injected rather than imported (D-26).
///
/// `core` deliberately has no networking dependency, so the concrete client
/// lives in the binary crate and this trait is the seam.
///
/// # Why the head arrives *before* the body, as its own callback
///
/// The first version of this trait streamed the body through one callback and
/// returned the [`ResponseHead`] when the transfer completed. That reads
/// naturally and it **cannot express what `install` does**: Python reads
/// `Content-Length` off the response *before* the read loop, because it uses it
/// for two things that both have to happen before any byte is kept —
///
/// * `total = declared or release.size`, the denominator of every progress
///   report, so the percentage is wrong for the whole transfer if it arrives
///   late; and
/// * an immediate abort if `declared` exceeds the size cap, which is the cheap
///   check that makes the expensive one (counting bytes as they arrive)
///   unnecessary in the case the server is honest about being hostile.
///
/// Streaming the body through a callback and returning the head afterwards
/// makes both impossible, and no amount of care inside `install` fixes it —
/// the information does not exist yet. So the head is delivered by its own
/// callback first, and *both* callbacks can return `Err` to abandon the
/// transfer: `on_head` for a declared length that is already too large, `sink`
/// for a body that turns out to be. Python gets both from exceptions raised
/// inside its `with` block; these are the same two exits.
///
/// # Why the final URL rides in the head too
///
/// It was added third, for `installers.download_installer`, and it belongs in
/// the head for the same reason the length does: Python validates the origin
/// (`_validate_download_origin`, `installers.py:551-558`) against
/// `resp.geturl()` **before its read loop**, so no byte of a redirect to an
/// untrusted host is ever written. A client that reported the final URL only
/// on completion would still let the caller delete what it had written — but
/// only after receiving up to the 1 GiB size cap from a host the allowlist
/// never approved. The head is the point at which the answer exists and
/// nothing has been kept, so it is where the answer goes.
pub trait HttpClient {
    /// GET `url`, then hand the response to two callbacks in order.
    ///
    /// `on_head` is called once, before any body byte. `sink` is then called
    /// with each chunk as it arrives. An `Err` from either abandons the
    /// transfer and is propagated unchanged, so a caller can stop a download it
    /// can already tell is unacceptable without receiving the rest of it.
    ///
    /// A non-success status or a transport failure is `Err`.
    fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
        on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError>;
}

/// GET `url` and collect the whole body into a `String`, erroring on invalid
/// UTF-8. The shape both `fetch_available`-style callers want.
pub fn get_text(
    client: &dyn HttpClient,
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<String, RunnerError> {
    let mut body = Vec::new();
    client.get(url, headers, timeout, &mut |_head| Ok(()), &mut |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    String::from_utf8(body).map_err(|_| RunnerError::Http {
        // Divergence 4: Python raises an uncaught `UnicodeDecodeError`.
        message: "Unexpected GitHub releases response".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Python's int()
// ---------------------------------------------------------------------------

/// Python's `int()` on a `str`, or `None` for the `ValueError` Python raises.
///
/// # The trap, measured
///
/// `int()` is far more permissive than a Rust `parse::<i64>()`, and every one
/// of these was confirmed by running CPython before this was written:
///
/// | input | Python |
/// |---|---|
/// | `" 12 "`, `"\t7"` | `12`, `7` — whitespace is trimmed |
/// | `"+12"`, `"-0"` | `12`, `0` — a sign is allowed |
/// | `"1_0"`, `"1_0_0"`, `"١٢_٣"` | `10`, `100`, `123` — PEP 515 underscores |
/// | `"_10"`, `"10_"`, `"1__0"` | `ValueError` — an underscore must sit *between* digits |
/// | `"1e3"`, `"12.0"`, `"0x10"` | `ValueError` — base 10 only, no float or hex form |
/// | `""` | `ValueError` |
/// | `"٣"`, `"１２"` | `3`, `12` — **Unicode `Nd` digits** |
///
/// # The trim set is `is_whitespace()`, and that is *not* the `isspace()` set
///
/// The tempting move is to reuse `launch_opts`' predicate here, since both are
/// porting "Python trims whitespace". That would be wrong, and measurably so.
/// Enumerating every code point for which `str.isspace()` is true and asking
/// `int()` to parse each one prefixed to a digit gives **25 accepted and 4
/// rejected**: the rejected four are exactly `U+001C..U+001F`, the C0
/// separators. So `int()` trims `str.isspace()` **minus** the separators —
/// which is precisely `char::is_whitespace()`, Rust's `White_Space` property,
/// with no adjustment at all.
///
/// The two modules therefore need *opposite* corrections, and this is the one
/// that needs none: `launch_opts::python_is_space` must **add** `U+001C..U+001F`
/// to port `str.isspace()`, and this must **not**, because `int()` does not use
/// `str.isspace()`. A shared predicate would be wrong in one module whichever
/// direction it leaned. (The test below pins all four separators as rejected,
/// so a future refactor that unifies the two fails here rather than in the
/// field.)
///
/// # Why this rejects `Nd` where `launch_opts` accepts it
///
/// `launch_opts::is_numeric_digit` is a documented *superset* of Python's `\d`
/// (it is `char::is_numeric`, i.e. `Nd ∪ Nl ∪ No`). That is safe there because
/// its caller only ever asks *whether* every character is a digit — nothing
/// needs a digit's **value**. Here the value is the whole point, and
/// `char::to_digit(10)` is ASCII-only: there is no `std` way to learn that `٣`
/// is 3 without a Unicode table, which this crate does not carry. So the two
/// predicates differ on purpose, and neither can be reused for the other:
/// acceptance-only and value-requiring are different questions. A port that
/// shared one predicate between them would silently report `int("٣") == 0`.
///
/// The direction of this divergence is toward rejection, so a hostile header
/// fails closed. The test below pins both that `٣` is rejected here *and* that
/// CPython accepts it, so the divergence cannot be mistaken for a bug in
/// either implementation.
pub fn python_int(text: &str) -> Option<i64> {
    // `char::is_whitespace()`, unmodified — see the measurement above. Not
    // `launch_opts::python_is_space`, which is a *different* set.
    let trimmed = text.trim_matches(char::is_whitespace);
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'+') => (false, &trimmed[1..]),
        Some(b'-') => (true, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if digits.is_empty() {
        return None;
    }

    // PEP 515: an underscore is allowed only between two digits. Tracked as a
    // small state machine rather than by stripping underscores first, because
    // stripping would accept `_10` and `1__0`, which CPython rejects.
    //
    // The accumulator is `i128`, not `i64`, and that is not incidental. An
    // `i64` accumulator saturates at `i64::MAX` *during* accumulation, so the
    // magnitude can never reach 2^63 — and `i64::MIN` is exactly -2^63. The
    // first version of this function accumulated into `i64` and returned
    // `-9223372036854775807` for `"-9223372036854775808"`, one off the real
    // value. It is unreachable (a negative size is rejected downstream either
    // way) but it is still wrong, and the differential table below caught it.
    let mut magnitude: i128 = 0;
    let mut previous_was_digit = false;
    let mut any_digit = false;
    for character in digits.chars() {
        if character == '_' {
            if !previous_was_digit {
                return None;
            }
            previous_was_digit = false;
            continue;
        }
        let value = character.to_digit(10)?;
        // Saturating rather than wrapping: Python is unbounded here, so a
        // hostile header cannot be reported as a *small* number by wrapping
        // into the negative — the direction that would slip a size check.
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(i128::from(value));
        previous_was_digit = true;
        any_digit = true;
    }
    // A trailing underscore leaves `previous_was_digit` false.
    if !any_digit || !previous_was_digit {
        return None;
    }
    // The two limits are asymmetric in magnitude: |i64::MIN| is one more than
    // i64::MAX, which is the case an `i64` accumulator cannot reach.
    const MAX_MAGNITUDE: i128 = i64::MAX as i128;
    const MIN_MAGNITUDE: i128 = -(i64::MIN as i128);
    if negative {
        if magnitude >= MIN_MAGNITUDE {
            Some(i64::MIN)
        } else {
            Some(-(magnitude as i64))
        }
    } else if magnitude > MAX_MAGNITUDE {
        Some(i64::MAX)
    } else {
        Some(magnitude as i64)
    }
}

/// The `int(value or 0)` coercions in `parse_releases`, over a JSON value.
///
/// Python applies `or 0` *before* `int()`, and `or` is a truthiness test, not a
/// null test — so the falsy values that become `0` include `false`, `0`, `""`,
/// `[]` and `{}`, each confirmed by execution. Everything else goes through
/// `int()`, which means:
///
/// * an integer passes through;
/// * a **float truncates toward zero** (`1.5` is `1`), it does not round;
/// * `true` is `1` and `false` is `0`, because `bool` is an `int` in Python;
/// * a string is parsed by [`python_int`], so `"12"` is `12` and `"1e3"` is a
///   `ValueError`;
/// * a **non-empty** list or object raises `TypeError`, uncaught.
///
/// The last is the one worth stating: a malformed GitHub asset whose `size` is
/// an object does not degrade to `0`, it aborts the listing. That is Python's
/// behaviour, so it is this port's, and [`Result`] is the shape that says so
/// rather than a silent `unwrap_or(0)`.
pub fn int_from_json(value: &Value) -> Result<i64, RunnerError> {
    if !is_truthy(value) {
        return Ok(0);
    }
    match value {
        // `bool` is an `int` subclass in Python, so `int(True)` is 1. Only
        // `true` reaches here — `false` is falsy and returned `0` above — but
        // both arms are written, because a reader checking "did we handle
        // bool?" should find the answer rather than have to infer it from
        // `is_truthy`.
        Value::Bool(flag) => Ok(i64::from(*flag)),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Ok(integer);
            }
            if let Some(unsigned) = number.as_u64() {
                // Above `i64::MAX`: Python holds it, `i64` cannot. Saturate,
                // per divergence 2 in the module docs.
                return Ok(i64::try_from(unsigned).unwrap_or(i64::MAX));
            }
            let float = number.as_f64().unwrap_or(0.0);
            // `int(1e30)` is exact and unbounded in Python. Saturating here is
            // divergence 2; `as` on an out-of-range float is already saturating
            // in Rust, and truncates toward zero for in-range values, which is
            // what `int()` does.
            Ok(float as i64)
        }
        Value::String(text) => python_int(text).ok_or_else(|| RunnerError::Http {
            message: format!("invalid literal for int() with base 10: '{text}'"),
        }),
        other => Err(RunnerError::Http {
            message: format!(
                "int() argument must be a string, a bytes-like object or a real \
                 number, not '{}'",
                json_type_name(other)
            ),
        }),
    }
}

/// The Python type name, for the `TypeError` message above.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// The `Content-Length` header, as Python reads it while installing.
///
/// `int(resp.headers.get("Content-Length", 0) or 0)`, inside a
/// `try`/`except (TypeError, ValueError)` that turns a bad value into
/// `RuntimeError("Runner download has an invalid Content-Length")`.
///
/// The `or 0` is what makes an **absent** header and an **empty** header
/// identical: both are `0`, not an error. Only a present, non-empty, unparseable
/// value is invalid — which is why `None` and `Some("")` both succeed here and
/// the test asserts them separately from the failing cases. A port that treated
/// an empty header as invalid would reject a legal response.
///
/// The `TypeError` arm of Python's handler is unreachable for a header (headers
/// are always `str`), and is ported as unreachable rather than as a branch —
/// the message would be the `ValueError` one either way, as both raise the same
/// `RuntimeError`.
pub fn parse_content_length(header: Option<&str>) -> Result<i64, RunnerError> {
    match header {
        None | Some("") => Ok(0),
        Some(text) => python_int(text).ok_or_else(|| RunnerError::Http {
            message: "Runner download has an invalid Content-Length".to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Releases
// ---------------------------------------------------------------------------

/// Resolve the `family` argument the way all three entry points do.
///
/// `family_by_id(family)` for a string, the default for `None`. Python raises
/// `KeyError("Unknown runner family: {id}")` for an unknown id; `core`'s
/// `family_by_id` returns the message as a `String`, and it is carried through
/// unchanged so the text stays byte-identical.
fn resolve_family(family: Option<&str>) -> Result<&'static RunnerFamily, RunnerError> {
    family_by_id(family.unwrap_or(DEFAULT_FAMILY_ID))
        .map_err(|message| RunnerError::Http { message })
}

/// Turn a GitHub releases payload into [`ReleaseInfo`] values.
/// `ProtonManager.parse_releases` (`runners.py:787`).
///
/// A release with no matching asset is **skipped, not an error**: the payload
/// lists every asset GitHub holds, including source tarballs and signatures, so
/// most releases in a listing are expected to be unusable. That is why this
/// returns fewer entries than it is given, and why the corpus includes payloads
/// where every release is skipped.
///
/// The `tag` fallback chain is `tag_name or name or ""` — truthiness, so a
/// literal `""` falls through to `name` exactly as a missing key does. An entry
/// with no asset is skipped *before* the tag matters, so a release with neither
/// key never contributes a nameless entry.
pub fn parse_releases(
    data: &[Value],
    family: Option<&str>,
) -> Result<Vec<ReleaseInfo>, RunnerError> {
    let resolved = resolve_family(family)?;

    let mut releases = Vec::new();
    for release in data {
        let tag_value = release.get("tag_name").filter(|value| is_truthy(value));
        let tag = match tag_value.or_else(|| release.get("name").filter(|v| is_truthy(v))) {
            Some(value) => python_str(value),
            None => String::new(),
        };

        // `release.get("assets") or []`, which is a *truthiness* test rather
        // than an array check: an absent, null, empty-array, empty-object or
        // empty-string `assets` all arrive here as "no assets". A **truthy
        // non-list** (`"assets": "x"`) is different — Python's `or` keeps it
        // and `pick_asset` then dies on `str.get` — so it is reported rather
        // than skipped, on the same reasoning as the non-array payload check in
        // `fetch_available`: treating it as "no assets" would render a
        // malformed payload as "this family publishes no builds".
        let assets: &[Value] = match release.get("assets") {
            Some(value) if is_truthy(value) => match value {
                Value::Array(items) => items.as_slice(),
                _ => {
                    return Err(RunnerError::Http {
                        message: "Unexpected GitHub releases response".to_string(),
                    });
                }
            },
            _ => &[],
        };
        let Some(asset) = pick_asset(assets, resolved) else {
            continue;
        };

        releases.push(ReleaseInfo {
            tag,
            name: str_or_empty(asset.get("name")),
            download_url: str_or_empty(asset.get("browser_download_url")),
            size: int_from_json(asset.get("size").unwrap_or(&Value::Null))?,
            family_id: resolved.id.to_string(),
        });
    }
    Ok(releases)
}

/// Python's `str(value or "")` for one field of a JSON object.
///
/// The `or ""` is a truthiness test that runs **before** `str()`, so this is
/// not `python_str` with a default. A missing `name`, a `null` one, and an
/// empty-string one all become `""`; only a truthy value is stringified.
///
/// Getting the order wrong is a silent, plausible-looking bug: `python_str` of
/// a missing key is `"None"`, so applying it first would file every nameless
/// asset under a runner literally called `None`. The test below pins the two
/// apart by name, because the wrong version is the one a reader would write.
fn str_or_empty(value: Option<&Value>) -> String {
    match value.filter(|value| is_truthy(value)) {
        Some(value) => python_str(value),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Fetching (D-26's injected client)
// ---------------------------------------------------------------------------

/// The `Accept` and `User-Agent` headers GitHub's API wants.
fn release_headers() -> Vec<(&'static str, &'static str)> {
    vec![("Accept", GITHUB_ACCEPT), ("User-Agent", USER_AGENT)]
}

/// Fetch the available releases for a family, newest first, capped at `limit`.
/// `ProtonManager.fetch_available` (`runners.py:815`).
///
/// The `limit` is applied **after** parsing, not before: Python parses the whole
/// payload and slices. So a release skipped for having no matching asset does
/// not consume one of the `limit` slots — asking for 15 returns the first 15
/// *usable* releases, which can come from arbitrarily far down the payload.
/// Slicing first would return fewer than requested and is the plausible way to
/// get this wrong; the corpus case where the first entries are unusable pins it.
///
/// A payload that is not a JSON **array** is an error rather than an empty
/// listing — Python checks `isinstance(data, list)` explicitly, because a
/// GitHub error object (`{"message": "Not Found"}`) is valid JSON and would
/// otherwise parse to zero releases, which the UI would render as "this family
/// has no builds" instead of surfacing the failure.
pub fn fetch_available(
    client: &dyn HttpClient,
    family: Option<&str>,
    limit: usize,
    timeout: Duration,
) -> Result<Vec<ReleaseInfo>, RunnerError> {
    let resolved = resolve_family(family)?;
    let headers = release_headers();
    let text = get_text(client, &resolved.releases_url(), &headers, timeout)?;

    let data: Value = serde_json::from_str(&text).map_err(|_| RunnerError::Http {
        message: "Unexpected GitHub releases response".to_string(),
    })?;
    let Value::Array(releases) = data else {
        return Err(RunnerError::Http {
            message: "Unexpected GitHub releases response".to_string(),
        });
    };
    let mut parsed = parse_releases(&releases, Some(resolved.id))?;
    parsed.truncate(limit);
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Installed-build probes
// ---------------------------------------------------------------------------

/// Is a build with this tag installed?
/// `ProtonManager.is_installed` (`runners.py:836`).
///
/// Two directory layouts are recognised, and the second is the whole reason
/// this is not a `Path::exists`:
///
/// * The current scheme, where the directory name is the release's
///   [`install_id`](ReleaseInfo::install_id) — namespaced by family for
///   anything but Proton-GE, so two families publishing the same tag cannot
///   collide.
/// * The **earlier, non-namespaced scheme**, whose directories are found by
///   reading each one's `.gamehandler.json` and matching on the recorded
///   `family` and `tag`. Without this, every runner a user installed before the
///   namespacing change would read as not-installed, and the UI would offer to
///   install a build that is already on disk.
///
/// The metadata path also re-checks that the directory is a *usable* runner
/// (`find_wine_binary` or a `proton` file), so a directory that merely carries
/// matching metadata — a half-removed install, an unrelated folder that happens
/// to hold a copied metadata file — is not reported as installed.
///
/// `family_id` being absent means the caller is asking about a bare tag with no
/// family, so only the un-namespaced name is tried and the metadata scan is
/// skipped: with no family to match on, the scan has nothing to compare.
pub fn is_installed(runners_directory: &Path, tag: &str, family_id: Option<&str>) -> bool {
    let target = match family_id {
        Some(family_id) => {
            let Ok(install_id) = super::families::install_id_for(tag, family_id) else {
                // Python builds a `ReleaseInfo` and reads `.install_id`, which
                // raises the same error the caller would have hit at install
                // time. A tag that cannot form a directory name cannot be
                // installed under it, so it is not installed.
                return false;
            };
            runners_directory.join(safe_install_id(&install_id).unwrap_or(install_id))
        }
        None => match sanitise_release_tag(tag) {
            Ok(name) => runners_directory.join(name),
            Err(_) => return false,
        },
    };

    if proton_entry_exists(&target) {
        return true;
    }

    let Some(family_id) = family_id else {
        return false;
    };
    if !runners_directory.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(runners_directory) else {
        return false;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        let metadata = read_metadata(&child);
        // `metadata.get("family") == family_id` — Python's `==`, so only a
        // *string* equal to the id matches, which is why this is not a
        // `python_str` comparison: that would make the number `5` match the
        // family id `"5"`, which Python does not. A metadata file written by
        // this app always holds strings, so the difference is only reachable
        // from a hand-edited or foreign one.
        let matches_family =
            matches!(metadata.get("family"), Some(Value::String(text)) if *text == family_id);
        let matches_tag = matches!(metadata.get("tag"), Some(Value::String(text)) if *text == tag);
        if matches_family && matches_tag && is_staged_runner(&child) {
            return true;
        }
    }
    false
}

/// [`is_installed`] for a release, which is the form every caller has.
pub fn is_release_installed(runners_directory: &Path, release: &ReleaseInfo) -> bool {
    is_installed(runners_directory, &release.tag, Some(&release.family_id))
}

/// Was a `proton` entry found by `Path::exists` semantics — i.e. may
/// `is_installed`'s **first** check accept a tree?
///
/// Python spells this predicate twice with two different predicates, and the
/// difference is observable, so it is reproduced rather than unified:
///
/// * `runners.py:843` — the tree named by the release's `install_id` — uses
///   `(target / "proton").exists()`, which **follows symlinks and is false for
///   a broken one**.
/// * `runners.py:855` (the legacy-metadata scan), `:936` and `:946`
///   (`_resolve_staged`) and `:659` (`_validate_staged_runner`) all use
///   `.is_file()`, which is false for a directory called `proton` and false for
///   a symlink to a directory.
///
/// So a tree whose `proton` is a **directory** is "installed" to the first
/// check and "not a runner" to the other four. That looks like an oversight and
/// it may well be one, but it is on the path that decides whether the UI offers
/// to install a build that is already on disk, and the two readers of this
/// module must not quietly disagree about it. Both spellings are therefore
/// kept, with a test pinning each.
///
/// `find_wine_binary` is `.exists()`-based in both, so only the `proton` half
/// differs.
fn proton_entry_exists(path: &Path) -> bool {
    find_wine_binary(path).is_some() || path.join("proton").exists()
}

/// Was a `proton` entry found by `Path::is_file` semantics?
///
/// The `.is_file()` spelling, used by `_resolve_staged` and the legacy scan.
/// See [`proton_entry_exists`] for why both exist.
fn is_staged_runner(path: &Path) -> bool {
    find_wine_binary(path).is_some() || path.join("proton").is_file()
}

// ---------------------------------------------------------------------------
// The filesystem half: stage, validate, rename into place
// ---------------------------------------------------------------------------

/// A staging directory inside the runners directory that removes itself.
///
/// Port of `tempfile.TemporaryDirectory(dir=runners_directory, prefix=".install-")`.
/// Three properties matter and all three are load-bearing rather than
/// incidental:
///
/// * **It is inside `runners_directory`.** Not tidiness — the final step is a
///   rename, and a rename across filesystems is not atomic and fails outright
///   on some pairs. Staging beside the destination is what makes
///   [`rename_noreplace`] the atomic operation the install relies on.
/// * **It is removed on every exit path**, including the error ones, which is
///   what `TemporaryDirectory`'s context manager does and what `Drop` does
///   here. A failed install must not leave a half-extracted tree behind for the
///   next `is_installed` scan to find.
/// * **It is created `0700`.** The tree is attacker-influenced and is validated
///   *after* extraction, so between those two moments it must not be readable
///   by anyone else.
///
/// The name is `tempfile`'s own `mkdtemp` shape (`.install-<8 random chars>`)
/// drawn from the same `[a-z0-9_]` alphabet, so an interrupted staging
/// directory is recognisable to a human reading `runners/` and indistinguishable
/// from one Python would have left.
///
/// **A collision advances to the next name; it never clears the existing
/// one.** `mkdtemp` retries and eventually raises, and it does not delete
/// anything on the way — which matters here because the parent directory is
/// shared with the user's installed runners and a name that is unexpectedly
/// occupied may not be ours to remove.
struct StagingDirectory {
    path: PathBuf,
}

impl StagingDirectory {
    fn create(parent: &Path) -> Result<Self, RunnerError> {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";

        // `getrandom` is not a dependency and this does not need cryptographic
        // randomness: the name only has to not collide with a sibling. The
        // process id and a nanosecond clock are enough to separate concurrent
        // installs, and the counter separates sequential ones inside a process.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let mut seed = {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.subsec_nanos() as u64)
                .unwrap_or(0);
            (u64::from(std::process::id()) << 32) ^ nanos
        };

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        let mut last_error = None;
        for _ in 0..1000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407)
                ^ COUNTER.fetch_add(1, Ordering::Relaxed);
            let suffix: String = (0..8)
                .map(|index| {
                    let slot = ((seed >> (index * 6)) & 0x3f) as usize;
                    ALPHABET[slot % ALPHABET.len()] as char
                })
                .collect();
            let path = parent.join(format!(".install-{suffix}"));
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(RunnerError::Io(last_error.unwrap_or_else(|| {
            std::io::Error::other("could not create a staging directory")
        })))
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        // Best effort: a failure here must not mask the error that caused the
        // drop, and an install that succeeded has already moved its tree out.
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Atomically rename `source` to `target`, which must not already exist.
/// `_rename_noreplace` (`runners.py:663`).
///
/// `RENAME_NOREPLACE` is the whole point: a plain `rename` replaces an existing
/// destination *silently*, so two installs racing on the same release would
/// produce one good tree and one interleaved with the other's files, and a
/// user's existing runner could be destroyed by a name collision. With
/// `NOREPLACE` the loser gets `EEXIST` and nothing is lost.
///
/// DECISIONS D-25: `rustix::fs::renameat_with` rather than `libc::renameat2`,
/// because the workspace denies `unsafe_code` and this is a security boundary —
/// an `unsafe` block here would need its invariant restated and maintained,
/// where `rustix` already encapsulates it.
///
/// Python looks the symbol up at runtime and raises
/// `"Atomic no-replace runner installation is unavailable"` when `libc` has no
/// `renameat2`. That arm is unreachable here by construction: `rustix` links
/// the call, so a kernel without it is a link-time problem rather than a
/// runtime branch. Recorded rather than ported, because a port of it would be
/// dead code that no test could reach honestly.
fn rename_noreplace(source: &Path, target: &Path) -> Result<(), RunnerError> {
    match renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => Err(RunnerError::AlreadyInstalled {
            id: target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }),
        Err(error) => Err(std::io::Error::from_raw_os_error(error.raw_os_error()).into()),
    }
}

/// Write the metadata a later `is_installed` reads. `_write_metadata`
/// (`runners.py:709`).
///
/// Four fields in this order, `json.dump(.., indent=2)` and a trailing newline,
/// so the bytes match Python's exactly. The order comes from a struct's
/// declaration order and **not** from a `serde_json::Map`, which is the trap
/// D-33 is about: a map's order is alphabetical or insertion-ordered depending
/// on whether `serde_json/preserve_order` got unified in, so a map here would
/// write different bytes in the two build configurations.
///
/// `asset` and `source` are the *asset* name and the family's `github` slug,
/// not the runner's directory name — this file describes where the build came
/// from, which is what makes it usable for the legacy-directory scan in
/// [`is_installed`].
///
/// `open("x")` is exclusive-create, so a second write to the same tree is an
/// error rather than a silent clobber. The tree is freshly extracted, so that
/// can only happen if the archive itself contained a `.gamehandler.json` —
/// which [`super::archive::validate_staged_runner`] rejects first, by name.
#[derive(serde::Serialize)]
struct Metadata<'a> {
    family: &'a str,
    tag: &'a str,
    asset: &'a str,
    source: &'a str,
}

fn write_metadata(root: &Path, release: &ReleaseInfo) -> Result<(), RunnerError> {
    // `release.family.github`, and the property **raises** for an unknown
    // family. So an install of a release from a family this build does not know
    // fails *here* — after extraction, at metadata time — rather than writing a
    // metadata file with an empty `source` that the legacy-directory scan would
    // then match on. Unreachable from the catalogue, which only ever builds
    // releases from families that exist, but reachable from a caller that
    // constructs a `ReleaseInfo` by hand, and the two outcomes are not
    // equivalent: one is a failed install, the other a corrupt record.
    let family = release
        .family()
        .map_err(|message| RunnerError::Http { message })?;
    let payload = Metadata {
        family: &release.family_id,
        tag: &release.tag,
        asset: &release.name,
        source: family.github,
    };
    let mut text = crate::json::to_python_string(&payload)
        .map_err(|error| RunnerError::Io(std::io::Error::other(error)))?;
    text.push('\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(METADATA_NAME))?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// Select a usable root from the immediate, non-symlink entries of the staged
/// archive. `ProtonManager._resolve_staged` (`runners.py:931`).
///
/// Three outcomes, and the middle one is the interesting rule:
///
/// * The extraction root is **itself** a runner (`bin/wine` somewhere below it,
///   or a `proton` file at the top) — some Wine archives are rootless — so the
///   whole extracted tree is kept rather than guessing one child to move.
/// * Any top-level entry is a **symlink** → refuse. Not because this link
///   escapes, but because a rootless archive must be moved wholesale and a
///   symlink among the entries means the "pick one child" path below would be
///   renaming a link whose target may sit outside the tree. The re-check inside
///   [`super::archive::validate_staged_runner`] happens against the tree as it
///   will land; this is the earlier, cheaper refusal.
/// * Exactly one **directory** that is a runner and whose name is a safe
///   install id → that directory. More than one is ambiguous, and none is a
///   failure; both raise rather than guessing, because guessing wrong installs
///   the wrong tree or an empty one.
///
/// The `safe_install_id` call on each directory's name is a *validation*, and
/// its result is discarded — the name it returns is not the one used. A
/// directory whose name cannot be an install id (empty, `.`, `..`, containing a
/// separator, or **starting with a dot**) is therefore a **hard error**, not a
/// skipped entry: an archive that ships `.wine/bin/wine` alongside a real
/// `GE-Proton9-5/` fails rather than silently installing the second. Python
/// raises and so does this. It is easy to "improve" into a `continue` while
/// porting, and doing so would accept an archive the reference refuses.
///
/// The rootless check comes **before** the symlink refusal, which is the order
/// that matters: a rootless archive whose own top level contains a symlink is
/// kept wholesale and the links are judged later, by
/// [`super::archive::validate_staged_runner`], against the tree as it lands.
pub fn resolve_staged(extraction_root: &Path) -> Result<PathBuf, RunnerError> {
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(extraction_root)? {
        entries.push(entry?.path());
    }
    // `sorted(key=lambda path: path.name)` — by file name, not by full path.
    // Sorting is not cosmetic: it is what makes "exactly one candidate" a
    // deterministic outcome rather than a filesystem-order one, and the error
    // for an ambiguous archive reproducible.
    entries.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    if is_staged_runner(extraction_root) {
        return Ok(extraction_root.to_path_buf());
    }
    if entries.iter().any(|entry| entry.is_symlink()) {
        return Err(RunnerError::StagedTopLevelLink);
    }

    let mut candidates = Vec::new();
    for entry in &entries {
        // `entry.is_dir()` follows symlinks in Python; the refusal above has
        // already removed every symlink from consideration, so the two agree.
        if !entry.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        safe_install_id(&name)?;
        if is_staged_runner(entry) {
            candidates.push(entry.clone());
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        _ => Err(RunnerError::NoUsableRunner),
    }
}

/// Download, stage, validate and atomically install a runner build.
/// `ProtonManager.install` (`runners.py:864`).
///
/// The order of operations is the security property, so it is worth stating
/// plainly: the archive is written to a **private 0700 staging directory
/// inside the runners directory**, extracted there, the *extracted tree* is
/// validated (not the archive), metadata is written into the validated tree,
/// and only then is the whole tree renamed into place under a name that cannot
/// already exist. Nothing attacker-controlled is reachable under its final name
/// until every check has passed, and no partial state is ever visible there.
///
/// `progress` is called with a fraction in `0.0..=1.0`. Python guards with
/// `if progress_cb and total`, so the callback is not called at all when the
/// total is unknown — a caller that needs to know the download started must
/// read that as "no progress reports yet", not as "zero percent". The final
/// `1.0` is reported unconditionally, so a caller must tolerate a repeat of the
/// value it just saw.
pub fn install(
    client: &dyn HttpClient,
    runners_directory: &Path,
    release: &ReleaseInfo,
    progress: &dyn Fn(f32),
    timeout: Duration,
) -> Result<PathBuf, RunnerError> {
    install_with(
        client,
        runners_directory,
        release,
        progress,
        timeout,
        MAX_RUNNER_ARCHIVE_BYTES,
    )
}

/// [`install`] with an explicit download cap.
///
/// The cap is a parameter for the same reason [`extract_archive_with`]'s bounds
/// are: the default is 2 GiB, and a test that cannot reach a limit can only
/// assert the code path exists, not that it works. The streaming guard is the
/// one that matters — a server is free to send `Content-Length: 10` and then
/// three gigabytes — and with the real constant no honest test could ever
/// exercise it. `install` delegates with [`MAX_RUNNER_ARCHIVE_BYTES`], so the
/// production path and the tested path are the same code.
///
/// [`extract_archive_with`]: super::archive::extract_archive_with
pub fn install_with(
    client: &dyn HttpClient,
    runners_directory: &Path,
    release: &ReleaseInfo,
    progress: &dyn Fn(f32),
    timeout: Duration,
    download_cap: u64,
) -> Result<PathBuf, RunnerError> {
    fs::create_dir_all(runners_directory)?;

    let install_id = safe_install_id(&release.install_id()?)?;
    let target = runners_directory.join(&install_id);
    // `is_symlink` is checked separately because a **broken** symlink is not
    // `exists()` and would otherwise be silently replaced by the rename —
    // destroying a link the user made deliberately.
    if target.exists() || target.is_symlink() {
        return Err(RunnerError::AlreadyInstalled { id: install_id });
    }
    // `release.size < 0 or release.size > MAX_RUNNER_ARCHIVE_BYTES` — a
    // **negative** declared size is reported as "exceeds the download size
    // limit" rather than as an invalid size. That reads like a mistake and it is
    // Python's behaviour, so it is kept: the message is user-visible and a bug
    // report may quote it. The same asymmetry is in the `on_head` check below,
    // which is why a negative `Content-Length` is an oversized archive and not
    // an invalid header, even though `parse_content_length` would happily
    // produce the number.
    if release.size < 0 || release.size as u64 > download_cap {
        return Err(RunnerError::ArchiveTooLarge);
    }

    let staging = StagingDirectory::create(runners_directory)?;
    // The archive's remote name is irrelevant once it is inside the private
    // staging directory, so it is not used as a local path at all. That is
    // Python's comment and it is a real defence: an asset named `../x.tar.gz`
    // never becomes a path here.
    let archive = staging.path.join("runner.archive");
    let extraction_root = staging.path.join("extracted");
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(&extraction_root)?;

    let headers = [("User-Agent", USER_AGENT)];
    let mut stream = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)?;

    // Set by `on_head`, read by `sink`: the declared length is only available
    // in the first callback and is needed by the second. Held in a `Cell`
    // rather than threaded through a struct because the two closures are handed
    // to one call and this is the only state they share.
    let total = std::cell::Cell::new(0i64);
    let downloaded = std::cell::Cell::new(0u64);

    client.get(
        &release.download_url,
        &headers,
        timeout,
        &mut |head| {
            let declared = parse_content_length(head.content_length.as_deref())?;
            if declared < 0 || declared as u64 > download_cap {
                return Err(RunnerError::ArchiveTooLarge);
            }
            total.set(if declared != 0 {
                declared
            } else {
                release.size
            });
            Ok(())
        },
        &mut |chunk| {
            let seen = downloaded.get() + chunk.len() as u64;
            downloaded.set(seen);
            if seen > download_cap {
                return Err(RunnerError::ArchiveTooLarge);
            }
            stream.write_all(chunk)?;
            let denominator = total.get();
            if denominator > 0 {
                let fraction = (seen as f64 / denominator as f64).min(1.0) as f32;
                progress(fraction);
            }
            Ok(())
        },
    )?;

    crate::runners::archive::extract_archive(&archive, &extraction_root)?;
    let extracted = resolve_staged(&extraction_root)?;
    crate::runners::archive::validate_staged_runner(&extracted, &extraction_root)?;
    write_metadata(&extracted, release)?;
    progress(1.0);
    rename_noreplace(&extracted, &target)?;
    Ok(target)
}

/// Remove an installed runner build. `ProtonManager.uninstall`
/// (`runners.py:922`).
///
/// Refuses the two ids that name something the user did not install: the empty
/// id, and [`SYSTEM_WINE`] — the distro's Wine is not ours to delete.
///
/// An id that resolves to a **symlink** or to anything that is not a directory
/// is a no-op rather than an error, and that is deliberate rather than sloppy:
/// the call site is a UI action that should converge on "it is gone", so an
/// unknown id, a directory already deleted, and a name that is really a link to
/// somewhere else all leave the build uninstalled, which is what was asked.
/// Following the symlink would delete the *target*, which is why the check is
/// `is_symlink()` first and not just "is this a directory".
///
/// An **unsafe** id is a different matter and is an error, not a no-op:
/// `safe_install_id` raises in Python and this propagates it. The two read
/// alike — "nothing was deleted either way" — but they are not the same
/// outcome, and the difference is visible to the UI, which reports the failure.
/// A `safe_install_id` that returned `Ok` for `"../.."` here would be a delete
/// outside the runners directory.
///
/// Note also that the set membership test is on the **raw** id, before
/// trimming, so `" wine-system "` passes the guard and is then trimmed into the
/// system runner's directory name by `safe_install_id` — i.e. it is deletable.
/// That is Python's behaviour and it is kept, because a UI can only produce this
/// id by reading it from somewhere the user already edited; it is recorded here
/// rather than fixed because "fixing" it would make a `safe_install_id` result
/// and a runner id disagree, and every other caller in this module relies on
/// them agreeing.
pub fn uninstall(runners_directory: &Path, runner_id: &str) -> Result<(), RunnerError> {
    if runner_id.is_empty() || runner_id == SYSTEM_WINE {
        return Err(RunnerError::SystemWineCannotBeUninstalled);
    }
    let install_id = safe_install_id(runner_id)?;
    let target = runners_directory.join(install_id);
    // A **symlinked** build is listed as installed — `installed_protons` filters
    // on `is_dir()`, which follows the link, and `is_available` then resolves a
    // wine binary or `proton` script through it — so the row is drawn with a
    // working Remove button. Removing the *link* is what that button promises,
    // and it is safe: `remove_file` unlinks the symlink and never the directory
    // it points at, so the build it refers to is untouched.
    //
    // This deliberately diverges from the reference. Python's guard returns
    // early for a symlink, and it has to: `shutil.rmtree` on one raises
    // `OSError` (measured, not assumed), so the guard is load-bearing there. But
    // Python's Remove button then toasts `Removed {id}` for a build that is
    // still listed and still launchable — the defect is in the reference too,
    // and the port is the last place it can change. See `BUG-03`.
    if target.is_symlink() {
        fs::remove_file(&target)?;
        return Ok(());
    }
    if !target.is_dir() {
        return Ok(());
    }
    fs::remove_dir_all(&target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::RunnerManager;
    use crate::runners::env::tests::{FakeLaunchEnv, scratch};
    use std::fs;

    // -----------------------------------------------------------------
    // python_int against CPython
    // -----------------------------------------------------------------

    /// Every line of this table was produced by running CPython, not by
    /// reading the docs: the input on the left, `int()`'s answer on the right.
    /// It is the whole of the behaviour the port has to reproduce, and the
    /// three cases it cannot appear as their own tests below.
    ///
    /// The four C0 separators (`U+001C..U+001F`) are in here deliberately.
    /// They are the only code points where `int()`'s trim set differs from
    /// `str.isspace()`, and `launch_opts` needs the *opposite* answer for them,
    /// so a refactor that shares one predicate between the two modules breaks
    /// this test.
    #[test]
    fn python_int_matches_cpython_case_for_case() {
        let cases: &[(&str, Option<i64>)] = &[
            ("", None),
            (" ", None),
            ("\u{9}", None),
            ("\u{a}", None),
            ("\u{d}", None),
            ("\u{b}", None),
            ("\u{c}", None),
            ("\u{1c}", None),
            ("\u{1d}", None),
            ("\u{1e}", None),
            ("\u{1f}", None),
            ("\u{85}", None),
            ("\u{a0}", None),
            ("\u{1680}", None),
            ("\u{200b}", None),
            ("\u{2000}", None),
            ("\u{2028}", None),
            ("\u{2029}", None),
            ("\u{202f}", None),
            ("\u{205f}", None),
            ("\u{3000}", None),
            (" 12 ", Some(12)),
            ("\u{9}7", Some(7)),
            ("\u{a}5", Some(5)),
            ("\u{b}5", Some(5)),
            ("\u{c}5", Some(5)),
            ("\u{1c}5", None),
            ("\u{1d}5", None),
            ("\u{1e}5", None),
            ("\u{1f}5", None),
            ("\u{a0}5", Some(5)),
            ("\u{2028}5", Some(5)),
            ("\u{3000}5", Some(5)),
            ("\u{200b}5", None),
            ("5 ", Some(5)),
            (" 5 ", Some(5)),
            ("12", Some(12)),
            ("+12", Some(12)),
            ("-12", Some(-12)),
            ("-0", Some(0)),
            ("+0", Some(0)),
            ("0", Some(0)),
            ("007", Some(7)),
            ("1_0", Some(10)),
            ("1_0_0", Some(100)),
            ("1__0", None),
            ("_10", None),
            ("10_", None),
            ("1_", None),
            ("+_1", None),
            ("-_1", None),
            ("1e3", None),
            ("12.0", None),
            ("0x10", None),
            ("12\u{a}", Some(12)),
            ("0b1", None),
            ("1j", None),
            ("9223372036854775807", Some(9223372036854775807)),
            ("-9223372036854775808", Some(-9223372036854775808)),
            ("+ 12", None),
            ("--1", None),
            ("++1", None),
            ("+-1", None),
            ("+", None),
            ("-", None),
            ("_", None),
            ("1-2", None),
            ("1+2", None),
        ];
        for (input, expected) in cases {
            assert_eq!(python_int(input), *expected, "int({input:?})");
        }
        // Not vacuous: the table has to contain both outcomes in quantity, or a
        // `python_int` that returned one of them for everything would pass.
        let some = cases.iter().filter(|(_, v)| v.is_some()).count();
        let none = cases.len() - some;
        assert!(some > 20 && none > 20, "table is one-sided: {some}/{none}");
    }

    /// `-9223372036854775808` is `i64::MIN` and is the reason the accumulator
    /// is `i128`.
    ///
    /// An `i64` accumulator saturates at `i64::MAX` while accumulating, so the
    /// magnitude maxes out one *below* 2^63 and this input came back as
    /// `-9223372036854775807`. The differential table caught it; this test
    /// names it, because the failure is one off and reads as a rounding
    /// curiosity rather than as a bug.
    #[test]
    fn the_asymmetric_i64_limit_is_reached_exactly() {
        assert_eq!(python_int("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(python_int("9223372036854775807"), Some(i64::MAX));
        // One past each edge saturates rather than wrapping — divergence 2.
        assert_eq!(python_int("9223372036854775808"), Some(i64::MAX));
        assert_eq!(python_int("-9223372036854775809"), Some(i64::MIN));
        assert_eq!(python_int(&"9".repeat(40)), Some(i64::MAX));
        assert_eq!(python_int(&format!("-{}", "9".repeat(40))), Some(i64::MIN));
        // A wrapped accumulator would have produced a *small* number here, and
        // a small number is what slips a size check, so wrap-around is the
        // dangerous direction rather than mere inaccuracy.
        assert!(python_int(&"9".repeat(40)).unwrap() > 0);
    }

    /// Divergence 1, pinned from both sides.
    ///
    /// CPython accepts any Unicode `Nd` digit; the port accepts ASCII only,
    /// because `char::to_digit(10)` is ASCII-only and this crate carries no
    /// Unicode tables. The test asserts the rejection *and* records the value
    /// CPython gives, so the divergence cannot be mistaken for a bug in either
    /// implementation — and cannot be "fixed" by loosening the predicate
    /// without noticing that the value is then wrong rather than merely absent.
    #[test]
    fn unicode_decimal_digits_are_rejected_where_cpython_accepts_them() {
        // (input, what CPython's int() returns)
        let divergence: &[(&str, i64)] = &[
            ("\u{661}\u{662}", 12),  // Arabic-Indic ١٢
            ("\u{663}", 3),          // ٣
            ("\u{661}_\u{662}", 12), // with a PEP 515 underscore
            ("\u{661}\u{662}_\u{663}", 123),
            ("\u{665}\u{665}\u{665}", 555), // ٥٥٥
            ("\u{663}\u{664}", 34),         // ٣٤
            ("\u{ff11}\u{ff12}", 12),       // fullwidth １２
            ("\u{ff11}\u{ff12}\u{ff13}\u{ff14}\u{ff15}", 12345),
        ];
        for (input, cpython) in divergence {
            assert_eq!(
                python_int(input),
                None,
                "the port rejects {input:?} (CPython gives {cpython})"
            );
        }
        // The contrast that makes this a divergence rather than a shared
        // limitation: ASCII is accepted, so the predicate is not rejecting
        // digits wholesale.
        assert_eq!(python_int("12"), Some(12));
        // And the `Nd` class really does reach Rust: `char::is_numeric` is
        // true for these, which is why `launch_opts` accepts them — proving
        // that the two predicates are answering different questions rather
        // than one of them being broken.
        assert!('\u{663}'.is_numeric());
        assert!('\u{663}'.to_digit(10).is_none(), "to_digit is ASCII-only");
    }

    /// The measurement that the trim set is `is_whitespace()`, not `isspace()`.
    ///
    /// Enumerating every `isspace()` code point and asking `int()` to parse it
    /// gives 25 trimmed and 4 rejected, the four being exactly
    /// `U+001C..U+001F`. So this side does **not** adjust `char::is_whitespace`,
    /// while `launch_opts` must. A shared predicate would be wrong in one of
    /// the two modules in whichever direction it leaned.
    #[test]
    fn the_four_c0_separators_are_not_whitespace_to_int() {
        // The load-bearing measurement: Rust's `char::is_whitespace` is the
        // `White_Space` property, which is **false** for all four separators,
        // while Python's `str.isspace()` is **true** for them. That identity —
        // not any adjustment — is what makes `char::is_whitespace` the correct
        // predicate for `int()`, and the opposite of what `launch_opts` needs.
        for separator in ['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}'] {
            assert!(
                !separator.is_whitespace(),
                "{separator:?} is not White_Space, which is why int() rejects it"
            );
            assert_eq!(python_int(&format!("{separator}5")), None);
        }
        // The contrast that shows the rule is the property and not "C0 is
        // rejected": `\v` and `\f` are C0 too, are White_Space, and are
        // trimmed by `int()`.
        for trimmed in ['\u{b}', '\u{c}', '\u{85}', '\u{a0}', '\u{2028}', '\u{3000}'] {
            assert!(trimmed.is_whitespace(), "{trimmed:?} is White_Space");
            assert_eq!(python_int(&format!("{trimmed}5")), Some(5));
        }
        // And the negated counterpart to `launch_opts`: a predicate that added
        // the separators back — which is right for `isspace()` — breaks three
        // of the rows in the differential table above.
        assert_eq!(python_int("\u{1c}5"), None);
        assert_ne!(python_int("\u{1c}5"), Some(5));
    }

    // -----------------------------------------------------------------
    // int_from_json
    // -----------------------------------------------------------------

    /// The `int(value or 0)` coercions in `parse_releases`, over the JSON types
    /// a hostile payload can actually deliver.
    ///
    /// Every row was confirmed against CPython. The two that matter are the
    /// float (which **truncates**, it does not round) and the container (which
    /// raises rather than degrading to 0).
    #[test]
    fn int_from_json_matches_pythons_coercions() {
        use serde_json::json;

        assert_eq!(int_from_json(&json!(0)).unwrap(), 0);
        assert_eq!(int_from_json(&json!(12)).unwrap(), 12);
        assert_eq!(int_from_json(&json!(-1)).unwrap(), -1);
        // `or 0` makes the falsy values zero, tested by identity rather than
        // by type: all five are distinct JSON shapes.
        for falsy in [
            json!(null),
            json!(false),
            json!(0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert_eq!(int_from_json(&falsy).unwrap(), 0, "{falsy} is falsy");
        }
        // A float truncates toward zero: `int(1.5)` is 1, `int(-1.5)` is -1.
        assert_eq!(int_from_json(&json!(1.5)).unwrap(), 1);
        assert_eq!(int_from_json(&json!(-1.5)).unwrap(), -1);
        assert_eq!(int_from_json(&json!(1.9)).unwrap(), 1);
        // `bool` is an `int` in Python, and `true` is truthy, so it is 1.
        assert_eq!(int_from_json(&json!(true)).unwrap(), 1);
        // A string goes through `int()`, so its traps apply here too.
        assert_eq!(int_from_json(&json!("12")).unwrap(), 12);
        assert!(int_from_json(&json!("1e3")).is_err());
        assert!(int_from_json(&json!("12.0")).is_err());
        // A non-empty container is a `TypeError` in Python: an error, never 0.
        assert!(int_from_json(&json!([1])).is_err());
        assert!(int_from_json(&json!({"a": 1})).is_err());
        // And the error text names the Python type, as the `TypeError` does.
        let error = int_from_json(&json!({"a": 1})).unwrap_err().to_string();
        assert!(error.contains("not 'dict'"), "{error}");
        let error = int_from_json(&json!([1])).unwrap_err().to_string();
        assert!(error.contains("not 'list'"), "{error}");
    }

    #[test]
    fn an_oversized_integer_saturates_rather_than_wrapping_negative() {
        // Divergence 2. `1e30` is a legal JSON literal whose `int()` value is
        // 1000000000000000019884624838656, which no `i64` holds.
        let huge: Value = serde_json::from_str("1e30").unwrap();
        assert_eq!(int_from_json(&huge).unwrap(), i64::MAX);
        let huge_uint: Value = serde_json::from_str("18446744073709551615").unwrap();
        assert_eq!(int_from_json(&huge_uint).unwrap(), i64::MAX);
        // The dangerous direction is a *small* answer, so this asserts the sign
        // rather than only the saturation.
        assert!(int_from_json(&huge).unwrap() > 0);
    }

    // -----------------------------------------------------------------
    // parse_content_length
    // -----------------------------------------------------------------

    /// `int(resp.headers.get("Content-Length", 0) or 0)`, and the `or 0` is the
    /// part worth pinning: an **absent** header and an **empty** header are both
    /// `0`, not an error, and only a present non-empty unparseable value is
    /// invalid. A port that rejected `Some("")` would refuse a legal response.
    #[test]
    fn an_absent_or_empty_content_length_is_zero_but_a_bad_one_is_an_error() {
        assert_eq!(parse_content_length(None).unwrap(), 0);
        assert_eq!(parse_content_length(Some("")).unwrap(), 0);
        assert_eq!(parse_content_length(Some("12345")).unwrap(), 12345);
        assert_eq!(parse_content_length(Some("  99 ")).unwrap(), 99);
        assert_eq!(parse_content_length(Some("+7")).unwrap(), 7);
        // A negative length is parsed, not rejected here: Python's range check
        // happens in `install`, against the size cap, and doing it in two
        // places is how the two come to disagree.
        assert_eq!(parse_content_length(Some("-1")).unwrap(), -1);

        for bad in ["abc", "12.0", "1e3", "0x10", "12,5"] {
            let error = parse_content_length(Some(bad)).unwrap_err().to_string();
            assert_eq!(
                error, "Runner download has an invalid Content-Length",
                "int({bad:?}) is a ValueError in Python"
            );
        }
    }

    // -----------------------------------------------------------------
    // parse_releases
    // -----------------------------------------------------------------

    fn release(tag: &str, assets: Value) -> Value {
        serde_json::json!({ "tag_name": tag, "assets": assets })
    }

    fn asset(name: &str, url: &str, size: impl Into<Value>) -> Value {
        serde_json::json!({
            "name": name,
            "browser_download_url": url,
            "size": size.into(),
        })
    }

    #[test]
    fn a_release_with_no_matching_asset_is_skipped_not_an_error() {
        // The payload lists every asset GitHub holds — source tarballs,
        // signatures, checksums — so most releases in a real listing are
        // expected to be unusable. Returning fewer entries than inputs is the
        // normal case, not a failure.
        // Note which names are unusable. `proton-ge` requires and excludes
        // nothing, so it matches **any** name that looks like an archive — a
        // `source.tar.gz` is picked, which is the trap this test originally
        // fell into. What is skipped is a name that is not an archive at all,
        // which is most of a real release's assets: checksums, signatures,
        // release notes, source zips.
        let data = vec![
            release("v1", serde_json::json!([asset("SHA256SUMS", "u", 1)])),
            release("v2", serde_json::json!([])),
            release("v3", serde_json::json!([asset("proton-9.0.zip", "u", 1)])),
        ];
        let parsed = parse_releases(&data, Some("proton-ge")).unwrap();
        assert!(parsed.is_empty(), "{parsed:?}");

        // The contrast: an archive-like name is *not* skipped, and a bare
        // `source.tar.gz` is exactly what proton-ge will happily install.
        let data = vec![release(
            "v1",
            serde_json::json!([asset("source.tar.gz", "u", 7)]),
        )];
        let parsed = parse_releases(&data, Some("proton-ge")).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "source.tar.gz");
    }

    #[test]
    fn a_matching_release_carries_its_tag_name_url_and_size() {
        let data = vec![release(
            "GE-Proton9-20",
            serde_json::json!([asset("GE-Proton9-20.tar.gz", "https://x/y", 1024)]),
        )];
        let parsed = parse_releases(&data, Some("proton-ge")).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].tag, "GE-Proton9-20");
        assert_eq!(parsed[0].name, "GE-Proton9-20.tar.gz");
        assert_eq!(parsed[0].download_url, "https://x/y");
        assert_eq!(parsed[0].size, 1024);
        assert_eq!(parsed[0].family_id, "proton-ge");
    }

    /// The tag fallback is `tag_name or name or ""`, and each step is a
    /// **truthiness** test — so an empty `tag_name` falls through to `name`
    /// exactly as a missing one does.
    #[test]
    fn the_tag_falls_back_through_truthiness_to_the_name_and_then_to_empty() {
        let good_asset = || serde_json::json!([asset("GE-Proton9-20.tar.gz", "u", 1)]);
        let parse_one = |release: Value| {
            parse_releases(&[release], Some("proton-ge")).unwrap()[0]
                .tag
                .clone()
        };

        // Present and truthy: used.
        assert_eq!(
            parse_one(serde_json::json!({"tag_name": "v1", "assets": good_asset()})),
            "v1"
        );
        // Missing: falls through to `name`.
        assert_eq!(
            parse_one(serde_json::json!({"name": "v2", "assets": good_asset()})),
            "v2"
        );
        // Empty string is falsy, so it falls through too — this is the case a
        // null check would get wrong.
        assert_eq!(
            parse_one(serde_json::json!({
                "tag_name": "", "name": "v3", "assets": good_asset()
            })),
            "v3"
        );
        // Neither: empty, and the release is still kept — the asset matched,
        // and the tag is only a label.
        assert_eq!(parse_one(serde_json::json!({"assets": good_asset()})), "");
        // A non-string tag is coerced rather than passed through as JSON, per
        // divergence 3: Python carries the number into a `str`-annotated field
        // and fails later; the port coerces at the boundary.
        assert_eq!(
            parse_one(serde_json::json!({"tag_name": 9, "assets": good_asset()})),
            "9"
        );
    }

    /// The `name` and `download_url` fields are `str(value or "")`, and the
    /// `or` runs **before** `str()`. Getting that order wrong is the plausible
    /// bug, and it is silent: `python_str(None)` is `"None"`, so a missing name
    /// would be filed as a runner literally called `None`.
    #[test]
    fn a_missing_asset_field_is_empty_rather_than_the_string_none() {
        // `name` has to stay archive-like for the asset to be picked at all —
        // `pick_asset` identifies an asset *by* its name, so a nameless one
        // cannot match and the release is skipped. That makes the `name` field
        // of an emitted `ReleaseInfo` always truthy, and it is the
        // `browser_download_url` beside it that can be absent. Asserting on a
        // nameless asset would only ever observe the skip.
        let nameless = vec![serde_json::json!({
            "tag_name": "v1",
            "assets": [{"size": 5}],
        })];
        assert!(
            parse_releases(&nameless, Some("proton-ge"))
                .unwrap()
                .is_empty(),
            "an asset with no name cannot be recognised as an archive"
        );

        // The reachable half: every falsy shape of `browser_download_url`
        // becomes `""`, not `"None"` and not `"0"`.
        for missing in [
            serde_json::json!(null),
            serde_json::json!(""),
            serde_json::json!(0),
            serde_json::json!(false),
        ] {
            let data = vec![serde_json::json!({
                "tag_name": "v1",
                "assets": [{
                    "name": "GE-Proton9-20.tar.gz",
                    "browser_download_url": missing,
                    "size": 5,
                }],
            })];
            let parsed = parse_releases(&data, Some("proton-ge")).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].download_url, "", "{missing} is falsy");
            assert_ne!(parsed[0].download_url, "None");
            assert_ne!(parsed[0].download_url, "0");
            // The name really is present, so the assertion above is not
            // observing an empty record.
            assert_eq!(parsed[0].name, "GE-Proton9-20.tar.gz");
        }

        // And a key that is absent entirely, which is the case the `str(value
        // or "")` shape exists for.
        let data = vec![serde_json::json!({
            "tag_name": "v1",
            "assets": [{"name": "GE-Proton9-20.tar.gz", "size": 5}],
        })];
        let parsed = parse_releases(&data, Some("proton-ge")).unwrap();
        assert_eq!(parsed[0].download_url, "");
    }

    #[test]
    fn a_falsy_assets_field_is_no_assets_but_a_truthy_non_list_is_an_error() {
        let good = serde_json::json!([asset("GE-Proton9-20.tar.gz", "u", 1)]);
        // Each falsy shape means "no assets": the release is skipped.
        for falsy in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(""),
            serde_json::json!(0),
        ] {
            let data = vec![serde_json::json!({"tag_name": "v1", "assets": falsy})];
            assert!(
                parse_releases(&data, Some("proton-ge")).unwrap().is_empty(),
                "{falsy} should mean no assets"
            );
        }
        let data = vec![serde_json::json!({"tag_name": "v1"})];
        assert!(parse_releases(&data, Some("proton-ge")).unwrap().is_empty());

        // A truthy non-list is *not* "no assets": Python's `or` keeps it and
        // `pick_asset` then dies on `str.get`. Reported rather than skipped, so
        // a malformed payload is not rendered as "this family has no builds".
        let data = vec![serde_json::json!({"tag_name": "v1", "assets": "x"})];
        assert!(parse_releases(&data, Some("proton-ge")).is_err());
        // And the good case still works, so the error above is not the arm
        // rejecting everything.
        let data = vec![serde_json::json!({"tag_name": "v1", "assets": good})];
        assert_eq!(parse_releases(&data, Some("proton-ge")).unwrap().len(), 1);
    }

    #[test]
    fn the_family_is_resolved_from_the_id_and_defaults_to_proton_ge() {
        let data = vec![serde_json::json!({
            "tag_name": "v1",
            "assets": [asset("GE-Proton9-20.tar.gz", "u", 1)],
        })];
        // An explicit `None` is the default family, not "no family".
        assert_eq!(
            parse_releases(&data, None).unwrap()[0].family_id,
            "proton-ge"
        );
        assert_eq!(
            parse_releases(&data, Some("proton-ge")).unwrap()[0].family_id,
            "proton-ge"
        );
        // An unknown id is an error carrying Python's `KeyError` text, rather
        // than silently listing nothing.
        let error = parse_releases(&data, Some("nope")).unwrap_err().to_string();
        assert!(error.contains("Unknown runner family: nope"), "{error}");
    }

    /// Family specificity is carried entirely by the `require` / `exclude`
    /// token lists, and **not** by the family id: `proton-ge` has both lists
    /// empty, so it matches any archive at all and is the wrong family to test
    /// distinctness with. The real exclusion rule is `wine-proton`'s, which
    /// rejects the 32-bit-only builds so a 64-bit host is not offered a
    /// `win32` archive.
    #[test]
    fn a_family_excludes_the_assets_its_token_list_rejects() {
        let one = |name: &str| {
            vec![serde_json::json!({
                "tag_name": "v1",
                "assets": [asset(name, "u", 1)],
            })]
        };

        // `wine-proton` excludes `win32` and `wow64`; the 64-bit build is
        // taken and the 32-bit-only one is not.
        let taken =
            parse_releases(&one("wine-10.0-proton-amd64.tar.xz"), Some("wine-proton")).unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].family_id, "wine-proton");

        for excluded in [
            "wine-10.0-proton-win32.tar.xz",
            "wine-10.0-proton-wow64.tar.xz",
        ] {
            let skipped = parse_releases(&one(excluded), Some("wine-proton")).unwrap();
            assert!(skipped.is_empty(), "{excluded} should be excluded");
        }

        // The contrast that shows the exclusion above is the family's rule and
        // not a blanket rejection: `proton-ge` has no token lists, so it takes
        // the very same 32-bit build. A port that applied one family's tokens
        // to another would fail one of these two.
        let ge = parse_releases(&one("wine-10.0-proton-win32.tar.xz"), Some("proton-ge")).unwrap();
        assert_eq!(ge.len(), 1);
        assert_eq!(ge[0].family_id, "proton-ge");
    }

    // -----------------------------------------------------------------
    // fetch_available (the injected client, D-26)
    // -----------------------------------------------------------------

    /// A client that answers with canned bytes and records the requests.
    ///
    /// Deliberately not a mock *library*: the trait has one required method, so
    /// a struct with a queue of responses is the whole double, and the tests
    /// below stay readable as the requests they make.
    /// One recorded request: the URL and the headers it was sent with.
    type SeenRequest = (String, Vec<(String, String)>);

    struct FakeClient {
        status: Result<(ResponseHead, Vec<u8>), RunnerError>,
        seen: std::cell::RefCell<Vec<SeenRequest>>,
    }

    impl FakeClient {
        /// The canned response's final URL is empty, which is what a client
        /// that cannot report one gives. `fetch_available` has no origin
        /// allowlist to check, so the empty string is inert here; the field's
        /// fail-closed shape is exercised by `install`'s redirect test in
        /// `installers.rs`.
        fn body(text: &str) -> Self {
            Self {
                status: Ok((
                    ResponseHead {
                        content_length: None,
                        final_url: String::new(),
                    },
                    text.as_bytes().to_vec(),
                )),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn bytes(raw: &[u8]) -> Self {
            Self {
                status: Ok((
                    ResponseHead {
                        content_length: None,
                        final_url: String::new(),
                    },
                    raw.to_vec(),
                )),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn url(&self) -> String {
            self.seen.borrow()[0].0.clone()
        }
    }

    impl HttpClient for FakeClient {
        fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            self.seen.borrow_mut().push((
                url.to_string(),
                headers
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            ));
            match &self.status {
                Ok((head, body)) => {
                    // The head is delivered first and its error is **not**
                    // swallowed: a double that called `on_head` and ignored the
                    // result would make the size-cap abort untestable and would
                    // hide a broken `install` behind a passing suite.
                    on_head(head)?;
                    sink(body)
                }
                Err(RunnerError::Http { message }) => Err(RunnerError::Http {
                    message: message.clone(),
                }),
                Err(_) => Err(RunnerError::Http {
                    message: "transport".to_string(),
                }),
            }
        }
    }

    fn payload(releases: Value) -> String {
        serde_json::to_string(&releases).unwrap()
    }

    #[test]
    fn fetch_available_asks_githubs_api_with_the_headers_it_requires() {
        // A request without an `Accept` is rejected by the API, and without a
        // `User-Agent` it is rejected outright, so both are pinned.
        let client = FakeClient::body(&payload(serde_json::json!([])));
        fetch_available(&client, None, 15, Duration::from_secs(30)).unwrap();
        assert_eq!(
            client.url(),
            "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases"
        );
        let headers = &client.seen.borrow()[0].1;
        assert!(headers.contains(&(
            "Accept".to_string(),
            "application/vnd.github+json".to_string()
        )));
        assert!(headers.contains(&("User-Agent".to_string(), USER_AGENT.to_string())));
    }

    /// The `limit` is applied **after** parsing, so unusable releases do not
    /// consume slots. Slicing the raw payload first is the plausible mistake,
    /// and it returns fewer than the caller asked for.
    #[test]
    fn the_limit_is_applied_after_parsing_so_skipped_releases_do_not_consume_slots() {
        let good = |tag: &str| {
            serde_json::json!({
                "tag_name": tag,
                "assets": [asset(&format!("GE-Proton{tag}.tar.gz"), "u", 1)],
            })
        };
        // Two unusable releases first, then three usable ones. "Unusable"
        // means the asset name is not archive-like: `proton-ge` has empty token
        // lists, so any `.tar.gz` *would* match and consume a slot, which is
        // how the first version of this test mis-measured the rule.
        let releases = serde_json::json!([
            {"tag_name": "junk1", "assets": []},
            {"tag_name": "junk2", "assets": [asset("SHA256SUMS", "u", 1)]},
            good("9-20"),
            good("9-19"),
            good("9-18"),
        ]);
        let client = FakeClient::body(&payload(releases));

        let limited = fetch_available(&client, None, 2, Duration::from_secs(30)).unwrap();
        assert_eq!(limited.len(), 2, "asked for 2 usable releases");
        assert_eq!(limited[0].tag, "9-20");
        assert_eq!(limited[1].tag, "9-19");

        // And the limit is a cap, not a requirement: fewer available than asked
        // is not an error.
        let client = FakeClient::body(&payload(serde_json::json!([good("9-20")])));
        assert_eq!(
            fetch_available(&client, None, 15, Duration::from_secs(30))
                .unwrap()
                .len(),
            1
        );
        // A limit of zero is an empty listing, not an error.
        let client = FakeClient::body(&payload(serde_json::json!([good("9-20")])));
        assert!(
            fetch_available(&client, None, 0, Duration::from_secs(30))
                .unwrap()
                .is_empty()
        );
    }

    /// A GitHub error object is valid JSON, so without the explicit array check
    /// it would parse to zero releases and the UI would say "no builds" instead
    /// of surfacing the failure.
    #[test]
    fn a_payload_that_is_not_an_array_is_an_error_not_an_empty_listing() {
        for body in [
            r#"{"message": "Not Found"}"#,
            r#"{"message":"API rate limit exceeded"}"#,
            r#""a string""#,
            "42",
            "null",
        ] {
            let client = FakeClient::body(body);
            let error = fetch_available(&client, None, 15, Duration::from_secs(30)).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Unexpected GitHub releases response",
                "{body}"
            );
        }
        // An empty array is a *successful* empty listing — the contrast that
        // shows the check is not simply rejecting everything.
        let client = FakeClient::body("[]");
        assert!(
            fetch_available(&client, None, 15, Duration::from_secs(30))
                .unwrap()
                .is_empty()
        );
    }

    /// Divergence 4: Python's `resp.read().decode("utf-8")` raises an uncaught
    /// `UnicodeDecodeError`, so the caller sees a traceback. The port returns a
    /// message. Same class as the DXVK-marker divergence in `launch_opts`.
    #[test]
    fn an_undecodable_body_is_an_error_rather_than_a_decode_panic() {
        let client = FakeClient::bytes(&[0xff, 0xfe, 0x00, 0x01, 0x80]);
        let error = fetch_available(&client, None, 15, Duration::from_secs(30)).unwrap_err();
        assert_eq!(error.to_string(), "Unexpected GitHub releases response");
        // Malformed *text* (valid UTF-8, invalid JSON) is the same error, so
        // the two failure modes are not told apart by the caller.
        let client = FakeClient::body("not json at all");
        assert!(fetch_available(&client, None, 15, Duration::from_secs(30)).is_err());
    }

    #[test]
    fn a_transport_failure_propagates_rather_than_becoming_an_empty_listing() {
        let client = FakeClient {
            status: Err(RunnerError::Http {
                message: "connection refused".to_string(),
            }),
            seen: std::cell::RefCell::new(Vec::new()),
        };
        let error = fetch_available(&client, None, 15, Duration::from_secs(30)).unwrap_err();
        assert_eq!(error.to_string(), "connection refused");
    }

    // -----------------------------------------------------------------
    // is_installed
    // -----------------------------------------------------------------

    /// A directory that `is_usable_runner` accepts: a `proton` file is enough.
    fn usable(directory: &Path) {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("proton"), "#!/bin/sh\n").unwrap();
    }

    #[test]
    fn a_runner_installed_under_its_install_id_is_recognised() {
        let root = scratch("proton-installed");
        let root = root.as_path();

        // Proton-GE keeps the bare sanitised tag.
        let ge = root.join("GE-Proton9-20");
        usable(&ge);
        assert!(is_installed(root, "GE-Proton9-20", Some("proton-ge")));
        assert!(is_installed(root, "GE-Proton9-20", None));

        // A *different* tag is not found, which is what makes the assertion
        // above mean something.
        assert!(!is_installed(root, "GE-Proton9-19", Some("proton-ge")));
        assert!(!is_installed(root, "GE-Proton9-19", None));

        // A non-Proton-GE family is namespaced, so the bare tag alone is not
        // found — this is the collision the namespacing exists to prevent, and
        // it is why `GE-Proton9-20` is *not* reported as a cachyos install.
        // Asserted here while no metadata exists on disk, so it is the directory
        // name being tested and not the metadata fallback.
        assert!(!is_installed(root, "GE-Proton9-20", Some("proton-cachyos")));

        // Once the namespaced directory exists it is found by name.
        let namespaced =
            crate::runners::families::install_id_for("v1.0", "proton-cachyos").unwrap();
        usable(&root.join(&namespaced));
        assert!(is_installed(root, "v1.0", Some("proton-cachyos")));
        // The un-namespaced path is a different directory, and there is no
        // metadata to fall back on, so asking with no family does not find it.
        assert!(!is_installed(root, "v1.0", None));
    }

    /// The earlier, non-namespaced scheme, recognised by the metadata the
    /// install wrote. Without this, every runner a user installed before
    /// namespacing would read as not-installed.
    #[test]
    fn a_runner_from_the_earlier_scheme_is_recognised_by_its_metadata() {
        let root = scratch("proton-legacy");
        let root = root.as_path();
        // A directory whose name is *not* the install id for its family...
        let legacy = root.join("v1.0");
        usable(&legacy);
        fs::write(
            legacy.join(crate::runners::METADATA_NAME),
            r#"{"family": "proton-cachyos", "tag": "v1.0"}"#,
        )
        .unwrap();
        // ...is still found, because its metadata says what it is. Note the
        // directory name is exactly what a caller asking about this tag with no
        // family would use, so `is_installed(root, "v1.0", Some(..))` above
        // succeeds through the *metadata* path — the directory name is not the
        // namespaced id.
        assert!(is_installed(root, "v1.0", Some("proton-cachyos")));
        assert_eq!(legacy.file_name().unwrap(), "v1.0");

        // The metadata must match on *both* fields. A wrong tag is not a match,
        // and neither is a right tag under a family that did not record it —
        // two families publish `v1.0`, so matching on the tag alone would
        // report the wrong family's build as installed.
        assert!(!is_installed(root, "v2.0", Some("proton-cachyos")));
        assert!(!is_installed(root, "v1.0", Some("wine-staging")));

        // A directory with matching metadata that is not a usable runner is not
        // an install — a half-removed one, or an unrelated folder holding a
        // copied metadata file.
        let broken = root.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(
            broken.join(crate::runners::METADATA_NAME),
            r#"{"family": "wine-staging", "tag": "v9.9"}"#,
        )
        .unwrap();
        assert!(!is_installed(root, "v9.9", Some("wine-staging")));

        // A metadata file that is not valid JSON reads as no metadata, rather
        // than as a match or a crash. This needs its own root: in the root
        // above, the *legitimate* `v1.0` directory also records this tag and
        // family, so the assertion would pass without the garbage file telling
        // us anything.
        let garbage_root = scratch("proton-garbage");
        let garbage_root = garbage_root.as_path();
        let garbage = garbage_root.join("v1.0");
        usable(&garbage);
        fs::write(garbage.join(crate::runners::METADATA_NAME), "{not json").unwrap();
        assert!(!is_installed(garbage_root, "v1.0", Some("proton-cachyos")));

        // The paired case, differing only in whether the metadata parses. Both
        // directories are named `v1.0`, which is not the namespaced install id,
        // so the metadata is the only path that could succeed — and it does
        // exactly when it is readable. Without this twin the assertion above
        // would hold for a directory name that never matches.
        let twin_root = scratch("proton-garbage-twin");
        let twin_root = twin_root.as_path();
        let twin = twin_root.join("v1.0");
        usable(&twin);
        fs::write(
            twin.join(crate::runners::METADATA_NAME),
            r#"{"family": "proton-cachyos", "tag": "v1.0"}"#,
        )
        .unwrap();
        assert!(is_installed(twin_root, "v1.0", Some("proton-cachyos")));
    }

    #[test]
    fn the_metadata_scan_runs_only_when_a_family_is_given() {
        // With no family there is nothing to match on, so the scan is skipped
        // and only the un-namespaced directory name is tried.
        let root = scratch("proton-nofamily");
        let root = root.as_path();
        let legacy = root.join("v1.0");
        usable(&legacy);
        fs::write(
            legacy.join(crate::runners::METADATA_NAME),
            r#"{"family": "proton-cachyos", "tag": "v1.0"}"#,
        )
        .unwrap();
        assert!(is_installed(root, "v1.0", None), "the bare name matches");
        assert!(!is_installed(root, "v-nothing", None));

        // A missing runners directory is "not installed", not an error.
        assert!(!is_installed(
            &root.join("does-not-exist"),
            "v1.0",
            Some("proton-ge")
        ));
    }

    #[test]
    fn a_tag_that_cannot_form_a_directory_name_is_not_installed() {
        let root = scratch("proton-badid");
        let root = root.as_path();
        // Empty, "." and ".." are all rejected by `safe_install_id` — they
        // would escape or alias the runners directory.
        for tag in ["", ".", ".."] {
            assert!(
                !is_installed(root, tag, Some("proton-ge")),
                "{tag:?} must not resolve"
            );
            assert!(!is_installed(root, tag, None), "{tag:?} must not resolve");
        }
    }

    // -----------------------------------------------------------------
    // The HTTP client seam
    // -----------------------------------------------------------------

    /// A client that hands out its body in several chunks, so the accumulation
    /// in `get_text` is exercised rather than assumed.
    struct Chunky {
        chunks: Vec<&'static [u8]>,
        head: ResponseHead,
    }

    impl HttpClient for Chunky {
        fn get(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            on_head(&self.head)?;
            for chunk in &self.chunks {
                sink(chunk)?;
            }
            Ok(())
        }
    }

    /// `get_text` accumulates every chunk, in order. A collector that dropped
    /// chunks would break every fetch in this module silently, because each one
    /// goes through here.
    #[test]
    fn text_accumulates_every_chunk_in_order() {
        let client = Chunky {
            chunks: vec![b"he", b"llo", b" world"],
            head: ResponseHead {
                content_length: Some("11".to_string()),
                final_url: String::new(),
            },
        };
        let text = get_text(&client, "u", &[], Duration::from_secs(1)).unwrap();
        assert_eq!(text, "hello world");
    }

    /// The two callbacks run in the order the trait promises — `on_head` before
    /// any body byte — because `install` sets its progress denominator in the
    /// first and reads it in the second. A client that called `sink` first
    /// would leave every progress report dividing by zero, and the suite would
    /// still pass if nothing pinned the order.
    #[test]
    fn the_head_arrives_before_any_body_byte() {
        // `FakeClient` records the request but not the callback order, so this
        // client does the reverse: it delivers a head and a body and nothing
        // else, and the log below is what pins the order. Both callbacks record
        // into *one* log, so the assertion is about the order they ran in
        // rather than about two independent observations that could both be
        // true of a client that ran them backwards.
        struct Ordered;
        impl HttpClient for Ordered {
            fn get(
                &self,
                _url: &str,
                _headers: &[(&str, &str)],
                _timeout: Duration,
                on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
                sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
            ) -> Result<(), RunnerError> {
                on_head(&ResponseHead {
                    content_length: Some("3".to_string()),
                    final_url: String::new(),
                })?;
                sink(b"abc")
            }
        }
        let log = std::cell::RefCell::new(Vec::new());
        Ordered
            .get(
                "u",
                &[],
                Duration::from_secs(1),
                &mut |_head| {
                    log.borrow_mut().push("head");
                    Ok(())
                },
                &mut |_chunk| {
                    log.borrow_mut().push("body");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*log.borrow(), ["head", "body"]);
    }

    /// Either callback's `Err` stops the transfer and reaches the caller
    /// unchanged — the property `install` relies on to abandon an oversized
    /// download rather than buffering it, from the declared length *and* from
    /// the streamed body.
    #[test]
    fn either_callback_can_stop_a_transfer_and_its_error_reaches_the_caller() {
        let client = FakeClient::body("0123456789");
        let mut delivered = Vec::new();
        let error = client
            .get(
                "u",
                &[],
                Duration::from_secs(1),
                &mut |_head| Ok(()),
                &mut |chunk| {
                    delivered.push(chunk.to_vec());
                    Err(RunnerError::Http {
                        message: "abandoned".to_string(),
                    })
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "abandoned");
        // The sink *was* reached — the error is not being produced before the
        // transfer starts, which would also satisfy the assertion above.
        assert!(!delivered.is_empty(), "the sink was never called");

        // The head's own exit: no body byte is delivered at all.
        let mut delivered = Vec::new();
        let error = client
            .get(
                "u",
                &[],
                Duration::from_secs(1),
                &mut |_head| {
                    Err(RunnerError::Http {
                        message: "too big".to_string(),
                    })
                },
                &mut |chunk| {
                    delivered.push(chunk.to_vec());
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "too big");
        assert!(
            delivered.is_empty(),
            "the body was read despite the head aborting"
        );
    }
    // -----------------------------------------------------------------
    // The filesystem half: staging, resolution, install, uninstall
    // -----------------------------------------------------------------

    /// Run `f` against a fresh scratch directory that is cleaned up after.
    fn in_scratch<T>(label: &str, f: impl FnOnce(&Path) -> T) -> T {
        let root = scratch(label);
        let result = f(&root);
        let _ = fs::remove_dir_all(&root);
        result
    }

    /// A release whose download URL points at nothing, for the tests that
    /// never reach the network.
    fn a_release(tag: &str) -> ReleaseInfo {
        ReleaseInfo::new(
            tag,
            "GE-Proton.tar.gz",
            "https://example.invalid/x.tar.gz",
            1024,
        )
    }

    /// The same, with a `size` small enough to pass a test's download cap.
    ///
    /// The size guard runs **first**, so a test that wants to reach the
    /// declared-length guard or the streaming guard must not hand `install` a
    /// release whose `size` already exceeds the cap — the release guard would
    /// fire and the guard under test would never run. Two of these tests were
    /// written that way first and were vacuous: they passed with the guard they
    /// named deleted.
    fn a_small_release(tag: &str, size: i64) -> ReleaseInfo {
        let mut subject = a_release(tag);
        subject.size = size;
        subject
    }

    /// A tar archive holding one directory that is a usable runner.
    fn runner_tar(directory: &str, data: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_path(format!("{directory}/")).unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();

        let mut file = tar::Header::new_gnu();
        file.set_path(format!("{directory}/proton")).unwrap();
        file.set_size(data.len() as u64);
        file.set_mode(0o755);
        file.set_cksum();
        builder.append(&file, data).unwrap();
        builder.into_inner().unwrap()
    }

    /// A client that serves one fixed body, with a head it can lie about.
    struct Serve {
        body: Vec<u8>,
        declared: Option<String>,
        chunks: usize,
        /// How many times `get` was entered. A test that claims a guard fires
        /// "before the network is touched" has to be able to see the network,
        /// and without this counter such a test passes for a guard that fires
        /// after the download — which is the whole property under test.
        calls: std::cell::Cell<usize>,
    }

    impl Serve {
        fn new(body: Vec<u8>) -> Self {
            Self {
                body,
                declared: None,
                chunks: 1,
                calls: std::cell::Cell::new(0),
            }
        }
        fn declaring(mut self, value: &str) -> Self {
            self.declared = Some(value.to_string());
            self
        }
        fn in_chunks(mut self, count: usize) -> Self {
            self.chunks = count.max(1);
            self
        }
        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl HttpClient for Serve {
        fn get(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _timeout: Duration,
            on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<(), RunnerError> {
            self.calls.set(self.calls.get() + 1);
            on_head(&ResponseHead {
                content_length: self.declared.clone(),
                final_url: String::new(),
            })?;
            if self.body.is_empty() {
                return Ok(());
            }
            let size = self.body.len().div_ceil(self.chunks).max(1);
            for chunk in self.body.chunks(size) {
                sink(chunk)?;
            }
            Ok(())
        }
    }

    /// The whole install path, happy: download, extract, validate, metadata,
    /// rename — and then `is_installed` must agree that it landed.
    #[test]
    fn an_install_downloads_stages_validates_and_lands_under_its_install_id() {
        let tar = runner_tar("GE-Proton9-5", b"#!/bin/sh\n");
        in_scratch("proton-install-ok", |root| {
            let runners = root.join("runners");
            let client = Serve::new(tar);
            let seen = std::cell::RefCell::new(Vec::new());
            let target = install(
                &client,
                &runners,
                &a_release("GE-Proton9-5"),
                &|fraction| seen.borrow_mut().push(fraction),
                Duration::from_secs(5),
            )
            .unwrap();

            assert_eq!(target, runners.join("GE-Proton9-5"));
            assert!(target.join("proton").is_file());
            assert!(is_installed(&runners, "GE-Proton9-5", Some("proton-ge")));

            // No staging directory survives, which is the `Drop` half of the
            // contract and is what stops a failed install poisoning the next
            // scan.
            let leftovers: Vec<String> = fs::read_dir(&runners)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".install-"))
                .collect();
            assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");

            // Progress ends at exactly 1.0, and every report is in range.
            let reports = seen.into_inner();
            assert_eq!(reports.last().copied(), Some(1.0));
            assert!(reports.iter().all(|value| (0.0..=1.0).contains(value)));
        });
    }

    /// The metadata file the install writes is the one `is_installed`'s legacy
    /// scan reads, in Python's byte order and with its trailing newline.
    #[test]
    fn the_written_metadata_is_the_four_keys_in_python_s_order_and_newline() {
        let tar = runner_tar("GE-Proton9-5", b"#!/bin/sh\n");
        in_scratch("proton-metadata-bytes", |root| {
            let runners = root.join("runners");
            let mut subject = a_release("GE-Proton9-5");
            subject.name = "GE-Proton9-5.tar.gz".to_string();
            install(
                &Serve::new(tar),
                &runners,
                &subject,
                &|_| {},
                Duration::from_secs(5),
            )
            .unwrap();
            let text =
                fs::read_to_string(runners.join("GE-Proton9-5").join(METADATA_NAME)).unwrap();
            assert_eq!(
                text,
                concat!(
                    "{\n",
                    "  \"family\": \"proton-ge\",\n",
                    "  \"tag\": \"GE-Proton9-5\",\n",
                    "  \"asset\": \"GE-Proton9-5.tar.gz\",\n",
                    "  \"source\": \"GloriousEggroll/proton-ge-custom\"\n",
                    "}\n"
                ),
                "key order, indent and the trailing newline are all Python's"
            );
        });
    }

    /// An install onto an existing directory is refused **before** the network
    /// is touched, and the refusal is `AlreadyInstalled` — the message Python
    /// raises as `FileExistsError`.
    #[test]
    fn an_install_onto_an_existing_directory_is_refused_without_downloading() {
        in_scratch("proton-install-exists", |root| {
            let runners = root.join("runners");
            fs::create_dir_all(runners.join("GE-Proton9-5")).unwrap();
            let client = Serve::new(runner_tar("GE-Proton9-5", b"x"));
            let error = install(
                &client,
                &runners,
                &a_release("GE-Proton9-5"),
                &|_| {},
                Duration::from_secs(5),
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Runner 'GE-Proton9-5' is already installed"
            );
            assert_eq!(client.calls(), 0, "the refusal must precede the download");
        });
    }

    /// A **broken symlink** at the target is refused too. `exists()` alone is
    /// false for one, so a port that checked only that would let the rename
    /// replace a link the user made deliberately.
    #[test]
    fn a_broken_symlink_at_the_target_is_not_silently_replaced() {
        in_scratch("proton-install-symlink", |root| {
            let runners = root.join("runners");
            fs::create_dir_all(&runners).unwrap();
            std::os::unix::fs::symlink("nowhere", runners.join("GE-Proton9-5")).unwrap();
            let client = Serve::new(runner_tar("GE-Proton9-5", b"x"));
            let error = install(
                &client,
                &runners,
                &a_release("GE-Proton9-5"),
                &|_| {},
                Duration::from_secs(5),
            )
            .unwrap_err();
            assert!(matches!(error, RunnerError::AlreadyInstalled { .. }));
            assert!(
                runners.join("GE-Proton9-5").is_symlink(),
                "the link must still be there, un-replaced"
            );
            // The `is_symlink()` half of the pre-check is what makes this
            // *early*. `RENAME_NOREPLACE` refuses the target too, so without
            // this assertion the test passes for a guard that runs after the
            // whole archive has been downloaded and extracted — the same error,
            // the same surviving link, and the download wasted.
            assert_eq!(
                client.calls(),
                0,
                "a broken symlink must be refused before the network is touched"
            );
        });
    }

    /// A declared `Content-Length` over the cap aborts from the **head**, so no
    /// body byte is written at all. That is the cheap check Python does before
    /// its read loop.
    #[test]
    fn a_declared_length_over_the_cap_aborts_before_the_body_is_read() {
        in_scratch("proton-cap-head", |root| {
            let runners = root.join("runners");
            let client = Serve::new(runner_tar("GE-Proton9-5", b"x")).declaring("999999");
            // The cap has to be larger than the body, or the *streaming* guard
            // fires first and this test passes without the declared-length
            // guard existing at all — which is exactly what it did. The body is
            // a small tar and the declared length is far above the cap, so only
            // the head guard can produce the error being asserted.
            let error = install_with(
                &client,
                &runners,
                &a_small_release("GE-Proton9-5", 10),
                &|_| {},
                Duration::from_secs(5),
                100_000,
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Runner archive exceeds the download size limit"
            );
            assert!(!runners.join("GE-Proton9-5").exists());
            // The head was read — the request happened, so the error is not
            // being produced before the transfer starts — and the body was
            // refused, because a `Serve` only reaches its body once `on_head`
            // has returned `Ok`.
            assert_eq!(client.calls(), 1);
            // Nothing was left in staging either — the abort is a `Drop`, not a
            // `return` that skipped cleanup.
            let leftovers: Vec<String> = fs::read_dir(&runners)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
        });
    }

    /// A server that **lies** — a short `Content-Length`, then an oversized
    /// body — is stopped by the streaming guard. This is the reason the guard
    /// exists, and with the real 2 GiB cap no honest test could reach it, which
    /// is why `install_with` takes the cap as a parameter.
    #[test]
    fn a_body_that_crosses_the_cap_is_stopped_while_streaming() {
        in_scratch("proton-cap-stream", |root| {
            let runners = root.join("runners");
            let body = vec![0u8; 4096];
            let client = Serve::new(body).declaring("10").in_chunks(8);
            let error = install_with(
                &client,
                &runners,
                // Size **and** declared length are both under the cap, so the
                // only guard that can produce this error is the streaming one.
                &a_small_release("GE-Proton9-5", 10),
                &|_| {},
                Duration::from_secs(5),
                100,
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Runner archive exceeds the download size limit"
            );
            assert!(!runners.join("GE-Proton9-5").exists());
        });
    }

    /// A **declared size** over the cap (`release.size`) is refused before the
    /// staging directory is even created — including a negative one, which
    /// Python reports as an oversized archive rather than as a bad size.
    #[test]
    fn a_release_size_over_the_cap_is_refused_including_a_negative_one() {
        in_scratch("proton-cap-size", |root| {
            let runners = root.join("runners");
            for size in [-1i64, 101] {
                let mut subject = a_release("GE-Proton9-5");
                subject.size = size;
                let client = Serve::new(runner_tar("GE-Proton9-5", b"x"));
                let error = install_with(
                    &client,
                    &runners,
                    &subject,
                    &|_| {},
                    Duration::from_secs(5),
                    100,
                )
                .unwrap_err();
                assert_eq!(
                    error.to_string(),
                    "Runner archive exceeds the download size limit",
                    "size {size} should read as oversized, not as invalid"
                );
            }
            assert!(!runners.join("GE-Proton9-5").exists());
        });
    }

    /// `_resolve_staged`: a rootless archive keeps its whole tree.
    #[test]
    fn a_rootless_archive_keeps_its_whole_extraction_root() {
        in_scratch("proton-rootless", |root| {
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::write(root.join("bin/wine"), "#!/bin/sh\n").unwrap();
            assert_eq!(resolve_staged(root).unwrap(), root);
        });
    }

    /// `_resolve_staged`: exactly one usable directory is selected.
    #[test]
    fn one_usable_directory_is_selected_from_the_staged_entries() {
        in_scratch("proton-one-dir", |root| {
            fs::create_dir_all(root.join("GE-Proton9-5")).unwrap();
            fs::write(root.join("GE-Proton9-5/proton"), "#!/bin/sh\n").unwrap();
            fs::create_dir_all(root.join("docs")).unwrap();
            fs::write(root.join("README.md"), "hi\n").unwrap();
            assert_eq!(resolve_staged(root).unwrap(), root.join("GE-Proton9-5"));
        });
    }

    /// `_resolve_staged`: two usable directories is ambiguous and raises.
    #[test]
    fn two_usable_directories_are_ambiguous_rather_than_a_guess() {
        in_scratch("proton-two-dirs", |root| {
            for name in ["GE-Proton9-5", "GE-Proton9-6"] {
                fs::create_dir_all(root.join(name)).unwrap();
                fs::write(root.join(name).join("proton"), "#!/bin/sh\n").unwrap();
            }
            let error = resolve_staged(root).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Could not locate one usable runner in the staged archive"
            );
        });
    }

    /// `_resolve_staged`: a usable directory whose name is **not** a safe
    /// install id is an error, not a skipped entry.
    ///
    /// This is the trap. Python calls `safe_install_id(entry.name)` and lets it
    /// raise; a porting hand reaches for `continue`, because "this entry cannot
    /// be the runner" reads like a filter. It is not: an archive shipping
    /// `.wine/proton` beside a real `GE-Proton9-5/` must fail, and a `continue`
    /// would install the second and never mention the first.
    #[test]
    fn a_usable_directory_whose_name_is_unsafe_is_an_error_not_a_skip() {
        in_scratch("proton-unsafe-name", |root| {
            for name in [".wine", "..wine"] {
                fs::create_dir_all(root.join(name)).unwrap();
                // A nested `bin/wine`, so the candidate is usable and the name
                // is the only thing wrong with it.
                fs::create_dir_all(root.join(name).join("bin")).unwrap();
                fs::write(root.join(name).join("bin/wine"), "#!/bin/sh\n").unwrap();
            }
            let error = resolve_staged(root).unwrap_err();
            assert!(
                error.to_string().starts_with("Unsafe runner id:"),
                "expected the id refusal, got {error}"
            );
        });
    }

    /// `_resolve_staged`: a **top-level symlink** refuses the archive — but
    /// only after the rootless check, which is why this fixture is not itself a
    /// runner.
    #[test]
    fn a_top_level_symlink_refuses_the_archive() {
        in_scratch("proton-top-link", |root| {
            std::os::unix::fs::symlink("/etc", root.join("etc")).unwrap();
            fs::create_dir_all(root.join("GE-Proton9-5")).unwrap();
            fs::write(root.join("GE-Proton9-5/proton"), "#!/bin/sh\n").unwrap();
            let error = resolve_staged(root).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Runner archive contains an unsafe top-level link"
            );
        });
    }

    /// `_resolve_staged`: the rootless check runs **before** the top-level
    /// symlink refusal, and that order is deliberate.
    ///
    /// A rootless archive is kept wholesale, so a symlink anywhere in it is
    /// judged later, by `validate_staged_runner`, against the tree as it will
    /// actually land. Refusing here first would reject a rootless archive whose
    /// own layout includes a link — which real Wine builds do.
    #[test]
    fn a_rootless_archive_is_kept_even_when_it_contains_a_top_level_link() {
        in_scratch("proton-rootless-link", |root| {
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::write(root.join("bin/wine"), "#!/bin/sh\n").unwrap();
            std::os::unix::fs::symlink("bin/wine", root.join("wine")).unwrap();
            assert_eq!(
                resolve_staged(root).unwrap(),
                root,
                "the rootless tree is kept, symlink and all — the link is \
                 `validate_staged_runner`'s to judge, not this function's"
            );
        });
    }

    /// `_resolve_staged`: nothing usable at all.
    #[test]
    fn a_staged_tree_with_no_runner_is_an_error() {
        in_scratch("proton-none", |root| {
            fs::create_dir_all(root.join("docs")).unwrap();
            let error = resolve_staged(root).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Could not locate one usable runner in the staged archive"
            );
        });
    }

    /// A `proton` entry that is a **directory** is usable to `is_installed`'s
    /// first check (`exists`) and not to `_resolve_staged` (`is_file`). Both
    /// spellings are Python's and both are pinned here, because unifying them
    /// would be an invisible change of behaviour on the path that decides
    /// whether the UI offers to install a build that is already on disk.
    #[test]
    fn a_proton_directory_installs_as_installed_but_does_not_resolve_as_staged() {
        in_scratch("proton-proton-dir", |root| {
            // `is_installed`'s first check: `(target / "proton").exists()`.
            let runners = root.join("runners");
            fs::create_dir_all(runners.join("GE-Proton9-5").join("proton")).unwrap();
            assert!(
                is_installed(&runners, "GE-Proton9-5", Some("proton-ge")),
                "`exists()` accepts a directory called `proton`"
            );

            // `_resolve_staged`: `(entry / "proton").is_file()`.
            let staged = root.join("staged");
            fs::create_dir_all(staged.join("GE-Proton9-5").join("proton")).unwrap();
            assert!(
                matches!(resolve_staged(&staged), Err(RunnerError::NoUsableRunner)),
                "`is_file()` does not accept a directory called `proton`"
            );
        });
    }

    /// `rename_noreplace` is the atomic half of the install, and its whole
    /// value is that it **refuses** rather than replacing.
    #[test]
    fn rename_noreplace_refuses_an_existing_target_and_leaves_it_untouched() {
        in_scratch("proton-rename", |root| {
            let source = root.join("source");
            let target = root.join("target");
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("proton"), "new\n").unwrap();
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join("proton"), "old\n").unwrap();

            let error = rename_noreplace(&source, &target).unwrap_err();
            assert_eq!(error.to_string(), "Runner 'target' is already installed");
            assert_eq!(fs::read_to_string(target.join("proton")).unwrap(), "old\n");
            assert!(
                source.is_dir(),
                "a refused rename must not consume the source"
            );

            // The happy path, once the target is gone.
            fs::remove_dir_all(&target).unwrap();
            rename_noreplace(&source, &target).unwrap();
            assert_eq!(fs::read_to_string(target.join("proton")).unwrap(), "new\n");
            assert!(!source.exists());
        });
    }

    /// `uninstall`: the two ids that name something the user did not install.
    #[test]
    fn the_system_runner_and_the_empty_id_cannot_be_uninstalled() {
        in_scratch("proton-uninstall-guard", |root| {
            for id in ["", SYSTEM_WINE] {
                let error = uninstall(root, id).unwrap_err();
                assert_eq!(error.to_string(), "System Wine cannot be uninstalled");
            }
        });
    }

    /// `uninstall`: an unsafe id **raises**, which is different from the
    /// silent no-op below — the UI reports the failure, and `safe_install_id`
    /// returning `Ok` for `"../.."` here would be a delete outside the runners
    /// directory.
    #[test]
    fn uninstalling_an_unsafe_id_raises_rather_than_deleting_outside() {
        in_scratch("proton-uninstall-unsafe", |root| {
            let runners = root.join("runners");
            fs::create_dir_all(&runners).unwrap();
            // A sibling that must survive: were the traversal followed, this is
            // what `../..` would reach.
            let outside = root.join("precious");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("keep"), "x").unwrap();

            for id in ["../precious", "..", "."] {
                let error = uninstall(&runners, id).unwrap_err();
                assert!(
                    error.to_string().starts_with("Unsafe runner id:"),
                    "{id:?} should be refused, got {error}"
                );
            }
            assert!(
                outside.join("keep").is_file(),
                "nothing outside was touched"
            );
        });
    }

    /// `uninstall`: an unknown id and a missing directory are silent no-ops, and
    /// a symlink is removed **as a link** — never followed.
    ///
    /// The link half changed under `BUG-03`. What the test has always been for is
    /// the safety property, and that is unchanged: `real/keep` must survive,
    /// because deleting through the link would destroy a directory the app was
    /// never asked to touch. What changed is that the link itself now goes, so
    /// that the "Removed {id}" the UI toasts is true.
    #[test]
    fn uninstalling_an_unknown_or_linked_id_is_a_silent_no_op() {
        in_scratch("proton-uninstall-noop", |root| {
            let runners = root.join("runners");
            fs::create_dir_all(&runners).unwrap();

            uninstall(&runners, "GE-Proton9-5").unwrap();

            // A symlink to a real directory of ours, outside the runners dir.
            let real = root.join("real");
            fs::create_dir_all(&real).unwrap();
            fs::write(real.join("keep"), "x").unwrap();
            std::os::unix::fs::symlink(&real, runners.join("GE-Proton9-6")).unwrap();
            uninstall(&runners, "GE-Proton9-6").unwrap();
            assert!(real.join("keep").is_file(), "the link target must survive");
            assert!(
                !runners.join("GE-Proton9-6").exists(),
                "the link itself must be gone, or the UI toasts a removal that did not happen"
            );

            // A plain file where a directory would be.
            fs::write(runners.join("GE-Proton9-7"), "x").unwrap();
            uninstall(&runners, "GE-Proton9-7").unwrap();
            assert!(runners.join("GE-Proton9-7").is_file());
        });
    }

    /// `BUG-03`, end to end: a symlinked build that the UI **lists** can be
    /// removed, and stops being listed.
    ///
    /// This is the assertion that was missing. The old test checked `uninstall`
    /// in isolation, where a silent no-op and a successful removal are the same
    /// value — `Ok(())` — so it passed while the Remove button was lying. What
    /// makes the toast true is that the row disappears, which is a property of
    /// the *listing* and the removal together.
    #[test]
    fn a_listed_symlinked_build_can_actually_be_removed() {
        in_scratch("proton-uninstall-linked-listing", |root| {
            let runners = root.join("runners");
            let real = root.join("real");
            fs::create_dir_all(real.join("files/bin")).unwrap();
            fs::write(real.join("proton"), "#!/bin/sh\n").unwrap();

            let manager = RunnerManager::new(
                &FakeLaunchEnv::new()
                    .with_vars(&[("GAMEHANDLER_DATA_HOME", root.to_str().unwrap())]),
            );
            let linked = runners.join("GE-Proton9-9");
            fs::create_dir_all(&runners).unwrap();
            std::os::unix::fs::symlink(&real, &linked).unwrap();

            // Precondition: it is listed, which is what puts a Remove button on
            // screen in the first place. Without this the test could pass on a
            // build that was never visible.
            let before: Vec<String> = manager
                .installed_protons()
                .iter()
                .map(|runner| runner.id.clone())
                .collect();
            assert!(
                before.contains(&"GE-Proton9-9".to_string()),
                "a symlinked build is listed as installed; got {before:?}"
            );

            uninstall(&runners, "GE-Proton9-9").unwrap();

            let after: Vec<String> = manager
                .installed_protons()
                .iter()
                .map(|runner| runner.id.clone())
                .collect();
            assert!(
                !after.contains(&"GE-Proton9-9".to_string()),
                "after Remove the build must no longer be listed; got {after:?}"
            );
            // And the build the link referred to is untouched.
            assert!(real.join("proton").is_file());
        });
    }

    /// `uninstall`: a real install goes, and takes its tree with it.
    #[test]
    fn uninstalling_a_real_install_removes_the_tree() {
        in_scratch("proton-uninstall-real", |root| {
            let runners = root.join("runners");
            let target = runners.join("GE-Proton9-5");
            fs::create_dir_all(target.join("files/bin")).unwrap();
            fs::write(target.join("proton"), "#!/bin/sh\n").unwrap();
            uninstall(&runners, "GE-Proton9-5").unwrap();
            assert!(!target.exists());
        });
    }

    /// Every install id the catalogue can produce is *already* a safe install
    /// id, so `install`'s extra `safe_install_id` call is a no-op today.
    ///
    /// That call is Python's (`safe_install_id(release.install_id)`) and it is
    /// kept as defence in depth: it is what stands between a future edit to
    /// `install_id` and a directory name that escapes `runners/`. But a
    /// redundant guard is invisible — with the outer call deleted the whole
    /// suite still passes, because the inner one did the work, and the mutation
    /// was measured surviving.
    ///
    /// Deletion is not the right answer, so this test states the invariant that
    /// justifies the redundancy instead. If `install_id` ever stops producing
    /// safe names, this fails and the outer call becomes load-bearing *visibly*
    /// rather than silently.
    #[test]
    fn every_install_id_the_catalogue_can_produce_is_already_safe() {
        let tags = [
            "GE-Proton9-5",
            "v1.0.0",
            "../../etc",
            "",
            "   ",
            "tag with/slash",
            "..",
            ".hidden",
            "日本 語",
            "~hdeadbeef",
        ];
        let padded = "x".repeat(300);
        let mut checked = 0;
        for family in crate::runners::families::families() {
            for tag in tags.iter().copied().chain(std::iter::once(padded.as_str())) {
                let Ok(id) = crate::runners::families::install_id_for(tag, family.id) else {
                    // An id that cannot be built at all is refused by
                    // `install_id` itself, which is a different guard.
                    continue;
                };
                assert_eq!(
                    safe_install_id(&id).unwrap_or_else(|error| panic!(
                        "install_id_for({tag:?}, {:?}) produced {id:?}, which \
                         `safe_install_id` refuses: {error}. The outer call in \
                         `install` has just become load-bearing.",
                        family.id
                    )),
                    id
                );
                checked += 1;
            }
        }
        assert!(
            checked > 10,
            "the loop should have exercised a real spread of tags, checked {checked}"
        );
    }

    /// The staging directory is created `0700` — the tree is attacker-
    /// influenced and is validated *after* extraction, so between those two
    /// moments nobody else may read it.
    #[test]
    fn the_staging_directory_is_private_while_it_exists() {
        use std::os::unix::fs::PermissionsExt;
        in_scratch("proton-staging-mode", |root| {
            let staging = StagingDirectory::create(root).unwrap();
            let mode = fs::metadata(&staging.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "staging must be owner-only");

            // And it is gone once dropped.
            let path = staging.path.clone();
            drop(staging);
            assert!(!path.exists(), "the staging directory must clean itself up");
        });
    }
}
