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

use diary_core::contract::{PullOutcome, SendOutcome, WireEntry, classify_pull, classify_response};
use diary_core::outbox;
use diary_core::sync::{self, Remote};
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

/// Queue one entry composed now. `written_at` is the composition second;
/// `enqueued_at_ms` orders the flush. Both arrive as f64 because JS numbers
/// do — they are integral and far inside f64's exact range. Returns the
/// placed row as JSON: its `id` is the predicted permalink key (the second
/// may have been probed forward past a neighbor), which is exactly what the
/// page stamps on its bubble — a same-second double-tap returns the
/// original row, so the page sees the duplicate in the existing `data-id`
/// and drops its extra clone.
#[wasm_bindgen]
pub async fn diary_enqueue(
    written_at: f64,
    body: String,
    enqueued_at_ms: f64,
) -> Result<String, JsError> {
    let db = db().await?;
    let placed = outbox::enqueue(&db, written_at as i64, &body, enqueued_at_ms as i64)
        .await
        .map_err(outbox_error)?;
    serde_json::to_string(&placed).map_err(json_error)
}

/// Every not-yet-synced row as JSON, oldest first — what the page renders
/// as pending/failed bubbles. Synced history is deliberately absent: the
/// server-rendered transcript already shows it.
#[wasm_bindgen]
pub async fn diary_snapshot() -> Result<String, JsError> {
    let db = db().await?;
    let entries = outbox::queued(&db).await.map_err(outbox_error)?;
    serde_json::to_string(&entries).map_err(json_error)
}

/// Drop one queued or failed entry — the page's discard button. (Synced
/// history is out of reach by design.)
#[wasm_bindgen]
pub async fn diary_discard(id: String) -> Result<(), JsError> {
    let db = db().await?;
    outbox::discard(&db, &id).await.map_err(outbox_error)
}

/// Import the legacy IndexedDB queue (the worker reads the old records out
/// and passes them here once). Returns how many were newly written.
#[wasm_bindgen]
pub async fn diary_import(json: String) -> Result<u32, JsError> {
    let legacy: Vec<outbox::LegacyEntry> = serde_json::from_str(&json).map_err(json_error)?;
    let db = db().await?;
    outbox::import(&db, &legacy).await.map_err(outbox_error)
}

/// One full sync pass — flush every pending entry to the replay endpoint,
/// then pull the snapshot and reconcile the mirror — as one call, so the
/// worker's single Web Lock hold covers both halves. Returns the
/// `FlushReport` as JSON in the shape the page's BroadcastChannel message
/// has always had (plus `pulled`, which stale pages ignore).
#[wasm_bindgen]
pub async fn diary_sync(push_url: String, pull_url: String) -> Result<String, JsError> {
    let db = db().await?;
    let remote = HttpRemote { push_url, pull_url };
    let report = sync::run(&db, &remote).await.map_err(outbox_error)?;
    serde_json::to_string(&report).map_err(json_error)
}

/// The HTTP transport: the replay POST and the snapshot GET, both through
/// the ambient `fetch` resolved off the GLOBAL scope rather than `window`
/// because this runs inside the service worker as well as the page. Any
/// JS-side failure classifies as Retry — transport trouble is never the
/// entry's fault, and a failed pull must be a mirror no-op.
struct HttpRemote {
    push_url: String,
    pull_url: String,
}

impl Remote for HttpRemote {
    async fn push(&self, entry: WireEntry) -> SendOutcome {
        match try_send(&self.push_url, &entry).await {
            Ok(outcome) => outcome,
            Err(_) => SendOutcome::Retry,
        }
    }

    async fn pull(&self) -> PullOutcome {
        match try_pull(&self.pull_url).await {
            Ok(outcome) => outcome,
            Err(_) => PullOutcome::Retry,
        }
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
    let (status, text) = perform(&request).await?;
    Ok(classify_response(status, &text))
}

async fn try_pull(api_url: &str) -> Result<PullOutcome, JsValue> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(api_url, &init)?;
    let (status, text) = perform(&request).await?;
    Ok(classify_pull(status, &text))
}

async fn perform(request: &Request) -> Result<(u16, String), JsValue> {
    let global = js_sys::global();
    let fetch = Reflect::get(&global, &JsValue::from_str("fetch"))?.dyn_into::<Function>()?;
    let promise: Promise = fetch.call1(&global, request)?.dyn_into()?;
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
    Ok((status, text))
}

fn outbox_error(error: outbox::OutboxError) -> JsError {
    JsError::new(&error.to_string())
}

fn json_error(error: serde_json::Error) -> JsError {
    JsError::new(&error.to_string())
}
