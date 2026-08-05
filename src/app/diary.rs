//! `/diary` — the admin's completely private, database-backed diary.
//!
//! Deliberately NOT a hidden page: a `HIDDEN_PAGES` entry would render a
//! grant form for it at `/admin/permissions`, and one mistyped grant would
//! share a diary. Admin-only means exactly one identity, ever — the same
//! constant comparison as `/admin`, working through database outages. The
//! page otherwise follows every hidden-page invariant (docs/auth.md): out of
//! all public registries, `no-store` before `shell()` on every variant,
//! `analytics: false`, signed-out → login redirect, signed-in non-admin →
//! the real 404. Its one listing is the `/admin` tool card.
//!
//! An entry is a timestamp and text, nothing else yet (metadata fields can
//! join the schema later). The record key is the entry's Eastern public path
//! (`eastern::public_path`, the `/lifting/{path}` permalink shape), so keys
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
//! which keys the entry by the CLIENT's composition second and dedupes
//! replays (same second + same body) — Background Sync retries can never
//! double-post, and a queued entry keeps the time it was written, not the
//! time it synced. Details in docs/auth.md.

use diary_core::contract::{WireEntry, normalize_body, written_at_in_window};
use diary_core::eastern;
use diary_core::store::{self, DiaryEntry, MAX_PAGE, PAGE_SIZE, SaveError, SavedWrite};
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
use crate::util::urlencode;

use super::analytics::is_same_origin;
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
/// be a hashed asset), and the queue's Rust half rides `diary_sync.rs`.
const DIARY_JS: Asset = asset!("./diary/diary.js");

const META_LABEL: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const TEXTAREA: &str = "w-full min-w-0 min-h-[3rem] px-3 py-[0.65rem] text-ink bg-card \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-relaxed outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

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
                return Err(redirect(&page_url(last_page(total))).into());
            }
            (entries, total, true)
        }
        Err(error) => {
            log_failure("list", &error);
            (Vec::new(), 0, false)
        }
    };
    let last = last_page(total);
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Diary",
            active: "",
            runtime: false,
            analytics: false,
            pwa: true,
            <div class="diary-room" data-page=(page_number)>
                <div class="diary-room-bar">
                    <span class=(META_LABEL)>"diary · just you"</span>
                    if total > PAGE_SIZE {
                        <span class="font-meta text-xs text-muted">
                            (format!("page {page_number} of {last}"))
                        </span>
                    }
                </div>
                <div id="diary-transcript" class="diary-transcript" tabindex="0">
                    if store_ok && page_number < last {
                        <p class="text-center font-meta text-xs">
                            <a class="quiet-link" href=(page_url(page_number + 1))>
                                "↑ older messages"
                            </a>
                        </p>
                    }
                    if let Some(message) = notice {
                        <p class="border-l-2 border-oxide pl-3 font-meta text-sm text-ink2">
                            (message)
                        </p>
                    }
                    if !store_ok {
                        <p class="text-ink2">
                            "The diary store is unreachable, so nothing can be listed "
                            "right now. Entries are safe where they are; try again in "
                            "a moment."
                        </p>
                    }
                    // diary.js hides this the moment any bubble renders —
                    // an optimistic first message must not sit under a
                    // placeholder that contradicts it.
                    if store_ok && total == 0 {
                        <p id="diary-empty" class="my-auto text-center text-sm text-muted">
                            "No messages yet. Say something below."
                        </p>
                    }
                    <section class="diary-history" aria-label="Diary messages">
                        for entry in entries.iter().rev() {
                            // data-id is the reconciliation contract with
                            // diary.js: entry ids are stable from the moment
                            // of enqueue, so "does the DOM already show this
                            // id" is the page's entire dedupe logic.
                            <article
                                class="diary-message"
                                data-id=(entry.id.as_str())
                                data-state="synced"
                            >
                                <p class="leading-relaxed whitespace-pre-wrap text-ink2">
                                    (entry.body.as_str())
                                </p>
                                <p class="mt-2 text-right font-meta text-[0.6875rem] text-muted">
                                    <a class="quiet-link" href=(entry_url(&entry.id))>
                                        (entry_stamp(entry))
                                    </a>
                                </p>
                            </article>
                        }
                    </section>
                    // diary.js appends bubbles here for rows the server
                    // HTML does not show, by cloning the template below.
                    <section id="diary-queue" class="diary-queue" hidden=""></section>
                    // The one definition of bubble markup the page JS may
                    // draw from — cloned, never built from strings, so the
                    // Tailwind scan sees every class and the markup cannot
                    // drift from the server's transcript articles.
                    <template id="diary-bubble">
                        <article class="diary-message" data-id="" data-state="draft">
                            <p class="leading-relaxed whitespace-pre-wrap text-ink2"></p>
                            <p class="mt-2 text-right font-meta text-[0.6875rem] text-muted">
                                <span class="diary-note text-ink2"></span>
                                <a class="quiet-link" hidden=""></a>
                                <button
                                    type="button"
                                    class="diary-discard quiet-link ml-3 cursor-pointer font-meta text-xs"
                                    hidden=""
                                >"discard"</button>
                            </p>
                        </article>
                    </template>
                    if store_ok && page_number > 1 {
                        <p class="text-center font-meta text-xs">
                            <a class="quiet-link" href=(page_url(page_number - 1))>
                                "newer messages ↓"
                            </a>
                        </p>
                    }
                </div>
                <form
                    method="post"
                    action="/diary/write"
                    id="diary-compose"
                    class="diary-compose"
                >
                    <div class="diary-compose-row">
                        <label class="min-w-0 flex-1" for="diary-body">
                            <span class="sr-only">"New diary message"</span>
                            // autofocus lands the cursor in the box on
                            // desktop; touch browsers ignore it rather
                            // than popping the keyboard. diary.js adds
                            // Enter-to-send for keyboard environments.
                            <textarea
                                class=(TEXTAREA)
                                id="diary-body"
                                name="body"
                                rows="2"
                                required=""
                                autofocus=""
                                placeholder="Message yourself…"
                            ></textarea>
                        </label>
                        <button
                            type="submit"
                            class="oxlink mb-3 shrink-0 cursor-pointer font-meta text-sm"
                        >"send ↑"</button>
                    </div>
                </form>
            </div>
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
        return Err(redirect(&entry_url(entry_path)).into());
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
    let heading = entry.map(entry_date).unwrap_or_else(|| "Diary".to_string());
    let title = format!("Diary · {heading}");
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: title.as_str(),
            active: "",
            runtime: false,
            analytics: false,
            pwa: true,
            page_head(stamp: "diary", title: heading.as_str(), lede: "")
            if let Some(entry) = entry {
                <section class="mt-8 max-w-prose">
                    <p class="font-meta text-xs text-muted">(entry_stamp(entry))</p>
                    <p class="mt-3 leading-relaxed whitespace-pre-wrap text-ink2">
                        (entry.body.as_str())
                    </p>
                    <form
                        method="post"
                        action="/diary/delete"
                        class="mt-10 border-t border-hairline pt-4 text-right"
                    >
                        <input type="hidden" name="path" value=(entry.id.as_str())>
                        <button
                            type="submit"
                            class="quiet-link cursor-pointer font-meta text-xs"
                        >"delete this entry"</button>
                    </form>
                </section>
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
    let Some(raw) = parse_single_field(&bytes, "body") else {
        return Ok(back("invalid"));
    };
    let Some(entry_body) = normalize_body(&raw) else {
        return Ok(back("invalid"));
    };
    let Some((id, written_at)) = now_entry() else {
        return Ok(back("unavailable"));
    };
    match insert_entry(app_context::<Data>(cx), &id, written_at, &entry_body).await {
        Ok(()) => Ok(back("saved")),
        Err(error) => {
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
/// 303-to-login is invisible), the CLIENT's composition second keys the
/// entry, and replays dedupe instead of erroring. Worker-initiated fetches
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
    // `WireEntry` is the shared wire shape — the exact struct the worker
    // serialized, deserialized with `deny_unknown_fields` so a drift fails
    // loudly instead of half-parsing.
    let entry: WireEntry = match serde_json::from_slice(&bytes) {
        Ok(entry) => entry,
        Err(_) => return Ok(api_error(StatusCode::BAD_REQUEST, "malformed entry")),
    };
    if !written_at_in_window(entry.written_at, Timestamp::now().as_second()) {
        return Ok(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "timestamp out of range",
        ));
    }
    let Some(entry_body) = normalize_body(&entry.body) else {
        return Ok(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "that didn't validate",
        ));
    };
    match save_queued_entry(app_context::<Data>(cx), entry.written_at, &entry_body).await {
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

/// The probe-and-dedupe algorithm itself lives in `diary_core::store` now —
/// the same code the device-local store runs — so this adapter only fetches
/// the handle. (Its native `mem://` tests, including the killer-replay walk,
/// moved with it.)
async fn save_queued_entry(
    data: &Data,
    written_at: i64,
    body: &str,
) -> std::result::Result<SavedWrite, SaveError> {
    let db = open_db(data).await.map_err(SaveError::Store)?;
    store::save_entry(&db, written_at, body).await
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
    data.db().await.map_err(|error| error.to_string())
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

async fn insert_entry(
    data: &Data,
    id: &str,
    written_at: i64,
    body: &str,
) -> std::result::Result<(), String> {
    store::insert_entry(&open_db(data).await?, id, written_at, body).await
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

/// The record key and stored timestamp for a new form entry, from one
/// instant. (`store::entry_key` is the projection itself — shared with the
/// queue, which keys offline entries by their composition second.)
fn now_entry() -> Option<(String, i64)> {
    let now = Timestamp::now().as_second();
    Some((store::entry_key(now)?, now))
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

fn page_url(page_number: usize) -> String {
    if page_number <= 1 {
        PATH.to_string()
    } else {
        format!("{PATH}?page={page_number}")
    }
}

fn entry_url(id: &str) -> String {
    format!("{PATH}/{id}")
}

/// "Jul 27, 2026 · 2:30 PM", from the id's Eastern wall clock. Stored ids
/// always parse; anything else falls back to the raw id.
fn entry_stamp(entry: &DiaryEntry) -> String {
    match eastern::parse_public_path(&entry.id).and_then(|instant| display_parts(&instant.local)) {
        Some((date, time)) => format!("{date} · {time}"),
        None => entry.id.clone(),
    }
}

/// The date half alone — the entry page's heading.
fn entry_date(entry: &DiaryEntry) -> String {
    match eastern::parse_public_path(&entry.id).and_then(|instant| display_parts(&instant.local)) {
        Some((date, _)) => date,
        None => entry.id.clone(),
    }
}

/// `YYYY-MM-DD HH:MM:SS` → ("Jul 27, 2026", "2:30 PM"). The shape is
/// guaranteed by `parse_public_path`; anything else returns `None`.
fn display_parts(local: &str) -> Option<(String, String)> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if local.len() != 19 || !local.is_ascii() {
        return None;
    }
    let month: usize = local[5..7].parse().ok()?;
    let day: u32 = local[8..10].parse().ok()?;
    let hour: u32 = local[11..13].parse().ok()?;
    if !(1..=12).contains(&month) || hour > 23 {
        return None;
    }
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let clock_hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    Some((
        format!("{} {day}, {}", MONTHS[month - 1], &local[..4]),
        format!("{clock_hour}:{} {suffix}", &local[14..16]),
    ))
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
    use diary_core::contract::MAX_ENTRY_CHARS;

    use super::*;

    /// The whole point of the page: unlisted everywhere, untrackable, and —
    /// unlike a hidden page — impossible to grant to anyone at
    /// `/admin/permissions`, because it has no `HIDDEN_PAGES` entry.
    #[test]
    fn diary_is_unlisted_ungrantable_and_untrackable() {
        assert!(
            !crate::content::routes::site_routes().contains(&PATH.to_string()),
            "{PATH} leaked into site_routes()"
        );
        assert!(!crate::content::routes::is_trackable_route(PATH));
        assert!(!crate::content::routes::is_trackable_route(
            "/diary/2026-07-27T10-00-00-04-00"
        ));
        assert!(!crate::content::routes::is_trackable_route(
            "/api/diary/entries"
        ));
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

    /// The protocol items now live in `diary-core` (where the wasm worker
    /// compiles them too); what stays here is the glue that must keep
    /// matching them: the route literal in the `#[route]` attribute, and the
    /// schema ASSERT that `MAX_ENTRY_CHARS` mirrors.
    #[test]
    fn shared_contract_matches_the_server_glue() {
        assert_eq!(diary_core::contract::API_PATH, "/api/diary/entries");
        assert!(
            include_str!("../schema.surql")
                .contains(&format!("string::len($value) <= {MAX_ENTRY_CHARS}")),
            "schema ASSERT no longer mirrors diary-core's MAX_ENTRY_CHARS"
        );
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
        assert_eq!(page_url(1), "/diary");
        assert_eq!(page_url(3), "/diary?page=3");
        assert_eq!(
            entry_url("2026-07-27T10-00-00-04-00"),
            "/diary/2026-07-27T10-00-00-04-00"
        );
    }

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
        let entry = DiaryEntry {
            id,
            written_at: 0,
            body: String::new(),
        };
        assert_eq!(entry_stamp(&entry), "Jul 27, 2026 · 2:30 PM");
        assert_eq!(entry_date(&entry), "Jul 27, 2026");
    }

    #[test]
    fn stamps_handle_midnight_noon_and_garbage() {
        assert_eq!(
            display_parts("2026-01-05 00:07:00").unwrap(),
            ("Jan 5, 2026".to_string(), "12:07 AM".to_string())
        );
        assert_eq!(
            display_parts("2026-12-31 12:00:59").unwrap(),
            ("Dec 31, 2026".to_string(), "12:00 PM".to_string())
        );
        assert_eq!(display_parts("not a stamp"), None);
        let unparseable = DiaryEntry {
            id: "garbage".to_string(),
            written_at: 0,
            body: String::new(),
        };
        assert_eq!(entry_stamp(&unparseable), "garbage");
    }

    /// Key projection and probe-validity tests moved to `diary_core::store`
    /// with the code; what stays here is the one seam the server owns — the
    /// form path's "now" must key exactly like the queue's client epochs.
    #[test]
    fn fresh_entry_keys_parse_and_stamp_now() {
        let (id, written_at) = now_entry().expect("now projects");
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
            "DIARY_SYNC.glue",
            "module_or_path: self.DIARY_SYNC.wasm",
            "diary_enqueue",
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
            "saved_entries",
            "diary-message-queued",
            // the placeholder toggle and the offline fallback guard
            "diary-empty",
            "navigator.onLine === false",
            // Enter-to-send stays desktop-only and IME-safe
            "(hover: hover) and (pointer: fine)",
            "isComposing",
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
    }
}
