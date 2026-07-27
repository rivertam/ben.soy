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
//! sort chronologically and the permalink IS the id. `/diary` renders bodies
//! fully inline, newest first, `PAGE_SIZE` per page; `/diary/{path}` is one
//! entry's own page and the only place it can be deleted.
//!
//! Both POSTs repeat the admin identity check, require positive same-origin
//! evidence, and bound the body before parsing it — the forms are not an
//! authorization boundary. Redirecting responses are hand-built `Ok(303)`s
//! like the admin routes' so every branch carries `no-store`.

use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, Response, StatusCode, header, headers, page, path_param,
        query_params, redirect, route, to_bytes, uri,
    },
    view::view,
};

use benjisponge::data::Data;

use crate::components::{back_link, page_head, shell};
use crate::content::access::is_admin;
use crate::util::urlencode;

use super::analytics::is_same_origin;
use super::interests::lifting::archive::eastern;
use super::login::viewer;
use super::not_found::not_found_page;

pub(crate) const PATH: &str = "/diary";
const LOGIN_REDIRECT: &str = "/login?next=%2Fdiary";
const PAGE_SIZE: usize = 20;
/// Far past any real diary and far under the store's signed-64-bit `START`
/// limit. Wilder `?page=` values behave like unparseable ones (redirect to
/// `/diary`) instead of surfacing a SurrealDB parse error as a fake outage.
const MAX_PAGE: usize = 1_000_000;
/// Mirrors the schema's `string::len($value) <= 65536` ASSERT.
const MAX_ENTRY_CHARS: usize = 65_536;
/// A worst-case urlencoded body of `MAX_ENTRY_CHARS` multibyte characters
/// runs to several hundred KB; 1 MiB matches the fitness import's bound.
const BODY_LIMIT_BYTES: usize = 1024 * 1024;
const NO_STORE: &str = "no-store";

const META_LABEL: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const TEXTAREA: &str = "w-full min-w-0 min-h-[9rem] px-3 py-[0.65rem] text-ink bg-page \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-relaxed outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

/// One stored entry. `id` is the Eastern public-path record key;
/// `written_at` is UTC epoch seconds, the instant the key projects.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
struct DiaryEntry {
    id: String,
    written_at: i64,
    body: String,
}

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
            if let Some(message) = notice {
                <p class="mt-6 max-w-prose border-l-2 border-oxide pl-3 font-meta text-sm text-ink2">
                    (message)
                </p>
            }
            <form method="post" action="/diary/write" class="mt-10 max-w-prose">
                <label class="flex flex-col gap-[0.35rem]" for="diary-body">
                    <span class=(META_LABEL)>"new entry"</span>
                    <textarea
                        class=(TEXTAREA)
                        id="diary-body"
                        name="body"
                        rows="6"
                        required=""
                        placeholder="What happened?"
                    ></textarea>
                </label>
                <button
                    type="submit"
                    class="oxlink mt-3 cursor-pointer font-meta text-sm"
                >"save →"</button>
            </form>
            if !store_ok {
                <p class="mt-12 max-w-prose text-ink2">
                    "The diary store is unreachable, so nothing can be listed "
                    "right now. Entries are safe where they are; try again in "
                    "a moment."
                </p>
            }
            if store_ok && total == 0 {
                <p class="mt-12 max-w-prose text-sm text-muted">
                    "No entries yet. The first one goes just above."
                </p>
            }
            <section class="mt-12 max-w-prose">
                for entry in entries.iter() {
                    <article class="border-t border-hairline py-6">
                        <p class="font-meta text-xs text-muted">
                            <a class="quiet-link" href=(entry_url(&entry.id))>
                                (entry_stamp(entry))
                            </a>
                        </p>
                        <p class="mt-2 leading-relaxed whitespace-pre-wrap text-ink2">
                            (entry.body.as_str())
                        </p>
                    </article>
                }
            </section>
            if store_ok && total > PAGE_SIZE {
                <nav
                    class="mt-4 flex max-w-prose items-baseline justify-between border-t \
                         border-hairline pt-4 font-meta text-xs"
                    aria-label="Diary pages"
                >
                    if page_number > 1 {
                        <a class="quiet-link" href=(page_url(page_number - 1))>"← newer"</a>
                    } else {
                        <span class="text-muted opacity-50" aria-disabled="true">"← newer"</span>
                    }
                    <span class="text-muted">(format!("page {page_number} of {last}"))</span>
                    if page_number < last {
                        <a class="quiet-link" href=(page_url(page_number + 1))>"older →"</a>
                    } else {
                        <span class="text-muted opacity-50" aria-disabled="true">"older →"</span>
                    }
                </nav>
            }
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

/// One page of entries, newest first, plus the total count — one round trip.
async fn entry_page(
    data: &Data,
    page_number: usize,
) -> std::result::Result<(Vec<DiaryEntry>, usize), String> {
    #[derive(Deserialize, SurrealValue)]
    struct CountRow {
        count: i64,
    }

    let db = data.db().await.map_err(|error| error.to_string())?;
    let start = page_number.saturating_sub(1).saturating_mul(PAGE_SIZE);
    // LIMIT/START are server-computed integers, formatted rather than bound
    // to keep the statement inside plainly supported syntax. The id
    // tie-break is deterministic: same-table keys compare as strings, and
    // the DST-fold pair (…-04-00 before …-05-00) even sorts chronologically.
    let mut response = db
        .query(format!(
            "SELECT *, record::id(id) AS id FROM diary_entries \
                 ORDER BY written_at DESC, id DESC LIMIT {PAGE_SIZE} START {start};
             SELECT count() FROM diary_entries GROUP ALL;"
        ))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let entries: Vec<DiaryEntry> = response.take(0).map_err(|error| error.to_string())?;
    let counts: Vec<CountRow> = response.take(1).map_err(|error| error.to_string())?;
    let total = counts
        .into_iter()
        .next()
        .map(|row| row.count.max(0) as usize)
        .unwrap_or(0);
    Ok((entries, total))
}

async fn entry_by_id(data: &Data, id: &str) -> std::result::Result<Option<DiaryEntry>, String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    let mut response = db
        .query("SELECT *, record::id(id) AS id FROM type::record('diary_entries', $id)")
        .bind(("id", id.to_string()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let entries: Vec<DiaryEntry> = response.take(0).map_err(|error| error.to_string())?;
    Ok(entries.into_iter().next())
}

/// CREATE, not UPSERT: two entries in the same second would collide on the
/// key, and overwriting a diary entry is worse than asking me to resubmit.
async fn insert_entry(
    data: &Data,
    id: &str,
    written_at: i64,
    body: &str,
) -> std::result::Result<(), String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    db.query(
        "CREATE ONLY type::record('diary_entries', $id)
             SET written_at = $written_at,
                 body = $body",
    )
    .bind(("id", id.to_string()))
    .bind(("written_at", written_at))
    .bind(("body", body.to_string()))
    .await
    .map_err(|error| error.to_string())?
    .check()
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Idempotent: deleting an already-deleted entry succeeds quietly.
async fn remove_entry(data: &Data, id: &str) -> std::result::Result<(), String> {
    let db = data.db().await.map_err(|error| error.to_string())?;
    db.query("DELETE type::record('diary_entries', $id)")
        .bind(("id", id.to_string()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    Ok(())
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

/// Browser textareas submit CRLF line ends; store LF. Trimmed at both ends,
/// interior blank lines survive. `None` is "not an entry we would store" —
/// the char bound mirrors the schema ASSERT.
fn normalize_body(raw: &str) -> Option<String> {
    let body = raw.replace("\r\n", "\n").replace('\r', "\n");
    let body = body.trim();
    if body.is_empty() || body.chars().count() > MAX_ENTRY_CHARS {
        return None;
    }
    Some(body.to_string())
}

/// The record key and stored timestamp for a new entry, from one instant:
/// the Eastern public path (which becomes the permalink) plus epoch seconds.
fn now_entry() -> Option<(String, i64)> {
    let now = Timestamp::now();
    let utc = now
        .to_zoned(TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();
    let instant = eastern::eastern_instant(&utc, 0).ok()?;
    Some((eastern::public_path(&instant), now.as_second()))
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
    fn bodies_normalize_line_ends_and_bounds() {
        assert_eq!(
            normalize_body("Dear diary,\r\n\r\nIt me.\r\n").as_deref(),
            Some("Dear diary,\n\nIt me.")
        );
        assert_eq!(
            normalize_body("solo\rreturn").as_deref(),
            Some("solo\nreturn")
        );
        assert_eq!(normalize_body(""), None);
        assert_eq!(normalize_body("  \r\n\t "), None);
        let exactly_max = "é".repeat(MAX_ENTRY_CHARS);
        assert_eq!(
            normalize_body(&exactly_max).as_deref(),
            Some(exactly_max.as_str())
        );
        assert_eq!(normalize_body(&"a".repeat(MAX_ENTRY_CHARS + 1)), None);
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

    #[test]
    fn fresh_entry_keys_parse_and_stamp_now() {
        let (id, written_at) = now_entry().expect("now projects");
        assert!(eastern::parse_public_path(&id).is_some(), "bad key {id}");
        assert!(written_at > 1_750_000_000, "implausible epoch {written_at}");
    }
}
