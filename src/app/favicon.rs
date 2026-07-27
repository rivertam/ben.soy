//! Root `/favicon.ico` for clients that never read HTML — RSS readers
//! grabbing a feed icon, legacy crawlers. Browsers use the hashed PNG links
//! in the shell head (`components/chrome.rs`); this is the fallback at the
//! one path everything guesses. Not a page: no shell, stays out of
//! `site_routes()` (the 404 index is for pages).

use topcoat::{Result, router::route};

/// 16 + 32 px frames rendered from the same source as the shell's PNGs.
const FAVICON_ICO: &[u8] = include_bytes!("../components/favicon/favicon.ico");

#[route(GET "/favicon.ico")]
async fn favicon_ico() -> Result<([(&'static str, &'static str); 2], &'static [u8])> {
    Ok((
        [
            ("Content-Type", "image/x-icon"),
            // Unhashed URL, so cap browser caching at a day; deploy CI purges
            // the CDN, so s-maxage can ride the same value.
            ("Cache-Control", "public, max-age=86400, s-maxage=86400"),
        ],
        FAVICON_ICO,
    ))
}
