//! `/fitness` — the unified fitness landing page.

use super::*;

const LOG_LAUNCHER_JS: Asset = asset!("./log-launcher.js");

#[page("/fitness")]
async fn lifting(cx: &Cx) -> Result {
    // Filtered archive links remain useful; the archive itself now lives at
    // `/fitness/log`.
    if let Some(query) = uri(cx).query() {
        return Err(redirect(&format!("{LOG_PATH}?{query}")).into());
    }

    let meta = interest("fitness");
    let can_log = viewer(cx).is_some_and(|current| is_admin(&current.email));
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "",
            runtime: true,
            fitness_pwa: true,
            fitness_home_content()
            if can_log {
                log_dialogs()
            }
            back_link(href: "/", label: "~")
            <script type="module" src=(crate::app::interests::running::PWA_JS)></script>
        )
    }
}

#[route(GET "/lifting")]
async fn legacy_lifting(cx: &Cx) -> Result {
    let target = if uri(cx).query().is_some() {
        with_raw_query(cx, LOG_PATH)
    } else {
        FITNESS_PATH.to_string()
    };
    Err(redirect_permanent(&target).into())
}

#[route(GET "/interests/lifting")]
async fn legacy_interest_lifting() -> Result {
    Err(redirect_permanent(FITNESS_PATH).into())
}

/// The landing page's body: heatmap, open interruptions, training focus, and
/// the most recent lift. The standalone `/fitness` page wraps it in the
/// shell above; the home deck renders it as the phone's fitness pane. Home
/// keeps its own 60-second edge TTL, so the pane can trail the archive by up
/// to a minute where this page itself stays no-store.
#[component]
pub(crate) async fn fitness_home_content(cx: &Cx) -> Result {
    let meta = interest("fitness");
    let can_log = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let (lifting_home, runs) = tokio::join!(
        fitness::load_home(app_context::<FitnessStore>(cx)),
        crate::app::interests::running::load(app_context::<benjisponge::data::Data>(cx)),
    );
    let (calendar, latest, focus, interruptions) = lifting_home;
    if let Err(error) = &calendar {
        eprintln!("fitness calendar fetch failed: {error}");
    }
    if let Err(error) = &latest {
        eprintln!("fitness latest workout fetch failed: {error}");
    }
    if let Err(error) = &focus {
        eprintln!("fitness training focus failed: {error}");
    }
    if let Err(error) = &interruptions {
        eprintln!("fitness interruptions fetch failed: {error}");
    }
    if !runs.live {
        eprintln!("fitness running activity fetch failed");
    }

    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let run_days = heatmap::run_days(&runs.activities);
    let interruption_rows = interruptions.unwrap_or_default();
    let open_interruptions = interruptions::open_rows(&interruption_rows);
    let focus_summary = focus
        .as_ref()
        .ok()
        .filter(|summary| !summary.muscles.is_empty());
    let latest_error = latest.as_ref().err();
    let latest_workout = latest
        .as_ref()
        .ok()
        .and_then(|detail| detail.workout.as_ref());
    let latest_run = runs.activities.first();
    let run_first = match (latest_workout, latest_run) {
        (Some(workout), Some(activity)) => {
            crate::app::interests::running::start_time_seconds(activity)
                > workout_start_seconds(workout)
        }
        (None, Some(_)) => true,
        _ => false,
    };

    view! {
            <header class="rail-row mt-16">
                <p class="rail-stamp rail-stamp-label">(meta.slug)</p>
                <div class="flex min-w-0 items-start justify-between gap-4">
                    <div class="min-w-0">
                        <h1 class="font-display text-4xl font-bold tracking-tight">
                            (meta.title)
                        </h1>
                        <p class="mt-2 max-w-prose text-sm leading-relaxed text-ink2">
                            "Lifts, runs, and the breaks between them—one training history."
                        </p>
                    </div>
                    if can_log {
                        log_launcher()
                    }
                </div>
            </header>
            <div
                class=(class!(
                    "relative",
                    "min-[90rem]:min-h-[40rem]" if focus_summary.is_some(),
                ))
            >
                if let Some(focus) = focus_summary {
                    <details class="group mt-8 min-[90rem]:hidden">
                        <summary
                            class="flex min-h-11 w-full cursor-pointer list-none items-center \
                                 justify-between gap-4 rounded-[0.2rem] border border-oxide px-4 \
                                 py-2.5 font-meta text-xs text-oxide \
                                 after:text-base after:leading-none after:content-['+'] \
                                 group-open:after:content-['−'] \
                                 group-open:bg-oxide group-open:text-card \
                                 hover:bg-oxide hover:text-card \
                                 focus-visible:outline-solid focus-visible:outline-2 \
                                 focus-visible:outline-oxide focus-visible:outline-offset-2 \
                                 [&::-webkit-details-marker]:hidden"
                        >
                            <span class="group-open:hidden">
                                "show muscle load + next focus"
                            </span>
                            <span class="hidden group-open:inline">
                                "hide muscle load + next focus"
                            </span>
                        </summary>
                        <div class="mt-3 rounded-[0.2rem] border border-hairline bg-card p-4">
                            training_focus::panel(
                                focus: focus,
                                heading_id: "training-focus-mobile"
                            )
                        </div>
                    </details>
                    <aside
                        class="hidden border-t border-hairline pt-4 \
                             min-[90rem]:absolute min-[90rem]:left-full min-[90rem]:top-10 \
                             min-[90rem]:ml-8 min-[90rem]:block min-[90rem]:w-[14.5rem]"
                        aria-label="Muscle load and next focus"
                    >
                        training_focus::panel(
                            focus: focus,
                            heading_id: "training-focus-desktop"
                        )
                    </aside>
                }

                rail_section(
                    class: "mt-10",
                    stamp: "volume",
                    <header id="volume">
                        if let Some(days) = calendar_days {
                            heatmap::calendar_heatmap(
                                days: days,
                                runs: run_days.clone(),
                                interruptions: interruption_rows.clone()
                            )
                        } else {
                            <section class="p-4 bg-card border border-hairline">
                                <p class=(EMPTY_COPY)>
                                    "Daily volume is unavailable right now."
                                </p>
                            </section>
                        }
                        if !runs.live {
                            <p class="mt-3 font-meta text-xs text-muted">
                                "Running activities are unavailable right now; lifting and \
                                 interruption history are still shown."
                            </p>
                        }
                    </header>
                )

                if !open_interruptions.is_empty() {
                    rail_section(
                        class: "mt-12",
                        stamp: "notes",
                        interruptions::open_panel(
                            rows: open_interruptions.as_slice(),
                            can_edit: can_log
                        )
                    )
                }

                rail_section(
                    class: "mt-12",
                    stamp: "recent",
                    <header class="flex items-end justify-between gap-4" id="set-log">
                        <div>
                            <h2 class="font-display text-2xl font-semibold">
                                "Recent training"
                            </h2>
                        </div>
                        <a class=(class!(LIFT_LINK, "flex-none")) href=(LOG_PATH)>
                            "search full log →"
                        </a>
                    </header>
                )

                <section class=(LIST) aria-label="Recent fitness activities">
                    if run_first
                        && let Some(activity) = latest_run
                    {
                        crate::app::interests::running::activity_card(activity: activity)
                    }
                    if let Some(workout) = latest_workout {
                        workout_sheet(workout: workout, permalink: true)
                    }
                    if !run_first
                        && let Some(activity) = latest_run
                    {
                        crate::app::interests::running::activity_card(activity: activity)
                    }
                    if latest_error.is_some() {
                        <div class=(EMPTY_ERROR_CARD)>
                            <p class=(EMPTY_TITLE)>
                                "The latest lift did not load."
                            </p>
                            <p class=(EMPTY_COPY)>
                                "Try the workout archive again in a moment."
                            </p>
                            <a class=(EMPTY_RESET) href="/fitness#set-log">
                                "retry"
                            </a>
                        </div>
                    }
                    if !runs.live {
                        <div class=(EMPTY_ERROR_CARD)>
                            <p class=(EMPTY_TITLE)>"The latest run did not load."</p>
                            <p class=(EMPTY_COPY)>
                                "The lifting history is intact. Try the combined log again in a moment."
                            </p>
                        </div>
                    }
                    if latest_error.is_none()
                        && latest_workout.is_none()
                        && runs.live
                        && latest_run.is_none()
                    {
                        <div class=(EMPTY_CARD)>
                            <p class=(EMPTY_TITLE)>"No training logged yet."</p>
                            <p class=(EMPTY_COPY)>
                                "Log a lift or run to start this history."
                            </p>
                        </div>
                    }
                </section>
            </div>
    }
}

/// One owner action opens a small training-ledger menu. Each item is a real
/// anchor for the no-script path and is progressively enhanced by the shared
/// native-dialog driver in `components::modal`.
#[component]
pub(crate) async fn log_launcher() -> Result {
    view! {
        <details
            class="group relative mt-1 flex-none"
            name="fitness-log-launcher"
            data-fitness-log-launcher=""
        >
            <summary
                class="flex min-h-11 cursor-pointer list-none items-center gap-2 rounded-sm \
                       border border-oxide bg-oxide px-3.5 py-2 font-meta text-xs \
                       font-semibold text-card hover:bg-oxide-hot hover:text-white \
                       focus-visible:outline-solid focus-visible:outline-2 \
                       focus-visible:outline-oxide focus-visible:outline-offset-2 \
                       [&::-webkit-details-marker]:hidden"
            >
                <span>"log"</span>
                <span
                    class="text-sm leading-none transition-transform group-open:rotate-45"
                    aria-hidden="true"
                >"+"</span>
            </summary>
            <nav
                class="absolute right-0 top-[calc(100%+0.45rem)] z-30 w-56 overflow-hidden \
                       rounded-sm border border-hairline bg-page shadow-xl"
                aria-label="Choose what to log"
            >
                <a
                    href="#fitness-lift-dialog"
                    data-modal-open="fitness-lift-dialog"
                    class="group/item flex min-h-11 items-center gap-3 border-b border-hairline \
                           px-3 py-2.5 text-ink no-underline hover:bg-oxide/8 \
                           focus-visible:bg-oxide/8 focus-visible:outline-solid \
                           focus-visible:outline-2 focus-visible:outline-oxide \
                           focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-oxide font-meta text-[0.65rem] font-semibold text-oxide"
                        aria-hidden="true"
                    >"L"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">"Lift"</span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "Lyfta workout text"
                        </span>
                    </span>
                </a>
                <a
                    href="#fitness-run-dialog"
                    data-modal-open="fitness-run-dialog"
                    class="group/item flex min-h-11 items-center gap-3 border-b border-hairline \
                           px-3 py-2.5 text-ink no-underline hover:bg-patina/8 \
                           focus-visible:bg-patina/8 focus-visible:outline-solid \
                           focus-visible:outline-2 focus-visible:outline-patina \
                           focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-patina font-meta text-[0.65rem] font-semibold text-patina"
                        aria-hidden="true"
                    >"R"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">"Run"</span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "distance + time"
                        </span>
                    </span>
                </a>
                <a
                    href="#fitness-interruption-dialog"
                    data-modal-open="fitness-interruption-dialog"
                    class="group/item flex min-h-11 items-center gap-3 px-3 py-2.5 text-ink \
                           no-underline hover:bg-brass/8 focus-visible:bg-brass/8 \
                           focus-visible:outline-solid focus-visible:outline-2 \
                           focus-visible:outline-brass focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-brass font-meta text-[0.65rem] font-semibold text-brass"
                        aria-hidden="true"
                    >"—"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">
                            "Interruption"
                        </span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "sickness, travel, rest"
                        </span>
                    </span>
                </a>
            </nav>
        </details>
    }
}

#[component]
pub(crate) async fn log_dialogs() -> Result {
    view! {
        workout_upload_dialog()
        crate::app::interests::running::manual_run_dialog()
        interruptions::create_dialog()

        // Closed dialogs are display:none without the driver. Make the three
        // real forms ordinary in-flow sections when scripting is disabled, so
        // each launcher's fragment link remains useful.
        <noscript>
            <style>
                "#fitness-lift-dialog,#fitness-run-dialog,#fitness-interruption-dialog{display:block;position:static;width:100%;max-height:none;margin:1rem 0 0;overflow:visible;box-shadow:none}#fitness-lift-dialog .modal-panel,#fitness-run-dialog .modal-panel,#fitness-interruption-dialog .modal-panel{max-height:none}#fitness-lift-dialog .modal-label,#fitness-run-dialog .modal-label,#fitness-interruption-dialog .modal-label,#fitness-lift-dialog .modal-close,#fitness-run-dialog .modal-close,#fitness-interruption-dialog .modal-close{display:none}"
            </style>
        </noscript>
        <script type="module" src=(LOG_LAUNCHER_JS)></script>
    }
}

fn workout_start_seconds(workout: &fitness::Workout) -> i64 {
    workout
        .id
        .strip_prefix("fitness:")
        .map(|value| value.replacen('T', " ", 1))
        .and_then(|value| archive::eastern::utc_timestamp(&value).ok())
        .map_or(0, |timestamp| timestamp.as_second())
}
