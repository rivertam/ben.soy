//! The wasm build of the diary offline queue (docs/diary-sync.md).
//!
//! This crate is a thin wasm-bindgen skin over `diary-core`: the queue lives
//! in a device-local SurrealDB reached as `indxdb://diary` (IndexedDB), and
//! the only code here that is not plumbing is [`send`] — the browser `fetch`
//! implementation of the flush transport. Both the /diary page and the
//! service worker load this module; IndexedDB serializes their transactions,
//! and their shared Web Lock keeps each epoch check and store operation in one
//! critical section.
//!
//! This crate lives in its own excluded workspace (see Cargo.toml), so
//! `just check` never builds it — breakage here surfaces at `just wasm` or
//! the Docker wasm stage. The crate root is `#![cfg]`-gated so a stray
//! native build compiles an empty library, and the real logic is all in
//! `diary-core`, which native tests cover against `mem://`.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use diary_core::contract::{
    DirectTokenGrant, PullOutcome, PushWire, SendOutcome, classify_pull, classify_response,
    decode_compose_command,
};
use diary_core::entry::ComposedEntry;
use diary_core::outbox;
use diary_core::sync::{self, Remote};
use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCache, RequestCredentials, RequestInit, Response};

const LOCAL_ENDPOINT: &str = "indxdb://diary";

/// Let the paired page asset stamp compose commands with this wasm build's
/// schema epoch instead of duplicating a literal in JavaScript.
#[wasm_bindgen]
pub fn diary_schema_epoch() -> u16 {
    diary_core::contract::CURRENT_SCHEMA_EPOCH
}

thread_local! {
    /// One connection per JS context (wasm is single-threaded; there is no
    /// cross-context sharing to guard — that is IndexedDB's job).
    static DB: RefCell<Option<outbox::Db>> = const { RefCell::new(None) };
}

async fn db() -> Result<outbox::Db, JsError> {
    if let Some(db) = DB.with(|cell| cell.borrow().clone()) {
        outbox::require_current(&db).await.map_err(outbox_error)?;
        return Ok(db);
    }
    let db = outbox::open(LOCAL_ENDPOINT).await.map_err(outbox_error)?;
    DB.with(|cell| *cell.borrow_mut() = Some(db.clone()));
    Ok(db)
}

/// Queue one entry from the single current compose command serialized by
/// the page. The wasm Interface stays fixed when business fields grow: they
/// live inside the canonical `ComposedEntry`. Returns the placed row as
/// JSON; its `id` is the predicted Entry Key.
#[wasm_bindgen]
pub async fn diary_enqueue(command_json: String) -> Result<String, JsError> {
    let command =
        decode_compose_command(&command_json).map_err(|error| JsError::new(&error.to_string()))?;
    let db = db().await?;
    let placed = outbox::enqueue(&db, command.entry, command.enqueued_at_ms)
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

/// One full sync pass — flush every pending entry, then pull and reconcile
/// the mirror — as one call, so the worker's single Web Lock hold covers
/// both halves. `direct_endpoint` non-empty = the flag is on: mint a token,
/// authenticate a remote `Surreal<Any>` handle, and sync store-to-store
/// (`DirectRemote` — the same probe the server runs, no app server in the
/// path). Any failure ARMING direct mode (mint refused, connect failed)
/// falls back to the HTTP endpoints for this pass — sync never goes quiet
/// because the flag half-works. Returns the `FlushReport` as JSON in the
/// shape the page's BroadcastChannel message has always had.
#[wasm_bindgen]
pub async fn diary_sync(
    push_url: String,
    pull_url: String,
    direct_endpoint: String,
) -> Result<String, JsError> {
    let db = db().await?;
    let report = if direct_endpoint.trim().is_empty() {
        let remote = HttpRemote { push_url, pull_url };
        sync::run(&db, &remote).await.map_err(outbox_error)?
    } else {
        match direct_remote(direct_endpoint.trim()).await {
            Ok(remote) => sync::run(&db, &remote).await.map_err(outbox_error)?,
            Err(_) => {
                let remote = HttpRemote { push_url, pull_url };
                sync::run(&db, &remote).await.map_err(outbox_error)?
            }
        }
    };
    serde_json::to_string(&report).map_err(json_error)
}

/// Arm the direct transport for ONE pass: fresh token, fresh short-lived
/// websocket connection (dropped with the pass — a cached connection would
/// die with the idling service worker anyway), authenticate, and then the
/// load-bearing canary: `$access` must name our access method. The same
/// authenticated query supplies the server clock used by the shared remote
/// acceptance policy; the composing device's clock is not an authority.
/// SurrealDB
/// filters permission-denied reads to EMPTY results rather than erroring,
/// so a session that silently failed to carry its grant would pull "an
/// empty diary" — the canary (plus diary-core's wipe guard) is what keeps
/// that from ever touching the mirror.
async fn direct_remote(endpoint: &str) -> Result<sync::DirectRemote, String> {
    let grant = fetch_token().await?;
    if !grant.supports_current() {
        return Err(format!(
            "direct sync schema epoch {} does not match {}",
            grant.schema_epoch,
            diary_core::contract::CURRENT_SCHEMA_EPOCH
        ));
    }
    let remote = surrealdb::engine::any::connect(endpoint)
        .await
        .map_err(|error| error.to_string())?;
    remote
        .use_ns(grant.ns)
        .use_db(grant.db)
        .await
        .map_err(|error| error.to_string())?;
    remote
        .authenticate(grant.token)
        .await
        .map_err(|error| error.to_string())?;
    let mut canary = remote
        .query("RETURN $access; RETURN time::unix();")
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let access: Option<String> = canary.take(0).map_err(|error| error.to_string())?;
    if access.as_deref() != Some(diary_core::contract::DIRECT_ACCESS) {
        return Err(format!(
            "session carries access {access:?}, not {}",
            diary_core::contract::DIRECT_ACCESS
        ));
    }
    let validation_now: Option<i64> = canary.take(1).map_err(|error| error.to_string())?;
    let validation_now = validation_now.ok_or_else(|| "server clock is missing".to_string())?;
    Ok(sync::DirectRemote::new(remote, validation_now))
}

/// Mint one token from the cookie-gated endpoint. Any non-200 (404 = flag
/// off server-side, 401 = signed out) is a setup failure — the caller falls
/// back to HTTP sync, whose own classification shows the right banner.
async fn fetch_token() -> Result<DirectTokenGrant, String> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(diary_core::contract::TOKEN_PATH, &init)
        .map_err(|_| "token request build failed".to_string())?;
    stamp_schema_epoch(&request)
        .map_err(|_| "token request schema epoch header failed".to_string())?;
    let (status, text) = perform(&request)
        .await
        .map_err(|_| "token fetch failed".to_string())?;
    if status != 200 {
        return Err(format!("token endpoint answered {status}"));
    }
    serde_json::from_str(&text).map_err(|error| error.to_string())
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
    async fn push(&self, entry: ComposedEntry) -> SendOutcome {
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

async fn try_send(api_url: &str, entry: &ComposedEntry) -> Result<SendOutcome, JsValue> {
    let body = serde_json::to_string(&PushWire::new(entry.clone()))
        .map_err(|error| JsValue::from(error.to_string()))?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_cache(RequestCache::NoStore);
    init.set_body(&JsValue::from_str(&body));
    let request = Request::new_with_str_and_init(api_url, &init)?;
    request.headers().set("Content-Type", "application/json")?;
    stamp_schema_epoch(&request)?;
    let (status, text) = perform(&request).await?;
    Ok(classify_response(status, &text))
}

async fn try_pull(api_url: &str) -> Result<PullOutcome, JsValue> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(api_url, &init)?;
    stamp_schema_epoch(&request)?;
    let (status, text) = perform(&request).await?;
    Ok(classify_pull(status, &text))
}

fn stamp_schema_epoch(request: &Request) -> Result<(), JsValue> {
    request.headers().set(
        diary_core::contract::SCHEMA_EPOCH_HEADER,
        &diary_core::contract::CURRENT_SCHEMA_EPOCH.to_string(),
    )
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

// --------------------------------------------------------------------------
// Offline SSR: the same topcoat router machinery the server runs — pages
// discovered by inventory, dispatched by `Router::handle`, no sockets, no
// hyper (the 0.5.0 `serve` split) — rendering the SAME diary_core::views the
// server page renders. sw.js calls `diary_render` when a navigation's
// network fetch fails; anything unmatched or errored returns None and the
// worker falls back to its offline stub.

mod ssr {
    use std::cell::Cell;

    use diary_core::outbox::{self, LocalEntry};
    use diary_core::store::PAGE_SIZE;
    use diary_core::views::{Bubble, diary_room, entry_date, entry_detail, offline_page};
    use topcoat::{
        Result,
        context::{Cx, app_context},
        router::{
            Body, Request, Router, RouterBuilderDiscoverExt, StatusCode, page, path_param,
            to_bytes, uri,
        },
        view::view,
    };
    use wasm_bindgen::prelude::*;

    use crate::db;

    /// The local store handle for page fns. The HANDLE is Send+Sync (an Arc
    /// over channels); only its query futures are !Send — hence the oneshot
    /// bridge below.
    #[derive(Clone)]
    struct WorkerStore(outbox::Db);

    /// The hashed asset URLs the offline chrome links, resolved server-side
    /// into the /diary-sync.js loader and passed through sw.js verbatim.
    #[derive(Clone, serde::Deserialize)]
    struct WorkerAssets {
        #[serde(default)]
        css: Vec<String>,
        #[serde(default)]
        js: String,
    }

    thread_local! {
        /// Built once per worker lifetime and leaked: `handle` borrows the
        /// router across awaits, and a &'static reference is the simple way
        /// to keep that borrow out of a RefCell. A worker's config cannot
        /// change after evaluation, so once is also correct.
        static ROUTER: Cell<Option<&'static Router>> = const { Cell::new(None) };
    }

    async fn router(assets_json: &str) -> std::result::Result<&'static Router, JsError> {
        if let Some(router) = ROUTER.with(|cell| cell.get()) {
            return Ok(router);
        }
        let db = db().await?;
        let assets: WorkerAssets = serde_json::from_str(assets_json).unwrap_or(WorkerAssets {
            css: Vec::new(),
            js: String::new(),
        });
        let built = Router::builder()
            .discover()
            .app_context(WorkerStore(db))
            .app_context(assets)
            .build();
        let leaked: &'static Router = Box::leak(Box::new(built));
        ROUTER.with(|cell| cell.set(Some(leaked)));
        Ok(leaked)
    }

    /// Render one GET as the offline diary. `None` = nothing to say (an
    /// unmatched path or a render failure) — the caller serves its stub.
    #[wasm_bindgen]
    pub async fn diary_render(url: String, assets_json: String) -> Result<Option<String>, JsError> {
        // `router` deliberately caches its store handle. Re-enter the shared
        // epoch guard for every render so an older installed worker pauses
        // after a newer context migrates the device store.
        db().await?;
        let router = router(&assets_json).await?;
        let request = Request::builder()
            .method("GET")
            .uri(url)
            .body(Body::empty())
            .map_err(|error| JsError::new(&error.to_string()))?;
        let response = router.handle(request).await;
        if response.status() != StatusCode::OK {
            return Ok(None);
        }
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// The Send bridge: run a !Send indxdb read inside `spawn_local` (same
    /// thread — wasm is single-threaded) and await the oneshot receiver,
    /// which IS Send, from the page future that must be.
    async fn all_rows(store: &WorkerStore) -> std::result::Result<Vec<LocalEntry>, String> {
        let (tx, rx) = futures_channel::oneshot::channel();
        let db = store.0.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(outbox::all_local(&db).await.map_err(|e| e.to_string()));
        });
        rx.await.map_err(|_| "render read dropped".to_string())?
    }

    async fn one_row(
        store: &WorkerStore,
        id: String,
    ) -> std::result::Result<Option<LocalEntry>, String> {
        let (tx, rx) = futures_channel::oneshot::channel();
        let db = store.0.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(outbox::entry(&db, &id).await.map_err(|e| e.to_string()));
        });
        rx.await.map_err(|_| "render read dropped".to_string())?
    }

    /// `?page=N`, clamped — worker page fns NEVER redirect (the server's
    /// bounce dance would loop straight back into the offline fallback).
    fn requested_page(query: Option<&str>) -> usize {
        query
            .and_then(|q| {
                q.split('&')
                    .find_map(|pair| pair.strip_prefix("page="))
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .filter(|number| *number >= 1)
            .unwrap_or(1)
    }

    #[page("/diary")]
    async fn offline_diary(cx: &Cx) -> Result {
        let store = app_context::<WorkerStore>(cx);
        let assets = app_context::<WorkerAssets>(cx);
        let (bubbles, total, last, page_number, store_ok) = match all_rows(store).await {
            Ok(mut rows) => {
                rows.sort_by(|a, b| {
                    (a.written_at, a.id.as_str()).cmp(&(b.written_at, b.id.as_str()))
                });
                let total = rows.len();
                let last = total.div_ceil(PAGE_SIZE).max(1);
                let page_number = requested_page(uri(cx).query()).min(last);
                // Newest-first pages, each rendered oldest→newest — the
                // ascending twin of the server's DESC LIMIT/START + rev().
                let end = total.saturating_sub((page_number - 1) * PAGE_SIZE);
                let start = end.saturating_sub(PAGE_SIZE);
                let bubbles: Vec<Bubble> = rows[start..end]
                    .iter()
                    .map(|row| Bubble::from_local(row, true))
                    .collect();
                (bubbles, total, last, page_number, true)
            }
            Err(_) => (Vec::new(), 0, 1, 1, false),
        };
        let empty_notice = view! {}?;
        let room = view! {
            diary_room(
                page_number: page_number,
                last_page: last,
                total: total,
                store_ok: store_ok,
                entries: bubbles,
                notice: empty_notice,
            )
        }?;
        view! {
            offline_page(
                title: "Diary — offline",
                css_hrefs: assets.css.clone(),
                diary_js: assets.js.clone(),
                (room)
            )
        }
    }

    #[topcoat::router::path_param]
    struct EntryPath(str);

    #[page("/diary/{entry_path}")]
    async fn offline_entry(cx: &Cx) -> Result {
        let store = app_context::<WorkerStore>(cx);
        let assets = app_context::<WorkerAssets>(cx);
        let entry_path = path_param::<EntryPath>(cx);
        let found = if diary_core::eastern::parse_public_path(entry_path).is_some() {
            one_row(store, entry_path.to_string()).await.ok().flatten()
        } else {
            None
        };
        let body = match &found {
            Some(row) => {
                let heading = entry_date(&row.id);
                view! {
                    <h1 class="mt-8 font-display text-xl">(heading)</h1>
                    entry_detail(entry: row.entry.clone())
                }?
            }
            None => view! {
                <p class="mt-8 max-w-prose text-ink2">
                    "This entry is not in the device's local store. It may "
                    "exist on the server — try again online."
                </p>
            }?,
        };
        view! {
            offline_page(
                title: "Diary — offline",
                css_hrefs: assets.css.clone(),
                diary_js: assets.js.clone(),
                <div>
                    (body)
                    <p class="mt-6"><a class="quiet-link" href="/diary">"← diary"</a></p>
                </div>
            )
        }
    }
}
