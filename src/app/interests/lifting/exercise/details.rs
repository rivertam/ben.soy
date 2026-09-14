//! One server-rendered exercise inspector for both catalog views and dialog fetches.
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        HeaderValue, StatusCode, header, query_params,
        response::{IntoResponse, Response},
        route,
    },
    view::{component, view},
};

use super::super::{
    archive::{exercise_definition::Definition, snapshot::ExerciseProfile, store::FitnessStore},
    filters::{EQUIPMENT, MOVEMENT_DETAILS, MOVEMENTS},
    muscle_taxonomy, muscles,
};
use crate::{app::login::viewer, content::access::is_admin};

pub(in super::super) const DETAILS_JS: Asset = asset!("./details.js");

#[query_params(error = redirect("?"))]
struct DetailsQuery {
    view: Option<String>,
    exercise: Option<String>,
    details: Option<String>,
    notice: Option<String>,
}

#[component]
pub(in super::super) async fn host(cx: &Cx) -> Result {
    let query = query_params::<DetailsQuery>(cx)?;
    let name = query
        .exercise
        .as_deref()
        .filter(|name| super::plausible_exercise_name(name));
    let opened = query.details.as_deref() == Some("1") && name.is_some();
    let close_url = if query.view.as_deref() == Some("list") {
        "/fitness/exercises?view=list".to_string()
    } else {
        name.map_or_else(
            || super::super::exercise_space::PATH.to_string(),
            super::page_url,
        )
    };
    view! {
        <dialog class="exercise-details" data-exercise-details-dialog="" aria-label="Exercise details" open=(opened)>
            <a class="exercise-details__close" href=(close_url.as_str()) data-exercise-details-close="" aria-label="Close exercise details">"×"</a>
            <div data-exercise-details-body="">
                if opened { content(name: name.unwrap(), notice: query.notice.as_deref()) }
            </div>
        </dialog>
        <template data-exercise-details-loading=""><p class="exercise-details__message" role="status">"Loading exercise…"</p></template>
        <script type="module" src=(DETAILS_JS)></script>
    }
}

#[route(GET "/fitness/exercises/details")]
async fn details_endpoint(cx: &Cx) -> Result<Response> {
    let query = query_params::<DetailsQuery>(cx)?;
    let Some(name) = query
        .exercise
        .as_deref()
        .filter(|name| super::plausible_exercise_name(name))
    else {
        return Ok(super::plain(StatusCode::BAD_REQUEST, "Choose an exercise."));
    };
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        content(name: name, notice: query.notice.as_deref())
    }?
    .into_response(cx)
}

#[component]
async fn content(cx: &Cx, name: &str, notice: Option<&str>) -> Result {
    let Ok(snapshot) = app_context::<FitnessStore>(cx).snapshot().await else {
        return view! { (StatusCode::SERVICE_UNAVAILABLE) <p class="exercise-details__message">"Exercise details could not load. Try again."</p> };
    };
    let Some(name) = snapshot.canonical_exercise_name(name) else {
        return view! { (StatusCode::NOT_FOUND) <p class="exercise-details__message">"That exercise was not found."</p> };
    };
    let definition = Definition::from_snapshot(&snapshot, &name);
    let history = snapshot.exercise_profile(&name);
    let aliases = snapshot.exercise_aliases(&name);
    let can_edit = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let provenance = if can_edit {
        super::db_sources(cx, &name)
            .await
            .ok()
            .and_then(|sources| super::provenance_line(&sources))
    } else {
        None
    };
    view! {
        inspector(definition: &definition, aliases: &aliases, can_edit: can_edit, provenance: provenance.as_deref(), notice: notice, history: history.as_ref())
    }
}

#[component]
async fn inspector(
    definition: &Definition,
    aliases: &[String],
    can_edit: bool,
    provenance: Option<&str>,
    notice: Option<&str>,
    #[default(None)] history: Option<&ExerciseProfile>,
) -> Result {
    let mut involvement = muscles::MuscleInvolvement::default();
    for (id, _) in muscle_taxonomy::muscles() {
        match definition.weights.get(id).copied().unwrap_or(0) {
            75..=100 => involvement.primary.push(id),
            1..=74 => involvement.secondary.push(id),
            _ => {}
        }
    }
    let notice = if can_edit {
        notice.and_then(|code| match code {
            "saved" => Some("Changes saved."),
            "identity-saved" => Some("Name and aliases saved."),
            "identity-stale" => Some("The archive changed. Review the name and aliases again."),
            "invalid" => Some("That did not validate. Nothing changed."),
            "unavailable" => Some("The exercise could not be saved. Try again."),
            _ => None,
        })
    } else {
        None
    };
    view! {
        <section data-exercise-details-content="" data-exercise-name=(definition.name.as_str())>
            <header class="exercise-details__header">
                <div><h2 tabindex="-1" data-details-heading="">(definition.name.as_str())</h2>
                    if let Some(history) = history {
                        <p title=(format!("{} through {}", history.first_date, history.last_date))>
                            (format!("{} {} across {} {}", history.set_count, super::super::format::plural(history.set_count, "set", "sets"), history.workout_count, super::super::format::plural(history.workout_count, "workout", "workouts")))
                        </p>
                    } else { <p>"No sets logged yet"</p> }
                    if !aliases.is_empty() { <p>(format!("Also known as {}", aliases.join(", ")))</p> }
                    if !can_edit { <p>"Muscle weights and exercise setup"</p> }
                </div>
                <div class="exercise-details__body-map" aria-label="Muscles worked">
                    muscles::muscle_figure(paths: muscles::FRONT_PATHS, caption: "front", involvement: &involvement, compact: true)
                    muscles::muscle_figure(paths: muscles::BACK_PATHS, caption: "back", involvement: &involvement, compact: true)
                </div>
            </header>
            if let Some(notice) = notice { <p class="exercise-details__notice" role="status">(notice)</p> }
            <form action=(format!("{}/definition", super::write_url(&definition.name))) method="post" data-exercise-definition-form="">
                if can_edit {
                    <input type="hidden" name="name" value=(definition.name.as_str())>
                    <input type="hidden" name="editing" value=(definition.name.as_str())>
                }
                <section class="exercise-editor-group" data-editor-group="muscle">
                    <h3>"Muscles"</h3>
                    <p class="exercise-details__hint">"100 = full credit · 50 = half. Weights don’t need to total 100."</p>
                    <div class="exercise-editor-muscles" data-editor-selected="">
                        for (id, label) in muscle_taxonomy::muscles() {
                            if let Some(ratio) = definition.weights.get(id).filter(|ratio| **ratio > 0) {
                                muscle_row(id: id, label: label, ratio: *ratio, can_edit: can_edit)
                            }
                        }
                    </div>
                    <p class="exercise-editor-empty">"No muscles added."</p>
                    if can_edit {
                        <details class="exercise-editor-add" data-editor-picker="">
                            <summary>"+ Add muscles"</summary>
                            <input type="search" aria-label="Search muscles" placeholder="Search muscles or body regions" data-editor-search="" hidden="">
                            <div class="exercise-editor-options" data-editor-available="">
                                for (id, label) in muscle_taxonomy::muscles() {
                                    if !definition.weights.contains_key(id) { muscle_row(id: id, label: label, ratio: 0, can_edit: true) }
                                }
                            </div>
                            <p class="exercise-details__hint" data-editor-no-matches="" hidden="">"No more matching muscles."</p>
                        </details>
                    }
                </section>
                tag_group(kind: "movement", heading: "Movement patterns", options: MOVEMENTS.iter().chain(MOVEMENT_DETAILS).copied().collect(), selected: &definition.movements, can_edit: can_edit)
                tag_group(kind: "equipment", heading: "Equipment", options: EQUIPMENT.to_vec(), selected: &definition.equipment, can_edit: can_edit)
                if can_edit {
                    <footer class="exercise-details__save"><p role="status" data-editor-status=""></p><button class="entry-button entry-button--primary" type="submit">"Save changes"</button></footer>
                }
            </form>
            if can_edit {
                <details class="exercise-details__identity"><summary>"Name & aliases"</summary>
                    super::identity_form(name: definition.name.as_str(), aliases: aliases)
                    if let Some(provenance) = provenance { <p class="exercise-details__hint">(provenance)</p> }
                </details>
            }
        </section>
    }
}

#[component]
async fn muscle_row(id: &str, label: &str, ratio: u32, can_edit: bool) -> Result {
    let region = muscle_taxonomy::coarse_tag_for(id).unwrap_or("");
    view! {
        <div class="exercise-editor-muscle" data-editor-item=(id) data-editor-terms=(format!("{label} {id} {region}"))>
            <label for=(format!("detail-ratio-{id}"))>(label)</label>
            if can_edit {
                <input id=(format!("detail-ratio-{id}")) name=(format!("ratio_{id}")) type="number" min="0" max="100" step="1" inputmode="numeric" value=(ratio) data-editor-value="" aria-label=(format!("{label} weight"))>
                <button type="button" data-editor-add="" aria-label=(format!("Add {label}")) hidden="">"+"</button>
                <button type="button" data-editor-remove="" aria-label=(format!("Remove {label}")) hidden="">"×"</button>
            } else { <strong>(ratio)</strong> }
        </div>
    }
}

#[component]
async fn tag_group(
    kind: &str,
    heading: &str,
    options: Vec<(&str, &str)>,
    selected: &[String],
    can_edit: bool,
) -> Result {
    view! {
        <section class="exercise-editor-group" data-editor-group=(kind)>
            <h3>(heading)</h3>
            <div class="exercise-editor-tags" data-editor-selected="">
                for (id, label) in &options {
                    if selected.iter().any(|value| value == id) { tag_row(kind: kind, id: id, label: label, selected: true, can_edit: can_edit) }
                }
            </div>
            <p class="exercise-editor-empty">"None added."</p>
            if can_edit {
                <details class="exercise-editor-add" data-editor-picker="">
                    <summary>(format!("+ Add {}", heading.to_lowercase()))</summary>
                    <input type="search" aria-label=(format!("Search {}", heading.to_lowercase())) placeholder=(format!("Search {}", heading.to_lowercase())) data-editor-search="" hidden="">
                    <div class="exercise-editor-options" data-editor-available="">
                        for (id, label) in &options {
                            if !selected.iter().any(|value| value == id) { tag_row(kind: kind, id: id, label: label, selected: false, can_edit: true) }
                        }
                    </div>
                    <p class="exercise-details__hint" data-editor-no-matches="" hidden="">"No more matching options."</p>
                </details>
            }
        </section>
    }
}

#[component]
async fn tag_row(kind: &str, id: &str, label: &str, selected: bool, can_edit: bool) -> Result {
    view! {
        <div class="exercise-editor-tag" data-editor-item=(id) data-editor-terms=(format!("{label} {id}"))>
            <label>
                if can_edit { <input type="checkbox" name=(kind) value=(id) checked=(selected) data-editor-value=""> }
                <span>(label)</span>
            </label>
            if can_edit {
                <button type="button" data-editor-add="" aria-label=(format!("Add {label}")) hidden="">"+"</button>
                <button type="button" data-editor-remove="" aria-label=(format!("Remove {label}")) hidden="">"×"</button>
            }
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inspector_exposes_only_existing_values_until_expanded_and_never_public_writes() {
        let definition = Definition {
            name: "Press & pull".into(),
            weights: [("biceps".into(), 65)].into(),
            movements: vec!["horizontal-pull".into()],
            ..Definition::default()
        };
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! { inspector(definition: &definition, aliases: &[], can_edit: false, provenance: None, notice: None) };
        let public = result.unwrap().render(__cx);
        assert!(public.contains("Press &amp; pull"));
        assert!(public.contains("<strong>65</strong>"));
        assert!(!public.contains("name=\"ratio_"));
        assert!(!public.contains("Save changes"));
        assert!(!public.contains("data-editor-picker"));
        let result: Result = view! { inspector(definition: &definition, aliases: &[], can_edit: true, provenance: None, notice: None) };
        let owner = result.unwrap().render(__cx);
        assert!(owner.contains("data-editor-picker"));
        assert_eq!(owner.matches("name=\"ratio_biceps\"").count(), 1);
        assert!(owner.contains("value=\"65\""));
        assert!(owner.contains("data-exercise-identity-form"));
    }
}
