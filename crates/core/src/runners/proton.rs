//! Proton and Wine build management: listing, installing and removing builds
//! published on GitHub. Port of `runners.py:776-936` (the `ProtonManager`
//! class), `_write_metadata` (`runners.py:709`) and `_rename_noreplace`
//! (`runners.py:663`).
//!
//! # What this slice covers, and what lands next
//!
//! This is the *pure* half: the GitHub payload parser, the header parser, the
//! installed-build probe, and the injected [`HttpClient`] of D-26. Together
//! they are everything that can be exercised without a network or a live
//! install, which is what lets all of it run offline.
//!
//! The filesystem half — `install`, `uninstall`, `_resolve_staged`,
//! `_write_metadata`, `_rename_noreplace` — is the next increment, for the same
//! reason `archive.rs` and `families.rs` were split from `mod.rs`: the pure
//! layer is where the compatibility risk is, and pinning it first means the
//! staging logic is written against a parser that is already known to agree
//! with Python. [`MAX_RUNNER_ARCHIVE_BYTES`] and [`extract_archive`] are
//! already in place for it.
//!
//! # Deliberate divergences
//!
//! Four, and each is pinned by a test rather than left to a reader:
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

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::archive::{safe_install_id, sanitise_release_tag};
use super::families::{
    family_by_id, find_wine_binary, is_truthy, pick_asset, python_str, ReleaseInfo, RunnerFamily,
};
use super::{read_metadata, RunnerError, USER_AGENT};

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

/// What a completed [`HttpClient::get_chunked`] call learned about the response.
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
}

/// A blocking HTTP GET, injected rather than imported (D-26).
///
/// `core` deliberately has no networking dependency, so the concrete client
/// lives in the binary crate and this trait is the seam. Only one method is
/// required, because a whole-body fetch is a chunked fetch that happens to
/// accumulate — which is what keeps a test double from having to implement two
/// behaviours that could disagree.
///
/// # Why the sink returns a `Result`
///
/// `install` must abandon a transfer the moment it exceeds the size cap rather
/// than buffering a hostile multi-gigabyte body first, so the callback has to
/// be able to stop the transfer. Python gets this from an exception raised
/// inside its `while` loop; here it is the sink's `Err`, which the client
/// propagates unchanged.
pub trait HttpClient {
    /// GET `url`, calling `sink` with each chunk as it arrives.
    ///
    /// Returns the response head once the body is complete. A non-success
    /// status, a transport failure, or an `Err` from `sink` is `Err`.
    fn get_chunked(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
    ) -> Result<ResponseHead, RunnerError>;

    /// GET `url` and collect the whole body. Provided, not required.
    fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<(ResponseHead, Vec<u8>), RunnerError> {
        let mut body = Vec::new();
        let head = self.get_chunked(url, headers, timeout, &mut |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        })?;
        Ok((head, body))
    }
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
        magnitude = magnitude.saturating_mul(10).saturating_add(i128::from(value));
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
        Some(text) => python_int(text).ok_or_else(|| {
            RunnerError::Http {
                message: "Runner download has an invalid Content-Length".to_string(),
            }
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
    family_by_id(family.unwrap_or(DEFAULT_FAMILY_ID)).map_err(|message| RunnerError::Http {
        message,
    })
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
    let (_, body) = client.get(&resolved.releases_url(), &headers, timeout)?;

    let text = String::from_utf8(body).map_err(|_| RunnerError::Http {
        // Divergence 4: Python raises an uncaught `UnicodeDecodeError` here.
        message: "Unexpected GitHub releases response".to_string(),
    })?;
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

    if is_usable_runner(&target) {
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
        let matches_family = matches!(metadata.get("family"), Some(Value::String(text)) if *text == family_id);
        let matches_tag = matches!(metadata.get("tag"), Some(Value::String(text)) if *text == tag);
        if matches_family && matches_tag && is_usable_runner(&child) {
            return true;
        }
    }
    false
}

/// [`is_installed`] for a release, which is the form every caller has.
pub fn is_release_installed(runners_directory: &Path, release: &ReleaseInfo) -> bool {
    is_installed(runners_directory, &release.tag, Some(&release.family_id))
}

/// `find_wine_binary(target) is not None or (target / "proton").exists()`.
///
/// The `proton` check is `.exists()`, not `.is_file()`, matching Python — a
/// `proton` entry that is a **broken symlink** therefore counts as usable here.
/// That is Python's behaviour and it is kept rather than tidied, because this
/// predicate also decides whether a directory the user can see is offered for
/// re-installation, and the two implementations must agree on that.
fn is_usable_runner(path: &Path) -> bool {
    find_wine_binary(path).is_some() || path.join("proton").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::env::tests::scratch;
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
            ("\u{661}\u{662}", 12),          // Arabic-Indic ١٢
            ("\u{663}", 3),                  // ٣
            ("\u{661}_\u{662}", 12),         // with a PEP 515 underscore
            ("\u{661}\u{662}_\u{663}", 123),
            ("\u{665}\u{665}\u{665}", 555),  // ٥٥٥
            ("\u{663}\u{664}", 34),          // ٣٤
            ("\u{ff11}\u{ff12}", 12),        // fullwidth １２
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
        for falsy in [json!(null), json!(false), json!(0), json!(""), json!([]), json!({})] {
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
        let data = vec![release("v1", serde_json::json!([asset("source.tar.gz", "u", 7)]))];
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
        assert_eq!(
            parse_one(serde_json::json!({"assets": good_asset()})),
            ""
        );
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
            parse_releases(&nameless, Some("proton-ge")).unwrap().is_empty(),
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
        assert_eq!(parse_releases(&data, None).unwrap()[0].family_id, "proton-ge");
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
        let taken = parse_releases(&one("wine-10.0-proton-amd64.tar.xz"), Some("wine-proton"))
            .unwrap();
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
        let ge = parse_releases(&one("wine-10.0-proton-win32.tar.xz"), Some("proton-ge"))
            .unwrap();
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
        fn body(text: &str) -> Self {
            Self {
                status: Ok((ResponseHead { content_length: None }, text.as_bytes().to_vec())),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn bytes(raw: &[u8]) -> Self {
            Self {
                status: Ok((ResponseHead { content_length: None }, raw.to_vec())),
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn url(&self) -> String {
            self.seen.borrow()[0].0.clone()
        }
    }

    impl HttpClient for FakeClient {
        fn get_chunked(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            _timeout: Duration,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
        ) -> Result<ResponseHead, RunnerError> {
            self.seen.borrow_mut().push((
                url.to_string(),
                headers
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            ));
            match &self.status {
                Ok((head, body)) => {
                    sink(body)?;
                    Ok(head.clone())
                }
                Err(RunnerError::Http { message }) => {
                    Err(RunnerError::Http { message: message.clone() })
                }
                Err(_) => Err(RunnerError::Http { message: "transport".to_string() }),
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
        assert_eq!(client.url(), "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases");
        let headers = &client.seen.borrow()[0].1;
        assert!(headers.contains(&("Accept".to_string(), "application/vnd.github+json".to_string())));
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
            assert_eq!(error.to_string(), "Unexpected GitHub releases response", "{body}");
        }
        // An empty array is a *successful* empty listing — the contrast that
        // shows the check is not simply rejecting everything.
        let client = FakeClient::body("[]");
        assert!(fetch_available(&client, None, 15, Duration::from_secs(30)).unwrap().is_empty());
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
            status: Err(RunnerError::Http { message: "connection refused".to_string() }),
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
        let namespaced = crate::runners::families::install_id_for("v1.0", "proton-cachyos").unwrap();
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
        fs::write(
            garbage.join(crate::runners::METADATA_NAME),
            "{not json",
        )
        .unwrap();
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
        assert!(!is_installed(&root.join("does-not-exist"), "v1.0", Some("proton-ge")));
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
    // The HTTP client's provided method
    // -----------------------------------------------------------------

    /// `get` is provided in terms of `get_chunked`, so a double only implements
    /// one method. This pins that the accumulation is correct — a provided
    /// method that dropped chunks would break every fetch above silently.
    #[test]
    fn the_provided_get_accumulates_every_chunk_in_order() {
        struct Chunky;
        impl HttpClient for Chunky {
            fn get_chunked(
                &self,
                _url: &str,
                _headers: &[(&str, &str)],
                _timeout: Duration,
                sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
            ) -> Result<ResponseHead, RunnerError> {
                for chunk in [b"he".as_slice(), b"llo".as_slice(), b" world".as_slice()] {
                    sink(chunk)?;
                }
                Ok(ResponseHead { content_length: Some("11".to_string()) })
            }
        }
        let (head, body) = Chunky
            .get("u", &[], Duration::from_secs(1))
            .unwrap();
        assert_eq!(body, b"hello world");
        assert_eq!(head.content_length.as_deref(), Some("11"));
    }

    /// The sink's `Err` stops the transfer and reaches the caller unchanged —
    /// the property `install` will rely on to abandon an oversized download
    /// rather than buffering it.
    ///
    /// Written against `get_chunked` directly, because `get`'s sink never
    /// fails: testing the stop through `get` would require a sink that cannot
    /// exist, and the first version of this test asserted an error that the
    /// provided sink could never produce.
    #[test]
    fn the_sink_can_stop_a_transfer_and_its_error_reaches_the_caller() {
        let client = FakeClient::body("0123456789");
        let mut delivered = Vec::new();
        let error = client
            .get_chunked("u", &[], Duration::from_secs(1), &mut |chunk| {
                delivered.push(chunk.to_vec());
                Err(RunnerError::Http { message: "abandoned".to_string() })
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "abandoned");
        // The sink *was* reached — the error is not being produced before the
        // transfer starts, which would also satisfy the assertion above.
        assert!(!delivered.is_empty(), "the sink was never called");
    }
}
