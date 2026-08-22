//! Garmin-backed running support for the unified `/fitness` log and its
//! Android PWA share target. Running is never a synthetic lifting set: it cannot
//! affect lifting volume, records, muscle load, or Podrick announcements.

mod db;
mod garmin;

use std::time::{SystemTime, UNIX_EPOCH};

use benjisponge::data::{Data, running_models::RunningActivity};
use diary_core::eastern::{self, EasternInstant};
use sha2::{Digest, Sha256};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, IntoResponse, Response, StatusCode, error::not_found,
        error::redirect, error::redirect_permanent, header, headers, page, parse_query_params,
        path_param, route, to_bytes, uri,
    },
    view::{component, view},
};

use crate::{
    app::interests::lifting::archive::manual,
    app::{login::viewer, not_found::not_found_page},
    components::{back_link, ext_link, modal, page_head, shell},
    content::access::is_admin,
    util::{is_same_origin, urlencode},
};

const FITNESS_PATH: &str = "/fitness";
const LOG_PATH: &str = "/fitness/log";
const RUN_PATH: &str = "/fitness/run";
const SHARE_PATH: &str = "/fitness/share";
const IMPORT_PATH: &str = "/fitness/run/import";
const MANUAL_IMPORT_PATH: &str = "/fitness/run/manual";
const NO_STORE: &str = "no-store";
const IMPORT_BODY_LIMIT_BYTES: usize = 1_024;
const MANUAL_BODY_LIMIT_BYTES: usize = 512;
// Form encoding may expand every decoded byte to `%HH`; keep the decoded
// Lyfta bound authoritative without letting native-share ingress go unbounded.
const SHARE_BODY_LIMIT_BYTES: usize = manual::LYFTA_TEXT_LIMIT * 3 + 1_024;
const MAX_SHARED_FIELD_BYTES: usize = manual::LYFTA_TEXT_LIMIT;
const MILE_MILLIMETERS: u128 = 1_609_344;
const MAX_RUN_MILLIMETERS: u128 = 1_000_000_000;
const MAX_RUN_MILLISECONDS: u64 = 604_800_000;
const MANUAL_SOURCE: &str = "manual";
pub(crate) const MANUAL_RUN_DIALOG_ID: &str = "fitness-run-dialog";
pub(crate) const PWA_JS: Asset = asset!("./pwa.js");

pub(crate) struct RunLog {
    pub activities: Vec<RunningActivity>,
    pub live: bool,
}

pub(crate) async fn load(data: &Data) -> RunLog {
    let database = match data.db().await {
        Ok(database) => database,
        Err(error) => {
            log_failure("list connection", error);
            return RunLog {
                activities: Vec::new(),
                live: false,
            };
        }
    };
    match db::list(&database).await {
        Ok(activities) => RunLog {
            activities,
            live: true,
        },
        Err(error) => {
            log_failure("list query", error);
            RunLog {
                activities: Vec::new(),
                live: false,
            }
        }
    }
}

pub(crate) fn public_url(activity: &RunningActivity) -> String {
    let instant = EasternInstant {
        local: activity.started_at_local.clone(),
        offset_minutes: activity.eastern_offset_minutes as i32,
    };
    format!(
        "{RUN_PATH}/{}/{}",
        eastern::public_path(&instant),
        activity.id
    )
}

pub(crate) fn activity_date(activity: &RunningActivity) -> &str {
    activity.started_at_local.get(..10).unwrap_or("")
}

pub(crate) fn start_time_seconds(activity: &RunningActivity) -> i64 {
    eastern::utc_timestamp(&activity.started_at_utc)
        .map(|timestamp| timestamp.as_second())
        .unwrap_or(0)
}

pub(crate) fn distance_label(activity: &RunningActivity) -> String {
    let distance = u128::try_from(activity.distance_millimeters).unwrap_or(0);
    let hundredths = (distance * 100 + MILE_MILLIMETERS / 2) / MILE_MILLIMETERS;
    format!("{}.{:02} mi", hundredths / 100, hundredths % 100)
}

pub(crate) fn duration_label(activity: &RunningActivity) -> String {
    clock_label(activity.duration_milliseconds)
}

pub(crate) fn pace_label(activity: &RunningActivity) -> String {
    let duration = u128::try_from(activity.duration_milliseconds).unwrap_or(0);
    let distance = u128::try_from(activity.distance_millimeters).unwrap_or(0);
    if duration == 0 || distance == 0 {
        return "—".to_string();
    }
    let denominator = distance * 1_000;
    let seconds = (duration * MILE_MILLIMETERS + denominator / 2) / denominator;
    format!("{}:{:02} /mi", seconds / 60, seconds % 60)
}

pub(crate) fn feed_description(activity: &RunningActivity) -> String {
    let mut description = format!(
        "{} in {} at {}",
        distance_label(activity),
        duration_label(activity),
        pace_label(activity)
    );
    if let Some(ascent) = ascent_label(activity) {
        description.push_str(&format!(" · {ascent} ascent"));
    }
    description.push('.');
    description
}

fn clock_label(milliseconds: i64) -> String {
    let seconds = u64::try_from(milliseconds.saturating_add(500) / 1_000).unwrap_or(0);
    let hours = seconds / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn ascent_label(activity: &RunningActivity) -> Option<String> {
    let millimeters = u128::try_from(activity.ascent_millimeters?).ok()?;
    let feet = (millimeters * 10 + 1_524) / 3_048;
    Some(format!("{feet} ft"))
}

fn activity_type_label(activity_type: &str) -> String {
    let words = activity_type.replace(['_', '-'], " ");
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
        None => "Running".to_string(),
    }
}

struct Timing {
    date: String,
    time: String,
    datetime: String,
}

fn timing(activity: &RunningActivity) -> Timing {
    let datetime = machine_datetime(
        &activity.started_at_local,
        activity.eastern_offset_minutes as i32,
    );
    let parsed = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", &activity.started_at_local);
    match parsed {
        Ok(local) => Timing {
            date: local.strftime("%b %-d, %Y").to_string(),
            time: local.strftime("%-I:%M %p").to_string(),
            datetime,
        },
        Err(_) => Timing {
            date: activity.started_at_local.clone(),
            time: "time unavailable".to_string(),
            datetime,
        },
    }
}

fn machine_datetime(local: &str, offset_minutes: i32) -> String {
    let offset = offset_minutes.unsigned_abs();
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    format!(
        "{}{sign}{:02}:{:02}",
        local.replace(' ', "T"),
        offset / 60,
        offset % 60
    )
}

#[component]
pub(crate) async fn activity_card(
    activity: &RunningActivity,
    #[default(false)] detail: bool,
) -> Result {
    let href = public_url(activity);
    let distance = distance_label(activity);
    let duration = duration_label(activity);
    let pace = pace_label(activity);
    let ascent = ascent_label(activity);
    let moving = activity.moving_duration_milliseconds.map(clock_label);
    let timing = timing(activity);
    view! {
        <article
            class="rail-row border-t border-hairline py-5"
            data-rail-item=""
        >
            <div class="rail-stamp font-meta text-[0.7rem] text-muted">
                <time datetime=(timing.datetime.as_str())>(activity_date(activity))</time>
            </div>
            <div class="min-w-0">
                <div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
                    if detail {
                        <h2 class="font-display text-xl font-semibold">
                            (activity.title.as_str())
                        </h2>
                    } else {
                        <h3 class="font-display text-xl font-semibold">
                            <a class="oxlink" href=(href.as_str()) data-rail-enter="">
                                (activity.title.as_str())
                            </a>
                        </h3>
                    }
                    <p class="font-meta text-xs text-muted">
                        (timing.date.as_str()) " · " (timing.time.as_str())
                    </p>
                </div>
                // A finish-tape line: source order is the actual running
                // relationship—distance, then elapsed time, then derived pace.
                <p class="mt-3 flex flex-wrap items-baseline gap-x-3 font-meta tabular-nums">
                    <strong class="text-lg font-medium text-ink">(distance.as_str())</strong>
                    <span aria-hidden="true" class="text-hairline">"→"</span>
                    <span class="text-ink2">(duration.as_str())</span>
                    <span aria-hidden="true" class="text-hairline">"→"</span>
                    <span class="text-patina">(pace.as_str())</span>
                </p>
                <p class="mt-2 font-meta text-xs text-muted">
                    (activity_type_label(&activity.activity_type))
                    if let Some(ascent) = &ascent {
                        " · " (ascent.as_str()) " ascent"
                    }
                    if let Some(moving) = &moving {
                        " · " (moving.as_str()) " moving"
                    }
                </p>
            </div>
        </article>
    }
}

/// The owner-facing distance + elapsed-time form. Keeping the idempotency
/// token in the server-rendered form means a browser retry reuses it, while a
/// fresh dialog render creates a distinct run identity.
#[component]
pub(crate) async fn manual_run_form() -> Result {
    let submission_token = new_submission_token();
    view! {
        <form method="post" action=(MANUAL_IMPORT_PATH) class="mt-5 space-y-5">
            <input
                type="hidden"
                name="submission_token"
                value=(submission_token.as_str())
            >
            <label for="manual-run-distance" class="block space-y-2">
                <span class="font-meta text-sm text-ink2">"Distance (miles)"</span>
                <input
                    id="manual-run-distance"
                    name="distance"
                    type="text"
                    inputmode="decimal"
                    required=""
                    autofocus=""
                    autocomplete="off"
                    pattern="[0-9]+([.][0-9]{1,3})?"
                    placeholder="3.1"
                    class="block w-full rounded-sm border border-hairline bg-page px-3 py-2 \
                           font-meta tabular-nums text-ink focus-visible:outline-solid \
                           focus-visible:outline-2 focus-visible:outline-oxide \
                           focus-visible:outline-offset-2"
                >
            </label>
            <fieldset class="space-y-2">
                <legend class="font-meta text-sm text-ink2">"Elapsed time"</legend>
                <div class="grid grid-cols-2 gap-3">
                    <label for="manual-run-minutes" class="block space-y-1">
                        <span class="font-meta text-xs text-muted">"minutes"</span>
                        <input
                            id="manual-run-minutes"
                            name="minutes"
                            type="number"
                            inputmode="numeric"
                            min="0"
                            max="10080"
                            step="1"
                            required=""
                            autocomplete="off"
                            placeholder="28"
                            class="block w-full rounded-sm border border-hairline bg-page px-3 py-2 \
                                   font-meta tabular-nums text-ink focus-visible:outline-solid \
                                   focus-visible:outline-2 focus-visible:outline-oxide \
                                   focus-visible:outline-offset-2"
                        >
                    </label>
                    <label for="manual-run-seconds" class="block space-y-1">
                        <span class="font-meta text-xs text-muted">"seconds"</span>
                        <input
                            id="manual-run-seconds"
                            name="seconds"
                            type="number"
                            inputmode="numeric"
                            min="0"
                            max="59"
                            step="1"
                            required=""
                            autocomplete="off"
                            placeholder="30"
                            class="block w-full rounded-sm border border-hairline bg-page px-3 py-2 \
                                   font-meta tabular-nums text-ink focus-visible:outline-solid \
                                   focus-visible:outline-2 focus-visible:outline-oxide \
                                   focus-visible:outline-offset-2"
                        >
                    </label>
                </div>
            </fieldset>
            <div class="flex flex-wrap items-center gap-4">
                <button
                    type="submit"
                    class="cursor-pointer rounded-sm border border-oxide bg-oxide px-5 py-2.5 \
                           font-meta text-sm text-card hover:bg-oxide-hot"
                >"Log run"</button>
                <a class="quiet-link font-meta text-sm" href=(FITNESS_PATH)>"Cancel"</a>
            </div>
        </form>
    }
}

/// Garmin remains a secondary way to add a run inside the same owner dialog.
/// The GET is review-only; the existing confirmation route owns the fetch and
/// write boundary.
#[component]
async fn garmin_import_form() -> Result {
    view! {
        <details class="group mt-6 border-t border-hairline pt-5">
            <summary
                class="flex min-h-11 cursor-pointer list-none items-center justify-between gap-4 \
                       font-meta text-sm text-patina hover:underline \
                       focus-visible:outline-solid focus-visible:outline-2 \
                       focus-visible:outline-patina focus-visible:outline-offset-2 \
                       [&::-webkit-details-marker]:hidden"
            >
                <span>"Import from Garmin Connect"</span>
                <span
                    class="text-base leading-none transition-transform group-open:rotate-45"
                    aria-hidden="true"
                >"+"</span>
            </summary>
            <div class="mt-3 max-w-prose">
                <p class="text-sm leading-relaxed text-ink2">
                    "On Android, Fitness must appear in Settings → Apps before Garmin can \
                     share to it. Open this page in Chrome and use “Install Fitness app” \
                     when it appears below—a browser-badged home-screen shortcut cannot \
                     receive shares. Then, for an Everyone-visible activity in Garmin, \
                     choose Share → Web Link → Fitness—not Activity Details, which shares \
                     an image. Or paste its URL here. You will review the summary before \
                     it is logged."
                </p>
                <div
                    data-fitness-install=""
                    hidden=""
                    class="mt-4 rounded-sm border border-patina/50 bg-card p-4"
                >
                    <p class="font-meta text-sm font-semibold text-ink">
                        "Install the Android app"
                    </p>
                    <p
                        id="fitness-install-copy"
                        class="mt-1 text-sm leading-relaxed text-ink2"
                    >
                        "This opens the browser's app-install prompt. After it finishes, \
                         confirm Fitness is listed in Android Settings → Apps."
                    </p>
                    <button
                        type="button"
                        data-fitness-install-button=""
                        aria-describedby="fitness-install-copy fitness-install-status"
                        class="mt-3 cursor-pointer rounded-sm border border-patina bg-patina \
                               px-4 py-2 font-meta text-sm text-card hover:bg-ink \
                               disabled:cursor-wait disabled:opacity-60 \
                               focus-visible:outline-solid focus-visible:outline-2 \
                               focus-visible:outline-patina focus-visible:outline-offset-2"
                    >
                        "Install Fitness app"
                    </button>
                    <p
                        id="fitness-install-status"
                        data-fitness-install-status=""
                        class="mt-2 font-meta text-xs leading-relaxed text-muted"
                        role="status"
                        aria-live="polite"
                        aria-atomic="true"
                    ></p>
                </div>
                <p class="mt-2 font-meta text-xs leading-relaxed text-muted">
                    "The import keeps summary statistics and the canonical Garmin activity \
                     link—not the map, route, account, device, or raw sensor data. Restore the \
                     activity's Garmin privacy after importing it."
                </p>
                <form action=(SHARE_PATH) method="get" class="mt-4 space-y-3">
                    <label for="garmin-run-url" class="block space-y-2">
                        <span class="font-meta text-sm text-ink2">"Garmin activity URL"</span>
                        <input
                            id="garmin-run-url"
                            type="url"
                            name="url"
                            required=""
                            autocomplete="url"
                            placeholder="https://connect.garmin.com/app/activity/…"
                            class="block w-full rounded-sm border border-hairline bg-page px-3 \
                                   py-2 font-meta text-sm text-ink placeholder:text-muted \
                                   focus-visible:outline-solid focus-visible:outline-2 \
                                   focus-visible:outline-patina focus-visible:outline-offset-2"
                        >
                    </label>
                    <button
                        type="submit"
                        class="cursor-pointer rounded-sm border border-patina px-4 py-2 \
                               font-meta text-sm text-patina hover:bg-patina hover:text-card"
                    >
                        "Review Garmin run →"
                    </button>
                </form>
            </div>
        </details>
    }
}

/// Dialog-only surface for the unified Fitness “log” launcher. The launcher
/// owns the single visible trigger and targets [`MANUAL_RUN_DIALOG_ID`].
#[component]
pub(crate) async fn manual_run_dialog() -> Result {
    let child = view! {
        <p class="font-meta text-[0.68rem] uppercase tracking-[0.04em] text-muted">
            "running"
        </p>
        <h2 id="fitness-run-dialog-heading" class="mt-1 font-display text-2xl font-semibold">
            "Log a run"
        </h2>
        <p class="mt-2 max-w-prose text-sm leading-relaxed text-ink2">
            "Enter the distance, minutes, and seconds. The run starts when you log it; pace is derived."
        </p>
        manual_run_form()
        garmin_import_form()
    }?;
    view! {
        modal(
            id: MANUAL_RUN_DIALOG_ID,
            label: "Run",
            labelledby: "fitness-run-dialog-heading",
            child: child
        )
    }
}

#[route(GET "/running")]
async fn legacy_running(cx: &Cx) -> Result {
    let target = with_raw_query(cx, LOG_PATH);
    Err(redirect_permanent(&target).into())
}

#[cfg(test)]
fn empty_activity() -> RunningActivity {
    RunningActivity {
        id: String::new(),
        source: String::new(),
        source_activity_id: String::new(),
        source_url: None,
        title: String::new(),
        activity_type: String::new(),
        started_at_utc: String::new(),
        started_at_local: String::new(),
        eastern_offset_minutes: -240,
        duration_milliseconds: 0,
        moving_duration_milliseconds: None,
        distance_millimeters: 0,
        ascent_millimeters: None,
        imported_at: 0,
    }
}

#[path_param]
struct RunPath(str);

#[path_param]
struct RunId(str);

#[page("/fitness/run/{run_path}/{run_id}")]
async fn run_detail(cx: &Cx) -> Result {
    let run_path = path_param::<RunPath>(cx);
    let run_id = path_param::<RunId>(cx);
    if eastern::parse_public_path(run_path).is_none() || !is_sha256_hex(run_id) {
        return Err(not_found().into());
    }
    let canonical = format!("{RUN_PATH}/{run_path}/{run_id}");
    if uri(cx).query().is_some() {
        return Err(redirect(&canonical).into());
    }
    let log = load(app_context::<Data>(cx)).await;
    if log.live
        && !log
            .activities
            .iter()
            .any(|activity| activity.id == run_id && public_url(activity) == canonical)
    {
        return Err(not_found().into());
    }
    let index = log
        .activities
        .iter()
        .position(|activity| activity.id == run_id && public_url(activity) == canonical);
    let activity = index.and_then(|index| log.activities.get(index));
    let newer = index
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| log.activities.get(index));
    let older = index.and_then(|index| log.activities.get(index + 1));
    let title = activity
        .map(|activity| format!("{} · Running", activity.title))
        .unwrap_or_else(|| "Running".to_string());

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: title.as_str(),
            active: "",
            runtime: false,
            fitness_pwa: true,
            if let Some(activity) = activity {
                page_head(stamp: "run", title: activity.title.as_str(), lede: "")
                <section class="mt-8" aria-label="Run summary">
                    activity_card(activity: activity, detail: true)
                </section>
                <div class="mt-8 border-l-2 border-hairline pl-4 text-sm leading-relaxed text-ink2">
                    <p>(source_note(activity))</p>
                    if let Some(source_url) = canonical_source_url(activity) {
                        <p class="mt-2 font-meta">
                            ext_link(
                                class: "oxlink",
                                href: source_url.as_str(),
                                label: "View in Garmin Connect →"
                            )
                        </p>
                    }
                </div>
                if newer.is_some() || older.is_some() {
                    <nav class="mt-8 grid grid-cols-2 gap-4 border-t border-hairline pt-4 font-meta text-sm" aria-label="Run navigation">
                        if let Some(newer) = newer {
                            <a class="quiet-link justify-self-start" href=(public_url(newer))>"← newer run"</a>
                        } else {
                            <span></span>
                        }
                        if let Some(older) = older {
                            <a class="quiet-link justify-self-end" href=(public_url(older))>"older run →"</a>
                        }
                    </nav>
                }
            } else {
                page_head(stamp: "run", title: "Run unavailable", lede: "")
                <p class="mt-8 max-w-prose text-ink2">
                    "The running log is unavailable right now. This run is safe; try again in a moment."
                </p>
            }
            back_link(href: LOG_PATH, label: "fitness log")
        )
    }
}

#[route(GET "/running/{run_path}/{run_id}")]
async fn legacy_run_detail(cx: &Cx) -> Result {
    let run_path = path_param::<RunPath>(cx);
    let run_id = path_param::<RunId>(cx);
    if eastern::parse_public_path(run_path).is_none() || !is_sha256_hex(run_id) {
        return Err(not_found().into());
    }
    let target = with_raw_query(cx, &format!("{RUN_PATH}/{run_path}/{run_id}"));
    Err(redirect_permanent(&target).into())
}

fn with_raw_query(cx: &Cx, target: &str) -> String {
    uri(cx)
        .query()
        .map_or_else(|| target.to_string(), |query| format!("{target}?{query}"))
}

#[derive(Default)]
struct ShareFields {
    title: Option<String>,
    text: Option<String>,
    url: Option<String>,
    garmin: Option<String>,
}

impl ShareFields {
    fn parse(cx: &Cx) -> Result<Self> {
        let pairs = parse_query_params::<Vec<(String, String)>>(cx)?;
        let mut fields = Self::default();
        for (key, value) in pairs {
            if value.len() > MAX_SHARED_FIELD_BYTES {
                return Ok(Self {
                    garmin: Some(String::new()),
                    ..Self::default()
                });
            }
            let slot = match key.as_str() {
                "title" => &mut fields.title,
                "text" => &mut fields.text,
                "url" => &mut fields.url,
                "garmin" => &mut fields.garmin,
                _ => {
                    return Ok(Self {
                        garmin: Some(String::new()),
                        ..Self::default()
                    });
                }
            };
            if slot.replace(value).is_some() {
                return Ok(Self {
                    garmin: Some(String::new()),
                    ..Self::default()
                });
            }
        }
        Ok(fields)
    }

    fn activity_id(&self) -> std::result::Result<String, garmin::GarminError> {
        if let Some(activity_id) = &self.garmin {
            if self.title.is_some() || self.text.is_some() || self.url.is_some() {
                return Err(garmin::GarminError::BadLink);
            }
            if (1..=20).contains(&activity_id.len())
                && activity_id.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Ok(activity_id.clone());
            }
            return Err(garmin::GarminError::BadLink);
        }
        garmin::shared_activity_id(&[
            self.title.as_deref().unwrap_or(""),
            self.text.as_deref().unwrap_or(""),
            self.url.as_deref().unwrap_or(""),
        ])
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NativeShare {
    Garmin(String),
    Lyfta(String),
    Ambiguous,
    Unknown,
}

/// Classify one Android share without changing state. Garmin contributes one
/// validated numeric id; Lyfta contributes the exact reconstructed share text
/// that passed the existing strict browser-upload parser. The combinations
/// cover Android moving a title or trailing URL into its dedicated manifest
/// field while never accepting a link-only Lyfta share as workout data.
fn classify_native_share(fields: &ShareFields) -> NativeShare {
    let garmin = fields.activity_id().ok();
    let mut candidates = Vec::<String>::new();
    let values = [
        fields.title.as_deref().filter(|value| !value.is_empty()),
        fields.text.as_deref().filter(|value| !value.is_empty()),
        fields.url.as_deref().filter(|value| !value.is_empty()),
    ];
    // Prefer the ordinary Android shape (the complete payload in `text`),
    // then try split-field reconstructions in semantic title/text/url order.
    for index in [1, 0, 2] {
        if let Some(value) = values[index] {
            candidates.push(value.to_string());
        }
    }
    for indexes in [&[0, 1][..], &[1, 2][..], &[0, 1, 2][..], &[0, 2][..]] {
        let parts: Vec<&str> = indexes.iter().filter_map(|index| values[*index]).collect();
        if parts.len() == indexes.len() {
            candidates.push(parts.join("\n"));
        }
    }
    candidates.retain(|candidate| candidate.len() <= manual::LYFTA_TEXT_LIMIT);
    let mut lyfta = Vec::new();
    for candidate in candidates {
        if lyfta.iter().any(|(raw, _)| raw == &candidate) {
            continue;
        }
        let Ok(parsed) = manual::parse_lyfta(&candidate) else {
            continue;
        };
        if !lyfta
            .iter()
            .any(|(_, existing): &(String, manual::ParsedWorkout)| existing == &parsed)
        {
            lyfta.push((candidate, parsed));
        }
    }

    match (garmin, lyfta.len()) {
        (Some(activity_id), 0) => NativeShare::Garmin(activity_id),
        (None, 1) => NativeShare::Lyfta(lyfta.pop().expect("one Lyfta match").0),
        (None, 0) => NativeShare::Unknown,
        _ => NativeShare::Ambiguous,
    }
}

/// Android's native share fields arrive in a POST body so Garmin URLs and
/// Lyfta workout text never enter browser history or proxy access logs. This
/// ingress is write-free: Garmin sheds everything but the validated numeric
/// id; Lyfta renders an escaped, deliberate review form.
#[route(POST "/fitness/share")]
async fn receive_shared_run(cx: &Cx, body: Body) -> Result<Response> {
    receive_shared_run_inner(cx, body).await
}

/// Compatibility for manifests or tabs rendered before the route migration.
#[route(POST "/running/share")]
async fn legacy_receive_shared_run(cx: &Cx, body: Body) -> Result<Response> {
    receive_shared_run_inner(cx, body).await
}

async fn receive_shared_run_inner(cx: &Cx, body: Body) -> Result<Response> {
    if !is_form_content_type(headers(cx)) {
        return Ok(text_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > SHARE_BODY_LIMIT_BYTES => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "share is too large",
            ));
        }
        Ok(_) => {}
        Err(()) => {
            return Ok(text_response(StatusCode::BAD_REQUEST, "bad Content-Length"));
        }
    }
    let bytes = match to_bytes(body, SHARE_BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "share is too large",
            ));
        }
    };
    let Some(fields) = parse_share_form(&bytes) else {
        return Ok(text_response(StatusCode::BAD_REQUEST, "bad share form"));
    };
    match classify_native_share(&fields) {
        NativeShare::Garmin(activity_id) => {
            Ok(see_other(&format!("{SHARE_PATH}?garmin={activity_id}")))
        }
        NativeShare::Lyfta(workout) => lyfta_review_response(cx, &workout).await,
        NativeShare::Ambiguous => {
            share_problem_response(
                cx,
                "Choose one activity",
                "That share contains more than one recognizable fitness activity. Share one Garmin run or one complete Lyfta workout at a time; nothing was logged.",
            )
            .await
        }
        NativeShare::Unknown => {
            share_problem_response(
                cx,
                "Share not recognized",
                "Fitness can receive a Garmin Connect activity URL or Lyfta's complete workout text. Lyfta sent only a link—or another app's share format was used—so nothing was logged. Open Fitness to paste the workout text instead.",
            )
            .await
        }
    }
}

async fn lyfta_review_response(cx: &Cx, workout: &str) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return share_problem_response(
            cx,
            "Sign in, then share again",
            "The Lyfta workout was not retained. Sign in to Fitness, return to Lyfta, and share it again so you can review the complete workout before publishing.",
        )
        .await;
    };
    if !is_admin(&current.email) {
        return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
    }
    let __cx = cx;
    let page = view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Review Lyfta workout",
            active: "",
            runtime: false,
            fitness_pwa: true,
            page_head(
                stamp: "share",
                title: "Publish this lift?",
                lede: "Review Lyfta's complete workout text before it joins the public fitness log."
            )
            <form method="post" action="/fitness/lift/import" class="mt-8 space-y-5">
                <label for="shared-lyfta-workout" class="block space-y-2">
                    <span class="font-meta text-sm text-ink2">"Lyfta workout text"</span>
                    <textarea
                        id="shared-lyfta-workout"
                        name="workout"
                        rows="16"
                        required=""
                        spellcheck="false"
                        autocomplete="off"
                        class="block w-full resize-y rounded-sm border border-hairline bg-page \
                             p-4 font-mono text-sm leading-relaxed text-ink \
                             focus-visible:outline-solid focus-visible:outline-2 \
                             focus-visible:outline-oxide focus-visible:outline-offset-2"
                    >(workout)</textarea>
                </label>
                <p class="max-w-prose font-meta text-xs leading-relaxed text-muted">
                    "Nothing has been logged yet. Publishing revalidates this text, requires your \
                     signed-in admin session and a same-origin form submission, and keeps the \
                     lifting archive create-only."
                </p>
                <div class="flex flex-wrap items-center gap-4">
                    <button
                        type="submit"
                        class="cursor-pointer rounded-sm border border-oxide bg-oxide px-5 py-2.5 \
                             font-meta text-sm text-card hover:bg-oxide-hot"
                    >
                        "Publish this lift"
                    </button>
                    <a class="quiet-link font-meta text-sm" href=(FITNESS_PATH)>"Cancel"</a>
                </div>
            </form>
            back_link(href: FITNESS_PATH, label: "fitness")
        )
    }?;
    page.into_response(cx)
}

async fn share_problem_response(cx: &Cx, title: &str, copy: &str) -> Result<Response> {
    let __cx = cx;
    let page = view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: title,
            active: "",
            runtime: false,
            fitness_pwa: true,
            page_head(stamp: "share", title: title, lede: "Nothing was logged.")
            <p class="mt-8 max-w-prose text-sm leading-relaxed text-ink2">(copy)</p>
            <p class="mt-6 font-meta text-sm">
                <a class="oxlink" href="/login?next=%2Ffitness">"Sign in or open Fitness →"</a>
            </p>
            back_link(href: FITNESS_PATH, label: "fitness")
        )
    }?;
    page.into_response(cx)
}

#[page("/fitness/share")]
async fn review_shared_run(cx: &Cx) -> Result {
    let fields = ShareFields::parse(cx)?;
    let activity_id = fields.activity_id();
    let Some(current) = viewer(cx) else {
        let next = activity_id
            .as_deref()
            .map(|activity_id| format!("{SHARE_PATH}?garmin={activity_id}"))
            .unwrap_or_else(|_| FITNESS_PATH.to_string());
        return Err(redirect(&format!("/login?next={}", urlencode(&next))).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: SHARE_PATH)
        };
    }

    let activity_id = match activity_id {
        Ok(activity_id) => activity_id,
        Err(error) => {
            return view! {
                ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
                shell(
                    title: "Review Garmin run",
                    active: "",
                    runtime: false,
                    fitness_pwa: true,
                    page_head(stamp: "share", title: "No run to review", lede: "")
                    <p class="mt-8 max-w-prose text-ink2">(error.message())</p>
                    <p class="mt-4 font-meta text-sm text-muted">
                        "Open the activity in Garmin Connect and share it again, or paste its link in Fitness's Run dialog."
                    </p>
                    back_link(href: FITNESS_PATH, label: "fitness")
                )
            };
        }
    };
    if fields.garmin.as_deref() != Some(activity_id.as_str()) {
        // The manual URL form and pre-POST share-target links can put
        // descriptive text in any of three query fields. Once the one safe
        // identifier is known, shed that payload from the address bar before
        // contacting Garmin.
        return Err(redirect(&format!("{SHARE_PATH}?garmin={activity_id}")).into());
    }
    let database = match app_context::<Data>(cx).db().await {
        Ok(database) => database,
        Err(error) => {
            log_failure("share connection", error);
            return view! { unavailable_review() };
        }
    };
    match db::by_source_activity_id(&database, &activity_id).await {
        Ok(Some(existing)) => {
            let href = public_url(&existing);
            return view! {
                ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
                shell(
                    title: "Run already logged",
                    active: "",
                    runtime: false,
                    fitness_pwa: true,
                    page_head(stamp: "share", title: "Already logged", lede: "This Garmin run is already on the tape.")
                    <section class="mt-8">activity_card(activity: &existing, detail: true)</section>
                    <p class="mt-6 font-meta text-sm">
                        <a class="oxlink" href=(href.as_str())>"Open logged run →"</a>
                    </p>
                    back_link(href: FITNESS_PATH, label: "fitness")
                )
            };
        }
        Ok(None) => {}
        Err(error) => {
            log_failure("share lookup", error);
            return view! { unavailable_review() };
        }
    }

    let preview = match garmin::fetch(&activity_id, unix_seconds()).await {
        Ok(preview) => preview,
        Err(error) => {
            return view! {
                ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
                shell(
                    title: "Review Garmin run",
                    active: "",
                    runtime: false,
                    fitness_pwa: true,
                    page_head(stamp: "share", title: "Run not ready", lede: "")
                    <p class="mt-8 max-w-prose text-ink2">(error.message())</p>
                    <p class="mt-4 font-meta text-sm text-muted">
                        "Nothing was logged. Garmin only shares activities whose privacy is set to Everyone."
                    </p>
                    back_link(href: FITNESS_PATH, label: "fitness")
                )
            };
        }
    };
    let summary_digest = garmin::summary_digest(&preview);

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Review Garmin run",
            active: "",
            runtime: false,
            fitness_pwa: true,
            page_head(stamp: "share", title: "Log this run?", lede: "Review Garmin's summary before it becomes public here.")
            <section class="mt-8">activity_card(activity: &preview, detail: true)</section>
            <div class="mt-6 border-l-2 border-patina pl-4">
                <p class="max-w-prose text-sm leading-relaxed text-ink2">
                    "Logging keeps this summary and the canonical Garmin activity link. It discards Garmin's map, route coordinates, account details, device identifiers, and raw response. After it logs, you can restore the activity's Garmin privacy."
                </p>
                <form method="post" action=(IMPORT_PATH) class="mt-4 flex flex-wrap items-center gap-4">
                    <input type="hidden" name="activity_id" value=(activity_id.as_str())>
                    <input type="hidden" name="summary_digest" value=(summary_digest.as_str())>
                    <button
                        type="submit"
                        class="cursor-pointer rounded-sm border border-oxide bg-oxide px-5 py-2.5 font-meta text-sm text-card hover:bg-oxide-hot"
                    >"Log this run"</button>
                    <a class="quiet-link font-meta text-sm" href=(FITNESS_PATH)>"Cancel"</a>
                </form>
            </div>
            back_link(href: FITNESS_PATH, label: "fitness")
        )
    }
}

#[route(GET "/running/share")]
async fn legacy_review_shared_run(cx: &Cx) -> Result {
    let target = with_raw_query(cx, SHARE_PATH);
    Err(redirect_permanent(&target).into())
}

#[component]
async fn unavailable_review() -> Result {
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            title: "Review Garmin run",
            active: "",
            runtime: false,
            fitness_pwa: true,
            page_head(stamp: "share", title: "Running log unavailable", lede: "")
            <p class="mt-8 max-w-prose text-ink2">
                "The database is unavailable, so nothing was logged. Try sharing the run again in a moment."
            </p>
            back_link(href: FITNESS_PATH, label: "fitness")
        )
    }
}

#[route(POST "/fitness/run/import")]
async fn import_shared_run(cx: &Cx, body: Body) -> Result<Response> {
    import_shared_run_inner(cx, body).await
}

#[route(POST "/running/import")]
async fn legacy_import_shared_run(cx: &Cx, body: Body) -> Result<Response> {
    import_shared_run_inner(cx, body).await
}

async fn import_shared_run_inner(cx: &Cx, body: Body) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return Ok(see_other("/login?next=%2Ffitness"));
    };
    if !is_admin(&current.email) {
        return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(text_response(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !is_form_content_type(headers(cx)) {
        return Ok(text_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > IMPORT_BODY_LIMIT_BYTES => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
        Ok(_) => {}
        Err(()) => {
            return Ok(text_response(StatusCode::BAD_REQUEST, "bad Content-Length"));
        }
    }
    let bytes = match to_bytes(body, IMPORT_BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
    };
    let form = match parse_import_form(&bytes) {
        Some(form) => form,
        None => return Ok(text_response(StatusCode::BAD_REQUEST, "bad form")),
    };
    let database = match app_context::<Data>(cx).db().await {
        Ok(database) => database,
        Err(error) => {
            log_failure("import connection", error);
            return Ok(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "The running log is unavailable right now.",
            ));
        }
    };
    match db::by_source_activity_id(&database, &form.activity_id).await {
        Ok(Some(existing)) => return Ok(see_other(&public_url(&existing))),
        Ok(None) => {}
        Err(error) => {
            log_failure("import lookup", error);
            return Ok(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "The running log is unavailable right now.",
            ));
        }
    }
    let incoming = match garmin::fetch(&form.activity_id, unix_seconds()).await {
        Ok(incoming) => incoming,
        Err(error) => {
            return Ok(text_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                error.message(),
            ));
        }
    };
    if garmin::summary_digest(&incoming) != form.summary_digest {
        return Ok(text_response(
            StatusCode::CONFLICT,
            "Garmin changed this summary after review. Return to the review page and check it again before logging.",
        ));
    }
    match db::create(&database, &incoming).await {
        Ok(db::CreateOutcome::Added | db::CreateOutcome::Duplicate) => {
            Ok(see_other(&public_url(&incoming)))
        }
        Ok(db::CreateOutcome::Conflict) => Ok(text_response(
            StatusCode::CONFLICT,
            "A different running summary already uses that Garmin activity id.",
        )),
        Err(error) => {
            log_failure("import write", error);
            Ok(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "The run could not be logged right now.",
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ManualWriteAuth {
    Allowed,
    Login,
    NotFound,
    Forbidden,
}

fn authorize_manual_write(viewer_email: Option<&str>, same_origin: bool) -> ManualWriteAuth {
    match viewer_email {
        None => ManualWriteAuth::Login,
        Some(email) if !is_admin(email) => ManualWriteAuth::NotFound,
        Some(_) if !same_origin => ManualWriteAuth::Forbidden,
        Some(_) => ManualWriteAuth::Allowed,
    }
}

/// Create-only manual logging: the browser supplies only measurements and an
/// opaque replay token. The server owns provenance, title, and the current
/// start instant; pace remains derived from the two stored integers.
#[route(POST "/fitness/run/manual")]
async fn import_manual_run(cx: &Cx, body: Body) -> Result<Response> {
    let viewer_email = viewer(cx).map(|current| current.email.clone());
    match authorize_manual_write(viewer_email.as_deref(), is_same_origin(headers(cx))) {
        ManualWriteAuth::Login => return Ok(see_other("/login?next=%2Ffitness")),
        ManualWriteAuth::NotFound => {
            return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
        }
        ManualWriteAuth::Forbidden => {
            return Ok(text_response(StatusCode::FORBIDDEN, "forbidden"));
        }
        ManualWriteAuth::Allowed => {}
    }
    if !is_form_content_type(headers(cx)) {
        return Ok(text_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > MANUAL_BODY_LIMIT_BYTES => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
        Ok(_) => {}
        Err(()) => {
            return Ok(text_response(StatusCode::BAD_REQUEST, "bad Content-Length"));
        }
    }
    let bytes = match to_bytes(body, MANUAL_BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            ));
        }
    };
    let form = match parse_manual_run_form(&bytes) {
        Some(form) => form,
        None => {
            return Ok(text_response(
                StatusCode::BAD_REQUEST,
                "Enter positive decimal miles, whole minutes, and 0–59 whole seconds.",
            ));
        }
    };
    let incoming = manual_activity(&form, unix_seconds());
    let database = match app_context::<Data>(cx).db().await {
        Ok(database) => database,
        Err(error) => {
            log_failure("manual connection", error);
            return Ok(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "The running log is unavailable right now.",
            ));
        }
    };
    match db::create(&database, &incoming).await {
        Ok(db::CreateOutcome::Added) => Ok(see_other(&public_url(&incoming))),
        Ok(db::CreateOutcome::Duplicate) => {
            // A delayed replay reconstructs a later server timestamp. Always
            // use the first write's row so its immutable permalink wins.
            match db::by_source_identity(&database, MANUAL_SOURCE, &form.submission_token).await {
                Ok(Some(existing)) => Ok(see_other(&public_url(&existing))),
                Ok(None) => Ok(text_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The logged run could not be opened right now.",
                )),
                Err(error) => {
                    log_failure("manual replay lookup", error);
                    Ok(text_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "The logged run could not be opened right now.",
                    ))
                }
            }
        }
        Ok(db::CreateOutcome::Conflict) => Ok(text_response(
            StatusCode::CONFLICT,
            "That run submission was already used with different measurements.",
        )),
        Err(error) => {
            log_failure("manual write", error);
            Ok(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "The run could not be logged right now.",
            ))
        }
    }
}

#[route(GET "/interests/running")]
async fn legacy_interest_running() -> Result {
    Err(redirect_permanent(FITNESS_PATH).into())
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, PartialEq, Eq)]
struct ManualRunForm {
    submission_token: String,
    distance_millimeters: i64,
    duration_milliseconds: i64,
}

fn parse_manual_run_form(body: &[u8]) -> Option<ManualRunForm> {
    let pairs = parse_form_pairs(body)?;
    if pairs.len() != 4 {
        return None;
    }
    let mut submission_token = None;
    let mut distance = None;
    let mut minutes = None;
    let mut seconds = None;
    for (key, value) in pairs {
        let slot = match key.as_str() {
            "submission_token" => &mut submission_token,
            "distance" => &mut distance,
            "minutes" => &mut minutes,
            "seconds" => &mut seconds,
            _ => return None,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    let submission_token = submission_token?;
    is_sha256_hex(&submission_token).then_some(())?;
    Some(ManualRunForm {
        submission_token,
        distance_millimeters: parse_miles(distance?.as_str())?,
        duration_milliseconds: parse_elapsed_duration(minutes?.as_str(), seconds?.as_str())?,
    })
}

/// Parse up to three fractional mile digits without floating-point drift,
/// then round once to the nearest millimeter. A mile is exactly 1,609,344 mm.
fn parse_miles(value: &str) -> Option<i64> {
    let mut parts = value.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<u128>().ok()?;
    let fractional_millimiles = match fraction {
        None => 0,
        Some(fraction)
            if (1..=3).contains(&fraction.len())
                && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            let digits = fraction.parse::<u128>().ok()?;
            digits.checked_mul(10u128.pow(3 - fraction.len() as u32))?
        }
        Some(_) => return None,
    };
    let millimiles = whole
        .checked_mul(1_000)?
        .checked_add(fractional_millimiles)?;
    let millimeters = millimiles.checked_mul(MILE_MILLIMETERS)?.checked_add(500)? / 1_000;
    (1..=MAX_RUN_MILLIMETERS)
        .contains(&millimeters)
        .then(|| i64::try_from(millimeters).ok())?
}

fn parse_elapsed_duration(minutes: &str, seconds: &str) -> Option<i64> {
    let minutes = parse_unsigned_decimal(minutes)?;
    let seconds = parse_unsigned_decimal(seconds)?;
    if seconds >= 60 {
        return None;
    }
    let milliseconds = minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1_000)?;
    (1..=MAX_RUN_MILLISECONDS)
        .contains(&milliseconds)
        .then(|| i64::try_from(milliseconds).ok())?
}

fn parse_unsigned_decimal(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())?
}

fn manual_activity(form: &ManualRunForm, now: i64) -> RunningActivity {
    let timestamp = jiff::Timestamp::from_second(now)
        .expect("a current Unix timestamp is representable by jiff");
    let started_at_utc = timestamp
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();
    let eastern = eastern::eastern_instant(&started_at_utc, 0)
        .expect("a current UTC timestamp projects to Eastern");
    RunningActivity {
        id: manual_storage_id(&form.submission_token),
        source: MANUAL_SOURCE.to_string(),
        source_activity_id: form.submission_token.clone(),
        source_url: None,
        title: "Run".to_string(),
        activity_type: "running".to_string(),
        started_at_utc,
        started_at_local: eastern.local,
        eastern_offset_minutes: i64::from(eastern.offset_minutes),
        duration_milliseconds: form.duration_milliseconds,
        moving_duration_milliseconds: None,
        distance_millimeters: form.distance_millimeters,
        ascent_millimeters: None,
        imported_at: now,
    }
}

fn manual_storage_id(submission_token: &str) -> String {
    Sha256::digest(format!("{MANUAL_SOURCE}\n{submission_token}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn new_submission_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn source_note(activity: &RunningActivity) -> &'static str {
    if activity.source == MANUAL_SOURCE {
        "Logged by hand from distance and elapsed time."
    } else {
        "Imported from Garmin Connect. This page stores the summary and canonical activity link, not the map, GPS route, or raw sensor data."
    }
}

/// Return only the canonical Garmin activity URL for a Garmin-backed row.
/// Newly imported rows persist it; legacy rows derive the same safe URL from
/// their digits-only source identity so they gain the focused-detail link
/// without rewriting create-only history.
fn canonical_source_url(activity: &RunningActivity) -> Option<String> {
    if activity.source != "garmin-connect"
        || !(1..=20).contains(&activity.source_activity_id.len())
        || !activity
            .source_activity_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let canonical = format!(
        "https://connect.garmin.com/app/activity/{}",
        activity.source_activity_id
    );
    match activity.source_url.as_deref() {
        Some(stored) if stored != canonical => None,
        _ => Some(canonical),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ImportForm {
    activity_id: String,
    summary_digest: String,
}

fn parse_import_form(body: &[u8]) -> Option<ImportForm> {
    let pairs = parse_form_pairs(body)?;
    if pairs.len() != 2 {
        return None;
    }
    let mut activity_id = None;
    let mut summary_digest = None;
    for (key, value) in pairs {
        let slot = match key.as_str() {
            "activity_id" => &mut activity_id,
            "summary_digest" => &mut summary_digest,
            _ => return None,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    let activity_id = activity_id?;
    let summary_digest = summary_digest?;
    ((1..=20).contains(&activity_id.len())
        && activity_id.bytes().all(|byte| byte.is_ascii_digit())
        && is_sha256_hex(&summary_digest))
    .then_some(ImportForm {
        activity_id,
        summary_digest,
    })
}

fn parse_share_form(body: &[u8]) -> Option<ShareFields> {
    let pairs = parse_form_pairs(body)?;
    let mut fields = ShareFields::default();
    for (key, value) in pairs {
        if value.len() > MAX_SHARED_FIELD_BYTES {
            return None;
        }
        let slot = match key.as_str() {
            "title" => &mut fields.title,
            "text" => &mut fields.text,
            "url" => &mut fields.url,
            _ => return None,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    Some(fields)
}

fn parse_form_pairs(body: &[u8]) -> Option<Vec<(String, String)>> {
    if body.is_empty() {
        return None;
    }
    body.split(|byte| *byte == b'&')
        .map(|pair| {
            if pair.is_empty() {
                return None;
            }
            let separator = pair.iter().position(|byte| *byte == b'=')?;
            let key = decode_form_component(&pair[..separator])?;
            let value = decode_form_component(&pair[separator + 1..])?;
            (!key.is_empty()).then_some((key, value))
        })
        .collect()
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn decode_form_component(encoded: &[u8]) -> Option<String> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        match encoded[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = encoded.get(index + 1).and_then(|byte| hex_value(*byte))?;
                let low = encoded.get(index + 2).and_then(|byte| hex_value(*byte))?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
    let mut response = text_response(StatusCode::SEE_OTHER, "see other");
    let location = HeaderValue::from_str(location).expect("running redirect is a valid path");
    response.headers_mut().insert(header::LOCATION, location);
    response
}

fn text_response(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::from(message.to_string()))
        .expect("running response uses static headers")
}

fn log_failure(stage: &str, error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "running operation failed",
            "stage": stage,
            "error": error.to_string(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const PWA_SOURCE: &str = include_str!("pwa.js");

    const LYFTA_SHARE: &str = "Morning lift
Friday 24. July 2026 at 10:38 AM

1min | 100lbs | 1 Exercises | 1 Sets

Squat
Set 1: 100lbs x 5 reps

Check out the workout and join me on Lyfta.
https://lyfta.app/wk/example";

    fn run(distance: i64, duration: i64) -> RunningActivity {
        RunningActivity {
            distance_millimeters: distance,
            duration_milliseconds: duration,
            ..empty_activity()
        }
    }

    #[test]
    fn running_metrics_round_like_garmin() {
        let run = run(6_480_750, 2_613_929);
        assert_eq!(distance_label(&run), "4.03 mi");
        assert_eq!(duration_label(&run), "43:34");
        assert_eq!(pace_label(&run), "10:49 /mi");
    }

    #[test]
    fn pwa_install_control_uses_the_native_install_event() {
        for needle in [
            "beforeinstallprompt",
            "event.preventDefault()",
            "prompt.prompt()",
            "prompt.userChoice",
            "appinstalled",
            "Settings → Apps",
        ] {
            assert!(PWA_SOURCE.contains(needle), "pwa.js lost {needle:?}");
        }
    }

    #[test]
    fn public_paths_use_the_eastern_projection() {
        let mut run = run(5_000_000, 1_800_000);
        run.started_at_local = "2026-08-21 17:22:15".to_string();
        run.eastern_offset_minutes = -240;
        run.id = "a".repeat(64);
        assert_eq!(
            public_url(&run),
            format!("/fitness/run/2026-08-21T17-22-15-04-00/{}", "a".repeat(64))
        );

        let mut simultaneous = run.clone();
        simultaneous.id = "b".repeat(64);
        assert_ne!(public_url(&run), public_url(&simultaneous));
    }

    #[test]
    fn manual_miles_are_strict_fixed_point_and_round_once_to_millimeters() {
        assert_eq!(parse_miles("1"), Some(1_609_344));
        assert_eq!(parse_miles("3.1"), Some(4_988_966));
        assert_eq!(parse_miles("0.001"), Some(1_609));
        assert_eq!(parse_miles("621.371"), Some(999_999_691));
        assert_eq!(parse_miles("621.372"), None);

        for malformed in [
            "",
            ".5",
            "1.",
            "1.0000",
            "-1",
            "+1",
            "1e2",
            "1,5",
            " 1",
            "1 ",
            "1..0",
            "0",
            "0.000",
            "622",
            "99999999999999999999999999999999999999",
        ] {
            assert_eq!(parse_miles(malformed), None, "accepted {malformed:?}");
        }
    }

    #[test]
    fn manual_elapsed_time_uses_bounded_whole_minutes_and_seconds() {
        assert_eq!(parse_elapsed_duration("0", "30"), Some(30_000));
        assert_eq!(parse_elapsed_duration("28", "30"), Some(1_710_000));
        assert_eq!(parse_elapsed_duration("60", "0"), Some(3_600_000));
        assert_eq!(parse_elapsed_duration("10080", "0"), Some(604_800_000));

        for (minutes, seconds) in [
            ("0", "0"),
            ("1", "60"),
            ("10080", "1"),
            ("-1", "0"),
            ("1.5", "0"),
            ("1e2", "0"),
            ("1", "-1"),
            ("1", "5.5"),
            (" 1", "0"),
            ("999999999999999999999999999999", "0"),
        ] {
            assert_eq!(
                parse_elapsed_duration(minutes, seconds),
                None,
                "accepted {minutes:?} minutes and {seconds:?} seconds"
            );
        }
    }

    #[test]
    fn manual_form_is_exact_and_builds_server_owned_metadata() {
        let token = "a".repeat(64);
        let body = format!("seconds=30&submission_token={token}&distance=3.1&minutes=28");
        let form = parse_manual_run_form(body.as_bytes()).unwrap();
        assert_eq!(
            form,
            ManualRunForm {
                submission_token: token.clone(),
                distance_millimeters: 4_988_966,
                duration_milliseconds: 1_710_000,
            }
        );

        let activity = manual_activity(&form, 0);
        assert_eq!(activity.id, manual_storage_id(&token));
        assert_eq!(activity.source, MANUAL_SOURCE);
        assert_eq!(activity.source_activity_id, token);
        assert_eq!(activity.title, "Run");
        assert_eq!(activity.activity_type, "running");
        assert_eq!(activity.started_at_utc, "1970-01-01 00:00:00");
        assert_eq!(activity.started_at_local, "1969-12-31 19:00:00");
        assert_eq!(activity.eastern_offset_minutes, -300);
        assert_eq!(activity.moving_duration_milliseconds, None);
        assert_eq!(activity.ascent_millimeters, None);
        assert_eq!(activity.imported_at, 0);

        assert_eq!(
            parse_manual_run_form(b"distance=3&minutes=1&seconds=0"),
            None
        );
        assert_eq!(
            parse_manual_run_form(
                format!("submission_token={token}&distance=3&minutes=1&seconds=0&extra=nope")
                    .as_bytes()
            ),
            None
        );
        assert_eq!(
            parse_manual_run_form(
                format!("submission_token={token}&distance=3&distance=4&minutes=1&seconds=0")
                    .as_bytes()
            ),
            None
        );
        assert_eq!(
            parse_manual_run_form(
                format!(
                    "submission_token={}&distance=3&minutes=1&seconds=0",
                    "A".repeat(64)
                )
                .as_bytes()
            ),
            None
        );
    }

    #[test]
    fn manual_write_authorization_is_exact_admin_plus_same_origin() {
        use crate::content::access::ADMIN_EMAIL;

        assert_eq!(
            authorize_manual_write(Some(ADMIN_EMAIL), true),
            ManualWriteAuth::Allowed
        );
        assert_eq!(authorize_manual_write(None, true), ManualWriteAuth::Login);
        assert_eq!(
            authorize_manual_write(Some("friend@example.com"), true),
            ManualWriteAuth::NotFound
        );
        assert_eq!(
            authorize_manual_write(Some(ADMIN_EMAIL), false),
            ManualWriteAuth::Forbidden
        );
    }

    #[test]
    fn manual_tokens_and_source_copy_are_source_aware() {
        let first = new_submission_token();
        let second = new_submission_token();
        assert!(is_sha256_hex(&first));
        assert!(is_sha256_hex(&second));
        assert_ne!(first, second);

        let mut activity = empty_activity();
        activity.source = MANUAL_SOURCE.to_string();
        assert_eq!(
            source_note(&activity),
            "Logged by hand from distance and elapsed time."
        );
        activity.source = "garmin-connect".to_string();
        assert!(source_note(&activity).starts_with("Imported from Garmin Connect."));
        activity.source_activity_id = "24065766206".to_string();
        assert_eq!(
            canonical_source_url(&activity).as_deref(),
            Some("https://connect.garmin.com/app/activity/24065766206")
        );
        activity.source_url =
            Some("https://connect.garmin.com/app/activity/24065766206".to_string());
        assert_eq!(
            canonical_source_url(&activity).as_deref(),
            Some("https://connect.garmin.com/app/activity/24065766206")
        );
        activity.source_url = Some("https://connect.garmin.com/app/activity/1".to_string());
        assert_eq!(canonical_source_url(&activity), None);
        activity.source = MANUAL_SOURCE.to_string();
        assert_eq!(canonical_source_url(&activity), None);

        let response = see_other("/fitness");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    }

    #[test]
    fn import_form_requires_one_numeric_id_and_review_digest() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_import_form(
                format!("activity_id=24065766206&summary_digest={digest}").as_bytes()
            ),
            Some(ImportForm {
                activity_id: "24065766206".to_string(),
                summary_digest: digest.clone(),
            })
        );
        assert_eq!(
            parse_import_form(format!("summary_digest={digest}&activity_id=1").as_bytes()),
            Some(ImportForm {
                activity_id: "1".to_string(),
                summary_digest: digest,
            })
        );
        assert_eq!(parse_import_form(b"activity_id=1"), None);
        assert_eq!(parse_import_form(b"activity_id=1&activity_id=2"), None);
        assert_eq!(
            parse_import_form(
                format!("activity_id=nope&summary_digest={}", "a".repeat(64)).as_bytes()
            ),
            None
        );
        assert_eq!(
            parse_import_form(
                format!("activity_id=1&summary_digest={}", "A".repeat(64)).as_bytes()
            ),
            None
        );
        assert_eq!(
            parse_import_form(b"activity_id=%&summary_digest=nope"),
            None
        );
        assert_eq!(
            parse_import_form(b"activity_id=%FF&summary_digest=nope"),
            None
        );
        assert_eq!(parse_import_form(b"activity_id=1&"), None);
        assert_eq!(parse_import_form(b"activity_id=1&other=nope"), None);
    }

    #[test]
    fn native_share_form_accepts_only_bounded_manifest_fields() {
        let fields = parse_share_form(
            b"title=Morning+run&text=look&url=https%3A%2F%2Fconnect.garmin.com%2Fapp%2Factivity%2F123",
        )
        .unwrap();
        assert_eq!(fields.title.as_deref(), Some("Morning run"));
        assert_eq!(fields.text.as_deref(), Some("look"));
        assert_eq!(fields.activity_id(), Ok("123".to_string()));

        assert!(parse_share_form(b"").is_none());
        assert!(parse_share_form(b"url=one&url=two").is_none());
        assert!(parse_share_form(b"garmin=123").is_none());
        assert!(parse_share_form(b"url=%").is_none());
        assert!(parse_share_form(b"url=one&").is_none());
        assert!(
            parse_share_form(format!("text={}", "x".repeat(MAX_SHARED_FIELD_BYTES + 1)).as_bytes())
                .is_none()
        );
    }

    #[test]
    fn native_share_dispatches_complete_lyfta_text_without_a_query_redirect() {
        let fields = ShareFields {
            text: Some(LYFTA_SHARE.to_string()),
            ..ShareFields::default()
        };
        assert_eq!(
            classify_native_share(&fields),
            NativeShare::Lyfta(LYFTA_SHARE.to_string())
        );

        let (title, rest) = LYFTA_SHARE.split_once('\n').unwrap();
        let split = ShareFields {
            title: Some(title.to_string()),
            text: Some(rest.to_string()),
            ..ShareFields::default()
        };
        assert!(matches!(
            classify_native_share(&split),
            NativeShare::Lyfta(_)
        ));
    }

    #[test]
    fn native_share_rejects_link_only_and_mixed_app_payloads() {
        let link_only = ShareFields {
            url: Some("https://lyfta.app/wk/example".to_string()),
            ..ShareFields::default()
        };
        assert_eq!(classify_native_share(&link_only), NativeShare::Unknown);

        let mixed = ShareFields {
            text: Some(LYFTA_SHARE.to_string()),
            url: Some("https://connect.garmin.com/app/activity/123".to_string()),
            ..ShareFields::default()
        };
        assert_eq!(classify_native_share(&mixed), NativeShare::Ambiguous);
    }
}
