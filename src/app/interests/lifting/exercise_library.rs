//! Public exercise library and the shared owner-only creation/definition wizard.
use std::collections::HashSet;

use super::{
    archive::{
        db,
        exercise_definition::{Definition, references},
        store::FitnessStore,
    },
    entry, exercise,
    filters::{EQUIPMENT, MOVEMENT_DETAILS, MOVEMENTS},
    muscle_taxonomy, muscles, taxonomy,
};
use crate::{
    app::login::viewer,
    components::shell,
    content::access::is_admin,
    util::{is_same_origin, urlencode},
};
use benjisponge::data::Data;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderValue, StatusCode, header, path_param, query_params,
        request::headers,
        response::{IntoResponse, Response},
        route, to_bytes,
    },
    view::{component, view},
};

pub(super) const WIZARD_JS: Asset = asset!("./exercise-wizard.js");
const LIBRARY: &str = "/fitness/exercises";
const PREVIEW: &str = "/fitness/exercises/preview";

#[query_params(error = redirect("?"))]
struct LibraryQuery {
    q: Option<String>,
    name: Option<String>,
}

#[route(GET "/fitness/exercises")]
async fn library(cx: &Cx) -> Result<Response> {
    let query = query_params::<LibraryQuery>(cx)?;
    let q = query.q.as_deref().unwrap_or("").trim();
    if q.len() > 200 {
        return Ok(error(StatusCode::BAD_REQUEST, "Search is too long."));
    }
    let guide = match entry::entry_guide(app_context::<FitnessStore>(cx)).await {
        Ok(guide) => guide,
        Err(_) => {
            return Ok(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The exercise library could not load. Try again.",
            ));
        }
    };
    let mut found = fitness_entry_core::search_exercises(&guide.exercises, q);
    if q.is_empty() {
        found.sort_by_key(|item| item.name.to_lowercase());
    }
    let owner = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let exact = guide.exercises.iter().any(|item| {
        std::iter::once(&item.name)
            .chain(&item.aliases)
            .any(|name| name.eq_ignore_ascii_case(q))
    });
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(page: "Exercises", active: "", runtime: false, fitness_pwa: true,
            <section class="exercise-library">
                <a href="/fitness" class="exercise-link">"← Fitness"</a>
                <header class="exercise-library__header">
                    <h1>"Exercises"</h1>
                    if owner { <a class="entry-button" href="/fitness/exercises/new">"New exercise"</a> }
                </header>
                <form method="get" action=(LIBRARY) class="exercise-library__search">
                    <label for="exercise-library-search">"Find an exercise"</label>
                    <div><input id="exercise-library-search" name="q" type="search" value=(q) placeholder="Name, movement, equipment, or muscle" maxlength="200"><button class="entry-button" type="submit">"Search"</button></div>
                </form>
                if owner && !q.is_empty() && !exact {
                    <a class="entry-button" href=(format!("{LIBRARY}/new?name={}", urlencode(q)))>(format!("Create “{q}”"))</a>
                }
                <p class="exercise-library__count">(format!("{} exercises", found.len()))</p>
                <ul class="exercise-library__list">
                    for item in found {
                        <li><a href=(exercise::page_url(&item.name))><strong>(item.name.as_str())</strong><span>(item.picker_meta.as_str())</span>
                            if !item.aliases.is_empty() { <span>(format!("Also known as {}", item.aliases.join(", ")))</span> }
                        </a></li>
                    }
                </ul>
            </section>
        )
    }?.into_response(cx)
}

#[route(GET "/fitness/exercises/new")]
async fn new_exercise(cx: &Cx) -> Result<Response> {
    if let Some(response) = owner_gate(cx, false) {
        return Ok(response);
    }
    let query = query_params::<LibraryQuery>(cx)?;
    let definition = Definition {
        name: query.name.clone().unwrap_or_default(),
        ..Definition::default()
    };
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(page: "New exercise", active: "", runtime: false, fitness_pwa: true,
            <section class="exercise-library"><a class="exercise-link" href=(LIBRARY)>"← Exercises"</a>
                wizard(definition: &definition, step: 1, editing: false, reference: "", choices: &[])
            </section>
            <script type="module" src=(WIZARD_JS)></script>
        )
    }?.into_response(cx)
}

#[component]
pub(super) async fn wizard(
    definition: &Definition,
    step: u8,
    editing: bool,
    reference: &str,
    choices: &[String],
) -> Result {
    let action = if editing {
        format!("{}/definition", exercise::page_url(&definition.name))
    } else {
        LIBRARY.into()
    };
    let mut involvement = muscles::MuscleInvolvement::default();
    for (id, _) in muscle_taxonomy::muscles() {
        match definition.weights.get(id).copied().unwrap_or(0) {
            75..=100 => involvement.primary.push(id),
            1..=74 => involvement.secondary.push(id),
            _ => {}
        }
    }
    view! {
        <form method="post" action=(action.as_str()) class="exercise-wizard" data-exercise-wizard="" data-wizard-step=(step) data-wizard-editing=(if editing { "true" } else { "false" })>
            <header class="exercise-wizard__header"><h2>(if editing { "Exercise setup" } else { "New exercise" })</h2><button type="button" data-wizard-close="" aria-label="Cancel exercise creation" hidden="">"×"</button></header>
            <p class="exercise-wizard__progress" data-wizard-progress="">"Name · Movement · Muscles"</p>
            <input type="hidden" name="editing" value=(if editing { definition.name.as_str() } else { "" })>
            <fieldset data-wizard-panel="1">
                <legend>"Name"</legend>
                <label for="new-exercise-name">"Exercise name"</label>
                <input id="new-exercise-name" name="name" type="text" value=(definition.name.as_str()) required="" maxlength="200" readonly=(editing) autocomplete="off">
                if !editing { <p>"Choose a name you’ll recognize when logging. You can finish the setup later."</p> }
                <div class="exercise-wizard__actions" data-wizard-actions="1">
                    if !editing { <button class="entry-button" type="submit" name="intent" value="name_only">"Save name only"</button> }
                    <button class="entry-button entry-button--primary" type="submit" name="intent" value="classify" formaction=(PREVIEW)>"Next: movement"</button>
                </div>
            </fieldset>
            <fieldset data-wizard-panel="2">
                <legend>"Movement and equipment"</legend>
                <p>"Choose the patterns this exercise uses. These help find a starting muscle breakdown."</p>
                <div class="exercise-wizard__choices" role="group" aria-label="Movement patterns">
                    for (id, label) in MOVEMENTS.iter().chain(MOVEMENT_DETAILS) {
                        <label><input type="checkbox" name="movement" value=(*id) checked=(definition.movements.iter().any(|value| value == id))><span>(*label)</span></label>
                    }
                </div>
                <p>"Equipment"</p>
                <div class="exercise-wizard__choices" role="group" aria-label="Equipment">
                    for (id, label) in EQUIPMENT {
                        <label><input type="checkbox" name="equipment" value=(*id) checked=(definition.equipment.iter().any(|value| value == id))><span>(*label)</span></label>
                    }
                </div>
                <div class="exercise-wizard__actions" data-wizard-actions="2">
                    <button class="entry-button" type="button" data-wizard-back="1" hidden="">"Back"</button>
                    if !editing { <button class="entry-button" type="submit" name="intent" value="suggest_save" data-wizard-quick-save="">"Save with suggested weights"</button> }
                    <button class="entry-button entry-button--primary" type="submit" name="intent" value="suggest" formaction=(PREVIEW)>"Suggest muscles"</button>
                </div>
            </fieldset>
            <fieldset data-wizard-panel="3">
                <legend>"Muscle breakdown"</legend>
                if !reference.is_empty() {
                    <p>"Based on "<a class="exercise-link" href=(exercise::page_url(reference)) target="_blank" rel="noopener">(reference)</a>". Adjust anything that differs."</p>
                } else if definition.weights.is_empty() {
                    <p>"No suggested breakdown yet. Set weights yourself or save and finish later."</p>
                }
                if !choices.is_empty() {
                    <label>"Use another starting point"<select name="reference"><option value="">"Best matching exercise"</option>
                        for name in choices { <option value=(name.as_str()) selected=(name == reference)>(name.as_str())</option> }
                    </select></label>
                    <button class="entry-button" type="submit" name="intent" value="suggest" formaction=(PREVIEW)>"Use these suggested weights"</button>
                }
                <div class="exercise-wizard__maps">
                    muscles::muscle_figure(paths: muscles::FRONT_PATHS, caption: "front", involvement: &involvement, compact: true)
                    muscles::muscle_figure(paths: muscles::BACK_PATHS, caption: "back", involvement: &involvement, compact: true)
                </div>
                <p>"Each weight is independent, from 0 to 100. They don’t need to add up to 100."</p>
                <div class="exercise-wizard__weights">
                    for (_, label, members) in muscle_taxonomy::MUSCLE_GROUPS {
                        <div><h3>(*label)</h3>
                            for (id, label) in *members {
                                <label><span>(*label)</span><input type="number" min="0" max="100" step="1" name=(format!("ratio_{id}")) value=(definition.weights.get(*id).copied().unwrap_or(0)) inputmode="numeric"></label>
                            }
                        </div>
                    }
                </div>
                <div class="exercise-wizard__actions" data-wizard-actions="3">
                    <button class="entry-button" type="button" data-wizard-back="2" hidden="">"Back"</button>
                    <button class="entry-button entry-button--primary" type="submit" name="intent" value="save" data-wizard-save="">(if editing { "Save setup" } else { "Create exercise" })</button>
                </div>
            </fieldset>
            <p class="exercise-wizard__status" role="status" data-wizard-status=""></p>
        </form>
    }
}

#[derive(Debug)]
struct WizardInput {
    definition: Definition,
    intent: String,
    reference: String,
    editing: String,
}

fn parse_form(bytes: &[u8]) -> std::result::Result<WizardInput, String> {
    let mut input = WizardInput {
        definition: Definition::default(),
        intent: "save".into(),
        reference: String::new(),
        editing: String::new(),
    };
    let mut seen = HashSet::new();
    for (key, value) in form_urlencoded::parse(bytes) {
        if !matches!(key.as_ref(), "movement" | "equipment") && !seen.insert(key.to_string()) {
            return Err("Duplicate form field.".into());
        }
        match key.as_ref() {
            "name" => input.definition.name = value.into_owned(),
            "movement" => input.definition.movements.push(value.into_owned()),
            "equipment" => input.definition.equipment.push(value.into_owned()),
            "intent" => input.intent = value.into_owned(),
            "reference" => input.reference = value.into_owned(),
            "editing" => input.editing = value.into_owned(),
            key if key.starts_with("ratio_") => {
                input.definition.weights.insert(
                    key[6..].into(),
                    if value.trim().is_empty() {
                        0
                    } else {
                        value
                            .parse()
                            .map_err(|_| "Enter whole-number muscle weights.".to_string())?
                    },
                );
            }
            _ => return Err("Unknown form field.".into()),
        }
    }
    if !matches!(
        input.intent.as_str(),
        "save" | "name_only" | "classify" | "suggest" | "suggest_save"
    ) || input.reference.len() > 200
        || input.editing.len() > 200
    {
        return Err("Invalid exercise form.".into());
    }
    input.definition = input.definition.validate()?;
    Ok(input)
}

fn owner_gate(cx: &Cx, writing: bool) -> Option<Response> {
    let Some(current) = viewer(cx) else {
        return Some(if writing {
            error(
                StatusCode::UNAUTHORIZED,
                "Sign in again, then retry. Your form is still here.",
            )
        } else {
            redirect("/login?next=%2Ffitness%2Fexercises%2Fnew")
        });
    };
    if !is_admin(&current.email) {
        return Some(error(StatusCode::NOT_FOUND, "Not found."));
    }
    if writing && !is_same_origin(headers(cx)) {
        return Some(error(
            StatusCode::FORBIDDEN,
            "This form must be submitted from this site.",
        ));
    }
    None
}

async fn input(cx: &Cx, body: Body) -> std::result::Result<WizardInput, Response> {
    if let Some(response) = owner_gate(cx, true) {
        return Err(response);
    }
    if !headers(cx)
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(';').next().is_some_and(|value| {
                value
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
    {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Expected an exercise form.",
        ));
    }
    let bytes = to_bytes(body, 16 * 1024)
        .await
        .map_err(|_| error(StatusCode::PAYLOAD_TOO_LARGE, "Exercise form is too large."))?;
    parse_form(&bytes).map_err(|message| error(StatusCode::UNPROCESSABLE_ENTITY, &message))
}

#[route(POST "/fitness/exercises/preview")]
async fn preview(cx: &Cx, body: Body) -> Result<Response> {
    let mut input = match input(cx, body).await {
        Ok(input) => input,
        Err(response) => return Ok(response),
    };
    let snapshot = match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            return Ok(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The library could not load. Try again.",
            ));
        }
    };
    if input.intent == "classify" && input.editing.is_empty() {
        let tags = taxonomy::exercise_tags(&input.definition.name);
        input.definition.movements = tags
            .iter()
            .filter(|tag| tag.kind == "movement")
            .map(|tag| tag.value.clone())
            .collect();
        input.definition.equipment = tags
            .iter()
            .filter(|tag| tag.kind == "equipment")
            .map(|tag| tag.value.clone())
            .collect();
    }
    let choices = references(&snapshot, &input.definition);
    let selected = if input.intent == "suggest" {
        choices
            .iter()
            .find(|name| **name == input.reference)
            .or_else(|| choices.first())
            .cloned()
            .unwrap_or_default()
    } else {
        String::new()
    };
    if input.intent == "suggest" {
        input.definition.weights = Definition::from_snapshot(&snapshot, &selected).weights;
    }
    let step = if input.intent == "classify" { 2 } else { 3 };
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(page: "Exercise setup", active: "", runtime: false, fitness_pwa: true,
            <section class="exercise-library"><a class="exercise-link" href=(LIBRARY)>"← Exercises"</a>
                wizard(definition: &input.definition, step: step, editing: !input.editing.is_empty(), reference: &selected, choices: &choices)
            </section>
            <script type="module" src=(WIZARD_JS)></script>
        )
    }?.into_response(cx)
}

#[route(POST "/fitness/exercises")]
async fn create(cx: &Cx, body: Body) -> Result<Response> {
    save(cx, body, None).await
}

path_param!(definition_name);
#[route(POST "/fitness/exercise/{definition_name}/definition")]
async fn update_definition(cx: &Cx, body: Body) -> Result<Response> {
    let name = path_param::<DefinitionName>(cx);
    save(cx, body, Some(name)).await
}

async fn save(cx: &Cx, body: Body, editing: Option<&str>) -> Result<Response> {
    let mut input = match input(cx, body).await {
        Ok(input) => input,
        Err(response) => return Ok(response),
    };
    if !matches!(input.intent.as_str(), "save" | "name_only" | "suggest_save") {
        return Ok(error(
            StatusCode::BAD_REQUEST,
            "Preview this setup before saving.",
        ));
    }
    if input.intent == "name_only" {
        if editing.is_some() {
            return Ok(error(
                StatusCode::BAD_REQUEST,
                "Use the setup form to edit this exercise.",
            ));
        }
        input.definition = Definition {
            name: input.definition.name,
            ..Definition::default()
        };
    }
    if input.intent == "suggest_save" {
        let snapshot = match app_context::<FitnessStore>(cx).snapshot().await {
            Ok(snapshot) => snapshot,
            Err(_) => {
                return Ok(error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The library could not load. Try again.",
                ));
            }
        };
        let choices = references(&snapshot, &input.definition);
        if let Some(reference) = choices.first() {
            input.definition.weights = Definition::from_snapshot(&snapshot, reference).weights;
        } else {
            input.definition.weights.clear();
        }
    }
    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(_) => {
            return Ok(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The exercise could not be saved. Try again.",
            ));
        }
    };
    let saved = match db::catalog::save(
        &db,
        &input.definition,
        editing,
        jiff::Timestamp::now().as_second(),
    )
    .await
    {
        Ok(saved) => saved,
        Err(error_value) => {
            eprintln!("exercise definition save failed: {error_value}");
            return Ok(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The exercise could not be saved. Retry or reload its setup.",
            ));
        }
    };
    if let Err(error) = app_context::<FitnessStore>(cx).rebuild().await {
        eprintln!("saved exercise snapshot refresh failed: {error}");
    }
    let location = exercise::page_url(&saved.name);
    if headers(cx)
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"))
    {
        return Ok(json(
            serde_json::json!({ "name": saved.name, "created": saved.created, "location": location }),
        ));
    }
    Ok(redirect(&location))
}

#[route(GET "/fitness/entry/guide")]
async fn exercise_guide_read(cx: &Cx) -> Result<Response> {
    if let Some(response) = owner_gate(cx, false) {
        return Ok(response);
    }
    match entry::entry_guide(app_context::<FitnessStore>(cx)).await {
        Ok(guide) => Ok(json(serde_json::to_value(guide).expect("guide serializes"))),
        Err(_) => Ok(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The exercise guide could not refresh. Try again.",
        )),
    }
}
fn json(value: serde_json::Value) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(value.to_string()))
        .expect("static response")
}
fn redirect(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from("see other"))
        .expect("encoded location")
}
fn error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(message.to_owned()))
        .expect("static headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ordinary_forms_render_every_step_and_escape_the_exercise_name() {
        let cx = Cx::default();
        let __cx = &cx;
        let definition = Definition {
            name: "Curl \"test\" & more".into(),
            ..Definition::default()
        };
        let result: Result = view! { wizard(definition: &definition, step: 1, editing: false, reference: "", choices: &[]) };
        let html = result.unwrap().render(__cx);
        assert!(html.contains("method=\"post\" action=\"/fitness/exercises\""));
        assert!(html.contains("formaction=\"/fitness/exercises/preview\""));
        assert!(html.contains("Curl &quot;test&quot; &amp; more"));
        assert_eq!(html.matches("<fieldset data-wizard-panel=").count(), 3);
        assert_eq!(html.matches("name=\"ratio_").count(), 28);
        assert!(!html.contains("data-wizard-panel=\"2\" hidden"));
        assert!(html.contains("value=\"name_only\""));
        assert!(html.contains("value=\"suggest_save\""));
    }

    #[test]
    fn wizard_forms_validate_names_and_classification_without_requiring_weights() {
        let form =
            parse_form(b"name=New++Curl&movement=elbow-flexion&equipment=dumbbell&intent=save")
                .unwrap();
        assert_eq!(form.definition.name, "New Curl");
        assert!(form.definition.weights.is_empty());
        assert_eq!(form.definition.movements, ["elbow-flexion"]);
        assert!(parse_form(b"name=A&name=B").is_err());
        assert!(parse_form(b"name=A&movement=made-up").is_err());
        assert!(parse_form(b"name=A&ratio_biceps=101").is_err());
        assert!(parse_form(b"name=A&ratio_unknown=10").is_err());
        assert!(parse_form(b"name=A&ratio_biceps=10&ratio_biceps=20").is_err());
        assert!(parse_form(b"name=A&extra=unsafe").is_err());
        assert!(parse_form(b"name=").is_err());
        let form = parse_form(b"name=A&ratio_biceps=100&ratio_brachialis=45").unwrap();
        assert_eq!(form.definition.weights.values().sum::<u32>(), 145);
    }
}
