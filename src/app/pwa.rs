//! Stable-URL endpoints for the /diary PWA and the separate /fitness share
//! target: service workers, web app manifests, and launcher icons. Not pages:
//! no shell, out of `site_routes()` (the 404 index is for pages) —
//! `favicon.rs` is the pattern. The diary's interactive halves live in
//! `diary/sw.js` and `diary/diary.js`; the queue those two load is Rust served
//! by `diary_sync.rs`; the write endpoint is `POST /api/diary/entries` in
//! `diary.rs`.
//!
//! Deliberately ungated: Chrome fetches manifests without credentials (a
//! cookie gate would break install), and none of these bytes are private —
//! they disclose only that the two app surfaces exist, which their public
//! pages/repo already do. Every route declares an explicit
//! Content-Type: the response layer treats untyped bodies as HTML and runs
//! the em-dash rewriter through them. Fitness's worker owns its local entry
//! queue but deliberately registers no fetch handler and never caches pages.

use topcoat::{Result, router::route};

/// The worker's URL is its identity to the browser: a hashed `asset!` URL
/// would register a brand-new worker every deploy and orphan the old one.
/// Served `no-cache` so the edge always revalidates; browsers bypass the
/// HTTP cache for service-worker update checks regardless.
const SW_JS: &str = include_str!("diary/sw.js");

/// Fitness owns a distinct message-driven workout queue. It has no navigation
/// or asset interception; its narrow scope and storage names keep it separate
/// from Diary's local mirror and outbox.
const FITNESS_SW_JS: &str = include_str!("running/sw.js");

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
  "background_color": "#2e3626",
  "theme_color": "#2e3626",
  "icons": [
    { "src": "/diary-icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" },
    { "src": "/diary-icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }
  ]
}"##;

/// A second app identity for the public fitness log. Web Share Target actions
/// must live inside their manifest scope, so the diary's `/diary` manifest
/// cannot legally receive `/fitness/share` launches.
const FITNESS_MANIFEST: &str = r##"{
  "name": "Fitness",
  "short_name": "Fitness",
  "id": "/fitness",
  "start_url": "/fitness",
  "scope": "/fitness",
  "display": "standalone",
  "background_color": "#2e3626",
  "theme_color": "#2e3626",
  "icons": [
    { "src": "/diary-icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" },
    { "src": "/diary-icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }
  ],
  "share_target": {
    "action": "/fitness/share",
    "method": "POST",
    "enctype": "application/x-www-form-urlencoded",
    "params": {
      "title": "title",
      "text": "text",
      "url": "url"
    }
  }
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

#[route(GET "/fitness/sw.js")]
async fn fitness_service_worker() -> Result<([(&'static str, &'static str); 3], &'static str)> {
    Ok((
        [
            ("Content-Type", "text/javascript; charset=utf-8"),
            ("Cache-Control", "no-cache"),
            // The script lives one slash below the exact `/fitness` start
            // URL. This explicit allowance lets the queue worker control that
            // start URL as well as `/fitness/*`.
            ("Service-Worker-Allowed", "/fitness"),
        ],
        FITNESS_SW_JS,
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

#[route(GET "/fitness.webmanifest")]
async fn fitness_manifest() -> Result<([(&'static str, &'static str); 2], &'static str)> {
    Ok((
        [
            ("Content-Type", "application/manifest+json"),
            ("Cache-Control", DAY_CACHE),
        ],
        FITNESS_MANIFEST,
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
    const FITNESS_PWA_JS: &str = include_str!("interests/running/pwa.js");

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

    #[test]
    fn fitness_manifest_declares_a_scoped_share_target() {
        let parsed: serde_json::Value =
            serde_json::from_str(FITNESS_MANIFEST).expect("fitness manifest parses");
        assert_eq!(parsed["name"], "Fitness");
        assert_eq!(parsed["id"], "/fitness");
        assert_eq!(parsed["start_url"], "/fitness");
        assert_eq!(parsed["scope"], "/fitness");
        assert_eq!(parsed["share_target"]["action"], "/fitness/share");
        assert_eq!(parsed["share_target"]["method"], "POST");
        assert_eq!(
            parsed["share_target"]["enctype"],
            "application/x-www-form-urlencoded"
        );
        assert_eq!(parsed["share_target"]["params"]["title"], "title");
        assert_eq!(parsed["share_target"]["params"]["text"], "text");
        assert_eq!(parsed["share_target"]["params"]["url"], "url");
        assert!(FITNESS_SW_JS.contains("self.skipWaiting()"));
        assert!(FITNESS_SW_JS.contains("self.clients.claim()"));
        assert!(!FITNESS_SW_JS.contains("addEventListener(\"fetch\""));
        for needle in [
            "importScripts(LOADER)",
            "importScripts(self.FITNESS_ENTRY_WASM.glue)",
            "\"fitness-entry\"",
            "\"state\"",
            "\"outbox\"",
            "\"fitness-entry-flush\"",
            "commitFinalization",
            "commitRestore",
            "response.status",
            "includeUncontrolled: true",
            "fitness_pending_outbox",
            "fitness_order_outbox",
            "registration.sync?.register",
            "if (flush.retry_pending)",
            "wasmReady === attempt",
            "indexedDB.open(DATABASE, DATABASE_VERSION)",
            "tx.objectStore(OUTBOX_STORE).add(queued)",
            "AbortController",
        ] {
            assert!(
                FITNESS_SW_JS.contains(needle),
                "Fitness worker lost {needle:?}"
            );
        }
        assert!(!FITNESS_SW_JS.contains("caches."));
        assert!(
            !FITNESS_SW_JS.contains("if (stillPending) await registerBackgroundSync()"),
            "a firing sync event must not re-register its own tag"
        );
        assert!(FITNESS_SW_JS.contains("case \"flush_only\""));
        for needle in [
            "const sourceClientId = event.source?.id || null",
            "broadcastChange(sourceClientId)",
            "if (client.id === excludeClientId) continue",
            "case \"draft_status\"",
            "return draftStatus(db)",
        ] {
            assert!(
                FITNESS_SW_JS.contains(needle),
                "Fitness worker lost {needle:?}"
            );
        }
        assert!(FITNESS_PWA_JS.contains(&format!(
            "const FITNESS_ENTRY_PROTOCOL = {};",
            fitness_entry_core::PROTOCOL_VERSION
        )));
        for trigger in [
            "requestFitnessFlush()",
            "window.addEventListener(\"online\"",
            "window.addEventListener(\"pageshow\"",
            "document.addEventListener(\"visibilitychange\"",
            "requestFitnessWorker(\"draft_status\")",
            "location.replace(\"/fitness/entry\")",
            "display-mode: standalone",
        ] {
            assert!(
                FITNESS_PWA_JS.contains(trigger),
                "Fitness registration adapter lost {trigger:?}"
            );
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
            "\"diary-store\"",
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
            "await navigator.locks.request(STORE_LOCK, () => flush());",
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
            "\"diary-store\"",
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

    /// The local epoch check lives inside each wasm export, so the browser
    /// must keep that check and the subsequent store use in one shared lock
    /// hold. Otherwise a stale page could project a newer business field
    /// between those two operations.
    #[test]
    fn page_and_worker_fence_every_device_store_entry_with_one_lock() {
        for (call, guarded) in [
            (
                "wasm.diary_enqueue",
                "withStoreLock(() => wasm.diary_enqueue",
            ),
            (
                "wasm.diary_snapshot",
                "withStoreLock(() => wasm.diary_snapshot",
            ),
            (
                "wasm.diary_discard",
                "withStoreLock(() => wasm.diary_discard",
            ),
        ] {
            assert_eq!(DIARY_JS.matches(call).count(), 1, "unexpected {call} call");
            assert!(
                DIARY_JS.contains(guarded),
                "{call} escaped the diary-store lock"
            );
        }
        assert!(SW_JS.contains(
            "navigator.locks.request(STORE_LOCK, () =>\n        wasm_bindgen.diary_render"
        ));
        assert!(SW_JS.contains("navigator.locks.request(STORE_LOCK, () => flush())"));
    }

    /// Same rule as favicon.rs and /diary itself: reachable, never listed.
    #[test]
    fn pwa_routes_stay_unlisted() {
        for path in [
            "/sw.js",
            "/diary.webmanifest",
            "/fitness/sw.js",
            "/fitness.webmanifest",
            "/diary-icon-192.png",
            "/diary-icon-512.png",
        ] {
            assert!(
                !crate::content::routes::site_routes().contains(&path.to_string()),
                "{path} leaked into site_routes()"
            );
        }
    }
}
