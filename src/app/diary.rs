//! `/diary` — the admin's completely private, database-backed diary.
//!
//! Deliberately NOT a hidden page: a `HIDDEN_PAGES` entry would render a
//! grant form for it at `/admin/permissions`, and one mistyped grant would
//! share a diary. Admin-only means exactly one identity, ever — the same
//! constant comparison as `/admin`, working through database outages. The
//! page otherwise follows every hidden-page invariant (docs/auth.md): out of
//! all public registries, `no-store` before `shell()` on every variant,
//! signed-out → login redirect, signed-in non-admin → the real 404. Its one
//! listing is the `/admin` tool card.
//!
//! An entry's business fields live in `EntryContent`; placement pairs them
//! with a timestamp and Eastern public-path record key
//! (`eastern::public_path`, the `/fitness/lift/{path}` permalink shape), so keys
//! sort chronologically and the permalink IS the id. `/diary` fetches newest
//! first but renders each `PAGE_SIZE` page oldest-to-newest in a bottom-pinned
//! chat transcript; `/diary/{path}` is one entry's own page and the only place
//! it can be deleted.
//!
//! Both POSTs repeat the admin identity check, require positive same-origin
//! evidence, and bound the body before parsing it — the forms are not an
//! authorization boundary. Redirecting responses are hand-built `Ok(303)`s
//! like the admin routes' so every branch carries `no-store`.
//!
//! The page doubles as an installable PWA with an offline write queue
//! (`diary/diary.js` + `diary/sw.js`; stable routes in `pwa.rs`): with JS,
//! saves go IndexedDB-first and replay through `POST /api/diary/entries`,
//! which starts placement from the client's proposed second and dedupes
//! replays (same probe walk + same Entry Content) — Background Sync retries
//! can never double-post, and a queued entry keeps its placement time rather
//! than the time it synced. Details in docs/auth.md.

use diary_core::contract::{DirectTokenGrant, SnapshotWire, WireError, decode_push};
use diary_core::eastern;
use diary_core::entry::{ComposedEntry, DiaryEntry};
use diary_core::store::{self, MAX_PAGE, PAGE_SIZE, SaveError, SavedWrite};
use diary_core::views::{self, Bubble, diary_room, entry_detail};
use jiff::Timestamp;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, Response, StatusCode, error::redirect, header, headers, page,
        path_param, query_params, route, to_bytes, uri,
    },
    view::view,
};

use benjisponge::data::Data;

use crate::components::{back_link, page_head, shell};
use crate::content::access::is_admin;
use crate::util::{is_same_origin, urlencode};

use super::login::viewer;
use super::not_found::not_found_page;

pub(crate) const PATH: &str = "/diary";
const LOGIN_REDIRECT: &str = "/login?next=%2Fdiary";
/// A worst-case urlencoded body of `MAX_ENTRY_CHARS` multibyte characters
/// runs to several hundred KB; 1 MiB matches the fitness import's bound.
/// (The char bound itself, the replay window, and the collision probes now
/// live in `diary-core` — the crate the wasm service worker also compiles,
/// so the two halves of the protocol cannot drift.)
const BODY_LIMIT_BYTES: usize = 1024 * 1024;
const NO_STORE: &str = "no-store";
/// The offline queue's page-side half; the worker and manifest ride stable
/// routes in `pwa.rs` (a service worker's URL is its identity, so it cannot
/// be a hashed asset), and the queue's Rust half rides `diary_sync.rs` —
/// which also resolves THIS const into the loader for offline SSR. One
/// declaration on purpose: a second `asset!` of the same file registers a
/// duplicate serving route and panics at router build (which `just check`
/// never runs).
pub(super) const DIARY_JS: Asset = asset!("./diary/diary.js");

#[query_params(error = redirect("?"))]
struct DiaryQuery {
    page: Option<String>,
    notice: Option<String>,
}

#[page("/diary")]
async fn diary(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(LOGIN_REDIRECT).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: PATH)
        };
    }
    let query = query_params::<DiaryQuery>(cx)?;
    let Some(page_number) = requested_page(query.page.as_deref()) else {
        return Err(redirect(PATH).into());
    };
    let notice = query.notice.as_deref().map(|code| match code {
        "saved" => "Saved.",
        "deleted" => "Deleted.",
        "invalid" => "That didn't validate; nothing changed.",
        "unavailable" => "The diary store didn't answer; nothing changed.",
        _ => "Nothing changed.",
    });
    let (entries, total, store_ok) = match entry_page(app_context::<Data>(cx), page_number).await {
        Ok((entries, total)) => {
            // Past-the-end page numbers bounce to the last real page.
            if page_number > last_page(total) {
                return Err(redirect(&views::page_url(last_page(total))).into());
            }
            (entries, total, true)
        }
        Err(error) => {
            log_failure("list", &error);
            (Vec::new(), 0, false)
        }
    };
    let last = last_page(total);
    // The transcript, bubbles, template, and compose form are the SHARED
    // components (diary_core::views) — the same markup the service worker's
    // offline SSR renders, so the two renderers cannot drift. Server-only
    // inserts ride the notice slot.
    let notice_view = view! {
        if let Some(message) = notice {
            <p class="border-l-2 border-oxide pl-3 font-meta text-sm text-ink2">
                (message)
            </p>
        }
    }?;
    let bubbles: Vec<Bubble> = entries.iter().rev().map(Bubble::synced).collect();
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: "Diary",
            active: "",
            runtime: false,
            pwa: true,
            diary_room(
                page_number: page_number,
                last_page: last,
                total: total,
                store_ok: store_ok,
                entries: bubbles,
                notice: notice_view,
            )
            <script type="module" src=(DIARY_JS)></script>
        )
    }
}

#[path_param]
struct EntryPath(str);

#[page("/diary/{entry_path}")]
async fn diary_entry(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(&format!("/login?next={}", urlencode(uri(cx).path()))).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: uri(cx).path())
        };
    }
    let entry_path = path_param::<EntryPath>(cx);
    // The strict permalink shape gates everything below, including the
    // query-strip redirect: path params arrive percent-DECODED and
    // `redirect()` panics on Location values with control bytes, so only a
    // validated id — 25 header-safe ASCII bytes — may enter a Location.
    // (Lifting's twin redirect is safe differently: `workout_url` re-encodes.)
    if eastern::parse_public_path(entry_path).is_none() {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: uri(cx).path())
        };
    }
    if uri(cx).query().is_some() {
        return Err(redirect(&views::entry_url(entry_path)).into());
    }
    let loaded = entry_by_id(app_context::<Data>(cx), entry_path).await;
    let entry = match &loaded {
        Ok(Some(entry)) => Some(entry),
        Ok(None) => {
            return view! {
                ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
                not_found_page(requested: uri(cx).path())
            };
        }
        Err(error) => {
            log_failure("detail", error);
            None
        }
    };
    let heading = entry
        .map(|found| views::entry_date(&found.id))
        .unwrap_or_else(|| "Diary".to_string());
    let title = format!("Diary · {heading}");
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: title.as_str(),
            active: "",
            runtime: false,
            pwa: true,
            page_head(stamp: "diary", title: heading.as_str(), lede: "")
            if let Some(entry) = entry {
                // The stamp/body/delete core is the shared component the
                // worker's offline permalink pages render too.
                entry_detail(entry: entry.clone())
            } else {
                <p class="mt-8 max-w-prose text-ink2">
                    "The diary store is unreachable, so this entry did not "
                    "load. It is safe where it is; try again in a moment."
                </p>
            }
            back_link(href: PATH, label: "diary")
            <script type="module" src=(DIARY_JS)></script>
        )
    }
}

#[route(POST "/diary/write")]
async fn write_entry(cx: &Cx, body: Body) -> Result<Response> {
    let bytes = match gate(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(response),
    };
    let Some((raw, reply_to)) = parse_write_form(&bytes) else {
        return Ok(back("invalid"));
    };
    let validation_now = Timestamp::now().as_second();
    let entry = ComposedEntry::new(validation_now, raw).with_reply_to(reply_to);
    match save_queued_entry(app_context::<Data>(cx), entry, validation_now).await {
        Ok(_) => Ok(back("saved")),
        Err(SaveError::Rejected(_)) => Ok(back("invalid")),
        Err(SaveError::Exhausted) => Ok(back("unavailable")),
        Err(SaveError::Store(error)) => {
            log_failure("write", &error);
            Ok(back("unavailable"))
        }
    }
}

#[route(POST "/diary/delete")]
async fn delete_entry(cx: &Cx, body: Body) -> Result<Response> {
    let bytes = match gate(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(response),
    };
    let Some(path) = parse_single_field(&bytes, "path") else {
        return Ok(back("invalid"));
    };
    if eastern::parse_public_path(&path).is_none() {
        return Ok(back("invalid"));
    }
    match remove_entry(app_context::<Data>(cx), &path).await {
        Ok(()) => Ok(back("deleted")),
        Err(error) => {
            log_failure("delete", &error);
            Ok(back("unavailable"))
        }
    }
}

/// `POST /api/diary/entries` — the queue-replay twin of `/diary/write`.
/// Same authorization gate; the differences are deliberate: JSON in and out
/// (real status codes a background fetch can act on, where the form's
/// 303-to-login is invisible), the client's proposed second starts
/// placement, and replays dedupe instead of erroring. Worker-initiated fetches
/// pass `is_same_origin` — Chrome stamps same-origin `Sec-Fetch-Site` and
/// `Origin` on them like any page fetch.
#[route(POST "/api/diary/entries")]
async fn api_write_entry(cx: &Cx, body: Body) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return Ok(api_error(StatusCode::UNAUTHORIZED, "sign in"));
    };
    if !is_admin(&current.email) {
        return Ok(api_error(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(api_error(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !has_current_schema_epoch(headers(cx)) {
        return Ok(schema_epoch_mismatch());
    }
    if !is_json_content_type(headers(cx)) {
        return Ok(api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }
    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(api_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "entry is too large",
            ));
        }
    };
    let entry = match decode_push(&bytes) {
        Ok(entry) => entry,
        Err(WireError::SchemaEpochMismatch(_)) => return Ok(schema_epoch_mismatch()),
        Err(WireError::Malformed) => {
            return Ok(api_error(StatusCode::BAD_REQUEST, "malformed entry"));
        }
    };
    match save_queued_entry(app_context::<Data>(cx), entry, Timestamp::now().as_second()).await {
        Ok(saved) => Ok(api_json(
            StatusCode::OK,
            serde_json::json!({
                "status": "saved",
                "id": saved.id,
                "written_at": saved.written_at,
                "deduped": saved.deduped,
            })
            .to_string(),
        )),
        Err(SaveError::Rejected(rejection)) => Ok(api_error(
            StatusCode::from_u16(rejection.status_code())
                .expect("entry rejections use valid HTTP status codes"),
            rejection.message(),
        )),
        Err(SaveError::Exhausted) => Ok(api_error(
            StatusCode::CONFLICT,
            "no free second near that timestamp",
        )),
        Err(SaveError::Store(error)) => {
            log_failure("api-write", &error);
            Ok(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "store unreachable",
            ))
        }
    }
}

fn requested_schema_epoch(headers: &HeaderMap) -> Option<u16> {
    headers
        .get(diary_core::contract::SCHEMA_EPOCH_HEADER)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

fn has_current_schema_epoch(headers: &HeaderMap) -> bool {
    requested_schema_epoch(headers) == Some(diary_core::contract::CURRENT_SCHEMA_EPOCH)
}

fn schema_epoch_mismatch() -> Response {
    // Retryable by every worker. A mismatch is deployment skew, never a
    // permanent rejection of the queued entry itself.
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "diary schema epoch mismatch",
    )
}

/// `GET /api/diary/snapshot` — the pull half of sync: every entry, in the
/// exact `SnapshotWire` shape the worker strictly parses (a drift fails
/// loudly on the client, which classifies it Retry and leaves the mirror
/// alone). Same admin gate as the page; `no-store` like every diary
/// response. No same-origin requirement: it is a GET, and cross-origin
/// callers can neither read the JSON (no CORS headers) nor change anything.
#[route(GET "/api/diary/snapshot")]
async fn api_snapshot(cx: &Cx) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return Ok(api_error(StatusCode::UNAUTHORIZED, "sign in"));
    };
    if !is_admin(&current.email) {
        return Ok(api_error(StatusCode::NOT_FOUND, "not found"));
    }
    if !has_current_schema_epoch(headers(cx)) {
        return Ok(schema_epoch_mismatch());
    }
    let entries = match snapshot_entries(app_context::<Data>(cx)).await {
        Ok(entries) => entries,
        Err(error) => {
            log_failure("snapshot", &error);
            return Ok(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "store unreachable",
            ));
        }
    };
    let wire = SnapshotWire::new(entries);
    let body = serde_json::to_string(&wire).expect("snapshot shape always serializes");
    Ok(api_json(StatusCode::OK, body))
}

async fn snapshot_entries(data: &Data) -> std::result::Result<Vec<DiaryEntry>, String> {
    store::all_entries(&open_db(data).await?).await
}

/// The private half of the direct-sync keypair (PKCS#8 PEM). Both halves
/// set = flag on; the public half lives in `data.rs` (it defines the access
/// method at bootstrap) and the browser-facing endpoint in `diary_sync.rs`
/// (the loader advertises it).
const DIRECT_SYNC_PRIVATE_KEY_VAR: &str = "DIARY_SYNC_JWT_PRIVATE_KEY";
/// Fifteen minutes — a sync pass mints fresh every time, so the TTL only
/// bounds how long a leaked token stays live.
const DIRECT_SYNC_TOKEN_TTL_SECONDS: i64 = 900;

/// `POST /api/diary/token` — mint one short-lived record-access token for
/// direct client→SurrealDB sync (docs/diary-sync.md). Admin cookie plus
/// positive same-origin evidence, like every diary POST; 404 while the
/// flag is off so the surface does not advertise itself. The claims MUST
/// carry `id` (the record identity): that is what makes SurrealDB build a
/// fully permission-bound RECORD session instead of a database-level one.
#[route(POST "/api/diary/token")]
async fn api_token(cx: &Cx, body: Body) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return Ok(api_error(StatusCode::UNAUTHORIZED, "sign in"));
    };
    if !is_admin(&current.email) {
        return Ok(api_error(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(api_error(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !has_current_schema_epoch(headers(cx)) {
        return Ok(schema_epoch_mismatch());
    }
    // No meaningful body; drain within the same bound as the other POSTs.
    if to_bytes(body, BODY_LIMIT_BYTES).await.is_err() {
        return Ok(api_error(StatusCode::PAYLOAD_TOO_LARGE, "no body needed"));
    }
    let (Some(private_key), Ok(namespace), Ok(database)) = (
        optional_env(DIRECT_SYNC_PRIVATE_KEY_VAR),
        std::env::var(benjisponge::data::NAMESPACE_VAR),
        std::env::var(benjisponge::data::DATABASE_VAR),
    ) else {
        return Ok(api_error(StatusCode::NOT_FOUND, "not found"));
    };
    // A current direct token must never get ahead of its table migration.
    // Opening the shared handle applies and verifies the diary ledger first.
    if let Err(error) = open_db(app_context::<Data>(cx)).await {
        log_failure("token-migration", &error);
        return Ok(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "store unreachable",
        ));
    }
    match mint_direct_token(&private_key, &namespace, &database) {
        Ok(token) => Ok(api_json(
            StatusCode::OK,
            serde_json::to_string(&DirectTokenGrant::new(token, namespace, database))
                .expect("direct token grants always serialize"),
        )),
        Err(error) => {
            log_failure("token", &error);
            Ok(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "token minting failed",
            ))
        }
    }
}

fn optional_env(variable: &str) -> Option<String> {
    std::env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn mint_direct_token(
    private_key_pem: &str,
    namespace: &str,
    database: &str,
) -> std::result::Result<String, String> {
    let key = jsonwebtoken::EncodingKey::from_ec_pem(private_key_pem.as_bytes())
        .map_err(|error| format!("private key rejected (must be PKCS#8 PEM): {error}"))?;
    let now = Timestamp::now().as_second();
    let claims = direct_token_claims(namespace, database, now);
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
        &claims,
        &key,
    )
    .map_err(|error| format!("token encoding failed: {error}"))
}

fn direct_token_claims(namespace: &str, database: &str, now: i64) -> serde_json::Value {
    serde_json::json!({
        "ns": namespace,
        "db": database,
        "ac": diary_core::contract::DIRECT_ACCESS,
        "id": "diary_device:admin",
        "diary_schema_epoch": diary_core::contract::CURRENT_SCHEMA_EPOCH,
        "iat": now,
        "exp": now + DIRECT_SYNC_TOKEN_TTL_SECONDS,
    })
}

/// The probe-and-dedupe algorithm itself lives in `diary_core::store` now —
/// the same code the device-local store runs — so this adapter only fetches
/// the handle. (Its native `mem://` tests, including the killer-replay walk,
/// moved with it.)
async fn save_queued_entry(
    data: &Data,
    entry: ComposedEntry,
    validation_now: i64,
) -> std::result::Result<SavedWrite, SaveError> {
    let db = open_db(data).await.map_err(SaveError::Store)?;
    store::save_entry(&db, entry, validation_now).await
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn api_json(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(body))
        .expect("static headers")
}

fn api_error(status: StatusCode, message: &'static str) -> Response {
    api_json(status, serde_json::json!({ "error": message }).to_string())
}

/// The queries live in `diary_core::store` (the same code the device-local
/// store runs); these adapters fetch the shared handle from `Data` and keep
/// every call site's old shape, including the all-errors-are-Strings rule
/// the outage branches match on.
async fn open_db(data: &Data) -> std::result::Result<diary_core::Db, String> {
    data.diary_db().await.map_err(|error| error.to_string())
}

async fn entry_page(
    data: &Data,
    page_number: usize,
) -> std::result::Result<(Vec<DiaryEntry>, usize), String> {
    store::entry_page(&open_db(data).await?, page_number).await
}

async fn entry_by_id(data: &Data, id: &str) -> std::result::Result<Option<DiaryEntry>, String> {
    store::entry_by_id(&open_db(data).await?, id).await
}

async fn remove_entry(data: &Data, id: &str) -> std::result::Result<(), String> {
    store::remove_entry(&open_db(data).await?, id).await
}

/// The shared preamble both POSTs run before believing anything in the body.
async fn gate(cx: &Cx, body: Body) -> std::result::Result<Vec<u8>, Response> {
    let Some(current) = viewer(cx) else {
        return Err(see_other(LOGIN_REDIRECT));
    };
    if !is_admin(&current.email) {
        return Err(plain(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Err(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !is_form_content_type(headers(cx)) {
        return Err(plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(_) => Err(plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large")),
    }
}

/// Exactly one `name` field and nothing else. Invalid UTF-8 decodes lossily;
/// the strict validation happens on the decoded value upstream.
fn parse_single_field(body: &[u8], name: &str) -> Option<String> {
    let mut value = None;
    for (key, field) in form_urlencoded::parse(body) {
        if key.as_ref() == name && value.is_none() {
            value = Some(field.into_owned());
        } else {
            return None;
        }
    }
    value
}

/// The compose form's canonical content inputs: one body and, when JS has
/// selected a synced parent, one optional permalink. Unknown or duplicate
/// fields fail closed just like [`parse_single_field`]; the shared entry
/// acceptance boundary trims and validates both values.
fn parse_write_form(body: &[u8]) -> Option<(String, Option<String>)> {
    let mut entry_body = None;
    let mut reply_to = None;
    let mut saw_reply_to = false;
    for (key, field) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "body" if entry_body.is_none() => entry_body = Some(field.into_owned()),
            "reply_to" if !saw_reply_to => {
                saw_reply_to = true;
                let value = field.into_owned();
                if !value.is_empty() {
                    reply_to = Some(value);
                }
            }
            _ => return None,
        }
    }
    Some((entry_body?, reply_to))
}

fn requested_page(raw: Option<&str>) -> Option<usize> {
    match raw {
        None => Some(1),
        Some(value) => value
            .parse()
            .ok()
            .filter(|number| (1..=MAX_PAGE).contains(number)),
    }
}

fn last_page(total: usize) -> usize {
    total.div_ceil(PAGE_SIZE).max(1)
}

/// Bounce back to the diary with a static notice code — never echoed input.
fn back(notice: &'static str) -> Response {
    see_other(&format!("{PATH}?notice={notice}"))
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("static locations are valid headers")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(message))
        .expect("static headers")
}

fn is_form_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

fn log_failure(step: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "diary failed",
            "step": step,
            "error": error,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the page: unlisted everywhere, and — unlike a
    /// hidden page — impossible to grant to anyone at `/admin/permissions`,
    /// because it has no `HIDDEN_PAGES` entry.
    #[test]
    fn diary_is_unlisted_and_ungrantable() {
        assert!(
            !crate::content::routes::site_routes().contains(&PATH.to_string()),
            "{PATH} leaked into site_routes()"
        );
        assert!(
            crate::content::access::hidden_page(PATH).is_none(),
            "{PATH} must never be a grantable hidden page"
        );
        assert_eq!(LOGIN_REDIRECT, format!("/login?next={}", urlencode(PATH)));
    }

    #[test]
    fn single_field_forms_parse_strictly() {
        assert_eq!(
            parse_single_field(b"body=Dear+diary%2C", "body").as_deref(),
            Some("Dear diary,")
        );
        assert_eq!(
            parse_single_field(b"path=2026-07-27T10-00-00-04-00", "path").as_deref(),
            Some("2026-07-27T10-00-00-04-00")
        );
        assert_eq!(parse_single_field(b"", "body"), None);
        assert_eq!(parse_single_field(b"body=a&body=b", "body"), None);
        assert_eq!(parse_single_field(b"body=a&submit=Save", "body"), None);
        assert_eq!(parse_single_field(b"path=x", "body"), None);
    }

    #[test]
    fn write_forms_accept_one_optional_reply_target() {
        assert_eq!(parse_write_form(b"body=hi"), Some(("hi".to_string(), None)));
        assert_eq!(
            parse_write_form(b"body=hi&reply_to="),
            Some(("hi".to_string(), None))
        );
        assert_eq!(
            parse_write_form(b"body=hi&reply_to=2026-07-27T14-30-45-04-00"),
            Some((
                "hi".to_string(),
                Some("2026-07-27T14-30-45-04-00".to_string())
            ))
        );
        assert_eq!(parse_write_form(b"body=a&body=b"), None);
        assert_eq!(parse_write_form(b"body=a&extra=1"), None);
        assert_eq!(parse_write_form(b"reply_to=x"), None);
        assert_eq!(parse_write_form(b"body=a&reply_to=x&reply_to=y"), None);
    }

    #[test]
    fn sync_requests_require_the_exact_schema_epoch() {
        let mut request_headers = HeaderMap::new();
        assert_eq!(requested_schema_epoch(&request_headers), None);
        request_headers.insert(
            diary_core::contract::SCHEMA_EPOCH_HEADER,
            HeaderValue::from_str(&diary_core::contract::CURRENT_SCHEMA_EPOCH.to_string()).unwrap(),
        );
        assert_eq!(
            requested_schema_epoch(&request_headers),
            Some(diary_core::contract::CURRENT_SCHEMA_EPOCH)
        );
        assert!(has_current_schema_epoch(&request_headers));
        request_headers.insert(
            diary_core::contract::SCHEMA_EPOCH_HEADER,
            HeaderValue::from_static("not-an-epoch"),
        );
        assert_eq!(requested_schema_epoch(&request_headers), None);
        assert!(!has_current_schema_epoch(&request_headers));
    }

    #[test]
    fn direct_tokens_use_the_current_schema_epoch() {
        let claims = direct_token_claims("site", "prod", 100);
        assert_eq!(claims["ns"], "site");
        assert_eq!(claims["db"], "prod");
        assert_eq!(claims["iat"], 100);
        assert_eq!(claims["exp"], 100 + DIRECT_SYNC_TOKEN_TTL_SECONDS);
        assert_eq!(claims["ac"], diary_core::contract::DIRECT_ACCESS);
        assert_eq!(
            claims["diary_schema_epoch"],
            diary_core::contract::CURRENT_SCHEMA_EPOCH
        );
    }

    /// The protocol items now live in `diary-core` (where the wasm worker
    /// compiles them too); what stays here is the glue that must keep
    /// matching them: the route literals in the `#[route]` attributes.
    #[test]
    fn shared_contract_matches_the_server_glue() {
        assert_eq!(diary_core::contract::API_PATH, "/api/diary/entries");
        assert_eq!(diary_core::contract::SNAPSHOT_PATH, "/api/diary/snapshot");
        assert_eq!(diary_core::contract::TOKEN_PATH, "/api/diary/token");
    }

    #[test]
    fn page_numbers_parse_and_urls_stay_canonical() {
        assert_eq!(requested_page(None), Some(1));
        assert_eq!(requested_page(Some("1")), Some(1));
        assert_eq!(requested_page(Some("37")), Some(37));
        assert_eq!(requested_page(Some("1000000")), Some(MAX_PAGE));
        // Beyond MAX_PAGE the derived START would eventually not fit the
        // store's signed 64-bit literal — treated like nonsense, not an outage.
        for bad in [
            "0",
            "-1",
            "two",
            "",
            "1.5",
            "1000001",
            "18446744073709551615",
        ] {
            assert_eq!(requested_page(Some(bad)), None, "accepted {bad:?}");
        }
        assert_eq!(last_page(0), 1);
        assert_eq!(last_page(1), 1);
        assert_eq!(last_page(PAGE_SIZE), 1);
        assert_eq!(last_page(PAGE_SIZE + 1), 2);
        // URL shapes now live in views (the worker's SSR uses them too);
        // pinned here because these routes' redirects embed them.
        assert_eq!(views::page_url(1), "/diary");
        assert_eq!(views::page_url(3), "/diary?page=3");
        assert_eq!(
            views::entry_url("2026-07-27T10-00-00-04-00"),
            "/diary/2026-07-27T10-00-00-04-00"
        );
    }

    /// Stamp formatting (midnight/noon/garbage walks included) moved to
    /// `diary_core::views` with the markup; what stays pinned here is the
    /// projection the redirects rely on.
    #[test]
    fn entry_ids_project_like_lifting_permalinks() {
        let instant = eastern::eastern_instant("2026-07-27 18:30:45", 0).unwrap();
        let id = eastern::public_path(&instant);
        assert_eq!(id, "2026-07-27T14-30-45-04-00");
        assert_eq!(eastern::parse_public_path(&id).unwrap(), instant);
        // The detail page redirects to entry_url(id) only after the id passes
        // parse_public_path; that is safe because a valid id is plain
        // printable ASCII — the Location header build cannot fail on it.
        assert!(id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        assert_eq!(views::entry_stamp(&id), "Jul 27, 2026 · 2:30 PM");
        assert_eq!(views::entry_date(&id), "Jul 27, 2026");
    }

    /// Key projection and probe-validity tests moved to `diary_core::store`
    /// with the code; what stays here is the one seam the server owns — the
    /// form path's "now" must key exactly like the queue's client epochs.
    #[test]
    fn fresh_entry_keys_parse_and_stamp_now() {
        let written_at = Timestamp::now().as_second();
        let id = store::entry_key(written_at).expect("now projects");
        assert!(eastern::parse_public_path(&id).is_some(), "bad key {id}");
        assert!(written_at > 1_750_000_000, "implausible epoch {written_at}");
        assert_eq!(store::entry_key(written_at).as_deref(), Some(id.as_str()));
    }

    /// diary.js is the page half of the offline queue; these literals are
    /// load-bearing. (sw.js's are pinned in pwa.rs, shared names in both.)
    #[test]
    fn diary_js_pins_the_page_contract() {
        const DIARY_JS_SRC: &str = include_str!("diary/diary.js");
        for needle in [
            "const SW_URL = \"/sw.js\";",
            "const SCOPE = \"/diary\";",
            "{ scope: SCOPE }",
            "\"sync\" in registration",
            "Math.floor(Date.now() / 1000)",
            "storage.persist",
            "pageshow",
            "visibilitychange",
            "\"diary-compose\"",
            "\"diary-body\"",
            "\"diary-queue\"",
            "form.submit()",
            "preventDefault",
            // the Rust queue module: loader → pinned glue → instantiation,
            // with the form POST as the fallback when any of it refuses
            "const SYNC_LOADER = \"/diary-sync.js\";",
            "const STORE_LOCK = \"diary-store\";",
            "navigator.locks.request(STORE_LOCK",
            "DIARY_SYNC.glue",
            "module_or_path: self.DIARY_SYNC.wasm",
            "diary_enqueue",
            "diary_schema_epoch",
            "diary_snapshot",
            "diary_discard",
            // the reconciliation contract: bubbles clone the server-shipped
            // template and are keyed by data-id — the page's whole dedupe
            // rule is "does the DOM already show this id"
            "\"diary-bubble\"",
            ".diary-message[data-id=",
            "CSS.escape",
            "dataset.state",
            ".diary-note",
            ".diary-discard",
            ".diary-reply",
            ".diary-reply-to",
            ".diary-permalink",
            "diary-replying",
            "setReplyTarget",
            // current identity-only acknowledgements plus the rolling-deploy
            // name understood by a predecessor worker
            "saved_refs",
            "saved_entries",
            "diary-message-queued",
            // the placeholder toggle and the offline fallback guard
            "diary-empty",
            "navigator.onLine === false",
            // Enter-to-send stays desktop-only and IME-safe
            "(hover: hover) and (pointer: fine)",
            "isComposing",
            // Optional content must be absent for a top-level wire value,
            // never serialized as a null field.
            "content.reply_to = parent",
        ] {
            assert!(DIARY_JS_SRC.contains(needle), "diary.js lost {needle:?}");
        }
        // The old post-flush reload was a guaranteed disappear-and-reappear
        // flash; the optimistic transcript replaced it. Any full navigation
        // after a save is the jank regression this pins against.
        assert!(
            !DIARY_JS_SRC.contains("location.assign") && !DIARY_JS_SRC.contains("location.reload"),
            "diary.js reintroduced a post-flush navigation"
        );
        // The template is the ONE definition of bubble markup; JS must never
        // grow a hand-built article again.
        assert!(
            !DIARY_JS_SRC.contains("createElement(\"article\""),
            "diary.js rebuilt bubble markup outside the template"
        );
        assert!(
            !DIARY_JS_SRC.contains("reply_to: null"),
            "diary.js must omit absent optional content from exact wire JSON"
        );
    }
}
