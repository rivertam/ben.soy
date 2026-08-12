//! Fitness feature map and cross-layer invariants: `docs/fitness.md`.

pub(crate) mod archive;
mod badge;
mod data;
mod delete;
mod exercise;
mod filter_ui;
mod filters;
mod format;
mod heatmap;
mod home;
mod interruptions;
mod log;
mod muscle_seed;
mod muscle_taxonomy;
mod muscles;
mod results;
mod share;
mod taxonomy;
mod training_focus;

use self::archive::store::FitnessStore;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        HeaderValue, error::not_found, error::redirect, header, page, parse_query_params,
        path_param, uri,
    },
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
    filters::{Filters, LOG_PATH},
    format::{format_integer, plural},
    results::{WorkoutCard, make_pager, total_pages, workout_url},
};

pub(super) const AUTO_FILTER_JS: Asset = asset!("./auto-filter.js");
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

pub(super) const RESULT_COUNT: &str =
    "mt-[0.3rem] font-meta text-[0.72rem] leading-[1.5] text-muted";
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
pub(super) const PAGE_LINK: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-ink2 border border-hairline \
     hover:text-oxide hover:border-oxide focus-visible:text-oxide focus-visible:border-oxide";
pub(super) const PAGE_DISABLED: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-muted border border-transparent";
pub(super) const PAGE_CURRENT: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-card bg-ink border border-ink";
pub(super) const PAGE_GAP: &str = "inline-flex min-w-[2.1rem] min-h-[2.1rem] items-center \
     justify-center px-[0.55rem] py-[0.35rem] text-muted";
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
    let involvement = loaded.as_ref().ok().and_then(|(detail, weights)| {
        detail
            .workout
            .as_ref()
            .map(|workout| muscles::workout_involvement(workout, weights))
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
            active: "",
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
pub(super) async fn workout_sheet(workout: &fitness::Workout, permalink: bool) -> Result {
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
                        <h2 class="font-semibold text-ink2">
                            <a
                                class="hover:text-oxide hover:underline \
                                     hover:decoration-oxide/45 underline-offset-[0.2em]"
                                href=(exercise::page_url(group.name))
                            >
                                (group.name)
                            </a>
                        </h2>
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
                            <a
                                class="hover:text-oxide hover:underline \
                                     hover:decoration-oxide/45 underline-offset-[0.2em]"
                                href=(exercise::page_url(group.name))
                            >
                                (group.name)
                            </a>
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
