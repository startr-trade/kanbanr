//! The monitor, baked into the binary (FEAT-084).
//!
//! `build.rs` gzips everything under `web/dist` into a table of `(path, compressed bytes)`. This
//! module decompresses it once, on first use, into a map the router can serve from — so the binary
//! carries the compressed size while the serving path deals in plain bytes and needs no
//! content-negotiation. A build with no `web/dist` produces an empty table, which `is_empty`
//! reports so the daemon can say the monitor is missing rather than serving an API that silently
//! has no web view.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// Every embedded file, keyed by its path relative to `web/dist` (`index.html`, `assets/x.js`, …).
pub fn assets() -> &'static HashMap<String, Vec<u8>> {
    static UNPACKED: OnceLock<HashMap<String, Vec<u8>>> = OnceLock::new();
    UNPACKED.get_or_init(|| {
        ASSETS
            .iter()
            .filter_map(|(path, gz)| {
                let mut out = Vec::new();
                flate2::read::GzDecoder::new(*gz)
                    .read_to_end(&mut out)
                    .ok()?;
                Some((path.to_string(), out))
            })
            .collect()
    })
}

/// Was this binary built without the monitor? (No `web/dist` at build time.)
pub fn is_empty() -> bool {
    ASSETS.is_empty()
}

/// The content type for a path, from its extension. Deliberately a short explicit list rather than a
/// mime crate: these are the only kinds Vite emits, and an unknown one is better served as bytes the
/// browser will not guess at than as a wrong guess.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
