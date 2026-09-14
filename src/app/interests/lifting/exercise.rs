//! Exercise identity and legacy weight writes; details live in the catalog dialog.
pub(super) mod details;

use benjisponge::data::Data;
use sha2::{Digest, Sha256};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode,
        error::not_found,
        error::redirect_permanent,
        header, path_param,
        request::{headers, uri},
        response::{IntoResponse, Response},
        route, to_bytes,
    },
    view::{component, view},
};

use crate::{
    app::login::viewer,
    components::shell,
    content::access::is_admin,
    util::{is_same_origin, urlencode},
};

use super::{
    archive::{db, store::FitnessStore},
    muscle_taxonomy,
};

const BODY_LIMIT_BYTES: usize = 8 * 1024;
const NO_STORE: &str = "no-store";

/// Canonical page URL for one exercise; the name is always re-encoded, so
/// it is safe in `href`s and `Location` headers alike.
pub(super) fn page_url(name: &str) -> String {
    super::exercise_space::page_url(Some(name))
}

pub(super) fn details_url(name: &str) -> String {
    format!("{}&details=1", page_url(name))
}

// Write endpoints keep their established contracts while GET pages move to the catalog.
pub(super) fn write_url(name: &str) -> String {
    format!("/fitness/exercise/{}", urlencode(name))
}

path_param!(exercise_name);

#[route(GET "/fitness/exercise/{exercise_name}")]
async fn exercise_page(cx: &Cx) -> Result {
    legacy_exercise_redirect(cx)
}

#[route(GET "/lifting/exercise/{exercise_name}")]
async fn legacy_exercise_page(cx: &Cx) -> Result {
    legacy_exercise_redirect(cx)
}

fn legacy_exercise_redirect(cx: &Cx) -> Result {
    let name = path_param::<ExerciseName>(cx);
    if !plausible_exercise_name(name) {
        return Err(not_found().into());
    }
    let target = uri(cx).query().map_or_else(
        || details_url(name),
        |query| format!("{}&{query}", details_url(name)),
    );
    Err(redirect_permanent(target).into())
}

/// Canonical-name and alias editor. Renames keep the former canonical name
/// automatically, so the textarea is both transparent and reversible on a
/// later save.
#[component]
async fn identity_form(name: &str, aliases: &[String]) -> Result {
    let action = format!("{}/identity", write_url(name));
    let alias_lines = aliases.join("\n");
    view! {
        <form method="post" action=(action.as_str()) class="exercise-identity-form" data-exercise-identity-form="">
            <label class="block space-y-1.5" for="canonical-exercise-name">
                <span class="block font-meta text-[0.7rem] text-ink2">"canonical name"</span>
                <input
                    id="canonical-exercise-name"
                    name="canonical_name"
                    type="text"
                    required=""
                    maxlength="200"
                    autocomplete="off"
                    value=(name)
                    class="block w-full rounded-[0.2rem] border border-hairline bg-page px-3 \
                         py-2 font-meta text-sm text-ink outline-none \
                         focus-visible:outline-solid focus-visible:outline-2 \
                         focus-visible:outline-oxide focus-visible:outline-offset-2"
                >
            </label>
            <label class="block space-y-1.5" for="exercise-aliases">
                <span class="block font-meta text-[0.7rem] text-ink2">
                    "aliases · one per line"
                </span>
                <textarea
                    id="exercise-aliases"
                    name="aliases"
                    rows="4"
                    maxlength="6400"
                    autocomplete="off"
                    spellcheck="false"
                    class="block w-full resize-y rounded-[0.2rem] border border-hairline \
                         bg-page px-3 py-2 font-mono text-sm leading-relaxed text-ink \
                         outline-none focus-visible:outline-solid focus-visible:outline-2 \
                         focus-visible:outline-oxide focus-visible:outline-offset-2"
                >(alias_lines.as_str())</textarea>
            </label>
            <p class="max-w-prose font-meta text-[0.65rem] leading-[1.5] text-muted">
                "Renaming rewrites the normalized history and keeps the old name as an alias. \
                 Uploads using any listed name merge into this exercise. If a listed name \
                 already owns lift history, you will review the merge before anything changes."
            </p>
            <button
                type="submit"
                class="cursor-pointer rounded-sm border border-oxide px-3 py-2 font-meta \
                     text-xs text-oxide hover:bg-oxide hover:text-card \
                     focus-visible:outline-solid focus-visible:outline-2 \
                     focus-visible:outline-oxide focus-visible:outline-offset-2"
            >"Review name & aliases"</button>
            <p role="status" data-editor-status=""></p>
        </form>
    }
}

/// Server-rendered second step for every identity mutation. The digest binds
/// the confirm button to the exact names and fitness version shown here; if
/// anything changes before the second POST, the handler shows a fresh review
/// instead of applying a different merge.
async fn identity_review(
    cx: &Cx,
    plan: &db::ExerciseIdentityPlan,
    confirmation: &str,
) -> Result<Response> {
    let renamed = plan.current_name != plan.canonical_name;
    let merged: Vec<&str> = plan
        .merge_names
        .iter()
        .filter(|name| *name != &plan.current_name)
        .map(String::as_str)
        .collect();
    let heading = if merged.is_empty() {
        "Apply this exercise identity change?"
    } else {
        "Merge these exercise histories?"
    };
    let confirm_label = if merged.is_empty() {
        "confirm change"
    } else {
        "confirm merge"
    };
    let action = format!("{}/identity", write_url(&plan.current_name));
    let cancel = details_url(&plan.current_name);
    let alias_lines = plan.aliases.join("\n");
    let __cx = cx;
    let content = view! {
        <section data-exercise-details-content="" data-exercise-name=(plan.current_name.as_str()) class="exercise-identity-review">
            <h2 tabindex="-1" data-details-heading="">(heading)</h2>
            <p>"Nothing has changed yet. Review the names and histories below."</p>
            <h3>(if renamed { "Rename" } else { "Canonical name" })</h3>
            <p>
                if renamed { (plan.current_name.as_str()) " → " }
                <strong>(plan.canonical_name.as_str())</strong>
            </p>
            if !merged.is_empty() {
                <h3>"Existing histories to merge"</h3>
                <ul>for name in &merged { <li>(*name)</li> }</ul>
            }
            if !plan.added_aliases.is_empty() {
                <h3>"Aliases to add or carry forward"</h3>
                <ul>for alias in &plan.added_aliases { <li>(alias.as_str())</li> }</ul>
            }
            if !plan.removed_aliases.is_empty() {
                <h3>"Aliases to remove"</h3>
                <ul>for alias in &plan.removed_aliases { <li>(alias.as_str())</li> }</ul>
            }
            <p>"Confirming moves normalized set history under "<strong>(plan.canonical_name.as_str())</strong>
                ", recomputes records from the combined history, and preserves every raw imported exercise name. The exercise you started from keeps its taxonomy and muscle weights when present."
            </p>
            <form method="post" action=(action.as_str()) data-exercise-identity-form="">
                <input type="hidden" name="canonical_name" value=(plan.canonical_name.as_str())>
                <input type="hidden" name="aliases" value=(alias_lines.as_str())>
                <input type="hidden" name="confirmation" value=(confirmation)>
                <div class="exercise-details__save">
                    <a href=(cancel.as_str()) data-exercise-details-link="">"Back to details"</a>
                    <button class="entry-button entry-button--primary" type="submit">(confirm_label)</button>
                </div>
                <p role="status" data-editor-status=""></p>
            </form>
        </section>
    }?;
    if headers(cx)
        .get("x-exercise-dialog")
        .is_some_and(|value| value == "1")
    {
        return view! { ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE))) (content) }?
            .into_response(cx);
    }
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(page: "Exercises", active: "", runtime: false, fitness_pwa: true,
            <section class="exercise-library">
                super::exercise_library::catalog_header(list: false)
                <dialog class="exercise-details" data-exercise-details-dialog="" aria-label="Exercise details" open="">
                    <a class="exercise-details__close" href=(cancel.as_str()) data-exercise-details-close="" aria-label="Close exercise details">"×"</a>
                    <div data-exercise-details-body="">(content)</div>
                </dialog>
            </section>
            <script type="module" src=(details::DETAILS_JS)></script>
        )
    }?.into_response(cx)
}

fn identity_confirmation_digest(plan: &db::ExerciseIdentityPlan) -> String {
    fn field(hasher: &mut Sha256, value: &str) {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }

    let mut hasher = Sha256::new();
    hasher.update(b"fitness-exercise-identity-v1");
    hasher.update(plan.version.to_le_bytes());
    field(&mut hasher, &plan.current_name);
    field(&mut hasher, &plan.canonical_name);
    for alias in &plan.aliases {
        field(&mut hasher, alias);
    }
    for name in &plan.merge_names {
        field(&mut hasher, name);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[route(POST "/fitness/exercise/{exercise_name}")]
async fn save_weights(cx: &Cx, body: Body) -> Result<Response> {
    save_weights_inner(cx, body).await
}

/// Keep already-rendered admin forms functional during the permanent URL
/// migration. Successful responses still point at the canonical page.
#[route(POST "/lifting/exercise/{exercise_name}")]
async fn legacy_save_weights(cx: &Cx, body: Body) -> Result<Response> {
    save_weights_inner(cx, body).await
}

#[route(POST "/fitness/exercise/{exercise_name}/identity")]
async fn save_identity(cx: &Cx, body: Body) -> Result<Response> {
    save_identity_inner(cx, body).await
}

#[route(POST "/lifting/exercise/{exercise_name}/identity")]
async fn legacy_save_identity(cx: &Cx, body: Body) -> Result<Response> {
    save_identity_inner(cx, body).await
}

async fn save_identity_inner(cx: &Cx, body: Body) -> Result<Response> {
    let requested_name = path_param::<ExerciseName>(cx).to_string();
    if !plausible_exercise_name(&requested_name) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    let form = match gate_identity(cx, body).await {
        Ok(form) => form,
        Err(response) => return Ok(*response),
    };

    let store = app_context::<FitnessStore>(cx);
    let canonical_name = match store.snapshot().await {
        Ok(snapshot) => match snapshot.canonical_exercise_name(&requested_name) {
            Some(name) => name,
            None => return Ok(plain(StatusCode::NOT_FOUND, "not found")),
        },
        Err(error) => {
            eprintln!("fitness snapshot fetch failed for identity save: {error}");
            return Ok(back(&requested_name, "unavailable"));
        }
    };
    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(error) => {
            eprintln!("exercise identity save could not reach the database: {error}");
            return Ok(back(&canonical_name, "unavailable"));
        }
    };
    let plan =
        match db::plan_exercise_identity(&db, &canonical_name, &form.canonical_name, &form.aliases)
            .await
        {
            Ok(Some(plan)) => plan,
            Ok(None) => return Ok(plain(StatusCode::NOT_FOUND, "not found")),
            Err(error) => {
                eprintln!("exercise identity preview failed: {error}");
                return Ok(back(&canonical_name, "unavailable"));
            }
        };
    let confirmation = identity_confirmation_digest(&plan);
    if plan.mutated && form.confirmation.as_deref() != Some(confirmation.as_str()) {
        return identity_review(cx, &plan, &confirmation).await;
    }

    match db::replace_exercise_identity(&db, &plan, epoch_seconds()).await {
        Ok(db::ExerciseIdentityOutcome::Saved {
            canonical_name,
            mutated,
            ..
        }) => {
            if mutated && let Err(error) = store.rebuild().await {
                // The transaction committed. The version backstop will pick
                // it up even if this eager rebuild is temporarily unavailable.
                eprintln!("post-identity-save snapshot rebuild failed: {error}");
            }
            Ok(back(&canonical_name, "identity-saved"))
        }
        Ok(db::ExerciseIdentityOutcome::Stale) => Ok(back(&canonical_name, "identity-stale")),
        Ok(db::ExerciseIdentityOutcome::NotFound) => Ok(plain(StatusCode::NOT_FOUND, "not found")),
        Err(error) => {
            eprintln!("exercise identity save failed: {error}");
            Ok(back(&canonical_name, "unavailable"))
        }
    }
}

async fn save_weights_inner(cx: &Cx, body: Body) -> Result<Response> {
    let requested_name = path_param::<ExerciseName>(cx).to_string();
    if !plausible_exercise_name(&requested_name) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    let ratios = match gate(cx, body).await {
        Ok(ratios) => ratios,
        Err(response) => return Ok(*response),
    };

    // The exercise must exist in the archive; weights for phantom names
    // would be invisible everywhere and only invite typo rows.
    let store = app_context::<FitnessStore>(cx);
    let name = match store.snapshot().await {
        Ok(snapshot) => match snapshot.canonical_exercise_name(&requested_name) {
            Some(name) => name,
            None => return Ok(plain(StatusCode::NOT_FOUND, "not found")),
        },
        Err(error) => {
            eprintln!("fitness snapshot fetch failed for weight save: {error}");
            return Ok(back(&requested_name, "unavailable"));
        }
    };

    let kept: Vec<(String, u32)> = ratios.into_iter().filter(|(_, ratio)| *ratio > 0).collect();
    if kept.is_empty() {
        // An all-zero save would delete every row and re-open the exercise
        // to reseeding on the next reconcile — reject it instead.
        return Ok(back(&name, "invalid"));
    }

    let db = match app_context::<Data>(cx).db().await {
        Ok(db) => db,
        Err(error) => {
            eprintln!("weight save could not reach the database: {error}");
            return Ok(back(&name, "unavailable"));
        }
    };
    match db::replace_exercise_weights(&db, &name, &kept, epoch_seconds()).await {
        Ok(_) => {
            if let Err(error) = store.rebuild().await {
                // The commit already landed; the debounced version check
                // picks it up within seconds even if this rebuild failed.
                eprintln!("post-save snapshot rebuild failed: {error}");
            }
            Ok(back(&name, "saved"))
        }
        Err(error) => {
            eprintln!("weight save failed: {error}");
            Ok(back(&name, "unavailable"))
        }
    }
}

/// The shared preamble the POST runs before believing anything in the body.
/// Order is load-bearing: viewer → admin → same-origin → content type →
/// bounded body → strict parse (`src/app/admin.rs` is the pattern).
async fn gate(cx: &Cx, body: Body) -> std::result::Result<Vec<(String, u32)>, Box<Response>> {
    let bytes = admin_form_body(cx, body).await?;
    parse_weight_form(&bytes).ok_or_else(|| Box::new(plain(StatusCode::BAD_REQUEST, "bad form")))
}

async fn gate_identity(cx: &Cx, body: Body) -> std::result::Result<IdentityForm, Box<Response>> {
    let bytes = admin_form_body(cx, body).await?;
    parse_identity_form(&bytes).ok_or_else(|| Box::new(plain(StatusCode::BAD_REQUEST, "bad form")))
}

async fn admin_form_body(cx: &Cx, body: Body) -> std::result::Result<Vec<u8>, Box<Response>> {
    let name = path_param::<ExerciseName>(cx);
    if viewer(cx).is_none() {
        let login = format!("/login?next={}", urlencode(&page_url(name)));
        return Err(Box::new(see_other(&login)));
    }
    let current = viewer(cx).expect("viewer checked above");
    if !is_admin(&current.email) {
        return Err(Box::new(plain(StatusCode::NOT_FOUND, "not found")));
    }
    if !is_same_origin(headers(cx)) {
        return Err(Box::new(plain(StatusCode::FORBIDDEN, "forbidden")));
    }
    if !is_form_content_type(headers(cx)) {
        return Err(Box::new(plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        )));
    }
    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(Box::new(plain(
                StatusCode::PAYLOAD_TOO_LARGE,
                "form is too large",
            )));
        }
    };
    Ok(bytes.to_vec())
}

/// Exactly one `ratio_<muscle>` field per canonical muscle, nothing else.
/// Blank means zero; anything non-numeric or out of range fails the parse.
fn parse_weight_form(body: &[u8]) -> Option<Vec<(String, u32)>> {
    let mut ratios: Vec<(String, Option<u32>)> = muscle_taxonomy::muscles()
        .map(|(id, _)| (id.to_string(), None))
        .collect();
    for (key, value) in form_urlencoded::parse(body) {
        let muscle = key.strip_prefix("ratio_")?;
        let slot = ratios
            .iter_mut()
            .find(|(id, _)| id == muscle)
            .filter(|(_, seen)| seen.is_none())?;
        let trimmed = value.trim();
        let ratio = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse::<u32>().ok().filter(|ratio| *ratio <= 100)?
        };
        slot.1 = Some(ratio);
    }
    ratios
        .into_iter()
        .map(|(id, ratio)| ratio.map(|ratio| (id, ratio)))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentityForm {
    canonical_name: String,
    aliases: Vec<String>,
    confirmation: Option<String>,
}

/// Exactly one canonical-name field and one newline-delimited alias field.
/// Names normalize whitespace the same way the CSV and Lyfta parsers do.
fn parse_identity_form(body: &[u8]) -> Option<IdentityForm> {
    let mut canonical_name = None;
    let mut alias_text = None;
    let mut confirmation = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "canonical_name" => {
                if canonical_name.is_some() {
                    return None;
                }
                canonical_name = Some(normalize_exercise_name(&value)?);
            }
            "aliases" => {
                if alias_text.is_some() {
                    return None;
                }
                alias_text = Some(value.into_owned());
            }
            "confirmation" => {
                if confirmation.is_some()
                    || value.len() != 64
                    || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return None;
                }
                confirmation = Some(value.into_owned());
            }
            _ => return None,
        }
    }
    let canonical_name = canonical_name?;
    let alias_text = alias_text?;
    let mut seen = std::collections::HashSet::new();
    let mut aliases = Vec::new();
    for line in alias_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let alias = normalize_exercise_name(line)?;
        if !seen.insert(alias.clone()) || aliases.len() == 32 {
            return None;
        }
        aliases.push(alias);
    }
    Some(IdentityForm {
        canonical_name,
        aliases,
        confirmation,
    })
}

fn normalize_exercise_name(name: &str) -> Option<String> {
    let normalized = name.split_whitespace().collect::<Vec<_>>().join(" ");
    plausible_exercise_name(&normalized).then_some(normalized)
}

/// Printable, non-empty, and small enough for the schema — the same shape
/// the importer enforces on stored names.
fn plausible_exercise_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && !name.chars().any(char::is_control)
}

/// One human line for the admin: where the current rows came from.
fn provenance_line(sources: &[String]) -> Option<String> {
    if sources.is_empty() {
        return None;
    }
    let mut kinds: Vec<&str> = sources.iter().map(String::as_str).collect();
    kinds.sort_unstable();
    kinds.dedup();
    Some(match kinds.as_slice() {
        ["admin"] => "hand-tuned (admin)".to_string(),
        ["seed"] => "research seed defaults".to_string(),
        ["derived"] => "derived from taxonomy tags".to_string(),
        _ => format!("mixed sources: {}", kinds.join(", ")),
    })
}

async fn db_sources(cx: &Cx, name: &str) -> anyhow::Result<Vec<String>> {
    let db = app_context::<Data>(cx).db().await?;
    Ok(db::exercise_weights(&db, name)
        .await?
        .into_iter()
        .map(|(_, _, source)| source)
        .collect())
}

fn is_form_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

fn epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Bounce back to the catalog dialog with a static notice code — never
/// echoed input, and the name is re-encoded so the `Location` header is
/// always valid ASCII.
fn back(name: &str, notice: &'static str) -> Response {
    see_other(&format!("{}&notice={notice}", details_url(name)))
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("urlencoded locations are valid headers")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(message))
        .expect("static headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_wants_every_muscle_exactly_once() {
        let full: String = muscle_taxonomy::muscles()
            .map(|(id, _)| format!("ratio_{id}=0"))
            .collect::<Vec<_>>()
            .join("&");
        let ratios = parse_weight_form(full.as_bytes()).expect("all-zero parses");
        assert_eq!(ratios.len(), 28);
        assert!(ratios.iter().all(|(_, ratio)| *ratio == 0));

        let with_values = full
            .replace("ratio_quads=0", "ratio_quads=100")
            .replace("ratio_glute-max=0", "ratio_glute-max=");
        let ratios = parse_weight_form(with_values.as_bytes()).expect("blank means zero");
        assert!(ratios.contains(&("quads".to_string(), 100)));
        assert!(ratios.contains(&("glute-max".to_string(), 0)));

        // Missing, duplicate, unknown, or out-of-range fields fail.
        assert!(parse_weight_form(b"ratio_quads=100").is_none());
        assert!(parse_weight_form(format!("{full}&ratio_quads=50").as_bytes()).is_none());
        assert!(parse_weight_form(format!("{full}&ratio_bogus=50").as_bytes()).is_none());
        assert!(
            parse_weight_form(full.replace("ratio_quads=0", "ratio_quads=101").as_bytes())
                .is_none()
        );
        assert!(
            parse_weight_form(full.replace("ratio_quads=0", "ratio_quads=abc").as_bytes())
                .is_none()
        );
    }

    #[test]
    fn identity_form_normalizes_and_strictly_bounds_names() {
        let parsed = parse_identity_form(
            b"canonical_name=Barbell+Resurrection+Lifts&aliases=Barbell+Pullover+Crunches%0D%0A++Pullover+++Crunches++%0A",
        )
        .unwrap();
        assert_eq!(parsed.canonical_name, "Barbell Resurrection Lifts");
        assert_eq!(
            parsed.aliases,
            vec!["Barbell Pullover Crunches", "Pullover Crunches"]
        );
        assert_eq!(parsed.confirmation, None);

        let confirmed = parse_identity_form(
            b"canonical_name=Press&aliases=Military+Press&confirmation=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(
            confirmed.confirmation.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );

        assert!(parse_identity_form(b"canonical_name=Press&aliases=A&aliases=B").is_none());
        assert!(parse_identity_form(b"canonical_name=Press&aliases=A%0AA").is_none());
        assert!(parse_identity_form(b"canonical_name=&aliases=").is_none());
        assert!(parse_identity_form(b"canonical_name=Press&aliases=&extra=nope").is_none());
        assert!(parse_identity_form(b"canonical_name=Press&aliases=&confirmation=nope").is_none());
    }

    #[test]
    fn identity_confirmation_is_bound_to_the_reviewed_plan() {
        let plan = db::ExerciseIdentityPlan {
            current_name: "Military Press".into(),
            canonical_name: "Barbell Overhead Press".into(),
            aliases: vec!["Military Press".into()],
            merge_names: vec!["Military Press".into()],
            added_aliases: vec!["Military Press".into()],
            removed_aliases: Vec::new(),
            version: 7,
            mutated: true,
        };
        let digest = identity_confirmation_digest(&plan);
        assert_eq!(digest.len(), 64);

        let mut changed = plan.clone();
        changed.aliases.push("Strict Press".into());
        assert_ne!(identity_confirmation_digest(&changed), digest);
        changed = plan.clone();
        changed.version += 1;
        assert_ne!(identity_confirmation_digest(&changed), digest);
    }

    #[test]
    fn page_urls_reencode_names() {
        assert_eq!(
            page_url("Bench Press (Barbell)"),
            "/fitness/exercises?exercise=Bench%20Press%20%28Barbell%29"
        );
        assert!(plausible_exercise_name("Sled 45° Leg Press"));
        assert!(!plausible_exercise_name(""));
        assert!(!plausible_exercise_name("line\nbreak"));
    }

    #[test]
    fn provenance_lines_summarize_sources() {
        assert_eq!(provenance_line(&[]), None);
        assert_eq!(
            provenance_line(&["seed".into(), "seed".into()]).as_deref(),
            Some("research seed defaults")
        );
        assert_eq!(
            provenance_line(&["admin".into()]).as_deref(),
            Some("hand-tuned (admin)")
        );
        assert!(
            provenance_line(&["seed".into(), "admin".into()])
                .unwrap()
                .starts_with("mixed sources")
        );
    }

    /// Selected exercises use query strings, not separate registry entries.
    #[test]
    fn exercise_pages_stay_out_of_the_route_registry() {
        let sample = page_url("Bench Press");
        assert!(!crate::content::routes::site_routes().contains(&sample));
    }
}
