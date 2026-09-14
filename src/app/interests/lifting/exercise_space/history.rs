//! Set history stays server-rendered, including asynchronously loaded pages.
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        HeaderValue, StatusCode,
        error::redirect_permanent,
        header, query_params,
        response::{IntoResponse, Response},
        route,
    },
    view::{component, view},
};

use super::super::{
    archive::{filters::Filters, store::FitnessStore},
    badge::set_badge,
    results::WorkoutCard,
};
use super::{SpaceQuery, page_url};

const PAGE_SIZE: usize = 4;

fn history_url(name: &str, page: usize) -> String {
    format!("{}&history_page={page}#space-history", page_url(Some(name)))
}

#[route(GET "/fitness/space/history")]
async fn legacy_history(cx: &Cx) -> Result {
    Err(redirect_permanent(super::super::with_raw_query(
        cx,
        "/fitness/exercises/history",
    ))
    .into())
}

#[route(GET "/fitness/exercises/history")]
async fn history_endpoint(cx: &Cx) -> Result<Response> {
    let query = query_params::<SpaceQuery>(cx)?;
    let name = query.exercise.as_deref().filter(|name| !name.is_empty());
    let page = query.history_page.unwrap_or(1);
    if name.is_some_and(|name| name.len() > 200) || !(1..=10_000).contains(&page) {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CACHE_CONTROL, "no-store")],
            "Invalid history request.",
        )
            .into_response(cx);
    }
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        panel(name: name, page_number: page)
    }?
    .into_response(cx)
}

#[component]
pub(super) async fn panel(cx: &Cx, name: Option<&str>, page_number: usize) -> Result {
    let loaded = if let Some(name) = name {
        app_context::<FitnessStore>(cx)
            .snapshot()
            .await
            .map(|snapshot| {
                let canonical = snapshot
                    .canonical_exercise_name(name)
                    .unwrap_or_else(|| name.to_string());
                snapshot.sets_page(&Filters {
                    exercise: Some(canonical),
                    page: page_number,
                    per_page: PAGE_SIZE,
                    ..Filters::default()
                })
            })
            .map(Some)
    } else {
        Ok(None)
    };
    view! {
        <section id="space-history" class="space-history" data-space-history="" aria-label="Exercise set history" aria-live="polite" hidden=(name.is_none())>
            if let Some(name) = name {
                <header><div><p class="space-detail__eyebrow">"Set history"</p><h3>(name)</h3></div></header>
                if let Ok(Some(history)) = &loaded {
                    <p class="space-note">(format!("{} sets across {} workouts", history.total_sets, history.total_workouts))</p>
                    if history.workouts.is_empty() { <p class="space-history__empty">(if history.total_sets == 0 { "No sets logged here yet." } else { "No workouts on this page." })</p> }
                    for workout in &history.workouts {
                        session(card: WorkoutCard::from(workout))
                    }
                    <nav class="space-history__paging" aria-label="Set history pages">
                        if page_number > 1 { <a href=(history_url(name, page_number - 1)) data-space-history-page=(page_number - 1)>"← Newer workouts"</a> }
                        if (page_number * PAGE_SIZE) < history.total_workouts as usize { <a href=(history_url(name, page_number + 1)) data-space-history-page=(page_number + 1)>"Older workouts →"</a> }
                    </nav>
                } else {
                    <p class="space-history__empty">"Set history could not load. Try again in a moment."</p>
                    <a class="space-exercise-link" href=(history_url(name, page_number)) data-space-history-page=(page_number)>"Retry history"</a>
                }
            }
        </section>
    }
}

#[component]
async fn session(card: WorkoutCard<'_>) -> Result {
    view! {
        <article class="space-history__session">
            <h4><a href=(card.href.as_str())><time datetime=(card.datetime.as_str())>(card.date.as_str())</time><span>(card.title)</span></a></h4>
            <ul>
                for row in card.blocks.iter().flat_map(|block| &block.groups).flat_map(|group| &group.rows) {
                    <li>
                        set_badge(set: row.set, working_number: row.working_number, effort_popover_id: row.effort_popover_id.as_str())
                        <div><strong>(row.prescription.as_str())</strong>
                            if !row.details.is_empty() { <span class="space-note">(row.details.as_str())</span> }
                            if let Some(note) = row.note { <span class="space-note">(note)</span> }
                        </div>
                        if let Some(record) = &row.record { <span class="space-history__record">(record.as_str())</span> }
                    </li>
                }
            </ul>
        </article>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_paging_keeps_the_exercise_and_encodes_its_name() {
        assert_eq!(
            history_url("Press & pull", 2),
            "/fitness/exercises?exercise=Press%20%26%20pull&history_page=2#space-history"
        );
    }
}
