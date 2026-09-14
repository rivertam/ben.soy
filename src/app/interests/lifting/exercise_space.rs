//! A display projection of the live catalog, with exact 28-muscle neighbors.
mod embedding;
mod history;
mod load;
mod projection;

use std::collections::BTreeMap;

use fitness_entry_core::{
    ExerciseGuide, GuideConfig,
    muscle_load::{BASELINE_WEEKS, dot_product},
};
use serde::Serialize;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        HeaderValue, StatusCode,
        content::Json,
        error::redirect_permanent,
        header, query_params,
        response::{IntoResponse, Response},
        route,
    },
    view::{component, view},
};

use super::{
    archive::store::FitnessStore, entry, exercise, muscle_taxonomy, training_focus::TrainingFocus,
};
use crate::{components::shell, util::urlencode};

const SPACE_JS: Asset = asset!("./exercise-space.js");
pub(super) const PATH: &str = "/fitness/exercises";
const NEIGHBORS: usize = 6;
const SCORE_SCALE: f64 = (BASELINE_WEEKS * 100 * 100) as f64;

pub(super) fn page_url(name: Option<&str>) -> String {
    name.map_or_else(
        || PATH.to_string(),
        |name| format!("{PATH}?exercise={}", urlencode(name)),
    )
}

#[query_params(error = redirect("?"))]
struct SpaceQuery {
    exercise: Option<String>,
    q: Option<String>,
    history_page: Option<usize>,
}

#[derive(Default, Serialize)]
struct SearchResults {
    matches: Vec<SearchMatch>,
    total: usize,
}

#[derive(Serialize)]
struct SearchMatch {
    name: String,
    url: String,
    meta: String,
}

fn has_profile(item: &ExerciseGuide) -> bool {
    !item.movements.iter().any(|movement| movement == "cardio")
        && item
            .muscles
            .iter()
            .any(|(id, ratio)| *ratio > 0 && muscle_taxonomy::canonical_muscle(id).is_some())
}

fn search_catalog(guide: &GuideConfig, query: &str) -> SearchResults {
    if !query.chars().any(char::is_alphanumeric) {
        return SearchResults::default();
    }
    let found: Vec<_> = fitness_entry_core::search_exercises(&guide.exercises, query)
        .into_iter()
        .collect();
    SearchResults {
        total: found.len(),
        matches: found
            .into_iter()
            .take(12)
            .map(|item| SearchMatch {
                name: item.name.clone(),
                url: page_url(Some(&item.name)),
                meta: item.picker_meta.clone(),
            })
            .collect(),
    }
}

#[route(GET "/fitness/exercises/search")]
async fn search_endpoint(cx: &Cx) -> Result<Response> {
    let query = query_params::<SpaceQuery>(cx)?;
    let query = query.q.as_deref().unwrap_or("").trim();
    let headers = [(header::CACHE_CONTROL, "no-store")];
    if query.len() > 200 {
        return (StatusCode::BAD_REQUEST, headers, "Search is too long.").into_response(cx);
    }
    let Ok(guide) = entry::entry_guide(app_context::<FitnessStore>(cx)).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            headers,
            "Exercise search could not load.",
        )
            .into_response(cx);
    };
    (headers, Json(search_catalog(&guide, query))).into_response(cx)
}

#[derive(Serialize)]
struct Space {
    exercises: Vec<SpaceExercise>,
    points: Vec<SpacePoint>,
    muscles: Vec<SpaceMuscle>,
    landmarks: Vec<Landmark>,
    fit_order: Vec<usize>,
    selected: Option<usize>,
    omitted: usize,
    embedding: embedding::Embedding,
    linear_retained: f64,
}

#[derive(Serialize)]
struct SpaceExercise {
    name: String,
    url: String,
    map_url: String,
    point: Option<usize>,
    load_delta_centi: Vec<u32>,
    score: f64,
    neighbors: Vec<Neighbor>,
}

#[derive(Serialize)]
struct SpacePoint {
    position: [f64; 3],
    members: Vec<usize>,
}

#[derive(Serialize)]
struct SpaceMuscle {
    label: &'static str,
    current_centi: u32,
    usual_centi: f64,
    target_centi: Option<u32>,
}

#[derive(Serialize)]
struct Neighbor {
    index: usize,
    similarity: f64,
}

#[derive(Serialize)]
struct Landmark {
    label: &'static str,
    points: Vec<usize>,
}

fn build(guide: &GuideConfig, focus: &TrainingFocus, requested: Option<&str>) -> Space {
    let vocabulary: Vec<_> = muscle_taxonomy::muscles().collect();
    let mut profiles: BTreeMap<(Vec<u32>, Vec<String>), usize> = BTreeMap::new();
    let mut points: Vec<SpacePoint> = Vec::new();
    let mut normalized = Vec::new();
    let mut patterns = Vec::new();
    let mut exercises = Vec::new();
    let mut selected = None;
    let mut workouts = Vec::new();
    for item in &guide.exercises {
        let ratios: Vec<u32> = vocabulary
            .iter()
            .map(|(muscle, _)| {
                item.muscles
                    .iter()
                    .find(|(id, _)| id == muscle)
                    .map_or(0, |(_, ratio)| *ratio)
            })
            .collect();
        let divisor = ratios.iter().copied().fold(0, gcd).max(1);
        let key: Vec<u32> = ratios.iter().map(|ratio| ratio / divisor).collect();
        let movement = embedding::patterns(item);
        let index = exercises.len();
        let point = has_profile(item).then(|| {
            *profiles.entry((key, movement.clone())).or_insert_with(|| {
                normalized.push(projection::normalize(&ratios));
                patterns.push(movement);
                points.push(SpacePoint {
                    position: [0.0; 3],
                    members: Vec::new(),
                });
                points.len() - 1
            })
        });
        if let Some(point) = point {
            points[point].members.push(index);
        }
        if requested.is_some_and(|name| {
            item.name.eq_ignore_ascii_case(name)
                || item
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(name))
        }) {
            selected = Some(index);
        }
        workouts.push(item.workout_count);
        exercises.push(SpaceExercise {
            name: item.name.clone(),
            url: exercise::details_url(&item.name),
            map_url: page_url(Some(&item.name)),
            point,
            load_delta_centi: ratios
                .iter()
                .map(|ratio| load::preview_centi(*ratio))
                .collect(),
            score: dot_product(&guide.muscle_needs, &item.muscles) as f64 / SCORE_SCALE,
            neighbors: Vec::new(),
        });
    }
    // Fit unique directions so adding another equipment variant does not
    // move the map or overweight a common exercise's identical profile.
    let projection = projection::project(&normalized);
    for (point, position) in points.iter_mut().zip(projection.positions) {
        point.position = position;
    }
    for index in 0..exercises.len() {
        let Some(point) = exercises[index].point else {
            continue;
        };
        let mut neighbors: Vec<_> = exercises
            .iter()
            .enumerate()
            .filter(|(other, item)| *other != index && item.point.is_some())
            .map(|(other, item)| Neighbor {
                index: other,
                similarity: projection::similarity(
                    &normalized[point],
                    &normalized[item.point.unwrap()],
                ),
            })
            .collect();
        neighbors.sort_by(|left, right| {
            right
                .similarity
                .total_cmp(&left.similarity)
                .then_with(|| exercises[left.index].name.cmp(&exercises[right.index].name))
        });
        neighbors.truncate(NEIGHBORS);
        exercises[index].neighbors = neighbors;
    }
    let mut fit_order: Vec<_> = (0..exercises.len())
        .filter(|index| exercises[*index].point.is_some())
        .collect();
    fit_order.sort_by(|left, right| {
        exercises[*right]
            .score
            .total_cmp(&exercises[*left].score)
            .then_with(|| workouts[*right].cmp(&workouts[*left]))
            .then_with(|| exercises[*left].name.cmp(&exercises[*right].name))
    });
    let mut regions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, movements) in patterns.iter().enumerate() {
        for pattern in movements {
            regions
                .entry(embedding::label(pattern))
                .or_default()
                .push(index);
        }
    }
    let mut landmarks: Vec<_> = regions
        .into_iter()
        .filter(|(label, points)| !label.is_empty() && points.len() > 1)
        .map(|(label, points)| Landmark { label, points })
        .collect();
    landmarks.sort_by(|left, right| {
        right
            .points
            .len()
            .cmp(&left.points.len())
            .then_with(|| left.label.cmp(right.label))
    });
    let omitted = exercises.iter().filter(|item| item.point.is_none()).count();
    Space {
        exercises,
        points,
        landmarks,
        fit_order,
        selected,
        omitted,
        embedding: embedding::build(&normalized),
        linear_retained: projection.retained,
        muscles: vocabulary
            .iter()
            .map(|(id, label)| {
                let row = focus.muscles.iter().find(|row| row.id == *id);
                SpaceMuscle {
                    label,
                    current_centi: row.map_or(0, |row| row.recent_centi_points),
                    usual_centi: row.map_or(0.0, |row| {
                        f64::from(row.baseline_centi_points) / f64::from(BASELINE_WEEKS)
                    }),
                    target_centi: row.and_then(|row| row.target_centi_points),
                }
            })
            .collect(),
    }
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn score_text(score: f64) -> String {
    format!("{score:+.1}")
}

fn similarity_text(similarity: f64) -> String {
    if similarity >= 1.0 - 1e-10 {
        "100%".to_string()
    } else {
        format!("{:.1}%", (similarity * 100.0).min(99.9))
    }
}

#[route(GET "/fitness/space")]
async fn legacy_space(cx: &Cx) -> Result {
    Err(redirect_permanent(super::with_raw_query(cx, PATH)).into())
}

#[route(GET "/fitness/space/search")]
async fn legacy_search(cx: &Cx) -> Result {
    Err(redirect_permanent(super::with_raw_query(cx, "/fitness/exercises/search")).into())
}

#[component]
pub(super) async fn explorer(cx: &Cx) -> Result {
    let query = query_params::<SpaceQuery>(cx)?;
    let requested = query.exercise.as_deref().filter(|name| !name.is_empty());
    let q = query.q.as_deref().unwrap_or("").trim();
    let history_page = query.history_page.unwrap_or(1);
    if query.exercise.as_ref().is_some_and(|name| name.len() > 200)
        || q.len() > 200
        || !(1..=10_000).contains(&history_page)
    {
        return view! {
            (StatusCode::BAD_REQUEST)
            ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            "Invalid exercise, search, or history page."
        };
    }
    let loaded = entry::entry_guide_with_focus(app_context::<FitnessStore>(cx)).await;
    let space = loaded
        .as_ref()
        .ok()
        .map(|(guide, focus)| build(guide, focus, requested));
    let unavailable = space.is_none();
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        (if unavailable { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::OK })
        shell(page: "Exercises", active: "", runtime: false, fitness_pwa: true,
            <section class="exercise-space">
                super::exercise_library::catalog_header(list: false)
                if let Some(space) = &space {
                    if space.exercises.is_empty() {
                        <p class="space-empty">"No muscle profiles to map yet. "<a href="/fitness/exercises?view=list">"Browse the exercise list"</a>" to add muscle weights and start exploring."</p>
                    } else {
                        space_view(space: space, today: loaded.as_ref().unwrap().0.today.as_str(), q: q, search: search_catalog(&loaded.as_ref().unwrap().0, q), history_page: history_page)
                        if requested.is_some() && space.selected.is_none() { <p class="space-note">"That exercise was not found. Search for another exercise above."</p> }
                    }
                } else { <p class="space-empty">"The exercise map could not load. Try again in a moment."</p> }
            </section>
            exercise::details::host()
            <script type="module" src=(SPACE_JS)></script>
        )
    }
}

#[component]
async fn space_view(
    space: &Space,
    today: &str,
    q: &str,
    search: SearchResults,
    history_page: usize,
) -> Result {
    let data = serde_json::to_string(space).expect("finite exercise space serializes");
    let selected = space.selected.map(|index| &space.exercises[index]);
    let rows: Vec<(usize, Option<f64>)> = selected.map_or_else(
        || {
            space
                .fit_order
                .iter()
                .take(NEIGHBORS)
                .map(|index| (*index, None))
                .collect()
        },
        |item| {
            item.neighbors
                .iter()
                .map(|neighbor| (neighbor.index, Some(neighbor.similarity)))
                .collect()
        },
    );
    view! {
        <div data-exercise-space=(data) data-space-layout-worker=(embedding::WORKER_JS) data-space-layout-library=(embedding::LIBRARY_JS)>
            <form class="space-controls" method="get" action=(PATH) data-space-form="" data-space-query=(q)>
                <label for="space-exercise">"Explore an exercise"</label>
                <div class="space-controls__row">
                    <div class="space-controls__search">
                        <input id="space-exercise" name="q" type="search" inputmode="search" autocomplete="off" maxlength="200" placeholder="Search exercises" value=(if q.is_empty() { selected.map_or("", |item| item.name.as_str()) } else { q }) aria-controls="space-search-results" aria-expanded=(if search.matches.is_empty() { "false" } else { "true" }) data-space-search="">
                        <input type="hidden" name="exercise" value=(selected.map_or("", |item| item.name.as_str())) data-space-selected="">
                        <button type="submit" data-space-submit="">"Search"</button>
                    </div>
                    <div class="space-controls__actions">
                        <button type="button" data-space-reset="" hidden="">"Reset View"</button>
                        <a href=(PATH) data-space-overview="" hidden=(selected.is_none() && q.is_empty())>"Clear & See Suggestions"</a>
                    </div>
                </div>
                <p class="space-search-feedback" data-space-search-feedback="" aria-live="polite" hidden=(q.is_empty())>(if search.total == 0 { "No matching exercises.".to_string() } else { format!("{} matching exercises.", search.total) })</p>
                <div id="space-search-results" class="entry-quick__results space-search-results" data-space-search-results="" hidden=(search.matches.is_empty())>
                    for item in &search.matches {
                        <a class="entry-picker-option" href=(item.url.as_str()) data-space-search-choice=(item.name.as_str())><span class="entry-picker-option__name">(item.name.as_str())</span><span class="entry-picker-option__reason">(item.meta.as_str())</span><span class="entry-picker-option__action">"Explore"</span></a>
                    }
                </div>
            </form>
            <div class="space-layout">
                <div class="space-chart">
                    <div class="space-stage" data-space-stage="">
                        <canvas data-space-canvas="" aria-label="Rotatable 3D map of exercise muscle profiles. Use exercise search and nearby exercise links for keyboard navigation.">"Exercise search and the lists below provide the same comparisons without the map."</canvas>
                        <div class="space-stage__meta"><span>(format!("{} exercises · {} profiles", space.exercises.len() - space.omitted, space.points.len()))</span><span data-space-layout-label="">"3D projection"</span></div>
                        <p class="space-map-status" data-space-map-status="">"Enable JavaScript to rotate the map. Exercise comparisons work below."</p>
                        <div class="space-tooltip" role="status" data-space-tooltip="" hidden=""></div>
                        <div class="space-orbit" data-space-orbit="" hidden="" role="group" aria-label="Rotate and zoom the map">
                            <button type="button" data-space-turn="left" aria-label="Rotate left">"↶"</button>
                            <button type="button" data-space-turn="right" aria-label="Rotate right">"↷"</button>
                            <button type="button" data-space-turn="up" aria-label="Tilt up">"↑"</button>
                            <button type="button" data-space-turn="down" aria-label="Tilt down">"↓"</button>
                            <button type="button" data-space-turn="in" aria-label="Zoom in">"+"</button>
                            <button type="button" data-space-turn="out" aria-label="Zoom out">"−"</button>
                        </div>
                    </div>
                    <div class="space-legend" aria-label="Color shows training fit"><span>"More surplus"</span><span class="space-legend__scale" aria-hidden="true"></span><span>"More need"</span></div>
                    <p class="space-caption">"Drag to rotate · scroll or pinch to zoom · select a point"</p>
                    <p class="space-note" data-space-layout-note="">"Nearby matches use all 28 muscles. Distances between clusters are approximate."</p>
                </div>
                <aside class="space-detail" aria-label="Exercise comparison">
                    <div class="space-selection" aria-live="polite">
                        <p class="space-detail__eyebrow" data-space-selection-label="">(if selected.is_some() { "Selected exercise" } else { "Your training compass" })</p>
                        <h2 data-space-title="">(selected.map_or("Best training fits", |item| item.name.as_str()))</h2>
                        <p class="space-score" data-space-score="" data-sign=(if selected.is_some_and(|item| item.score < 0.0) { "negative" } else { "positive" }) hidden=(selected.is_none_or(|item| item.point.is_none()))>"Training fit "<strong data-space-score-value="">(selected.map_or_else(String::new, |item| score_text(item.score)))</strong></p>
                        <a class="space-exercise-link" data-space-exercise-link="" data-exercise-details-link="" href=(selected.map_or("/fitness/exercises", |item| item.url.as_str())) hidden=(selected.is_none())>"Exercise details"</a>
                        <p class="space-note" data-space-shared="" hidden=""></p>
                    </div>
                    <div class="space-neighbors">
                        <h3 data-space-neighbors-title="">(if selected.is_some() { "Closest muscle matches" } else { "Best matches for saved load" })</h3>
                        <p class="space-note" data-space-neighbors-note="">(if selected.is_some() { if selected.is_some_and(|item| item.point.is_none()) { "Add a muscle profile in exercise details to see similar exercises." } else { "Similarity uses the complete muscle profile." } } else { "Higher scores cover more of the current muscle gaps." })</p>
                        <ol data-space-neighbors="">
                            for (index, similarity) in &rows {
                                <li><a href=(space.exercises[*index].map_url.as_str()) data-space-choice=(*index)><span>(space.exercises[*index].name.as_str())</span><strong>(similarity.map_or_else(|| score_text(space.exercises[*index].score), similarity_text))</strong></a></li>
                            }
                        </ol>
                    </div>
                    load::panel(muscles: &space.muscles, deltas: selected.map(|item| item.load_delta_centi.as_slice()))
                </aside>
                history::panel(name: selected.map(|item| item.name.as_str()), page_number: history_page)
            </div>
            <details class="space-method">
                <summary>"How to read this space"</summary>
                <p>"UMAP first arranges local neighborhoods in three dimensions using muscle profiles. A whole-map distance fit (metric MDS) then balances the spacing between groups, keeping shared muscle involvement visible across movements. Each profile is scaled to its strongest muscle, with square roots giving secondary contributions more influence. Profiles can have different lengths, so focused grip or core work can sit between broader exercises instead of being pushed onto a common outer shell. Axes have no anatomical meaning, and distances remain approximate in three dimensions. Labels identify movement groups; movement tags do not affect distances."</p>
                <p>"The layout reports how many of each point’s closest neighbors stay among its closest neighbors in 3D, averaged across the map. The full comparison list uses cosine similarity across the original 28 muscle weights. A 100% match means the same proportions, even if equipment or total involvement differs. Exercises with matching muscle proportions and movement tags share a point."</p>
                <p>"Color and training fit use the compass’s signed dot product with the original, unnormalized exercise weights. Positive muscle gaps add credit; above-target load subtracts it. This is a fit score, not a predicted dose or a percentage."</p>
                <p>"The muscle-load preview adds two normal sets at RPE 9: eight volume points multiplied by each muscle’s stored weight. Bar ends stay at the weekly target, or usual weekly pace where no target is set. The percentage shows just the added load. An arrow marks a total beyond the bar’s target. Muscles with a zero target or no usual pace have no percentage scale, so their preview shows added points."</p>
                <p>(format!("Training fit uses saved workouts through {today}. In-progress sets and session fatigue are not included."))</p>
                if space.omitted > 0 { <p>(format!("{} catalog exercises have no usable muscle profile and are omitted from the map. ", space.omitted))<a href="/fitness/exercises?view=list">"See all exercises in the list."</a></p> }
            </details>
            <template data-space-neighbor-template=""><li><a data-space-choice=""><span></span><strong></strong></a></li></template>
            <template data-space-search-template=""><a class="entry-picker-option" data-space-search-choice=""><span class="entry-picker-option__name"></span><span class="entry-picker-option__reason"></span><span class="entry-picker-option__action">"Explore"</span></a></template>
            <template data-space-history-loading-template=""><header><div><p class="space-detail__eyebrow">"Set history"</p><h3></h3></div></header><p class="space-history__empty" role="status">"Loading sets…"</p></template>
            <template data-space-history-error-template=""><p class="space-history__empty">"Set history could not load."</p><a class="space-exercise-link" data-space-history-page="1">"Retry history"</a></template>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guide() -> GuideConfig {
        serde_json::from_value(serde_json::json!({
            "version": 1, "today": "2026-09-14", "weekly_pace_tenths": 20,
            "muscle_needs": {"mid-chest": 8000, "triceps": -4000},
            "exercises": [
                {"name":"Press", "bodyweight":false, "last_date":"", "set_count":0, "workout_count":0, "muscles":[["mid-chest",100],["triceps",50]], "movements":[], "coarse_muscles":[], "marks":[], "loads":[], "picker_meta":"", "picker_mark":""},
                {"name":"Light press", "aliases":["Old press"], "bodyweight":false, "last_date":"", "set_count":0, "workout_count":0, "muscles":[["mid-chest",50],["triceps",25]], "movements":[], "coarse_muscles":[], "marks":[], "loads":[], "picker_meta":"", "picker_mark":""},
                {"name":"Fly", "bodyweight":false, "last_date":"", "set_count":0, "workout_count":0, "muscles":[["mid-chest",100]], "movements":[], "coarse_muscles":[], "marks":[], "loads":[], "picker_meta":"", "picker_mark":""},
                {"name":"Unmapped", "bodyweight":false, "last_date":"", "set_count":0, "workout_count":0, "muscles":[], "movements":[], "coarse_muscles":[], "marks":[], "loads":[], "picker_meta":"", "picker_mark":""}
            ]
        })).unwrap()
    }

    fn focus() -> TrainingFocus {
        super::super::training_focus::derive([], "2026-09-14".parse().unwrap(), &Default::default())
    }

    #[test]
    fn similarity_normalization_never_changes_training_scores() {
        let space = build(&guide(), &focus(), Some("Old press"));
        assert_eq!(space.selected, Some(1));
        assert_eq!(space.points.len(), 2);
        assert_eq!(space.omitted, 1);
        assert_eq!(space.fit_order, [2, 0, 1]);
        assert_eq!(space.exercises[0].score, 7.5);
        assert_eq!(space.exercises[1].score, 3.75);
        assert_eq!(space.exercises[0].neighbors[0].index, 1);
        assert!((space.exercises[0].neighbors[0].similarity - 1.0).abs() < 1e-12);
        assert!(serde_json::to_string(&space).is_ok());
        assert_eq!(similarity_text(0.99999), "99.9%");
        assert_eq!(similarity_text(1.0), "100%");
    }

    #[test]
    fn search_shares_entry_ranking_and_keeps_unmapped_exercises_selectable() {
        let mut guide = guide();
        guide.exercises[0].equipment = vec!["dumbbell".into()];
        assert_eq!(
            search_catalog(&guide, "old press").matches[0].name,
            "Light press"
        );
        assert_eq!(
            search_catalog(&guide, "press dumbbell").matches[0].name,
            "Press"
        );
        assert_eq!(search_catalog(&guide, "mid chest").total, 3);
        let unmapped = search_catalog(&guide, "unmapped");
        assert_eq!(unmapped.total, 1);
        assert_eq!(unmapped.matches[0].url, exercise::page_url("Unmapped"));
        assert_eq!(
            search_catalog(&guide, "old press").matches[0].url,
            page_url(Some("Light press"))
        );
        assert_eq!(search_catalog(&guide, " -- ").total, 0);
        assert_eq!(search_catalog(&guide, "").total, 0);
        guide.exercises[0].movements = vec!["cardio".into()];
        assert_eq!(search_catalog(&guide, "press").total, 2);
        let space = build(&guide, &focus(), Some("Unmapped"));
        assert_eq!(space.exercises.len(), 4);
        assert_eq!(space.selected, Some(3));
        assert!(space.exercises[3].point.is_none());
        assert!(space.exercises[3].neighbors.is_empty());
        assert!(!space.fit_order.contains(&3));
    }

    #[test]
    fn empty_and_single_profile_catalogs_keep_comparisons_available() {
        let mut guide = guide();
        guide.exercises.truncate(1);
        let space = build(&guide, &focus(), None);
        assert_eq!(space.points[0].position, [0.0; 3]);
        assert!(space.exercises[0].neighbors.is_empty());
        guide.exercises.clear();
        assert!(
            build(&guide, &focus(), Some("Unknown"))
                .exercises
                .is_empty()
        );
        assert_eq!(
            page_url(Some("Press & pull")),
            "/fitness/exercises?exercise=Press%20%26%20pull"
        );
    }
}
