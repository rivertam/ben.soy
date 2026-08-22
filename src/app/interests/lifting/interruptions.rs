//! Annotate-only gym interruptions: illness, travel, and other gaps that
//! explain empty heatmap days without changing volume or training-focus math.
//!
//! Admin create / edit / delete lives here as form POSTs (same identity gate
//! as `/fitness/lift/import`). Open rows (no end date) surface on `/fitness`;
//! closed rows inject into the `/fitness/log` timeline. Heatmap chrome covers
//! open rows through today and closed rows through their end date.

use std::time::{SystemTime, UNIX_EPOCH};

use benjisponge::data::{Data, fitness_models::Interruption};
use jiff::{Timestamp, civil::Date};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, Bytes, HeaderMap, HeaderValue, Response, StatusCode, header, headers, path_param,
        route, to_bytes,
    },
    view::{class, component, view},
};

use crate::{
    app::login::viewer, components::modal, content::access::is_admin, util::is_same_origin,
};

use super::{
    META_LABEL,
    archive::{
        db::{self, InterruptionWrite, is_interruption_id},
        eastern,
        store::FitnessStore,
    },
    data as fitness,
};

const OPEN_REDIRECT: &str = "/fitness#interruptions";
const CLOSED_REDIRECT: &str = "/fitness/log#set-log";
const LOGIN_REDIRECT: &str = "/login?next=%2Ffitness%23interruptions";
const BODY_LIMIT_BYTES: usize = 4 * 1024;
const NO_STORE: &str = "no-store";
const NOTE_MAX: usize = 200;
const MAX_RANGE_DAYS: i32 = 365;
/// Default heatmap marker; also the coalesce for rows written before emoji shipped.
pub(super) const DEFAULT_EMOJI: &str = "🤒";
/// Curated choices for the admin form — keep in sync with form radios + validation.
pub(super) const EMOJI_CHOICES: &[&str] = &[
    "🤒", "🤧", "🤢", "🤕", "😷", "😴", "😭", "✈️", "🏖️", "🚗", "🏥", "💊",
];

const META: &str = "font-meta text-[0.7rem] leading-[1.55] text-muted";
const NOTE_TEXT: &str = "font-meta text-[0.82rem] leading-[1.45] text-ink2";
const FIELD: &str = "mt-1 block w-full rounded-[0.2rem] border border-hairline bg-card \
     px-2.5 py-1.5 font-meta text-[0.8rem] text-ink \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const LABEL: &str = "block font-meta text-[0.68rem] uppercase tracking-[0.04em] text-muted";
const EMOJI_PICK: &str = "mt-2 flex flex-wrap gap-1.5";
const EMOJI_OPTION: &str = "inline-flex size-9 cursor-pointer items-center justify-center \
     rounded-[0.2rem] border border-hairline bg-card text-base leading-none \
     has-[:checked]:border-oxide has-[:checked]:bg-oxide/10 \
     focus-within:outline-solid focus-within:outline-2 focus-within:outline-oxide \
     focus-within:outline-offset-2";
const EMOJI_RADIO: &str = "sr-only";
const BUTTON: &str = "px-3 py-[0.45rem] font-meta text-[0.7rem] text-card bg-oxide \
     border border-oxide rounded-[0.2rem] cursor-pointer hover:text-white hover:bg-oxide-hot \
     hover:border-oxide-hot focus-visible:text-white focus-visible:bg-oxide-hot \
     focus-visible:border-oxide-hot";
const QUIET: &str = "quiet-link cursor-pointer font-meta text-xs";

#[path_param]
struct InterruptionId(str);

#[route(POST "/fitness/interruptions")]
async fn create_interruption(cx: &Cx, body: Body) -> Result<Response> {
    create_interruption_inner(cx, body).await
}

#[route(POST "/lifting/interruptions")]
async fn legacy_create_interruption(cx: &Cx, body: Body) -> Result<Response> {
    create_interruption_inner(cx, body).await
}

async fn create_interruption_inner(cx: &Cx, body: Body) -> Result<Response> {
    let bytes = match gate(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(response),
    };
    let write = match parse_interruption_form(&bytes) {
        Ok(write) => write,
        Err(message) => return Ok(plain(StatusCode::BAD_REQUEST, message)),
    };
    let handle = match app_context::<Data>(cx).db().await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("interruption create could not reach the database: {error}");
            return Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.",
            ));
        }
    };
    match db::create_interruption(&handle, &write, epoch_seconds()).await {
        Ok(_) => {
            rebuild(cx).await;
            Ok(see_other(redirect_for(&write)))
        }
        Err(error) => {
            eprintln!("interruption create failed: {error}");
            Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The interruption could not be saved right now.",
            ))
        }
    }
}

#[route(POST "/fitness/interruptions/{interruption_id}")]
async fn update_interruption(cx: &Cx, body: Body) -> Result<Response> {
    update_interruption_inner(cx, body).await
}

#[route(POST "/lifting/interruptions/{interruption_id}")]
async fn legacy_update_interruption(cx: &Cx, body: Body) -> Result<Response> {
    update_interruption_inner(cx, body).await
}

async fn update_interruption_inner(cx: &Cx, body: Body) -> Result<Response> {
    let id = path_param::<InterruptionId>(cx);
    if !is_interruption_id(id) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    let bytes = match gate(cx, body).await {
        Ok(bytes) => bytes,
        Err(response) => return Ok(response),
    };
    let write = match parse_interruption_form(&bytes) {
        Ok(write) => write,
        Err(message) => return Ok(plain(StatusCode::BAD_REQUEST, message)),
    };
    let handle = match app_context::<Data>(cx).db().await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("interruption update could not reach the database: {error}");
            return Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.",
            ));
        }
    };
    match db::update_interruption(&handle, id, &write, epoch_seconds()).await {
        Ok(Some(_)) => {
            rebuild(cx).await;
            Ok(see_other(redirect_for(&write)))
        }
        Ok(None) => Ok(plain(StatusCode::NOT_FOUND, "not found")),
        Err(error) => {
            eprintln!("interruption update failed: {error}");
            Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The interruption could not be saved right now.",
            ))
        }
    }
}

#[route(POST "/fitness/interruptions/{interruption_id}/delete")]
async fn delete_interruption(cx: &Cx, body: Body) -> Result<Response> {
    delete_interruption_inner(cx, body).await
}

#[route(POST "/lifting/interruptions/{interruption_id}/delete")]
async fn legacy_delete_interruption(cx: &Cx, body: Body) -> Result<Response> {
    delete_interruption_inner(cx, body).await
}

async fn delete_interruption_inner(cx: &Cx, body: Body) -> Result<Response> {
    let id = path_param::<InterruptionId>(cx);
    if !is_interruption_id(id) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    // Delete posts an empty body; still run the auth gate (and discard bytes).
    if let Err(response) = gate(cx, body).await {
        return Ok(response);
    }
    let handle = match app_context::<Data>(cx).db().await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("interruption delete could not reach the database: {error}");
            return Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.",
            ));
        }
    };
    match db::delete_interruption(&handle, id).await {
        Ok(Some(_)) => {
            rebuild(cx).await;
            Ok(see_other(OPEN_REDIRECT))
        }
        Ok(None) => Ok(plain(StatusCode::NOT_FOUND, "not found")),
        Err(error) => {
            eprintln!("interruption delete failed: {error}");
            Ok(plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "The interruption could not be deleted right now.",
            ))
        }
    }
}

async fn gate(cx: &Cx, body: Body) -> std::result::Result<Bytes, Response> {
    let viewer_email = viewer(cx).map(|current| current.email.clone());
    match authorize_write(viewer_email.as_deref(), is_same_origin(headers(cx))) {
        WriteAuth::Login => return Err(see_other(LOGIN_REDIRECT)),
        WriteAuth::NotFound => return Err(plain(StatusCode::NOT_FOUND, "not found")),
        WriteAuth::Forbidden => return Err(plain(StatusCode::FORBIDDEN, "forbidden")),
        WriteAuth::Allowed => {}
    }
    if !is_form_content_type(headers(cx)) {
        return Err(plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > BODY_LIMIT_BYTES => {
            return Err(plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large"));
        }
        Ok(_) => {}
        Err(()) => return Err(plain(StatusCode::BAD_REQUEST, "bad Content-Length")),
    }
    to_bytes(body, BODY_LIMIT_BYTES)
        .await
        .map_err(|_| plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large"))
}

#[derive(Debug, PartialEq, Eq)]
enum WriteAuth {
    Allowed,
    Login,
    NotFound,
    Forbidden,
}

/// Viewer → admin → same-origin, matching `/fitness/lift/import`.
fn authorize_write(viewer_email: Option<&str>, same_origin: bool) -> WriteAuth {
    match viewer_email {
        None => WriteAuth::Login,
        Some(email) if !is_admin(email) => WriteAuth::NotFound,
        Some(_) if !same_origin => WriteAuth::Forbidden,
        Some(_) => WriteAuth::Allowed,
    }
}

async fn rebuild(cx: &Cx) {
    if let Err(error) = app_context::<FitnessStore>(cx).rebuild().await {
        eprintln!("post-interruption snapshot rebuild failed: {error}");
    }
}

fn redirect_for(write: &InterruptionWrite) -> &'static str {
    if write.to_date.is_none() {
        OPEN_REDIRECT
    } else {
        CLOSED_REDIRECT
    }
}

/// Open interruptions currently in progress (`to_date` absent).
pub(super) fn open_rows(rows: &[Interruption]) -> Vec<&Interruption> {
    rows.iter().filter(|row| row.to_date.is_none()).collect()
}

/// Home-page notes section: only when at least one open interruption exists.
#[component]
pub(super) async fn open_panel(rows: &[&Interruption], can_edit: bool) -> Result {
    view! {
        <section aria-labelledby="interruptions-title" id="interruptions">
            <header class="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <p class="font-meta text-[0.68rem] uppercase tracking-[0.04em] text-muted">
                        "archive notes"
                    </p>
                    <h2
                        id="interruptions-title"
                        class="font-display text-2xl font-semibold"
                    >
                        "Interruptions"
                    </h2>
                    <p class=(class!(META, "mt-[0.3rem]"))>
                        "Current gaps that explain missing gym days — illness, travel, and the like. \
                         They annotate the calendar only; volume points are unchanged."
                    </p>
                </div>
            </header>

            <ul class="mt-5 flex flex-col gap-3">
                for row in rows.iter().copied() {
                    interruption_row(row: row, can_edit: can_edit)
                }
            </ul>
        </section>
    }
}

/// Admin create dialog opened by the unified log launcher on `/fitness`.
#[component]
pub(super) async fn create_dialog() -> Result {
    view! {
        modal(
            id: "fitness-interruption-dialog",
            label: "Interruption",
            labelledby: "interruption-create-title",
            <p class=(META_LABEL)>"archive notes"</p>
            <h2
                id="interruption-create-title"
                class="mt-1 pr-10 font-display text-2xl font-semibold"
            >
                "Log an interruption"
            </h2>
            <p class=(class!(META, "mt-2"))>
                "Leave the end date blank while it is ongoing. Closed ranges appear in the \
                 activity log."
            </p>
            <div class="mt-4">
                interruption_form(
                    action: "/fitness/interruptions",
                    from_date: "",
                    to_date: "",
                    note: "",
                    emoji: DEFAULT_EMOJI,
                    submit: "save interruption",
                    autofocus: true
                )
            </div>
        )
    }
}

/// One closed interruption as a set-log timeline entry.
#[component]
pub(super) async fn log_entry(row: &Interruption, can_edit: bool) -> Result {
    let range = format_range(&row.from_date, row.to_date.as_deref());
    let stamp = row
        .to_date
        .as_deref()
        .map(format_stamp)
        .unwrap_or_else(|| range.clone());
    view! {
        <article class="rail-row rail-row-top" id=(format!("interruption-{}", row.id))>
            <div class="rail-stamp sm:pt-[0.35rem]">
                <p class="text-ink2">(stamp.as_str())</p>
            </div>
            <div class="min-w-0 p-4 bg-card border border-hairline sm:px-5 sm:py-[1.1rem]">
                <p class=(NOTE_TEXT)>
                    <span class="mr-1" aria-hidden="true">(row.emoji.as_str())</span>
                    <span class="text-ink">(range.as_str())</span>
                    " · "
                    (row.note.as_str())
                </p>
                if can_edit {
                    interruption_admin_controls(row: row)
                }
            </div>
        </article>
    }
}

/// Merged fitness-log rows: primary activities retain their exact UTC order,
/// while closed interruptions sit after every same-date primary.
pub(super) enum LogItem<'a> {
    Activity(&'a fitness::LogActivity),
    Interruption(&'a Interruption),
}

pub(super) fn merge_log_items<'a>(
    activities: &'a [fitness::LogActivity],
    interruptions: &'a [Interruption],
) -> Vec<LogItem<'a>> {
    let mut items: Vec<LogItem<'a>> = activities.iter().map(LogItem::Activity).collect();
    items.extend(interruptions.iter().map(LogItem::Interruption));
    items.sort_by(|a, b| {
        let (a_date, a_kind) = log_sort_key(a);
        let (b_date, b_kind) = log_sort_key(b);
        // `sort_by` is stable: equal-date activities retain the composite
        // model's exact UTC order. Interruptions are deterministic by id.
        b_date
            .cmp(a_date)
            .then_with(|| a_kind.cmp(&b_kind))
            .then_with(|| match (a, b) {
                (LogItem::Interruption(left), LogItem::Interruption(right)) => {
                    left.id.cmp(&right.id)
                }
                _ => std::cmp::Ordering::Equal,
            })
    });
    items
}

fn log_sort_key<'a>(item: &'a LogItem<'_>) -> (&'a str, u8) {
    match item {
        LogItem::Activity(activity) => (activity.date(), 0),
        LogItem::Interruption(row) => (row.to_date.as_deref().unwrap_or(""), 1),
    }
}

#[component]
async fn interruption_row(row: &Interruption, can_edit: bool) -> Result {
    let range = format_range(&row.from_date, row.to_date.as_deref());
    view! {
        <li class="border-t border-hairline pt-3 first:border-t-0 first:pt-0">
            <div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
                <p class=(NOTE_TEXT)>
                    <span class="mr-1" aria-hidden="true">(row.emoji.as_str())</span>
                    <span class="text-ink">(range.as_str())</span>
                    " · "
                    (row.note.as_str())
                </p>
            </div>
            if can_edit {
                interruption_admin_controls(row: row)
            }
        </li>
    }
}

#[component]
async fn interruption_admin_controls(row: &Interruption) -> Result {
    let action = format!("/fitness/interruptions/{}", row.id);
    let delete_action = format!("/fitness/interruptions/{}/delete", row.id);
    let to_value = row.to_date.as_deref().unwrap_or("");
    view! {
        <div class="mt-2 flex flex-wrap items-start gap-4">
            <details class="group">
                <summary class=(class!(QUIET, "list-none [&::-webkit-details-marker]:hidden"))>
                    "edit"
                </summary>
                <div class="mt-3 max-w-md">
                    interruption_form(
                        action: action.as_str(),
                        from_date: row.from_date.as_str(),
                        to_date: to_value,
                        note: row.note.as_str(),
                        emoji: row.emoji.as_str(),
                        submit: "save changes",
                        autofocus: false
                    )
                </div>
            </details>
            <details class="group">
                <summary class=(class!(QUIET, "list-none [&::-webkit-details-marker]:hidden"))>
                    "delete"
                </summary>
                <form
                    method="post"
                    action=(delete_action.as_str())
                    class="mt-2 flex flex-wrap items-center gap-3"
                >
                    <p class=(META)>"Remove this interruption?"</p>
                    <button type="submit" class=(BUTTON)>"delete permanently"</button>
                </form>
            </details>
        </div>
    }
}

#[component]
async fn interruption_form(
    action: &str,
    from_date: &str,
    to_date: &str,
    note: &str,
    emoji: &str,
    submit: &str,
    autofocus: bool,
) -> Result {
    let selected = if EMOJI_CHOICES.contains(&emoji) {
        emoji
    } else {
        DEFAULT_EMOJI
    };
    view! {
        <form method="post" action=(action) class="flex flex-col gap-3">
            <div class="grid gap-3 sm:grid-cols-2">
                <label class=(LABEL)>
                    "from"
                    <input
                        class=(FIELD)
                        type="date"
                        name="from"
                        autofocus=(autofocus)
                        required=""
                        value=(from_date)
                    />
                </label>
                <label class=(LABEL)>
                    "to (optional)"
                    <input
                        class=(FIELD)
                        type="date"
                        name="to"
                        value=(to_date)
                    />
                </label>
            </div>
            <fieldset class="min-w-0">
                <legend class=(LABEL)>"heatmap emoji"</legend>
                <div class=(EMOJI_PICK) role="radiogroup" aria-label="Heatmap emoji">
                    for choice in EMOJI_CHOICES.iter().copied() {
                        <label class=(EMOJI_OPTION) title=(choice)>
                            <input
                                class=(EMOJI_RADIO)
                                type="radio"
                                name="emoji"
                                value=(choice)
                                checked=(choice == selected)
                                required=""
                            />
                            <span aria-hidden="true">(choice)</span>
                        </label>
                    }
                </div>
            </fieldset>
            <label class=(LABEL)>
                "reason"
                <input
                    class=(FIELD)
                    type="text"
                    name="note"
                    required=""
                    maxlength="200"
                    value=(note)
                    placeholder="cold"
                />
            </label>
            <div>
                <button type="submit" class=(BUTTON)>(submit)</button>
            </div>
        </form>
    }
}

/// Inclusive Eastern range label; open rows end with an arrow.
pub(super) fn format_range(from: &str, to: Option<&str>) -> String {
    let Ok(start) = from.parse::<Date>() else {
        return match to {
            Some(to) => format!("{from}–{to}"),
            None => format!("{from} →"),
        };
    };
    let Some(to) = to else {
        return format!("{} →", start.strftime("%b %-d, %Y"));
    };
    let Ok(end) = to.parse::<Date>() else {
        return format!("{from}–{to}");
    };
    if start == end {
        return start.strftime("%b %-d, %Y").to_string();
    }
    if start.year() == end.year() && start.month() == end.month() {
        return format!(
            "{}–{}, {}",
            start.strftime("%b %-d"),
            end.strftime("%-d"),
            end.strftime("%Y"),
        );
    }
    if start.year() == end.year() {
        return format!(
            "{}–{}, {}",
            start.strftime("%b %-d"),
            end.strftime("%b %-d"),
            end.strftime("%Y"),
        );
    }
    format!(
        "{}–{}",
        start.strftime("%b %-d, %Y"),
        end.strftime("%b %-d, %Y"),
    )
}

fn format_stamp(date: &str) -> String {
    date.parse::<Date>()
        .map(|parsed| parsed.strftime("%b %-d, %Y").to_string())
        .unwrap_or_else(|_| date.to_string())
}

/// Validate and normalize the create/update form fields.
pub(super) fn parse_interruption_form(
    body: &[u8],
) -> std::result::Result<InterruptionWrite, &'static str> {
    let mut from = None;
    let mut to = None;
    let mut note = None;
    let mut emoji = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "from" => {
                if from.replace(value.into_owned()).is_some() {
                    return Err("duplicate from");
                }
            }
            "to" => {
                if to.replace(value.into_owned()).is_some() {
                    return Err("duplicate to");
                }
            }
            "note" => {
                if note.replace(value.into_owned()).is_some() {
                    return Err("duplicate note");
                }
            }
            "emoji" => {
                if emoji.replace(value.into_owned()).is_some() {
                    return Err("duplicate emoji");
                }
            }
            _ => return Err("unexpected field"),
        }
    }
    let from = from.ok_or("missing from")?;
    let to = to.unwrap_or_default();
    let note = note.ok_or("missing note")?;
    let emoji = emoji.ok_or("missing emoji")?;
    validate_interruption(&from, &to, &note, &emoji)
}

pub(super) fn validate_interruption(
    from: &str,
    to: &str,
    note: &str,
    emoji: &str,
) -> std::result::Result<InterruptionWrite, &'static str> {
    let from_date = parse_date(from).ok_or("bad from date")?;
    let to_date = if to.trim().is_empty() {
        None
    } else {
        let to_date = parse_date(to).ok_or("bad to date")?;
        if to_date < from_date {
            return Err("to must be on or after from");
        }
        let span_days = (to_date - from_date).get_days();
        if span_days > MAX_RANGE_DAYS {
            return Err("range is too long");
        }
        Some(to_date)
    };
    let note = note.trim();
    if note.is_empty() {
        return Err("note is required");
    }
    if note.chars().count() > NOTE_MAX {
        return Err("note is too long");
    }
    if note.chars().any(char::is_control) {
        return Err("note has control characters");
    }
    if !EMOJI_CHOICES.contains(&emoji) {
        return Err("bad emoji");
    }
    Ok(InterruptionWrite {
        from_date: from_date.to_string(),
        to_date: to_date.map(|date| date.to_string()),
        note: note.to_string(),
        emoji: emoji.to_string(),
    })
}

fn parse_date(raw: &str) -> Option<Date> {
    let date: Date = raw.parse().ok()?;
    // Reject non-canonical forms (`2026-8-2`) so stored keys match heatmap cells.
    (date.to_string() == raw).then_some(date)
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

fn declared_body_length(headers: &HeaderMap) -> std::result::Result<Option<usize>, ()> {
    let mut values = headers.get_all(header::CONTENT_LENGTH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .map(Some)
        .ok_or(())
}

fn see_other(location: &str) -> Response {
    let mut response = plain(StatusCode::SEE_OTHER, "see other");
    let location = HeaderValue::from_str(location).expect("interruption redirect is a valid path");
    response.headers_mut().insert(header::LOCATION, location);
    response
}

fn plain(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::from(message.to_string()))
        .expect("interruption response uses static headers")
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// One interruption covering an Eastern calendar day (newest-first order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DayMark<'a> {
    pub(super) emoji: &'a str,
    pub(super) note: &'a str,
}

/// Marks covering an Eastern calendar day, in snapshot interruption order.
/// Open rows cover through `today` (inclusive); closed rows use their end date.
pub(super) fn marks_covering<'a>(
    rows: &'a [Interruption],
    date: &str,
    today: &str,
) -> Vec<DayMark<'a>> {
    rows.iter()
        .filter(|row| {
            if row.from_date.as_str() > date {
                return false;
            }
            match row.to_date.as_deref() {
                Some(to) => date <= to,
                None => date <= today,
            }
        })
        .map(|row| DayMark {
            emoji: row.emoji.as_str(),
            note: row.note.as_str(),
        })
        .collect()
}

/// Convenience for live pages: coverage upper bound is today's Eastern date.
pub(super) fn marks_covering_today<'a>(rows: &'a [Interruption], date: &str) -> Vec<DayMark<'a>> {
    let today = eastern::eastern_date(Timestamp::now()).to_string();
    marks_covering(rows, date, &today)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_inclusive_eastern_ranges_and_open_ends() {
        let write = validate_interruption("2026-08-02", "2026-08-09", "cold", "🤒").unwrap();
        assert_eq!(write.from_date, "2026-08-02");
        assert_eq!(write.to_date.as_deref(), Some("2026-08-09"));
        assert_eq!(write.note, "cold");
        assert_eq!(write.emoji, "🤒");

        let open = validate_interruption("2026-08-02", "", "cold", "🤒").unwrap();
        assert_eq!(open.to_date, None);
        let open_ws = validate_interruption("2026-08-02", "   ", "cold", "🤒").unwrap();
        assert_eq!(open_ws.to_date, None);
    }

    #[test]
    fn rejects_inverted_or_overlong_ranges_and_bad_notes() {
        assert_eq!(
            validate_interruption("2026-08-09", "2026-08-02", "cold", "🤒").unwrap_err(),
            "to must be on or after from"
        );
        assert_eq!(
            validate_interruption("2026-01-01", "2027-01-02", "cold", "🤒").unwrap_err(),
            "range is too long"
        );
        assert_eq!(
            validate_interruption("2026-08-02", "2026-08-09", "   ", "🤒").unwrap_err(),
            "note is required"
        );
        assert_eq!(
            validate_interruption("2026-8-2", "2026-08-09", "cold", "🤒").unwrap_err(),
            "bad from date"
        );
        assert_eq!(
            validate_interruption("2026-08-02", "2026-08-09", "cold", "nope").unwrap_err(),
            "bad emoji"
        );
    }

    #[test]
    fn form_parser_allows_blank_to_and_requires_the_rest() {
        let write =
            parse_interruption_form(b"from=2026-08-02&to=2026-08-09&note=cold&emoji=%F0%9F%A4%92")
                .unwrap();
        assert_eq!(write.note, "cold");
        assert_eq!(write.emoji, "🤒");
        let open =
            parse_interruption_form(b"from=2026-08-02&to=&note=cold&emoji=%F0%9F%A4%92").unwrap();
        assert_eq!(open.to_date, None);
        // Browsers omit empty optional fields; missing `to` is open.
        let omitted =
            parse_interruption_form(b"from=2026-08-02&note=cold&emoji=%F0%9F%A4%92").unwrap();
        assert_eq!(omitted.to_date, None);
        assert!(parse_interruption_form(b"from=2026-08-02&to=2026-08-09&note=cold").is_err());
        assert!(
            parse_interruption_form(
                b"from=2026-08-02&to=2026-08-09&note=cold&emoji=%F0%9F%A4%92&extra=1"
            )
            .is_err()
        );
    }

    #[test]
    fn range_labels_collapse_same_month_and_mark_open_rows() {
        assert_eq!(
            format_range("2026-08-02", Some("2026-08-09")),
            "Aug 2–9, 2026"
        );
        assert_eq!(
            format_range("2026-08-02", Some("2026-08-02")),
            "Aug 2, 2026"
        );
        assert_eq!(
            format_range("2026-07-28", Some("2026-08-02")),
            "Jul 28–Aug 2, 2026"
        );
        assert_eq!(format_range("2026-08-02", None), "Aug 2, 2026 →");
    }

    #[test]
    fn coverage_is_inclusive_and_open_rows_stop_at_today() {
        let rows = [
            Interruption {
                id: "a".into(),
                from_date: "2026-08-02".into(),
                to_date: Some("2026-08-09".into()),
                note: "cold".into(),
                emoji: "🤒".into(),
                updated_at: 0,
            },
            Interruption {
                id: "b".into(),
                from_date: "2026-08-05".into(),
                to_date: None,
                note: "ongoing".into(),
                emoji: "😴".into(),
                updated_at: 0,
            },
        ];
        assert_eq!(
            marks_covering(&rows, "2026-08-02", "2026-08-11"),
            vec![DayMark {
                emoji: "🤒",
                note: "cold"
            }]
        );
        assert_eq!(
            marks_covering(&rows, "2026-08-09", "2026-08-11"),
            vec![
                DayMark {
                    emoji: "🤒",
                    note: "cold"
                },
                DayMark {
                    emoji: "😴",
                    note: "ongoing"
                }
            ]
        );
        assert!(marks_covering(&rows, "2026-08-01", "2026-08-11").is_empty());
        assert_eq!(
            marks_covering(&rows, "2026-08-10", "2026-08-11"),
            vec![DayMark {
                emoji: "😴",
                note: "ongoing"
            }]
        );
        // Open coverage does not extend past today.
        assert!(marks_covering(&rows, "2026-08-12", "2026-08-11").is_empty());
    }

    #[test]
    fn merge_puts_same_day_interruptions_after_primary_activities() {
        let lift = |id: &str, local: &str, start_time: i64| {
            fitness::LogActivity::Lift(
                crate::app::interests::lifting::archive::snapshot::FilteredWorkout {
                    workout: fitness::Workout {
                        id: id.into(),
                        path: local.into(),
                        title: "Lift".into(),
                        raw_title: "Lift".into(),
                        started_at_local: local.into(),
                        ended_at_local: local.into(),
                        eastern_offset_minutes: -240,
                        end_eastern_offset_minutes: -240,
                        duration_seconds: 3600,
                        duration_suspicious: false,
                        notes: None,
                        description: None,
                        sets: Vec::new(),
                    },
                    date: local[..10].into(),
                    start_time,
                },
            )
        };
        let activities = [
            lift("w1", "2026-08-09 10:00:00", 1),
            lift("w0", "2026-08-11 10:00:00", 2),
        ];
        let rows = [Interruption {
            id: "i".into(),
            from_date: "2026-08-02".into(),
            to_date: Some("2026-08-09".into()),
            note: "cold".into(),
            emoji: "🤒".into(),
            updated_at: 0,
        }];
        let merged = merge_log_items(&activities, &rows);
        assert!(matches!(
            merged[0],
            LogItem::Activity(fitness::LogActivity::Lift(w)) if w.workout.id == "w0"
        ));
        assert!(matches!(
            merged[1],
            LogItem::Activity(fitness::LogActivity::Lift(w)) if w.workout.id == "w1"
        ));
        assert!(matches!(merged[2], LogItem::Interruption(r) if r.id == "i"));
    }

    #[test]
    fn merge_preserves_exact_order_for_same_date_activities() {
        let workout = |id: &str| {
            fitness::LogActivity::Lift(
                crate::app::interests::lifting::archive::snapshot::FilteredWorkout {
                    workout: fitness::Workout {
                        id: id.into(),
                        path: id.into(),
                        title: "Lift".into(),
                        raw_title: "Lift".into(),
                        started_at_local: "2026-08-09 10:00:00".into(),
                        ended_at_local: "2026-08-09 11:00:00".into(),
                        eastern_offset_minutes: -240,
                        end_eastern_offset_minutes: -240,
                        duration_seconds: 3600,
                        duration_suspicious: false,
                        notes: None,
                        description: None,
                        sets: Vec::new(),
                    },
                    date: "2026-08-09".into(),
                    start_time: 0,
                },
            )
        };
        let activities = [workout("newer"), workout("older")];
        let rows = [Interruption {
            id: "i".into(),
            from_date: "2026-08-01".into(),
            to_date: Some("2026-08-09".into()),
            note: "cold".into(),
            emoji: "🤒".into(),
            updated_at: 0,
        }];
        let merged = merge_log_items(&activities, &rows);
        assert!(matches!(
            merged[0],
            LogItem::Activity(fitness::LogActivity::Lift(w)) if w.workout.id == "newer"
        ));
        assert!(matches!(
            merged[1],
            LogItem::Activity(fitness::LogActivity::Lift(w)) if w.workout.id == "older"
        ));
        assert!(matches!(merged[2], LogItem::Interruption(_)));
    }

    #[test]
    fn open_rows_filters_to_missing_end_dates() {
        let rows = [
            Interruption {
                id: "a".into(),
                from_date: "2026-08-02".into(),
                to_date: Some("2026-08-09".into()),
                note: "cold".into(),
                emoji: "🤒".into(),
                updated_at: 0,
            },
            Interruption {
                id: "b".into(),
                from_date: "2026-08-10".into(),
                to_date: None,
                note: "ongoing".into(),
                emoji: "😴".into(),
                updated_at: 0,
            },
        ];
        assert_eq!(
            open_rows(&rows)
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    #[test]
    fn interruption_ids_are_32_lowercase_hex() {
        assert!(is_interruption_id(&"a".repeat(32)));
        assert!(!is_interruption_id("nope"));
        assert!(!is_interruption_id(&"A".repeat(32)));
    }

    #[test]
    fn write_auth_matches_upload_gate() {
        use crate::content::access::ADMIN_EMAIL;

        assert_eq!(authorize_write(None, true), WriteAuth::Login);
        assert_eq!(
            authorize_write(Some("stranger@example.com"), true),
            WriteAuth::NotFound
        );
        assert_eq!(
            authorize_write(Some(ADMIN_EMAIL), false),
            WriteAuth::Forbidden
        );
        assert_eq!(authorize_write(Some(ADMIN_EMAIL), true), WriteAuth::Allowed);
    }
}
