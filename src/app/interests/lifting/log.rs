//! `/lifting/log` — the searchable, filterable set log.

use super::*;

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
    let can_edit = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let api_pairs = filters.api_pairs();
    let (facets, sets, calendar, interruptions, page_interruptions) =
        fitness::load(app_context::<FitnessStore>(cx), &api_pairs).await;
    if let Err(error) = &facets {
        eprintln!("fitness facets fetch failed: {error}");
    }
    if let Err(error) = &sets {
        eprintln!("fitness sets fetch failed: {error}");
    }
    if let Err(error) = &interruptions {
        eprintln!("fitness interruptions fetch failed: {error}");
    }
    if let Err(error) = &page_interruptions {
        eprintln!("fitness log interruptions fetch failed: {error}");
    }
    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let interruption_rows = interruptions.unwrap_or_default();
    let page_interruption_rows = page_interruptions.unwrap_or_default();
    let day_link_query = filters.day_link_query();
    if let Ok(page) = &sets {
        let last_page = total_pages(page);
        if page.page > last_page {
            return Err(redirect(&filters.page_url(last_page)).into());
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
    let log_items = sets
        .as_ref()
        .ok()
        .map(|page| interruptions::merge_log_items(&page.workouts, &page_interruption_rows));

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "",
            runtime: true,
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
                                link_query: day_link_query,
                                filtered: !active_filters.is_empty(),
                                interruptions: interruption_rows.clone()
                            )
                        </div>
                    )
                }

                rail_section(
                    class: "mt-12",
                    stamp: "sets",
                    <header id="set-log">
                        filter_ui::log_pager(
                            filters: &filters,
                            pager: pager.as_ref(),
                            result_summary: result_summary.as_str()
                        )
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
                    if let Ok(page) = &sets
                        && page.workouts.is_empty()
                        && page_interruption_rows.is_empty()
                    {
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
                                    "Loosen a movement, date, or filter and the log will reappear."
                                }
                            </p>
                            <a class=(EMPTY_RESET) href="/lifting/log#set-log">
                                "clear every filter"
                            </a>
                        </div>
                    }
                    if let Some(items) = &log_items {
                        for item in items.iter() {
                            if let interruptions::LogItem::Workout(workout) = item {
                                workout_sheet(workout: workout, permalink: true)
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
