//! Serving the diary queue's wasm build (docs/diary-sync.md).
//!
//! Three stable routes ship the Rust service-worker module built by
//! `just wasm` into `wasm-dist/`:
//!
//! - `/diary-sync.js` — a two-line loader naming the current glue/wasm pair
//!   by content hash. Served `no-cache`: it is the one mutable URL, the
//!   worker `importScripts` it at evaluation time, and its bytes changing is
//!   what tells the browser a new worker version exists.
//! - `/diary-sync-glue.js` + `/diary-sync_bg.wasm` — the wasm-bindgen glue
//!   and the module itself. Immutable when requested with the loader's
//!   current `?v=`; any other query gets `no-cache` so a deploy-race copy
//!   can never stick under a fresh key. The glue and wasm only work as a
//!   matched pair, and the loader is the only place the pairing is written
//!   down — sw.js and diary.js never hardcode a versioned URL.
//!
//! The artifacts are read from disk at request time, not compiled in:
//! `just check` and `just build` stay green without a wasm toolchain, and a
//! running `just dev` picks up a fresh `just wasm` on the next request (the
//! cache below revalidates by file stamp). Absent artifacts 404; the page
//! then falls back to the plain form POST and the worker skips flushing —
//! the queue needs the build, the diary never does.
//!
//! Like every hand-served byte route, Content-Type is explicit (the response
//! layer treats untyped bodies as HTML), and the immutable variants are
//! exactly what the response layer's signed-in exemption expects: shared,
//! never-personalized bytes worth caching in the admin's browser.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use topcoat::{
    Result,
    asset::asset_config,
    context::Cx,
    router::{Body, Response, StatusCode, header, route, uri},
};

// The worker's offline SSR links the site stylesheet and the diary page
// script; both are resolved HERE (the server is the only side holding an
// `AssetConfig`) and carried to the worker inside the loader — no Asset
// machinery ever runs on wasm. The script asset is diary.rs's one
// declaration (a second `asset!` of the same file would register a
// duplicate route and panic at router build). Fonts are deliberately
// absent: offline pages fall back to system fonts.
use super::diary::DIARY_JS;

const GLUE_PATH: &str = "/diary-sync-glue.js";
const WASM_PATH: &str = "/diary-sync_bg.wasm";

/// Where `just wasm` (and deploy/Dockerfile) put the artifacts, relative to
/// the working directory; the env var exists for unusual layouts.
const DIST_DIR_VAR: &str = "DIARY_SYNC_DIST";
const DIST_DIR: &str = "wasm-dist";
const GLUE_FILE: &str = "diary_sync.js";
const WASM_FILE: &str = "diary_sync_bg.wasm";

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

struct Dist {
    version: String,
    glue: Vec<u8>,
    wasm: Vec<u8>,
}

/// (modified, len) per file — cheap to stat on every request, so a dev
/// rebuild is picked up without restarting, while steady state serves the
/// cached bytes.
type Stamp = ((SystemTime, u64), (SystemTime, u64));

static CACHE: Mutex<Option<(Stamp, Arc<Dist>)>> = Mutex::new(None);

// Route fns expand into module items named after the fn, so they get
// serve_* names — a route item literally named `wasm` or `glue` would turn
// same-named local bindings below into constant patterns.
#[route(GET "/diary-sync.js")]
async fn serve_loader(cx: &Cx) -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    let config = asset_config(cx);
    let css = config.resolve(crate::components::SITE_CSS);
    let js = config.resolve(DIARY_JS);
    let direct = std::env::var(DIRECT_ENDPOINT_VAR).unwrap_or_default();
    Ok(bytes_response(
        "text/javascript; charset=utf-8",
        NO_CACHE,
        loader_js(&dist.version, &css, &js, direct.trim()).into_bytes(),
    ))
}

#[route(GET "/diary-sync-glue.js")]
async fn serve_glue(cx: &Cx) -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    Ok(bytes_response(
        "text/javascript; charset=utf-8",
        cache_control(uri(cx).query(), &dist.version),
        dist.glue.clone(),
    ))
}

#[route(GET "/diary-sync_bg.wasm")]
async fn serve_wasm(cx: &Cx) -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    Ok(bytes_response(
        "application/wasm",
        cache_control(uri(cx).query(), &dist.version),
        dist.wasm.clone(),
    ))
}

/// The public SurrealDB endpoint the browser syncs against directly when
/// the flag is on — MUST be a websocket scheme (`wss://db.benjisponge.com`;
/// `ws://127.0.0.1:5800` in dev). Stateless HTTP is deliberately not
/// supported: server 3.2.3 does not persist http-session auth, so every
/// query after `authenticate()` would arrive anonymous and denied reads
/// filter to empty (probed; the worker also carries a `$access` canary and
/// diary-core a wipe guard for exactly this class of failure). Empty/unset
/// = HTTP-endpoint sync, the default. The URL is not a secret — auth is
/// the minted token.
const DIRECT_ENDPOINT_VAR: &str = "DIARY_DIRECT_SYNC_ENDPOINT";

/// The whole loader: one global naming the current pair, plus the hashed
/// asset URLs the worker's offline SSR links and the direct-sync endpoint
/// when flagged. Everything after `?v=` is this process's content hash, so
/// a page and a worker that read the same loader can never mix versions —
/// and because the loader is served no-cache, a deploy that changes only
/// the stylesheet (or flips the flag) still changes these bytes and rolls
/// a new worker version, which re-primes its own assets.
fn loader_js(version: &str, css: &str, js: &str, direct: &str) -> String {
    let assets = serde_json::json!({ "css": [css], "js": js, "direct": direct });
    format!(
        "self.DIARY_SYNC={{v:\"{version}\",glue:\"{GLUE_PATH}?v={version}\",wasm:\"{WASM_PATH}?v={version}\",assets:{assets}}};\n"
    )
}

/// Immutable only under the exact current version key. A stale or absent
/// `?v=` (deploy races, hand-typed URLs) must revalidate instead of pinning
/// wrong bytes under a year-long key.
fn cache_control(query: Option<&str>, version: &str) -> &'static str {
    if query.is_some_and(|q| {
        q.strip_prefix("v=")
            .is_some_and(|requested| requested == version)
    }) {
        IMMUTABLE
    } else {
        NO_CACHE
    }
}

fn dist() -> Option<Arc<Dist>> {
    let dir = dist_dir();
    let stamp = (stat(&dir.join(GLUE_FILE))?, stat(&dir.join(WASM_FILE))?);
    if let Some((cached_stamp, dist)) = CACHE.lock().unwrap().as_ref()
        && *cached_stamp == stamp
    {
        return Some(Arc::clone(dist));
    }
    let dist = Arc::new(load_dist(&dir)?);
    // A live `just wasm` rewrites both files over a multi-second window; a
    // read landing inside it could pair new glue with old wasm and mint an
    // immutable ?v for a combination that never works. Re-stat after the
    // read: any movement means torn bytes — refuse to serve, the next
    // request reads the settled pair.
    let settled = (stat(&dir.join(GLUE_FILE))?, stat(&dir.join(WASM_FILE))?);
    if settled != stamp {
        return None;
    }
    *CACHE.lock().unwrap() = Some((stamp, Arc::clone(&dist)));
    Some(dist)
}

fn dist_dir() -> PathBuf {
    std::env::var(DIST_DIR_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DIST_DIR))
}

fn stat(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn load_dist(dir: &Path) -> Option<Dist> {
    let glue = std::fs::read(dir.join(GLUE_FILE)).ok()?;
    let wasm = std::fs::read(dir.join(WASM_FILE)).ok()?;
    Some(Dist {
        version: version_of(&glue, &wasm),
        glue,
        wasm,
    })
}

/// First 16 hex chars of sha256(glue || wasm) — plenty for a cache key, and
/// hashing BOTH files is what makes the pair atomic: touch either and every
/// versioned URL moves at once.
fn version_of(glue: &[u8], wasm: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(glue);
    hash.update(wasm);
    hash.finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bytes_response(content_type: &'static str, cache: &'static str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(body))
        .expect("static headers")
}

fn missing() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_CACHE)
        .body(Body::from(
            "no wasm build is being served; see docs/diary-sync.md",
        ))
        .expect("static headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loader's own route path (the routes themselves are literals in
    /// the `#[route]` attributes; sw.js and diary.js pin the same string).
    const LOADER_PATH: &str = "/diary-sync.js";

    #[test]
    fn loader_pins_a_matched_pair_and_the_ssr_assets() {
        let js = loader_js(
            "abc123",
            "/_topcoat/assets/site-feed.css",
            "/_topcoat/assets/d.js",
            "",
        );
        assert_eq!(
            js,
            "self.DIARY_SYNC={v:\"abc123\",glue:\"/diary-sync-glue.js?v=abc123\",wasm:\"/diary-sync_bg.wasm?v=abc123\",assets:{\"css\":[\"/_topcoat/assets/site-feed.css\"],\"direct\":\"\",\"js\":\"/_topcoat/assets/d.js\"}};\n"
        );
        let flagged = loader_js("abc123", "/c.css", "/d.js", "https://db.example.com");
        assert!(flagged.contains("\"direct\":\"https://db.example.com\""));
    }

    #[test]
    fn only_the_current_version_is_immutable() {
        assert_eq!(cache_control(Some("v=abc123"), "abc123"), IMMUTABLE);
        assert_eq!(cache_control(Some("v=stale99"), "abc123"), NO_CACHE);
        assert_eq!(cache_control(None, "abc123"), NO_CACHE);
        assert_eq!(cache_control(Some("view=1"), "abc123"), NO_CACHE);
        assert_eq!(cache_control(Some(""), "abc123"), NO_CACHE);
    }

    #[test]
    fn versions_hash_the_pair_together() {
        let one = version_of(b"glue", b"wasm");
        assert_eq!(one.len(), 16);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(one, version_of(b"glue2", b"wasm"));
        assert_ne!(one, version_of(b"glue", b"wasm2"));
        assert_eq!(one, version_of(b"glue", b"wasm"));
    }

    #[test]
    fn dist_loads_a_pair_or_nothing() {
        let dir = std::env::temp_dir().join(format!("diary-sync-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_dist(&dir).is_none(), "empty dir must not serve");
        std::fs::write(dir.join(GLUE_FILE), b"glue").unwrap();
        assert!(load_dist(&dir).is_none(), "half a pair must not serve");
        std::fs::write(dir.join(WASM_FILE), b"wasm").unwrap();
        let dist = load_dist(&dir).expect("full pair serves");
        assert_eq!(dist.glue, b"glue");
        assert_eq!(dist.wasm, b"wasm");
        assert_eq!(dist.version, version_of(b"glue", b"wasm"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Same rule as the pwa.rs routes: reachable, never listed.
    #[test]
    fn sync_routes_stay_unlisted() {
        for path in [LOADER_PATH, GLUE_PATH, WASM_PATH] {
            assert!(
                !crate::content::routes::site_routes().contains(&path.to_string()),
                "{path} leaked into site_routes()"
            );
        }
    }
}
