//! The searchable fitness archive shared by `/fitness` and the phone pane.

use super::*;
use crate::app::interests::running;
use benjisponge::data::Data;

#[route(GET "/fitness/log")]
async fn legacy_fitness_log(cx: &Cx) -> Result {
    Err(redirect_permanent(with_raw_query(cx, FITNESS_PATH)).into())
}

#[component]
pub(super) async fn fitness_content(cx: &Cx, filters: &Filters) -> Result {
    let meta = interest("fitness");
    let can_edit = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let api_pairs = filters.api_pairs();
    let runs = running::load(app_context::<Data>(cx)).await;
    let (fitness_results, steps) = tokio::join!(
        fitness::load(
            app_context::<FitnessStore>(cx),
            &api_pairs,
            &runs.activities,
        ),
        archive::steps::load(app_context::<Data>(cx), archive::steps::HEATMAP_DAYS_LIMIT),
    );
    let fitness::FitnessPage {
        facets,
        activities,
        calendar,
        interruptions,
        focus,
        exercise_weights,
    } = fitness_results;
    if let Err(error) = &focus {
        eprintln!("fitness training focus failed: {error}");
    }
    if let Err(error) = &calendar {
        eprintln!("fitness calendar fetch failed: {error}");
    }
    let focus_summary = focus
        .as_ref()
        .ok()
        .filter(|summary| !summary.muscles.is_empty());
    if let Err(error) = &facets {
        eprintln!("fitness facets fetch failed: {error}");
    }
    if let Err(error) = &activities {
        eprintln!("fitness activity fetch failed: {error}");
    }
    if let Err(error) = &interruptions {
        eprintln!("fitness interruptions fetch failed: {error}");
    }
    if let Err(error) = &steps {
        eprintln!("fitness steps fetch failed: {error}");
    }
    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let run_days = activities
        .as_ref()
        .ok()
        .map(|page| heatmap::run_days(&page.matching_runs))
        .unwrap_or_default();
    let interruption_rows = interruptions.unwrap_or_default();
    let open_interruptions = interruptions::open_rows(&interruption_rows);
    let steps_unavailable = steps.is_err();
    let step_days = steps.unwrap_or_default();
    let day_link_query = filters.day_link_query();
    if let Ok(page) = &activities {
        let last_page = total_pages(page);
        if page.page > last_page {
            return Err(redirect(filters.page_url(last_page)).into());
        }
    }

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
    let result_summary = match &activities {
        Ok(page) => format!(
            "{} matching sets · {} lifts · {} runs · {} visible activities",
            format_integer(page.total_sets),
            format_integer(page.total_lifts),
            format_integer(page.total_runs),
            format_integer(page.activities.len() as u64),
        ),
        Err(error) => error
            .rejected_message()
            .map(|message| format!("A filter was rejected · {message}"))
            .unwrap_or_else(|| "Fitness database is unreachable.".to_string()),
    };
    let pager = activities
        .as_ref()
        .ok()
        .and_then(|page| make_pager(page, filters));
    let retry_url = filters.url(true);
    let log_items = activities
        .as_ref()
        .ok()
        .map(|page| interruptions::merge_log_items(&page.activities, &page.interruptions));

    view! {
        <header class="rail-row mt-16">
            <p class="rail-stamp rail-stamp-label">(meta.slug)</p>
            <div class="flex min-w-0 items-start justify-between gap-4">
                <div class="min-w-0">
                    <h1 class="font-display text-4xl font-bold tracking-tight">(meta.title)</h1>
                    <p class="mt-2 max-w-prose text-sm leading-relaxed text-ink2">
                        "Lifts, runs, steps, and the breaks between them—one training history."
                    </p>
                </div>
                <div class="flex items-center gap-3"><a href="/fitness/exercises" class="text-sm text-oxide underline">"Exercises"</a>
                if can_edit { home::log_launcher() }</div>
            </div>
        </header>
        <div class="relative min-[90rem]:min-h-[40rem]">
            <aside
                class="mt-8 pt-4 border-t border-hairline min-[90rem]:absolute \
                     min-[90rem]:left-full min-[90rem]:top-10 min-[90rem]:ml-8 \
                     min-[90rem]:w-[14.5rem] min-[90rem]:mt-0 min-[90rem]:pt-0 \
                     min-[90rem]:border-t-0"
                aria-label="Archive filters and muscle load"
            >
                filter_ui::filter_chrome(
                    filters: filters,
                    active: active_filters.as_slice(),
                    exercise_options: exercise_options.as_slice(),
                )
                if let Some(focus) = focus_summary {
                    <div class="hidden mt-8 border-t border-hairline pt-4 min-[90rem]:block">
                        training_focus::panel(focus: focus, heading_id: "training-focus-desktop")
                    </div>
                }
            </aside>
            if let Some(focus) = focus_summary {
                <details class="group mt-6 min-[90rem]:hidden">
                    <summary class="flex min-h-11 cursor-pointer list-none items-center \
                         justify-between gap-4 rounded-sm border border-hairline px-4 py-2 \
                         font-meta text-xs text-oxide hover:border-oxide \
                         after:content-['+'] group-open:after:content-['−'] \
                         focus-visible:outline-solid focus-visible:outline-2 \
                         focus-visible:outline-oxide [&::-webkit-details-marker]:hidden">
                        "Muscle load + next focus"
                    </summary>
                    <div class="mt-3 rounded-sm border border-hairline bg-card p-4">
                        training_focus::panel(focus: focus, heading_id: "training-focus-mobile")
                    </div>
                </details>
            }
            if let Some(days) = calendar_days {
                rail_section(
                    class: "mt-10",
                    stamp: "volume",
                    <div id="volume">
                        heatmap::calendar_heatmap(
                            days: days,
                            runs: run_days.clone(),
                            steps: step_days.clone(),
                            steps_unavailable: steps_unavailable,
                            link_query: day_link_query,
                            filtered: !active_filters.is_empty(),
                            interruptions: interruption_rows.clone()
                        )
                    </div>
                )
            }
            if !runs.live {
                <p class="mt-3 font-meta text-xs text-muted">
                    "Runs are unavailable right now; lift matches and interruptions are still shown."
                </p>
            }

            if !open_interruptions.is_empty() {
                rail_section(
                    class: "mt-12",
                    stamp: "notes",
                    interruptions::open_panel(rows: open_interruptions.as_slice(), can_edit: can_edit)
                )
            }

            rail_section(
                class: "mt-12",
                stamp: "activity",
                <header id="set-log">
                    filter_ui::log_pager(
                        filters: filters,
                        pager: pager.as_ref(),
                        result_summary: result_summary.as_str()
                    )
                </header>
            )

            <section class=(LIST) aria-label="Filtered fitness activities">
                if let Err(error) = &activities {
                    <div class=(EMPTY_ERROR_CARD)>
                        if let Some(message) = error.rejected_message() {
                            <p class=(EMPTY_TITLE)>
                                "That filter combination is not valid."
                            </p>
                            <p class=(EMPTY_COPY)>(message)</p>
                            <a class=(EMPTY_RESET) href="/fitness#set-log">
                                "clear every filter"
                            </a>
                        } else {
                            <p class=(EMPTY_TITLE)>
                                "The fitness log did not load."
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
                if let Ok(page) = &activities
                    && page.activities.is_empty()
                    && page.interruptions.is_empty()
                {
                    <div class=(EMPTY_CARD)>
                        <p class=(EMPTY_TITLE)>
                            if page.total_activities() > 0 {
                                "This page is empty."
                            } else {
                                "No matching activities."
                            }
                        </p>
                        <p class=(EMPTY_COPY)>
                            if page.total_activities() > 0 {
                                "Try a previous page."
                            } else {
                                "Loosen a movement, date, or filter and the log will reappear."
                            }
                        </p>
                        <a class=(EMPTY_RESET) href="/fitness#set-log">
                            "clear every filter"
                        </a>
                    </div>
                }
                if let Some(items) = &log_items {
                    for item in items.iter() {
                        if let interruptions::LogItem::Activity(activity) = item {
                            if let fitness::LogActivity::Lift(lift) = activity {
                                compact::workout_log_entry(workout: &lift.workout, weights: &exercise_weights)
                            }
                            if let fitness::LogActivity::Run(run) = activity {
                                running::activity_card(activity: &run.activity)
                            }
                        }
                        if let interruptions::LogItem::Interruption(row) = item {
                            interruptions::log_entry(row: row, can_edit: can_edit)
                        }
                    }
                }
            </section>
            if let Some(pager) = &pager {
                rail_section(
                    class: "mt-6",
                    stamp: "",
                    filter_ui::log_navigation(filters: filters, pager: pager)
                )
            }
        </div>

    }
}

#[route(GET "/lifting/log")]
async fn legacy_lifting_log(cx: &Cx) -> Result {
    Err(redirect_permanent(with_raw_query(cx, LOG_PATH)).into())
}
