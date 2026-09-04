//! Content-hashed serving for the Fitness entry wasm pair.
//!
//! `/fitness-entry-wasm.js` is the stable, `no-cache` loader imported by the
//! stable service worker. The generated glue and wasm routes are immutable
//! only when their query names the hash of both current files, so a deploy
//! race can never cache a mismatched pair.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use topcoat::{
    Result,
    context::Cx,
    router::{Body, StatusCode, header, request::uri, response::Response, route},
};

const GLUE_PATH: &str = "/fitness-entry-glue.js";
const WASM_PATH: &str = "/fitness-entry_bg.wasm";
const DIST_DIR_VAR: &str = "FITNESS_ENTRY_DIST";
const DIST_DIR: &str = "wasm-dist";
const GLUE_FILE: &str = "fitness_entry.js";
const WASM_FILE: &str = "fitness_entry_bg.wasm";
const JAVASCRIPT_TYPE: &str = "text/javascript; charset=utf-8";
const WASM_TYPE: &str = "application/wasm";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

struct Dist {
    version: String,
    glue: Vec<u8>,
    wasm: Vec<u8>,
}

type Stamp = ((SystemTime, u64), (SystemTime, u64));

static CACHE: Mutex<Option<(Stamp, Arc<Dist>)>> = Mutex::new(None);

#[route(GET "/fitness-entry-wasm.js")]
async fn serve_loader() -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    Ok(bytes_response(
        JAVASCRIPT_TYPE,
        NO_CACHE,
        loader_js(&dist.version).into_bytes(),
    ))
}

#[route(GET "/fitness-entry-glue.js")]
async fn serve_glue(cx: &Cx) -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    Ok(bytes_response(
        JAVASCRIPT_TYPE,
        cache_control(uri(cx).query(), &dist.version),
        dist.glue.clone(),
    ))
}

#[route(GET "/fitness-entry_bg.wasm")]
async fn serve_wasm(cx: &Cx) -> Result<Response> {
    let Some(dist) = dist() else {
        return Ok(missing());
    };
    Ok(bytes_response(
        WASM_TYPE,
        cache_control(uri(cx).query(), &dist.version),
        dist.wasm.clone(),
    ))
}

fn loader_js(version: &str) -> String {
    format!(
        "self.FITNESS_ENTRY_WASM={{v:\"{version}\",glue:\"{GLUE_PATH}?v={version}\",wasm:\"{WASM_PATH}?v={version}\",protocol:{}}};\n",
        fitness_entry_core::PROTOCOL_VERSION
    )
}

fn cache_control(query: Option<&str>, version: &str) -> &'static str {
    if query.is_some_and(|query| query.strip_prefix("v=") == Some(version)) {
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
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
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
        .header("x-content-type-options", "nosniff")
        .body(Body::from(body))
        .expect("static Fitness wasm headers")
}

fn missing() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_CACHE)
        .body(Body::from(
            "Fitness entry requires a wasm build; run `just fitness-wasm`.",
        ))
        .expect("static Fitness wasm error headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_versions_one_matched_pair() {
        assert_eq!(
            loader_js("abc123"),
            "self.FITNESS_ENTRY_WASM={v:\"abc123\",glue:\"/fitness-entry-glue.js?v=abc123\",wasm:\"/fitness-entry_bg.wasm?v=abc123\",protocol:1};\n"
        );
        assert_eq!(cache_control(Some("v=abc123"), "abc123"), IMMUTABLE);
        assert_eq!(cache_control(Some("v=old"), "abc123"), NO_CACHE);
        assert_eq!(cache_control(None, "abc123"), NO_CACHE);
    }

    #[test]
    fn version_hash_covers_glue_and_wasm() {
        assert_ne!(version_of(b"glue", b"wasm"), version_of(b"new", b"wasm"));
        assert_ne!(version_of(b"glue", b"wasm"), version_of(b"glue", b"new"));
    }

    #[test]
    fn half_a_pair_never_serves() {
        let dir =
            std::env::temp_dir().join(format!("fitness-entry-dist-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(GLUE_FILE), b"glue").unwrap();
        assert!(load_dist(&dir).is_none());
        std::fs::write(dir.join(WASM_FILE), b"wasm").unwrap();
        assert!(load_dist(&dir).is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_assets_keep_explicit_types_and_cache_policy() {
        let glue = bytes_response(JAVASCRIPT_TYPE, NO_CACHE, b"glue".to_vec());
        assert_eq!(glue.headers()[header::CONTENT_TYPE], JAVASCRIPT_TYPE);
        assert_eq!(glue.headers()[header::CACHE_CONTROL], NO_CACHE);

        let wasm = bytes_response(WASM_TYPE, IMMUTABLE, b"wasm".to_vec());
        assert_eq!(wasm.headers()[header::CONTENT_TYPE], WASM_TYPE);
        assert_eq!(wasm.headers()[header::CACHE_CONTROL], IMMUTABLE);
        assert_eq!(wasm.headers()["x-content-type-options"], "nosniff");
    }

    #[test]
    fn artifact_routes_are_not_site_pages() {
        for path in ["/fitness-entry-wasm.js", GLUE_PATH, WASM_PATH] {
            assert!(!crate::content::routes::site_routes().contains(&path.to_string()));
        }
    }
}
