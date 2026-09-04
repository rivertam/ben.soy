//! `/fitness/log` — the searchable, filterable fitness activity log.

use super::*;
use crate::app::interests::running;
use benjisponge::data::Data;

const ANONYMOUS_LOG_CACHE: &str = "public, max-age=0, s-maxage=60";

#[page("/fitness/log")]
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
        return Err(redirect(filters.url(false)).into());
    }

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
    let (facets, activities, calendar, interruptions) = fitness_results;
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
        .and_then(|page| make_pager(page, &filters));
    let retry_url = filters.url(true);
    let log_items = activities
        .as_ref()
        .ok()
        .map(|page| interruptions::merge_log_items(&page.activities, &page.interruptions));

    view! {
        // Anonymous archive reads can trail a sync by one minute at the CDN.
        // Browser caches stay cold, and response_layer.rs replaces this with
        // private, no-store whenever the viewer cookie is present.
        ((header::CACHE_CONTROL, HeaderValue::from_static(ANONYMOUS_LOG_CACHE)))
        shell(
            page: meta.title,
            active: "",
            runtime: true,
            fitness_pwa: true,
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            <div class="relative min-[90rem]:min-h-[28rem]">
                <aside
                    class="mt-8 pt-4 border-t border-hairline \
                         min-[90rem]:absolute min-[90rem]:left-full min-[90rem]:top-0 \
                         min-[90rem]:ml-8 min-[90rem]:w-[14.5rem] \
                         min-[90rem]:mt-0 min-[90rem]:pt-0 min-[90rem]:border-t-0"
                    aria-label="Archive filters"
                >
                    filter_ui::filter_chrome(
                        filters: &filters,
                        active: active_filters.as_slice(),
                        exercise_options: exercise_options.as_slice(),
                    )
                </aside>

                if let Some(days) = calendar_days {
                    rail_section(
                        class: "mt-10 min-[90rem]:mt-10",
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

                rail_section(
                    class: "mt-12",
                    stamp: "activity",
                    <header id="set-log">
                        filter_ui::log_pager(
                            filters: &filters,
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
                                <a class=(EMPTY_RESET) href="/fitness/log#set-log">
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
                            <a class=(EMPTY_RESET) href="/fitness/log#set-log">
                                "clear every filter"
                            </a>
                        </div>
                    }
                    if let Some(items) = &log_items {
                        for item in items.iter() {
                            if let interruptions::LogItem::Activity(activity) = item {
                                if let fitness::LogActivity::Lift(lift) = activity {
                                    workout_sheet(workout: &lift.workout, permalink: true)
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
            </div>
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/lifting/log")]
async fn legacy_lifting_log(cx: &Cx) -> Result {
    Err(redirect_permanent(with_raw_query(cx, LOG_PATH)).into())
}

#[cfg(test)]
mod tests {
    use super::ANONYMOUS_LOG_CACHE;

    #[test]
    fn anonymous_log_cache_is_browser_cold_and_edge_short_lived() {
        assert_eq!(
            ANONYMOUS_LOG_CACHE.split(", ").collect::<Vec<_>>(),
            ["public", "max-age=0", "s-maxage=60"]
        );
    }
}
