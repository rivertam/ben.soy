//! The wasm build of the diary offline queue (docs/diary-sync.md).
//!
//! This crate is a thin wasm-bindgen skin over `diary-core`: the queue lives
//! in a device-local SurrealDB reached as `indxdb://diary` (IndexedDB), and
//! the only code here that is not plumbing is [`send`] — the browser `fetch`
//! implementation of the flush transport. Both the /diary page and the
//! service worker load this module; IndexedDB serializes their transactions,
//! and the worker's Web Lock keeps flushes single-file.
//!
//! This crate lives in its own excluded workspace (see Cargo.toml), so
//! `just check` never builds it — breakage here surfaces at `just wasm` or
//! the Docker wasm stage. The crate root is `#![cfg]`-gated so a stray
//! native build compiles an empty library, and the real logic is all in
//! `diary-core`, which native tests cover against `mem://`.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use diary_core::contract::{SendOutcome, WireEntry, classify_response};
use diary_core::outbox;
use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCache, RequestCredentials, RequestInit, Response};

const LOCAL_ENDPOINT: &str = "indxdb://diary";

thread_local! {
    /// One connection per JS context (wasm is single-threaded; there is no
    /// cross-context sharing to guard — that is IndexedDB's job).
    static DB: RefCell<Option<outbox::Db>> = const { RefCell::new(None) };
}

async fn db() -> Result<outbox::Db, JsError> {
    if let Some(db) = DB.with(|cell| cell.borrow().clone()) {
        return Ok(db);
    }
    let db = outbox::open(LOCAL_ENDPOINT).await.map_err(outbox_error)?;
    DB.with(|cell| *cell.borrow_mut() = Some(db.clone()));
    Ok(db)
}

/// Queue one entry composed now. `written_at` is the composition second the
/// server will key the entry by; `enqueued_at_ms` orders the flush. Both
/// arrive as f64 because JS numbers do — they are integral and far inside
/// f64's exact range.
#[wasm_bindgen]
pub async fn diary_enqueue(
    written_at: f64,
    body: String,
    enqueued_at_ms: f64,
) -> Result<(), JsError> {
    let db = db().await?;
    outbox::enqueue(&db, written_at as i64, &body, enqueued_at_ms as i64)
        .await
        .map_err(outbox_error)?;
    Ok(())
}

/// The whole queue as JSON, oldest first — what the page renders under the
/// transcript.
#[wasm_bindgen]
pub async fn diary_snapshot() -> Result<String, JsError> {
    let db = db().await?;
    let entries = outbox::entries(&db).await.map_err(outbox_error)?;
    serde_json::to_string(&entries).map_err(json_error)
}

/// Drop one failed entry — the page's discard button.
#[wasm_bindgen]
pub async fn diary_discard(qid: String) -> Result<(), JsError> {
    let db = db().await?;
    outbox::remove(&db, &qid).await.map_err(outbox_error)
}

/// Import the legacy IndexedDB queue (the worker reads the old records out
/// and passes them here once). Returns how many were newly written.
#[wasm_bindgen]
pub async fn diary_import(json: String) -> Result<u32, JsError> {
    let legacy: Vec<outbox::LegacyEntry> = serde_json::from_str(&json).map_err(json_error)?;
    let db = db().await?;
    outbox::import(&db, &legacy).await.map_err(outbox_error)
}

/// Flush the queue to the replay endpoint; returns the `FlushReport` as
/// JSON in the shape the page's BroadcastChannel message has always had.
#[wasm_bindgen]
pub async fn diary_flush(api_url: String) -> Result<String, JsError> {
    let db = db().await?;
    let report = outbox::flush(&db, |entry| send(api_url.clone(), entry))
        .await
        .map_err(outbox_error)?;
    serde_json::to_string(&report).map_err(json_error)
}

/// One replay POST through the ambient `fetch`, resolved off the GLOBAL
/// scope rather than `window` because this runs inside the service worker
/// as well as the page. Any JS-side failure classifies as Retry: transport
/// trouble is never the entry's fault.
async fn send(api_url: String, entry: WireEntry) -> SendOutcome {
    match try_send(&api_url, &entry).await {
        Ok(outcome) => outcome,
        Err(_) => SendOutcome::Retry,
    }
}

async fn try_send(api_url: &str, entry: &WireEntry) -> Result<SendOutcome, JsValue> {
    let body = serde_json::to_string(entry).map_err(|error| JsValue::from(error.to_string()))?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_cache(RequestCache::NoStore);
    init.set_body(&JsValue::from_str(&body));
    let request = Request::new_with_str_and_init(api_url, &init)?;
    request.headers().set("Content-Type", "application/json")?;
    let global = js_sys::global();
    let fetch = Reflect::get(&global, &JsValue::from_str("fetch"))?.dyn_into::<Function>()?;
    let promise: Promise = fetch.call1(&global, &request)?.dyn_into()?;
    let response: Response = JsFuture::from(promise).await?.dyn_into()?;
    let status = response.status();
    let text = match response.text() {
        Ok(pending) => JsFuture::from(pending)
            .await
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    Ok(classify_response(status, &text))
}

fn outbox_error(error: outbox::OutboxError) -> JsError {
    JsError::new(&error.to_string())
}

fn json_error(error: serde_json::Error) -> JsError {
    JsError::new(&error.to_string())
}
