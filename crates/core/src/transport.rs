//! Shared bounded HTTP transport contract; compatibility runners re-export it.
use crate::runners::RunnerError;
use std::time::Duration;
/// What a completed [`HttpClient::get`] call learned about the response.
///
/// `content_length` is the **raw header text**, not a number, and that is
/// deliberate: Python parses it with `int()` inside a `try`/`except` that turns
/// a bad value into a specific `RuntimeError`, so the parse is part of the
/// behaviour being ported and belongs in `parse_content_length` where it is
/// tested — not hidden inside a client implementation whose only job is
/// transport. A client that pre-parsed would make that error path
/// unreachable, and D-26 exists precisely so the error paths stay reachable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseHead {
    /// The `Content-Length` header verbatim, or `None` if it was absent.
    pub content_length: Option<String>,
    /// The URL the response actually came from, **after every redirect**.
    ///
    /// Required rather than optional, and empty is not a valid answer: both
    /// callers that read it are origin checks — `crate::installers`'s
    /// `allowed_hosts` validation and `runners::proton::install_with`'s scheme check — and a
    /// field that could be `None` would need a rule for what `None` means. The
    /// only two candidates are "trust the request URL instead" — which turns a
    /// redirect to an attacker's host into an accepted download — and "reject",
    /// which is the same as an empty string here. Making it a plain `String`
    /// removes the question: a client that cannot report the final URL reports
    /// `""`, which no origin check accepts, so it fails closed.
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
