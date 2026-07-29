//! Fitness feature map and cross-layer invariants: `docs/fitness.md`.

pub(crate) mod archive;
mod badge;
mod data;
mod delete;
mod filters;
mod format;
mod heatmap;
mod muscles;
mod results;
mod share;
mod taxonomy;

use self::archive::store::FitnessStore;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{HeaderValue, header, not_found, page, parse_query_params, path_param, redirect, uri},
    view::{class, component, view},
};

use crate::{
    app::login::viewer,
    components::{back_link, page_head, rail_group, rail_section, shell},
    content::{access::is_admin, interests::interest},
};

use self::{
    badge::set_badge,
    data as fitness,
    filters::{EQUIPMENT, Filters, LOG_PATH, MOVEMENT_DETAILS, MOVEMENTS, MUSCLES, SET_TYPES},
    format::{format_archive_summary, format_integer, plural},
    results::{WorkoutCard, make_pager, total_pages, workout_url},
};

const AUTO_FILTER_JS: Asset = asset!("./auto-filter.js");
const WORKOUT_UPLOAD_JS: Asset = asset!("./workout-upload.js");

/// The single crate-visible seam used by the authenticated browser importer.
/// Keeping formatting here guarantees its clipboard response is byte-for-byte
/// the same as clicking "copy to clipboard" on the resulting workout page.
pub(crate) fn canonical_share_text(cx: &Cx, workout: &archive::api::Workout) -> String {
    share::share_text(workout, share::request_origin(cx).as_deref())
}

// Tailwind class vocabulary shared across the lifting views. Every utility
// stays whole on its own line: the build-time class scanner reads them
// straight from these source literals.
pub(super) const META_LABEL: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";

const FIELD: &str = "flex min-w-0 flex-col gap-[0.35rem]";
const CONTROL: &str = "w-full min-w-0 h-[2.65rem] px-3 py-[0.65rem] text-ink bg-page \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-[1.2] outline-none \
     placeholder:text-muted placeholder:opacity-100 \
     hover:border-[color-mix(in_srgb,var(--color-ink2)_45%,var(--color-hairline))] \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const FACET: &str = "min-w-0 mt-[1.2rem]";
const LEGEND: &str =
    "mb-[0.45rem] font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const CHIP_GRID: &str = "flex flex-wrap gap-[0.45rem]";
const CHIP: &str = "inline-flex items-center px-[0.7rem] border border-hairline rounded-full \
     text-ink2 bg-page font-meta text-[0.7rem] leading-none \
     transition-colors duration-[140ms] ease-[ease] \
     group-hover:text-oxide \
     group-hover:border-[color-mix(in_srgb,var(--color-oxide)_45%,var(--color-hairline))] \
     peer-checked:text-oxide peer-checked:border-oxide \
     peer-checked:bg-[color-mix(in_srgb,var(--color-oxide)_8%,var(--color-card))] \
     peer-checked:shadow-[inset_0_-2px_0_color-mix(in_srgb,var(--color-oxide)_25%,transparent)] \
     peer-focus-visible:outline-solid peer-focus-visible:outline-2 \
     peer-focus-visible:outline-oxide peer-focus-visible:outline-offset-2";
const FIELD_NOTE: &str = "mt-2 max-w-[44rem] text-[0.7rem] leading-[1.5] text-muted";
const FLAG: &str = "flex items-center gap-[0.55rem] text-[0.84rem] text-ink2 cursor-pointer";
const FLAG_INPUT: &str = "size-4 m-0 accent-oxide focus-visible:outline-solid \
     focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2";
const RESULT_COUNT: &str = "mt-[0.3rem] font-meta text-[0.72rem] leading-[1.5] text-muted";
const LIFT_LINK: &str =
    "text-oxide font-meta text-[0.72rem] decoration-oxide/45 underline-offset-[0.24em]";
const LIST: &str = "flex flex-col gap-6 mt-5";
const EMPTY_CARD: &str = "px-5 py-8 text-center bg-card border border-hairline";
const EMPTY_ERROR_CARD: &str = "px-5 py-8 text-center bg-card border \
     border-[color-mix(in_srgb,var(--color-oxide)_30%,var(--color-hairline))]";
const EMPTY_TITLE: &str = "font-display text-[1.2rem] font-semibold";
const EMPTY_COPY: &str =
    "mt-[0.4rem] mx-auto max-w-[32rem] text-ink2 text-[0.87rem] leading-[1.55]";
const EMPTY_RESET: &str = "inline-block mt-[0.8rem] py-1 font-meta text-[0.72rem] text-oxide \
     underline underline-offset-[0.25em]";
const PAGE_LINK: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center justify-center \
     px-[0.55rem] py-[0.35rem] text-ink2 border border-hairline \
     hover:text-oxide hover:border-oxide focus-visible:text-oxide focus-visible:border-oxide";
const PAGE_DISABLED: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-muted border border-transparent";
const PAGE_CURRENT: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-card bg-ink border border-ink";
const PAGE_GAP: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center justify-center \
     px-[0.55rem] py-[0.35rem] text-muted";
const META_SMALL: &str = "flex-none font-meta text-[0.67rem] leading-[1.5] text-muted";
const WORKOUT_NOTE: &str = "mt-[0.7rem] px-[0.7rem] py-[0.6rem] text-ink2 bg-brass/7 \
     border-l-2 border-brass text-[0.82rem] leading-[1.5]";

#[component]
async fn workout_upload_dialog() -> Result {
    view! {
        <details
            data-workout-upload-disclosure=""
            class="relative mt-1 flex-none open:before:fixed open:before:inset-0 \
                   open:before:z-40 open:before:bg-ink/50 open:before:content-['']"
        >
            <summary
                data-workout-upload-trigger=""
                class="list-none cursor-pointer rounded-sm border border-oxide px-3 py-2 \
                       font-meta text-xs text-oxide hover:bg-oxide hover:text-card \
                       focus-visible:outline-solid focus-visible:outline-2 \
                       focus-visible:outline-oxide focus-visible:outline-offset-2 \
                       [&::-webkit-details-marker]:hidden"
            >
                "upload lift"
            </summary>
            <div
                data-workout-upload-fallback=""
                class="fixed inset-x-4 top-4 z-50 mx-auto max-h-[calc(100dvh-2rem)] \
                       max-w-2xl overflow-y-auto"
            >
                <section
                    data-workout-upload-content=""
                    class="rounded-sm border border-hairline bg-card p-5 text-ink shadow-2xl sm:p-7"
                >
                    <header class="flex items-start justify-between gap-5">
                        <div>
                            <p class=(META_LABEL)>"Lyfta import"</p>
                            <h2
                                id="workout-upload-title"
                                class="mt-1 font-display text-2xl font-semibold"
                            >
                                "Publish a lift"
                            </h2>
                        </div>
                        <a
                            href="/lifting"
                            data-workout-upload-close=""
                            aria-label="Close upload form"
                            class="p-1 font-meta text-2xl leading-none text-muted hover:text-oxide \
                                   focus-visible:outline-solid focus-visible:outline-2 \
                                   focus-visible:outline-oxide focus-visible:outline-offset-2"
                        >
                            <span aria-hidden="true">"×"</span>
                        </a>
                    </header>
                    <p
                        id="workout-upload-description"
                        class="mt-3 max-w-prose text-sm leading-relaxed text-ink2"
                    >
                        "Paste the workout text copied from Lyfta. Publishing adds it to the \
                         lifting archive and RSS feed. “Upload from clipboard” also replaces \
                         the clipboard with this site's ready-to-share workout text."
                    </p>
                    <form
                        method="post"
                        action="/lifting/upload"
                        class="mt-5 space-y-5"
                        data-workout-upload=""
                    >
                        <label for="workout-upload-text" class="block space-y-2">
                            <span
                                class="font-meta text-sm text-ink2"
                                data-workout-upload-label=""
                            >
                                "Lyfta workout text"
                            </span>
                            <textarea
                                id="workout-upload-text"
                                name="workout"
                                rows="14"
                                required=""
                                spellcheck="false"
                                autocomplete="off"
                                class="block w-full resize-y rounded-sm border border-hairline \
                                       bg-page p-4 font-mono text-sm leading-relaxed text-ink \
                                       focus-visible:outline-solid focus-visible:outline-2 \
                                       focus-visible:outline-oxide focus-visible:outline-offset-2"
                            ></textarea>
                        </label>
                        <div class="flex flex-wrap items-center gap-4">
                            <button
                                type="button"
                                hidden=""
                                data-workout-upload-clipboard=""
                                class="cursor-pointer rounded-sm border border-oxide bg-oxide px-4 \
                                       py-2 font-meta text-sm text-card hover:bg-oxide-hot \
                                       disabled:cursor-wait disabled:opacity-60"
                            >
                                "Upload from clipboard"
                            </button>
                            <button
                                type="submit"
                                data-workout-upload-submit=""
                                class="cursor-pointer font-meta text-sm text-oxide underline \
                                       decoration-oxide/40 underline-offset-4 \
                                       hover:decoration-oxide disabled:cursor-wait \
                                       disabled:opacity-60"
                            >
                                "Publish pasted text →"
                            </button>
                            <button
                                type="button"
                                hidden=""
                                data-workout-upload-copy=""
                                class="cursor-pointer rounded-sm border border-oxide bg-oxide px-4 \
                                       py-2 font-meta text-sm text-card hover:bg-oxide-hot"
                            >
                                "Copy share text and open workout"
                            </button>
                            <a
                                hidden=""
                                data-workout-upload-result-open=""
                                class="font-meta text-sm text-oxide underline \
                                       decoration-oxide/40 underline-offset-4 \
                                       hover:decoration-oxide"
                            >
                                "Open published workout →"
                            </a>
                        </div>
                        <p
                            class="font-meta text-xs leading-relaxed text-muted"
                            data-workout-upload-status=""
                            aria-live="polite"
                        >
                            "Or paste normally, review the text, and publish it."
                        </p>
                    </form>
                </section>
            </div>
        </details>
        <dialog
            id="workout-upload-dialog"
            data-workout-upload-dialog=""
            aria-labelledby="workout-upload-title"
            aria-describedby="workout-upload-description"
            class="m-auto max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-2xl \
                   overflow-y-auto border-0 bg-transparent p-0 text-ink backdrop:bg-ink/50"
        >
            <div data-workout-upload-dialog-slot=""></div>
        </dialog>
        <script type="module" src=(WORKOUT_UPLOAD_JS)></script>
    }
}

#[path_param]
struct WorkoutPath(str);

#[page("/lifting")]
async fn lifting(cx: &Cx) -> Result {
    // Filtered archive links remain useful; the archive itself now lives at
    // `/lifting/log`.
    if let Some(query) = uri(cx).query() {
        return Err(redirect(&format!("{LOG_PATH}?{query}")).into());
    }

    let meta = interest("lifting");
    let can_upload = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let (calendar, latest) = fitness::load_home(app_context::<FitnessStore>(cx)).await;
    if let Err(error) = &calendar {
        eprintln!("fitness calendar fetch failed: {error}");
    }
    if let Err(error) = &latest {
        eprintln!("fitness latest workout fetch failed: {error}");
    }

    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let latest_error = latest.as_ref().err();
    let latest_workout = latest
        .as_ref()
        .ok()
        .and_then(|detail| detail.workout.as_ref());
    let next_lift_url = latest
        .as_ref()
        .ok()
        .and_then(|detail| detail.older_workout_path.as_deref())
        .map(workout_url);

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "interests",
            runtime: false,
            <header class="rail-row mt-16">
                <p class="rail-stamp rail-stamp-label">(meta.slug)</p>
                <div class="flex min-w-0 items-start justify-between gap-4">
                    <h1 class="font-display text-4xl font-bold tracking-tight">
                        (meta.title)
                    </h1>
                    if can_upload {
                        workout_upload_dialog()
                    }
                </div>
            </header>
            <div>
                rail_section(
                    class: "mt-10",
                    stamp: "volume",
                    if let Some(days) = calendar_days {
                        heatmap::calendar_heatmap(days: days)
                    } else {
                        <section class="p-4 bg-card border border-hairline">
                            <p class=(EMPTY_COPY)>
                                "Daily volume is unavailable right now."
                            </p>
                        </section>
                    }
                )

                rail_section(
                    class: "mt-12",
                    stamp: "sets",
                    <header class="flex items-end justify-between gap-4" id="set-log">
                        <div>
                            <h2 class="font-display text-2xl font-semibold">
                                "Most recent lift"
                            </h2>
                        </div>
                        <a class=(class!(LIFT_LINK, "flex-none")) href=(LOG_PATH)>
                            "search full log →"
                        </a>
                        if let Some(href) = &next_lift_url {
                            <a class=(LIFT_LINK) href=(href.as_str())>
                                "see next lift →"
                            </a>
                        }
                    </header>
                )

                <section class=(LIST) aria-label="Most recent workout">
                    if latest_error.is_some() {
                        <div class=(EMPTY_ERROR_CARD)>
                            <p class=(EMPTY_TITLE)>
                                "The latest lift did not load."
                            </p>
                            <p class=(EMPTY_COPY)>
                                "Try the workout archive again in a moment."
                            </p>
                            <a class=(EMPTY_RESET) href="/lifting#set-log">
                                "retry"
                            </a>
                        </div>
                    } else if let Some(workout) = latest_workout {
                        workout_sheet(workout: workout, permalink: true)
                    } else {
                        <div class=(EMPTY_CARD)>
                            <p class=(EMPTY_TITLE)>"No lifts yet."</p>
                            <p class=(EMPTY_COPY)>
                                "The workout archive will appear here after its first import."
                            </p>
                        </div>
                    }
                </section>
            </div>
            back_link(href: "/interests", label: "all interests")
        )
    }
}

#[page("/lifting/log")]
async fn lifting_log(cx: &Cx) -> Result {
    let raw = match parse_query_params::<Vec<(String, String)>>(cx) {
        Ok(raw) => raw,
        Err(_) => return Err(redirect(LOG_PATH).into()),
    };
    let Some(filters) = Filters::normalize(raw) else {
        return Err(redirect(LOG_PATH).into());
    };
    let canonical = filters.query();
    if uri(cx).query().is_some_and(|query| query != canonical) {
        return Err(redirect(&filters.url(false)).into());
    }

    let meta = interest("lifting");
    let api_pairs = filters.api_pairs();
    let (facets, sets, calendar) = fitness::load(app_context::<FitnessStore>(cx), &api_pairs).await;
    if let Err(error) = &facets {
        eprintln!("fitness facets fetch failed: {error}");
    }
    if let Err(error) = &sets {
        eprintln!("fitness sets fetch failed: {error}");
    }
    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let day_link_query = filters.day_link_query();
    if let Ok(page) = &sets {
        let last_page = total_pages(page);
        if page.page > last_page {
            return Err(redirect(&filters.page_url(last_page)).into());
        }
    }

    let archive_summary = facets
        .as_ref()
        .map(format_archive_summary)
        .unwrap_or_else(|_| "Workout archive · live totals unavailable".to_string());
    let selected_exercise = filters.value("exercise");
    let mut exercise_options = Vec::new();
    let selected_exercise_missing = match &facets {
        Ok(data) => !data
            .exercises
            .iter()
            .any(|option| option.value == selected_exercise),
        Err(_) => true,
    };
    if !selected_exercise.is_empty() && selected_exercise_missing {
        exercise_options.push((selected_exercise.to_string(), selected_exercise.to_string()));
    }
    if let Ok(data) = &facets {
        exercise_options.extend(data.exercises.iter().map(|option| {
            (
                option.value.clone(),
                format!("{} · {}", option.value, format_integer(option.count)),
            )
        }));
    }

    let active_filters = filters.active();
    let result_summary = match &sets {
        Ok(page) if page.total_sets > 0 => {
            let visible_sets = page
                .workouts
                .iter()
                .map(|workout| workout.sets.len() as u64)
                .sum::<u64>();
            format!(
                "{} matching sets across {} workouts · {} on this page",
                format_integer(page.total_sets),
                format_integer(page.total_workouts),
                format_integer(visible_sets),
            )
        }
        Ok(_) => "No sets match these filters.".to_string(),
        Err(error) => error
            .rejected_message()
            .map(|message| format!("A filter was rejected · {message}"))
            .unwrap_or_else(|| "Workout database is unreachable.".to_string()),
    };
    let pager = sets
        .as_ref()
        .ok()
        .and_then(|page| make_pager(page, &filters));
    let retry_url = filters.url(true);

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "interests",
            runtime: false,
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            <div>
                rail_section(
                    class: "mt-10",
                    stamp: "filters",
                    <section
                        class="relative p-5 overflow-hidden bg-card border border-hairline \
                             shadow-[0_10px_30px_color-mix(in_srgb,var(--color-ink)_4%,transparent)] \
                             before:content-[''] before:absolute before:inset-x-0 before:top-0 \
                             before:h-0.5 \
                             before:bg-[linear-gradient(90deg,var(--color-oxide)_0_36%,var(--color-ink)_36%_100%)] \
                             sm:p-6"
                        aria-labelledby="fitness-filter-title"
                    >
                        <div class="flex items-baseline justify-between gap-4">
                            <div>
                                <p id="fitness-filter-title" class=(META_LABEL)>
                                    "the whole archive"
                                </p>
                                <p class="mt-1 font-meta text-[0.8125rem] leading-[1.55] text-ink2">
                                    (archive_summary.as_str())
                                </p>
                            </div>
                            if !active_filters.is_empty() {
                                <a
                                    class="inline-block flex-none py-1 font-meta text-[0.72rem] \
                                         text-oxide underline decoration-oxide/35 \
                                         underline-offset-[0.25em]"
                                    href=(LOG_PATH)
                                >
                                    "clear filters"
                                </a>
                            }
                        </div>

                        <form
                            class="mt-5"
                            action="/lifting/log#set-log"
                            method="get"
                            data-lifting-filters=""
                        >
                            <div
                                class="grid grid-cols-[minmax(0,1fr)] gap-[0.85rem] \
                                     sm:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)]"
                            >
                                <label class=(FIELD) for="fitness-search">
                                    <span class=(META_LABEL)>"search"</span>
                                    <input
                                        class=(CONTROL)
                                        id="fitness-search"
                                        name="q"
                                        type="search"
                                        value=(filters.value("q"))
                                        placeholder="exercise, workout, or note"
                                        autocomplete="off"
                                    >
                                </label>
                                <label class=(FIELD) for="fitness-exercise">
                                    <span class=(META_LABEL)>"exact exercise"</span>
                                    <select
                                        class=(class!(CONTROL, "pr-8"))
                                        id="fitness-exercise"
                                        name="exercise"
                                    >
                                        <option value="" selected=(selected_exercise.is_empty())>
                                            "all exercises"
                                        </option>
                                        for option in exercise_options.iter() {
                                            <option
                                                value=(option.0.as_str())
                                                selected=(selected_exercise == option.0)
                                            >
                                                (option.1.as_str())
                                            </option>
                                        }
                                    </select>
                                </label>
                            </div>

                            <fieldset class=(FACET)>
                                <legend class=(LEGEND)>"movement pattern"</legend>
                                <div class=(CHIP_GRID)>
                                    for (value, label) in MOVEMENTS {
                                        check_chip(
                                            name: "movement",
                                            value: *value,
                                            label: *label,
                                            checked: filters.contains("movement", value),
                                            compact: false
                                        )
                                    }
                                </div>
                                <p class=(FIELD_NOTE)>
                                    "Squat-type includes squats, lunges, split squats, step-ups, and leg presses."
                                </p>
                            </fieldset>

                            <details
                                class="group mt-[1.1rem] border-t border-hairline"
                                open=(filters.advanced())
                            >
                                <summary
                                    class="w-fit mt-[0.8rem] py-[0.2rem] list-none \
                                         [&::-webkit-details-marker]:hidden text-ink2 font-meta \
                                         text-xs cursor-pointer select-none \
                                         before:content-['+'] before:inline-block before:w-5 \
                                         before:text-oxide group-open:before:content-['−'] \
                                         focus-visible:outline-solid focus-visible:outline-2 \
                                         focus-visible:outline-oxide \
                                         focus-visible:outline-offset-2"
                                >
                                    "more filters"
                                </summary>
                                <div class="pt-4">
                                    <div
                                        class="grid grid-cols-[minmax(0,1fr)] gap-[0.85rem] \
                                             sm:grid-cols-2 min-[54rem]:grid-cols-4"
                                    >
                                        <label class=(FIELD) for="fitness-from">
                                            <span class=(META_LABEL)>"from"</span>
                                            <input
                                                class=(CONTROL)
                                                id="fitness-from"
                                                name="from"
                                                type="date"
                                                value=(filters.value("from"))
                                            >
                                        </label>
                                        <label class=(FIELD) for="fitness-to">
                                            <span class=(META_LABEL)>"through"</span>
                                            <input
                                                class=(CONTROL)
                                                id="fitness-to"
                                                name="to"
                                                type="date"
                                                value=(filters.value("to"))
                                            >
                                        </label>
                                        <label class=(FIELD) for="fitness-daypart">
                                            <span class=(META_LABEL)>"time of day"</span>
                                            <select
                                                class=(class!(CONTROL, "pr-8"))
                                                id="fitness-daypart"
                                                name="time_of_day"
                                            >
                                                <option
                                                    value=""
                                                    selected=(filters.value("time_of_day").is_empty())
                                                >
                                                    "any time"
                                                </option>
                                                <option
                                                    value="morning"
                                                    selected=(filters.value("time_of_day") == "morning")
                                                >
                                                    "morning · 5–11"
                                                </option>
                                                <option
                                                    value="afternoon"
                                                    selected=(filters.value("time_of_day") == "afternoon")
                                                >
                                                    "afternoon · 12–4"
                                                </option>
                                                <option
                                                    value="evening"
                                                    selected=(filters.value("time_of_day") == "evening")
                                                >
                                                    "evening · 5–8"
                                                </option>
                                                <option
                                                    value="night"
                                                    selected=(filters.value("time_of_day") == "night")
                                                >
                                                    "night · 9–4"
                                                </option>
                                            </select>
                                        </label>
                                        <label class=(FIELD) for="fitness-weekday">
                                            <span class=(META_LABEL)>"weekday"</span>
                                            <select
                                                class=(class!(CONTROL, "pr-8"))
                                                id="fitness-weekday"
                                                name="weekday"
                                            >
                                                <option
                                                    value=""
                                                    selected=(filters.value("weekday").is_empty())
                                                >
                                                    "any day"
                                                </option>
                                                <option
                                                    value="mon"
                                                    selected=(filters.value("weekday") == "mon")
                                                >
                                                    "Monday"
                                                </option>
                                                <option
                                                    value="tue"
                                                    selected=(filters.value("weekday") == "tue")
                                                >
                                                    "Tuesday"
                                                </option>
                                                <option
                                                    value="wed"
                                                    selected=(filters.value("weekday") == "wed")
                                                >
                                                    "Wednesday"
                                                </option>
                                                <option
                                                    value="thu"
                                                    selected=(filters.value("weekday") == "thu")
                                                >
                                                    "Thursday"
                                                </option>
                                                <option
                                                    value="fri"
                                                    selected=(filters.value("weekday") == "fri")
                                                >
                                                    "Friday"
                                                </option>
                                                <option
                                                    value="sat"
                                                    selected=(filters.value("weekday") == "sat")
                                                >
                                                    "Saturday"
                                                </option>
                                                <option
                                                    value="sun"
                                                    selected=(filters.value("weekday") == "sun")
                                                >
                                                    "Sunday"
                                                </option>
                                            </select>
                                        </label>
                                    </div>

                                    <fieldset class=(FACET)>
                                        <legend class=(LEGEND)>"movement detail"</legend>
                                        <div class=(CHIP_GRID)>
                                            for (value, label) in MOVEMENT_DETAILS {
                                                check_chip(
                                                    name: "movement",
                                                    value: *value,
                                                    label: *label,
                                                    checked: filters.contains("movement", value),
                                                    compact: true
                                                )
                                            }
                                        </div>
                                    </fieldset>

                                    <fieldset class=(FACET)>
                                        <legend class=(LEGEND)>"muscle group"</legend>
                                        <div class=(CHIP_GRID)>
                                            for (value, label) in MUSCLES {
                                                check_chip(
                                                    name: "muscle",
                                                    value: *value,
                                                    label: *label,
                                                    checked: filters.contains("muscle", value),
                                                    compact: true
                                                )
                                            }
                                        </div>
                                    </fieldset>

                                    <fieldset class=(FACET)>
                                        <legend class=(LEGEND)>"equipment"</legend>
                                        <div class=(CHIP_GRID)>
                                            for (value, label) in EQUIPMENT {
                                                check_chip(
                                                    name: "equipment",
                                                    value: *value,
                                                    label: *label,
                                                    checked: filters.contains("equipment", value),
                                                    compact: true
                                                )
                                            }
                                        </div>
                                    </fieldset>

                                    <fieldset class=(FACET)>
                                        <legend class=(LEGEND)>"set kind"</legend>
                                        <div class=(CHIP_GRID)>
                                            for (value, label) in SET_TYPES {
                                                check_chip(
                                                    name: "set_type",
                                                    value: *value,
                                                    label: *label,
                                                    checked: filters.contains("set_type", value),
                                                    compact: true
                                                )
                                            }
                                        </div>
                                    </fieldset>

                                    <div
                                        class="grid grid-cols-[minmax(0,1fr)] gap-[0.85rem] \
                                             mt-[1.2rem] sm:grid-cols-3"
                                    >
                                        <fieldset
                                            class="grid min-w-0 items-center gap-[0.45rem] \
                                                 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]"
                                        >
                                            <legend class=(class!(LEGEND, "col-span-full"))>"load"</legend>
                                            <label for="fitness-min-load">
                                                <span class="sr-only">"minimum load"</span>
                                                <input
                                                    class=(CONTROL)
                                                    id="fitness-min-load"
                                                    name="min_load"
                                                    type="number"
                                                    inputmode="decimal"
                                                    step="any"
                                                    min="0"
                                                    value=(filters.value("min_load"))
                                                    placeholder="min"
                                                >
                                            </label>
                                            <span class="text-muted" aria-hidden="true">"–"</span>
                                            <label for="fitness-max-load">
                                                <span class="sr-only">"maximum load"</span>
                                                <input
                                                    class=(CONTROL)
                                                    id="fitness-max-load"
                                                    name="max_load"
                                                    type="number"
                                                    inputmode="decimal"
                                                    step="any"
                                                    min="0"
                                                    value=(filters.value("max_load"))
                                                    placeholder="max"
                                                >
                                            </label>
                                        </fieldset>
                                        <fieldset
                                            class="grid min-w-0 items-center gap-[0.45rem] \
                                                 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]"
                                        >
                                            <legend class=(class!(LEGEND, "col-span-full"))>"reps"</legend>
                                            <label for="fitness-min-reps">
                                                <span class="sr-only">"minimum reps"</span>
                                                <input
                                                    class=(CONTROL)
                                                    id="fitness-min-reps"
                                                    name="min_reps"
                                                    type="number"
                                                    inputmode="numeric"
                                                    step="1"
                                                    min="0"
                                                    value=(filters.value("min_reps"))
                                                    placeholder="min"
                                                >
                                            </label>
                                            <span class="text-muted" aria-hidden="true">"–"</span>
                                            <label for="fitness-max-reps">
                                                <span class="sr-only">"maximum reps"</span>
                                                <input
                                                    class=(CONTROL)
                                                    id="fitness-max-reps"
                                                    name="max_reps"
                                                    type="number"
                                                    inputmode="numeric"
                                                    step="1"
                                                    min="0"
                                                    value=(filters.value("max_reps"))
                                                    placeholder="max"
                                                >
                                            </label>
                                        </fieldset>
                                        <label class=(FIELD) for="fitness-effort">
                                            <span class=(META_LABEL)>"max RPE"</span>
                                            <input
                                                class=(CONTROL)
                                                id="fitness-effort"
                                                name="max_effort"
                                                type="number"
                                                inputmode="decimal"
                                                min="0"
                                                step="0.5"
                                                value=(filters.value("max_effort"))
                                                placeholder="any"
                                            >
                                        </label>
                                    </div>
                                    <p class=(FIELD_NOTE)>
                                        "Load is displayed in pounds."
                                    </p>

                                    <div
                                        class="grid grid-cols-[minmax(0,1fr)] gap-x-4 \
                                             gap-y-[0.6rem] mt-[1.2rem] pt-4 border-t \
                                             border-dashed border-hairline sm:grid-cols-2 \
                                             min-[54rem]:grid-cols-3"
                                    >
                                        <label class=(FLAG)>
                                            <input
                                                class=(FLAG_INPUT)
                                                type="checkbox"
                                                name="has_record"
                                                value="true"
                                                checked=(filters.contains("has_record", "true"))
                                            >
                                            <span>"personal records"</span>
                                        </label>
                                        <label class=(FLAG)>
                                            <input
                                                class=(FLAG_INPUT)
                                                type="checkbox"
                                                name="has_superset"
                                                value="true"
                                                checked=(filters.contains("has_superset", "true"))
                                            >
                                            <span>"supersets"</span>
                                        </label>
                                        <label class=(FLAG)>
                                            <input
                                                class=(FLAG_INPUT)
                                                type="checkbox"
                                                name="has_notes"
                                                value="true"
                                                checked=(filters.contains("has_notes", "true"))
                                            >
                                            <span>"with notes"</span>
                                        </label>
                                        <label class=(FLAG)>
                                            <input
                                                class=(FLAG_INPUT)
                                                type="checkbox"
                                                name="incomplete"
                                                value="true"
                                                checked=(filters.contains("incomplete", "true"))
                                            >
                                            <span>"incomplete rows"</span>
                                        </label>
                                        <label class=(FLAG)>
                                            <input
                                                class=(FLAG_INPUT)
                                                type="checkbox"
                                                name="duration"
                                                value="suspicious"
                                                checked=(filters.contains("duration", "suspicious"))
                                            >
                                            <span>"suspect timers only"</span>
                                        </label>
                                    </div>

                                    <label
                                        class=(class!(FIELD, "max-w-[11rem] mt-[1.2rem]"))
                                        for="fitness-page-size"
                                    >
                                        <span class=(META_LABEL)>"workouts per page"</span>
                                        <select
                                            class=(class!(CONTROL, "pr-8"))
                                            id="fitness-page-size"
                                            name="per_page"
                                        >
                                            <option value="10" selected=(filters.per_page() == "10")>
                                                "10"
                                            </option>
                                            <option value="20" selected=(filters.per_page() == "20")>
                                                "20"
                                            </option>
                                            <option value="40" selected=(filters.per_page() == "40")>
                                                "40"
                                            </option>
                                        </select>
                                    </label>
                                </div>
                            </details>

                            <div
                                class="flex items-end justify-between gap-4 mt-5 pt-4 \
                                     border-t border-hairline"
                            >
                                <div
                                    class="flex min-w-0 flex-1 flex-wrap items-center gap-[0.4rem]"
                                    aria-label="Active filters"
                                >
                                    if active_filters.is_empty() {
                                        <span class="font-meta text-[0.72rem] text-muted">
                                            "All sets are included."
                                        </span>
                                    }
                                    for filter in active_filters.iter() {
                                        <a
                                            class="px-[0.55rem] py-[0.35rem] font-meta \
                                                 text-[0.67rem] leading-[1.2] text-oxide \
                                                 bg-oxide/6 border border-oxide/30 rounded \
                                                 hover:border-oxide hover:underline \
                                                 hover:underline-offset-[0.2em] \
                                                 focus-visible:border-oxide \
                                                 focus-visible:underline \
                                                 focus-visible:underline-offset-[0.2em]"
                                            href=(filter.href.as_str())
                                            aria-label=(filter.aria_label.as_str())
                                        >
                                            (format!("{} ×", filter.label))
                                        </a>
                                    }
                                </div>
                                <button
                                    class="flex-none min-h-[2.65rem] px-4 py-[0.65rem] font-meta \
                                         text-[0.72rem] text-card bg-oxide border border-oxide \
                                         rounded-[0.2rem] cursor-pointer hover:text-white \
                                         hover:bg-oxide-hot hover:border-oxide-hot \
                                         focus-visible:text-white focus-visible:bg-oxide-hot \
                                         focus-visible:border-oxide-hot"
                                    type="submit"
                                >
                                    "apply filters"
                                </button>
                            </div>
                        </form>
                        <script type="module" src=(AUTO_FILTER_JS)></script>
                    </section>
                )

                if let Some(days) = calendar_days {
                    rail_section(
                        class: "mt-12",
                        stamp: "volume",
                        heatmap::calendar_heatmap(
                            days: days,
                            link_query: day_link_query,
                            filtered: !active_filters.is_empty()
                        )
                    )
                }

                rail_section(
                    class: "mt-12",
                    stamp: "sets",
                    <header class="flex items-end justify-between gap-4" id="set-log">
                        <div>
                            <h2 class="font-display text-2xl font-semibold">
                                "Set log"
                            </h2>
                            <p class=(RESULT_COUNT)>
                                (result_summary.as_str())
                            </p>
                        </div>
                        <p class=(class!(META_LABEL, "flex-none"))>"newest first"</p>
                    </header>
                )

                <section class=(LIST) aria-label="Filtered workout sets">
                    if let Err(error) = &sets {
                        <div class=(EMPTY_ERROR_CARD)>
                            if let Some(message) = error.rejected_message() {
                                <p class=(EMPTY_TITLE)>
                                    "That filter combination is not valid."
                                </p>
                                <p class=(EMPTY_COPY)>(message)</p>
                                <a class=(EMPTY_RESET) href="/lifting/log#set-log">
                                    "clear every filter"
                                </a>
                            } else {
                                <p class=(EMPTY_TITLE)>
                                    "The set log did not load."
                                </p>
                                <p class=(EMPTY_COPY)>
                                    "The filters are intact. Try the database again."
                                </p>
                                <a class=(EMPTY_RESET) href=(retry_url.as_str())>
                                    "retry"
                                </a>
                            }
                        </div>
                    }
                    if let Ok(page) = &sets && page.workouts.is_empty() {
                        <div class=(EMPTY_CARD)>
                            <p class=(EMPTY_TITLE)>
                                if page.total_sets > 0 {
                                    "This page is empty."
                                } else {
                                    "No matching sets."
                                }
                            </p>
                            <p class=(EMPTY_COPY)>
                                if page.total_sets > 0 {
                                    "Try a previous page."
                                } else {
                                    "Loosen a movement, date, or numeric filter and the log will reappear."
                                }
                            </p>
                            <a class=(EMPTY_RESET) href="/lifting/log#set-log">
                                "clear every filter"
                            </a>
                        </div>
                    }
                    if let Ok(page) = &sets {
                        for workout in page.workouts.iter() {
                            workout_sheet(workout: workout, permalink: true)
                        }
                    }
                </section>

                if let Some(pager) = &pager {
                    <nav
                        class="flex flex-wrap items-center justify-center gap-[0.35rem] mt-7 \
                             font-meta text-[0.72rem]"
                        aria-label="Workout log pages"
                    >
                        if let Some(href) = &pager.newer {
                            <a class=(PAGE_LINK) href=(href.as_str())>
                                "← newer"
                            </a>
                        } else {
                            <span class=(PAGE_DISABLED) aria-disabled="true">
                                "← newer"
                            </span>
                        }
                        for part in pager.parts.iter() {
                            if let Some(number) = part {
                                if *number == pager.current {
                                    <span class=(PAGE_CURRENT) aria-current="page">
                                        (number.to_string())
                                    </span>
                                } else {
                                    <a
                                        class=(PAGE_LINK)
                                        href=(filters.page_url(*number))
                                    >
                                        (number.to_string())
                                    </a>
                                }
                            } else {
                                <span class=(PAGE_GAP)>"…"</span>
                            }
                        }
                        if let Some(href) = &pager.older {
                            <a class=(PAGE_LINK) href=(href.as_str())>
                                "older →"
                            </a>
                        } else {
                            <span class=(PAGE_DISABLED) aria-disabled="true">
                                "older →"
                            </span>
                        }
                    </nav>
                }
            </div>
            back_link(href: "/interests", label: "all interests")
        )
    }
}

#[page("/lifting/{workout_path}")]
async fn lift_detail(cx: &Cx) -> Result {
    let workout_path = path_param::<WorkoutPath>(cx);
    if uri(cx).query().is_some() {
        return Err(redirect(&workout_url(workout_path)).into());
    }

    let loaded = fitness::load_workout_by_path(app_context::<FitnessStore>(cx), workout_path).await;
    if matches!(&loaded, Err(error) if error.is_not_found()) {
        return Err(not_found().into());
    }
    if let Err(error) = &loaded {
        eprintln!("fitness workout fetch failed: {error}");
    }

    let meta = interest("lifting");
    let detail = loaded.as_ref().ok().map(|(detail, _)| detail);
    let workout = detail.and_then(|detail| detail.workout.as_ref());
    let involvement = loaded.as_ref().ok().and_then(|(detail, tags)| {
        detail
            .workout
            .as_ref()
            .map(|workout| muscles::workout_involvement(workout, tags))
    });
    let share_text =
        workout.map(|workout| share::share_text(workout, share::request_origin(cx).as_deref()));
    let page_title = workout
        .map(|workout| format!("{} · {}", workout.title, meta.title))
        .unwrap_or_else(|| meta.title.to_string());
    let page_heading = workout
        .map(|workout| workout.title.as_str())
        .unwrap_or("Workout");
    let newer_lift_url = detail
        .and_then(|detail| detail.newer_workout_path.as_deref())
        .map(workout_url);
    let older_lift_url = detail
        .and_then(|detail| detail.older_workout_path.as_deref())
        .map(workout_url);
    // The delete control ships only to the admin. The page is already
    // `no-store` (and `response_layer.rs` forces `private, no-store` on any
    // cookie-bearing request), so this markup can never reach a shared cache.
    let can_delete =
        workout.is_some() && viewer(cx).is_some_and(|current| is_admin(&current.email));

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: page_title.as_str(),
            active: "interests",
            runtime: false,
            if let Some(workout) = workout {
                lift_detail_head(workout: workout, share_text: share_text.as_deref().unwrap_or(""))
            } else {
                page_head(stamp: "lift", title: page_heading, lede: "")
            }
            // `relative` + `min-h` anchor the muscle-map aside: in the
            // wide-viewport gutter it is absolutely positioned out of the
            // flow (nothing pushes the set log down), and the minimum
            // height keeps a short workout's footer from sliding under it.
            <div
                class=(class!(
                    "relative",
                    "min-[90rem]:min-h-[24rem]" if involvement.is_some(),
                ))
            >
                <section class=(LIST) aria-label="Workout">
                    if let Some(workout) = workout {
                        workout_detail(workout: workout)
                    } else {
                        <div class=(EMPTY_ERROR_CARD)>
                            <p class=(EMPTY_TITLE)>"This lift did not load."</p>
                            <p class=(EMPTY_COPY)>
                                "Try the latest workout or the full archive again in a moment."
                            </p>
                            <a class=(EMPTY_RESET) href="/lifting">
                                "latest lift"
                            </a>
                        </div>
                    }
                </section>

                // The muscle map rides the right page gutter on viewports
                // wide enough to hold a 14.5rem panel beside the centered
                // 56rem shell (56 + 2×16.5 = 89rem, so 90rem clears it);
                // below that it flows here, after the whole set log.
                if let Some(involvement) = &involvement {
                    <aside
                        class="mt-12 pt-4 border-t border-hairline \
                             min-[90rem]:absolute min-[90rem]:left-full min-[90rem]:top-0 \
                             min-[90rem]:ml-8 min-[90rem]:w-[14.5rem] \
                             min-[90rem]:mt-0 min-[90rem]:pt-0 min-[90rem]:border-t-0"
                    >
                        <p class=(META_LABEL)>"muscles worked"</p>
                        <div class="mt-3">
                            muscles::muscle_map(involvement: involvement)
                        </div>
                    </aside>
                }

                if newer_lift_url.is_some() || older_lift_url.is_some() {
                    <nav
                        class="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center \
                             gap-3 mt-6 pt-4 border-t border-hairline"
                        aria-label="Workout navigation"
                    >
                        if let Some(href) = &newer_lift_url {
                            <a
                                class=(class!(LIFT_LINK, "justify-self-start"))
                                href=(href.as_str())
                            >
                                "← newer lift"
                            </a>
                        } else {
                            <span></span>
                        }
                        <a
                            class=(class!(LIFT_LINK, "justify-self-center"))
                            href="/lifting"
                        >
                            "latest lift"
                        </a>
                        if let Some(href) = &older_lift_url {
                            <a
                                class=(class!(LIFT_LINK, "justify-self-end"))
                                href=(href.as_str())
                            >
                                "see next lift →"
                            </a>
                        } else {
                            <span></span>
                        }
                    </nav>
                }

                if let Some(workout) = workout.filter(|_| can_delete) {
                    delete::delete_control(
                        path: workout.path.as_str(),
                        set_count: workout.sets.len(),
                    )
                    <script type="module" src=(delete::DELETE_LIFT_JS)></script>
                }

                if share_text.is_some() {
                    <script type="module" src=(share::SHARE_JS)></script>
                }
            </div>
            back_link(href: "/lifting", label: "latest lift")
        )
    }
}

#[component]
async fn lift_detail_head(workout: &fitness::Workout, share_text: &str) -> Result {
    let workout = WorkoutCard::from(workout);
    view! {
        <header class="rail-row mt-16">
            <p class="rail-stamp rail-stamp-label">"lift"</p>
            <div class="min-w-0">
                <h1 class="font-display text-4xl font-bold tracking-tight">(workout.title)</h1>
            </div>
        </header>
        <dl class="mt-4 space-y-1.5">
            <div class="rail-row">
                <dt class="rail-stamp rail-stamp-label">"date"</dt>
                <dd class="font-meta text-sm text-ink2">
                    <time
                        datetime=(workout.datetime.as_str())
                        title="Eastern start and end time from the workout archive"
                    >
                        (workout.date.as_str())
                    </time>
                </dd>
            </div>
            <div class="rail-row">
                <dt class="rail-stamp rail-stamp-label">"time"</dt>
                <dd class="font-meta text-sm text-ink2">
                    <time
                        datetime=(workout.datetime.as_str())
                        title="Eastern start and end time from the workout archive"
                    >
                        (workout.time_range.as_str())
                    </time>
                </dd>
            </div>
            <div class="rail-row">
                <dt class="rail-stamp rail-stamp-label">"duration"</dt>
                <dd class="font-meta text-sm text-ink2">(workout.duration.as_str())</dd>
            </div>
            <div class="rail-row">
                <dt class="rail-stamp rail-stamp-label">"sets"</dt>
                <dd class="font-meta text-sm text-ink2">(format!(
                    "{} {}", workout.set_count, plural(workout.set_count, "set", "sets"),
                ))</dd>
            </div>
            if !share_text.is_empty() {
                share::share_row(text: share_text)
            }
        </dl>
    }
}

#[component]
async fn workout_sheet(workout: &fitness::Workout, permalink: bool) -> Result {
    let workout = WorkoutCard::from(workout);
    let workout_link_label = format!("Open {} workout", workout.title);
    debug_assert!(permalink, "workout cards are only used in archive listings");
    view! {
        <article class="rail-row rail-row-top">
            <div class="rail-stamp sm:pt-[0.35rem]">
                <time
                    class="flex flex-col gap-[0.1rem]"
                    datetime=(workout.datetime.as_str())
                    title="Eastern start and end time from the workout archive"
                >
                    <span class="text-ink2">(workout.date.as_str())</span>
                    <span class="text-[0.68rem] text-muted">
                        (workout.time_range.as_str())
                    </span>
                </time>
            </div>
            <div class="min-w-0 p-4 bg-card border border-hairline sm:px-5 sm:py-[1.1rem]">
                <header
                    class="flex items-start justify-between gap-4 pb-3 border-b border-hairline"
                >
                    <h3 class="min-w-0 font-display text-xl font-semibold leading-[1.2]">
                        if permalink {
                            <a
                                class="decoration-oxide/45 decoration-1 \
                                     underline-offset-[0.18em] hover:text-oxide \
                                     hover:decoration-current focus-visible:text-oxide \
                                     focus-visible:decoration-current"
                                href=(workout.href.as_str())
                                aria-label=(workout_link_label.as_str())
                            >
                                (workout.title)
                            </a>
                        } else {
                            (workout.title)
                        }
                    </h3>
                    <p class=(META_SMALL)>
                        (format!(
                            "{} · {} {}", workout.duration, workout.set_count,
                            plural(workout.set_count, "set", "sets"),
                        ))
                        if workout.duration_suspicious {
                            " · "
                            <span
                                class="text-oxide"
                                title="This source workout was left running for at least four hours, or recorded as zero."
                            >
                                "timer outlier"
                            </span>
                        }
                    </p>
                </header>
                workout_body(workout: workout)
            </div>
        </article>
    }
}

#[component]
async fn workout_detail(workout: &fitness::Workout) -> Result {
    let workout = WorkoutCard::from(workout);
    view! {
        <article class="space-y-4">
            if let Some(description) = workout.description {
                <p class=(WORKOUT_NOTE)>(description)</p>
            }
            if let Some(notes) = workout.notes {
                <p class=(WORKOUT_NOTE)>(notes)</p>
            }
            for block in workout.blocks.iter() {
                workout_detail_block(block: block)
            }
        </article>
    }
}

#[component]
async fn workout_detail_block(block: &results::ExerciseBlock<'_>) -> Result {
    let groups = view! {
        <div class="space-y-4">
            for group in block.groups.iter() {
                <section class="rail-row rail-row-top">
                    <div class="rail-stamp sm:pt-[0.2rem]">
                        <h2 class="font-semibold text-ink2">(group.name)</h2>
                        <p>(format!(
                            "{} {}", group.rows.len(), plural(group.rows.len(), "set", "sets"),
                        ))</p>
                        <p>(format!("{} volume points", format_integer(group.volume_points)))</p>
                    </div>
                    <ol>
                        for row in group.rows.iter() {
                            set_row(row: row, divided: false)
                        }
                    </ol>
                </section>
            }
        </div>
    }?;
    if let Some(id) = block.superset_id {
        let label = format!("Superset {id}");
        view! { rail_group(label: label.as_str(), (groups)) }
    } else {
        Ok(groups)
    }
}

#[component]
async fn workout_body(workout: WorkoutCard<'_>) -> Result {
    view! {
        if let Some(description) = workout.description {
            <p class=(WORKOUT_NOTE)>(description)</p>
        }
        if let Some(notes) = workout.notes {
            <p class=(WORKOUT_NOTE)>(notes)</p>
        }
        for block in workout.blocks.iter() {
            workout_body_block(block: block)
        }
    }
}

#[component]
async fn workout_body_block(block: &results::ExerciseBlock<'_>) -> Result {
    let groups = view! {
        <div>
            for group in block.groups.iter() {
                <section class="mt-3">
                    <div class="flex items-end justify-between gap-[0.7rem] pb-[0.35rem]">
                        <h4 class="text-[0.9rem] font-semibold leading-[1.3]">
                            (group.name)
                        </h4>
                        <span class=(META_SMALL)>
                            (format!(
                                "{} {} · {} volume points", group.rows.len(), plural(group
                                .rows.len(), "set", "sets"), format_integer(group
                                .volume_points),
                            ))
                        </span>
                    </div>
                    <ol>
                        for row in group.rows.iter() {
                            set_row(row: row, divided: true)
                        }
                    </ol>
                </section>
            }
        </div>
    }?;
    if let Some(id) = block.superset_id {
        let label = format!("Superset {id}");
        view! { rail_group(class: "rail-group-compact", label: label.as_str(), (groups)) }
    } else {
        Ok(groups)
    }
}

#[component]
async fn set_row(row: &results::SetRow<'_>, divided: bool) -> Result {
    view! {
        <li
            class=(class!(
                "grid min-w-0 grid-cols-[2rem_minmax(0,1fr)] \
                 items-center gap-x-[0.55rem] gap-y-1 py-[0.38rem] \
                 sm:gap-2",
                "border-b border-hairline/70 last:border-b-0" if divided,
            ))
        >
            set_badge(set: row.set, effort_popover_id: row.effort_popover_id.as_str())
            <div class="min-w-0">
                <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span
                        class="font-meta text-[0.78rem] font-medium text-ink \
                             tabular-nums text-left"
                    >
                        (row.prescription.as_str())
                    </span>
                    if let Some(record) = &row.record {
                        <span class=(results::RECORD_PR)>(record.as_str())</span>
                    }
                </div>
                if !row.details.is_empty() {
                    <span
                        class="block mt-1 min-w-0 font-meta text-[0.65rem] \
                             leading-[1.45] text-muted"
                    >
                        (row.details.as_str())
                    </span>
                }
            </div>
            if let Some(note) = row.note {
                <span
                    class="col-[2/-1] font-meta text-[0.67rem] italic \
                         leading-[1.45] text-ink2"
                >
                    (note)
                </span>
            }
        </li>
    }
}

/// One checkbox chip in a filter facet. The visually hidden input drives the
/// pill through `peer-checked:`; `compact` picks the tighter height the
/// secondary facets use.
#[component]
async fn check_chip(name: &str, value: &str, label: &str, checked: bool, compact: bool) -> Result {
    view! {
        <label class="group relative cursor-pointer">
            <input
                class="peer sr-only"
                type="checkbox"
                name=(name)
                value=(value)
                checked=(checked)
            >
            <span
                class=(class!(
                    CHIP,
                    "min-h-[2.1rem] py-[0.42rem]" if compact else "min-h-[2.35rem] py-2",
                ))
            >
                (label)
            </span>
        </label>
    }
}
