//! Stable-URL endpoints for the /diary PWA: the service worker, the web app
//! manifest, and the launcher icons. Not pages: no shell, out of
//! `site_routes()` (the 404 index is for pages) — `favicon.rs` is the
//! pattern. The interactive halves live in `diary/sw.js` and
//! `diary/diary.js`; the queue those two load is Rust served by
//! `diary_sync.rs`; the write endpoint is `POST /api/diary/entries` in
//! `diary.rs`.
//!
//! Deliberately ungated: Chrome fetches manifests without credentials (a
//! cookie gate would break install), and none of these bytes are private —
//! they disclose that a diary app exists, which the public repo and the
//! /diary login redirect already do. Every route declares an explicit
//! Content-Type: the response layer treats untyped bodies as HTML and runs
//! the em-dash rewriter through them.

use topcoat::{Result, router::route};

/// The worker's URL is its identity to the browser: a hashed `asset!` URL
/// would register a brand-new worker every deploy and orphan the old one.
/// Served `no-cache` so the edge always revalidates; browsers bypass the
/// HTTP cache for service-worker update checks regardless.
const SW_JS: &str = include_str!("diary/sw.js");

/// App identity + install metadata. `scope`/`start_url` stay on `/diary` so
/// the installed app never claims the public site; the colors match
/// `--color-page` in `styles/input.css` (and the shell's `theme-color`
/// meta) so the standalone status bar blends into the page.
const MANIFEST: &str = r##"{
  "name": "Diary",
  "short_name": "Diary",
  "id": "/diary",
  "start_url": "/diary",
  "scope": "/diary",
  "display": "standalone",
  "background_color": "#f4f5f7",
  "theme_color": "#f4f5f7",
  "icons": [
    { "src": "/diary-icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" },
    { "src": "/diary-icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }
  ]
}"##;

/// The favicon sponge, point-scaled full-bleed (32 → 192/512 are integer
/// multiples); the subject sits centered, which is what Android's maskable
/// circle crop needs.
const ICON_192: &[u8] = include_bytes!("diary/diary-icon-192.png");
const ICON_512: &[u8] = include_bytes!("diary/diary-icon-512.png");

/// Unhashed URLs, so cap caching at a day like `/favicon.ico`; deploys purge
/// the CDN, so `s-maxage` can ride the same value.
const DAY_CACHE: &str = "public, max-age=86400, s-maxage=86400";

#[route(GET "/sw.js")]
async fn service_worker() -> Result<([(&'static str, &'static str); 2], &'static str)> {
    Ok((
        [
            ("Content-Type", "text/javascript; charset=utf-8"),
            ("Cache-Control", "no-cache"),
        ],
        SW_JS,
    ))
}

#[route(GET "/diary.webmanifest")]
async fn manifest() -> Result<([(&'static str, &'static str); 2], &'static str)> {
    Ok((
        [
            ("Content-Type", "application/manifest+json"),
            ("Cache-Control", DAY_CACHE),
        ],
        MANIFEST,
    ))
}

#[route(GET "/diary-icon-192.png")]
async fn icon_192() -> Result<([(&'static str, &'static str); 2], &'static [u8])> {
    Ok((
        [("Content-Type", "image/png"), ("Cache-Control", DAY_CACHE)],
        ICON_192,
    ))
}

#[route(GET "/diary-icon-512.png")]
async fn icon_512() -> Result<([(&'static str, &'static str); 2], &'static [u8])> {
    Ok((
        [("Content-Type", "image/png"), ("Cache-Control", DAY_CACHE)],
        ICON_512,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIARY_JS: &str = include_str!("diary/diary.js");

    #[test]
    fn manifest_declares_the_diary_app() {
        let parsed: serde_json::Value = serde_json::from_str(MANIFEST).expect("manifest parses");
        assert_eq!(parsed["name"], "Diary");
        assert_eq!(parsed["id"], "/diary");
        assert_eq!(parsed["start_url"], "/diary");
        assert_eq!(parsed["scope"], "/diary");
        assert_eq!(parsed["display"], "standalone");
        let icons = parsed["icons"].as_array().expect("icon list");
        let sources: Vec<&str> = icons
            .iter()
            .filter_map(|icon| icon["src"].as_str())
            .collect();
        assert_eq!(sources, ["/diary-icon-192.png", "/diary-icon-512.png"]);
        for icon in icons {
            assert_eq!(icon["purpose"], "any maskable");
        }
    }

    /// The worker owns flushing; these literals ARE the protocol. If one
    /// vanishes in an edit, the offline queue stops working silently — fail
    /// loudly here instead. (The POST itself — same-origin credentials,
    /// no-store, the JSON body — moved into Rust: diary-worker's `try_send`,
    /// built from diary-core's contract.)
    #[test]
    fn sw_js_pins_the_offline_protocol() {
        for needle in [
            "\"diary-assets-v1\"",
            "\"/api/diary/entries\"",
            "\"/api/diary/snapshot\"",
            "\"diary-flush\"",
            "self.skipWaiting()",
            "clients.claim()",
            "\"navigate\"",
            // offline navigations RENDER from the mirror (the wasm router);
            // the stub only answers when the module itself refuses
            "diary_render(",
            "offlineStub()",
            "response.type === \"basic\"",
            "/_topcoat/assets/",
            "navigator.locks.request",
            "new BroadcastChannel(\"diary\")",
            // the Rust queue: both imports at evaluation time (Chrome refuses
            // lazy importScripts), instantiation deferred to the first flush
            "importScripts(SYNC_LOADER)",
            "importScripts(self.DIARY_SYNC.glue)",
            "module_or_path",
            // flush-then-pull as ONE call inside the one lock hold
            "diary_sync(API_PATH, SNAPSHOT_PATH, direct)",
            // the one-way legacy migration and the store it drains
            "diary_import",
            "\"diary-queue\"",
            "\"entries\"",
            // a failed POST navigation must error, never be answered with
            // the cached page (that would silently eat the form body)
            "request.method !== \"GET\"",
        ] {
            assert!(SW_JS.contains(needle), "sw.js lost {needle:?}");
        }
    }

    /// A page enqueues before it kicks the worker. If a slow flush already
    /// owns the Web Lock, that kick must dirty the active single-flight drain
    /// instead of disappearing; its shared promise must remain the value
    /// passed to `event.waitUntil`, so a network rejection still reaches
    /// Background Sync.
    #[test]
    fn worker_coalesces_flush_kicks_without_dropping_a_locked_request() {
        for needle in [
            "let flushFlight = null;",
            "let flushRequested = false;",
            "flushRequested = true;",
            "flushFlight = drainFlushRequests();",
            "return flushFlight;",
            "while (flushRequested)",
            "flushRequested = false;",
            "await navigator.locks.request(FLUSH_LOCK, () => flush());",
            "flushFlight = null;",
        ] {
            assert!(SW_JS.contains(needle), "sw.js lost {needle:?}");
        }
        assert!(
            !SW_JS.contains("ifAvailable"),
            "a busy lock must queue/coalesce the follow-up flush, never drop it"
        );
    }

    /// Names both sides must agree on — renaming in one file only would
    /// strand the other side's queue, caches, channel, or wasm pair. (The
    /// legacy "diary-queue"/"entries" names are now worker-only: the page
    /// never touches the old store, the migration drains it.)
    #[test]
    fn page_and_worker_agree_on_shared_names() {
        for shared in [
            "\"diary-flush\"",
            "\"diary-assets-v1\"",
            "\"/diary-sync.js\"",
            "DIARY_SYNC.glue",
            "DIARY_SYNC.wasm",
            "wasm_bindgen(",
            "new BroadcastChannel(\"diary\")",
            // the current identity-only flush acknowledgement — rename it in
            // one file only and saves silently stop reconciling
            "saved_refs",
        ] {
            assert!(SW_JS.contains(shared), "sw.js lost {shared:?}");
            assert!(DIARY_JS.contains(shared), "diary.js lost {shared:?}");
        }
        // New page/worker code can consume the predecessor wasm/worker's
        // content-bearing report field during activation.
        assert!(SW_JS.contains("saved_entries"));
        assert!(DIARY_JS.contains("saved_entries"));
    }

    /// Same rule as favicon.rs and /diary itself: reachable, never listed.
    #[test]
    fn pwa_routes_stay_unlisted_and_untrackable() {
        for path in [
            "/sw.js",
            "/diary.webmanifest",
            "/diary-icon-192.png",
            "/diary-icon-512.png",
        ] {
            assert!(
                !crate::content::routes::site_routes().contains(&path.to_string()),
                "{path} leaked into site_routes()"
            );
            assert!(!crate::content::routes::is_trackable_route(path));
        }
    }
}
