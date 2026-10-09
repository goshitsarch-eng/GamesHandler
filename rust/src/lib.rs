//! Coarse desktop API. Widgets and presentation state never enter this crate.
mod api;
mod jobs;
#[cfg(target_os = "linux")]
mod linux;
mod native;

/// The same request protocol used by the native desktop library and headless CLI.
pub fn request_json(bytes: &[u8]) -> serde_json::Value {
    api::execute(bytes)
}

use std::ffi::{CString, c_char};

#[unsafe(no_mangle)]
pub extern "C" fn gamehandler_abi_version() -> u32 {
    1
}

/// Execute one UTF-8 request and return an owned, NUL-terminated JSON response.
///
/// # Safety
/// `input` must reference `length` readable bytes for this call. The caller owns
/// input. Free the returned pointer exactly once with `gamehandler_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamehandler_request(input: *const u8, length: usize) -> *mut c_char {
    let response = if input.is_null() || length > 8 * 1024 * 1024 {
        serde_json::json!({"ok":false,"error":{"kind":"validation","message":"Invalid request buffer"}})
    } else {
        // SAFETY: the ABI caller guarantees a readable borrowed input buffer.
        let bytes = unsafe { std::slice::from_raw_parts(input, length) };
        std::panic::catch_unwind(|| api::execute(bytes)).unwrap_or_else(|_| {
            serde_json::json!({"ok":false,"error":{"kind":"internal","message":"The operation failed unexpectedly"}})
        })
    };
    // JSON serialization escapes NULs; this buffer therefore has no interior NUL.
    CString::new(response.to_string())
        .expect("JSON contains no raw NUL")
        .into_raw()
}

/// Release a response returned by `gamehandler_request`.
///
/// # Safety
/// `pointer` must be null or a live pointer returned by this library; no double free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamehandler_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        // SAFETY: the caller transfers the response allocation back exactly once.
        drop(unsafe { CString::from_raw(pointer) });
    }
}
