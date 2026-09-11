//! The concrete [`HttpClient`] this binary injects into `core` (DECISIONS D-26).
//!
//! `core` defines the trait and never imports a client, so its whole test suite
//! stays offline and hermetic; the binary picks the concrete one and passes it
//! in. (The count that used to stand where "whole test suite" now does was 358,
//! and it was stale within a commit — a number in a sentence about a property
//! is archaeology, and the property is what the sentence is for.)
//! This module is that choice and nothing else.
//!
//! # Which client, and why the system trust store
//!
//! `ureq` with `rustls`, choosing `RootCerts::PlatformVerifier` rather than
//! the default `RootCerts::WebPki`. The choice of the *system* store over a
//! bundled one is deliberate (D-48): a user who has installed a corporate or
//! self-signed CA into their own trust configuration expects this app to honour
//! it, and a bundled root list would silently ignore it. `ureq`'s default is
//! the bundled Mozilla list, which is the wrong thing here and would have been
//! easy to accept by not choosing — hence this note.
//!
//! # Why one agent for the process
//!
//! The TLS configuration is the expensive part to build: the platform verifier
//! reads the system trust store. Building an agent per request would re-read it
//! on every page fetch, so the agent (which also owns the connection pool) is
//! built once and shared. The per-call timeout in the trait is honoured with
//! `timeout_global` at request scope, so sharing the agent does not cost the
//! caller its timeout.

use std::io::Read;
use std::sync::LazyLock;
use std::time::Duration;

use gamehandler_core::runners::RunnerError;
use gamehandler_core::runners::proton::{HttpClient, ResponseHead};
use ureq::ResponseExt;

/// Bytes handed to [`HttpClient::get`]'s sink per call.
///
/// The trait's sink is a callback rather than a reader precisely so a caller
/// can stop a transfer it has already decided is unacceptable, and that only
/// works if the chunks arrive in pieces smaller than the whole body.
const CHUNK: usize = 16 * 1024;

/// The process-wide agent. See this module's note on why it is not per request.
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let config = ureq::config::Config::builder()
        .tls_config(tls)
        // A non-success status is `Err`, which is what the trait promises and
        // what Python's `raise_for_status` does. ureq's default is already
        // `true`; it is set here so this cannot be lost to a default change.
        .http_status_as_error(true)
        .build();
    ureq::Agent::new_with_config(config)
});

/// The blocking [`HttpClient`] over `ureq`.
pub struct UreqClient;

impl HttpClient for UreqClient {
    fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
        on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError> {
        let mut request = AGENT
            .get(url)
            .config()
            .timeout_global(Some(timeout))
            .build();
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.call().map_err(to_runner_error)?;

        // Before any body byte, which is what the trait requires: `install`
        // uses the declared length for the progress denominator and for the
        // cheap size-cap refusal, and both are wrong if the head arrives late.
        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // `get_uri()` is ureq's `resp.geturl()`: the URI this response is
        // *about*, which differs from the request URI exactly when a redirect
        // was followed (ureq's own doc says so, and `ResponseExt` is the trait
        // that exposes it). ureq follows up to ten redirects by default, so a
        // host that bounces this request elsewhere is reported here as
        // elsewhere — which is what makes an allowlist check possible at all.
        on_head(&ResponseHead {
            content_length,
            final_url: response.get_uri().to_string(),
        })?;

        let mut reader = response.into_body().into_reader();
        let mut buffer = vec![0u8; CHUNK];
        loop {
            let read = reader.read(&mut buffer).map_err(to_body_error)?;
            if read == 0 {
                return Ok(());
            }
            sink(&buffer[..read])?;
        }
    }
}

/// A transport failure or non-success status, as the trait's `Err`.
///
/// `RunnerError::Http` carries a message rather than a status code because
/// every other HTTP failure in this port does — `"Unexpected GitHub releases
/// response"`, `"Runner download has an invalid Content-Length"` — and the
/// message is what reaches the user through the status line.
fn to_runner_error(error: ureq::Error) -> RunnerError {
    RunnerError::Http {
        message: error.to_string(),
    }
}

/// A read failing **mid-body**, after the head was already delivered.
///
/// This is a different type from [`to_runner_error`] — the reader is a
/// `std::io::Read`, so a truncated or reset transfer arrives as an `io::Error`
/// rather than a `ureq::Error` — and it maps into the same variant on purpose.
/// It is a transport failure like any other, and both Display as the raw
/// message, which is what `_async`'s `str(exc)` puts in the status line
/// (`bridge.py:157`). Mapping it to [`RunnerError::Io`] would render the same
/// sentence while classifying it as a local filesystem fault, which is the
/// wrong half of the app to blame.
fn to_body_error(error: std::io::Error) -> RunnerError {
    RunnerError::Http {
        message: error.to_string(),
    }
}
