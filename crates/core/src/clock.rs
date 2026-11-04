/// Milliseconds since the Unix epoch.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Browsers have no system clock for Rust to read, so ask JavaScript.
#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}
