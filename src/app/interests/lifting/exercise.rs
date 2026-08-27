//! `/fitness/exercise/{name}` — one exercise's muscle weights, and the
//! admin's only surface for editing them.
//!
//! The URL segment is the percent-encoded exact exercise name, the same
//! convention as the `?exercise=` filter. Anyone can read the page; the
//! signed-in `ADMIN_EMAIL` additionally sees the weight inputs. The POST
//! repeats the admin check with positive same-origin evidence and bounds
//! the body before parsing — the form is not an authorization boundary
//! (`docs/auth.md`). A successful save replaces the exercise's rows with
//! `source='admin'`, bumps the fitness version, and rebuilds the snapshot,
//! so every page reflects the new ratios immediately. Responses that
//! redirect are hand-built `Ok(303)`s so every branch carries `no-store`.

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, error::not_found, error::redirect_permanent,
        header, page, path_param, query_params, request::headers, response::Response, route,
        to_bytes,
    },
    view::{class, component, view},
};

use crate::{
    app::login::viewer,
    components::shell,
    content::access::is_admin,
    util::{is_same_origin, urlencode},
};

use super::{
    META_LABEL,
    archive::{db, store::FitnessStore},
    filters::LOG_PATH,
    format::plural,
    muscle_taxonomy, muscles, with_raw_query,
};

const BODY_LIMIT_BYTES: usize = 8 * 1024;
const NO_STORE: &str = "no-store";

/// Canonical page URL for one exercise; the name is always re-encoded, so
/// it is safe in `href`s and `Location` headers alike.
pub(super) fn page_url(name: &str) -> String {
    format!("/fitness/exercise/{}", urlencode(name))
}

path_param!(exercise_name);

#[query_params(error = redirect("?"))]
struct ExerciseQuery {
    notice: Option<String>,
}

#[page("/fitness/exercise/{exercise_name}")]
async fn exercise_page(cx: &Cx) -> Result {
    let name = path_param::<ExerciseName>(cx);
    if !plausible_exercise_name(name) {
        return Err(not_found().into());
    }
    let snapshot = match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("fitness snapshot fetch failed for exercise page: {error}");
            return view! {
                ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
                shell(
                    page: "Exercise",
                    active: "",
                    runtime: false,
                    fitness_pwa: true,
                    <header class="rail-row mt-16">
                        <p class="rail-stamp rail-stamp-label">"exercise"</p>
                        <h1 class="font-display text-4xl font-bold tracking-tight break-words">
                            (name)
                        </h1>
                    </header>
                    <p class="mt-8 max-w-prose text-ink2">
                        "The archive is unreachable right now, so this exercise cannot \
                         be shown. It usually recovers within a few seconds."
                    </p>
                )
            };
        }
    };
    let Some(profile) = snapshot.exercise_profile(name) else {
        return Err(not_found().into());
    };
    let weights: Vec<(&'static str, u32)> = snapshot
        .exercise_weight_map()
        .get(name)
        .cloned()
        .unwrap_or_default();
    let tags: Vec<(String, String)> = snapshot
        .exercise_tag_map()
        .get(name)
        .cloned()
        .unwrap_or_default();
    let involvement = muscles::involvement_for_exercises([name], snapshot.exercise_weight_map());

    let can_edit = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let notice = if can_edit {
        query_params::<ExerciseQuery>(cx)?
            .notice
            .as_deref()
            .map(|code| match code {
                "saved" => "Saved — every page now uses the new ratios.",
                "invalid" => "That didn't validate; nothing changed.",
                "unavailable" => "The weight store didn't answer; nothing changed.",
                _ => "Nothing changed.",
            })
    } else {
        None
    };
    // Provenance rides only on the admin variant: it needs a second read and
    // a public page has no use for it.
    let provenance = if can_edit {
        match db_sources(cx, name).await {
            Ok(sources) => provenance_line(&sources),
            Err(error) => {
                eprintln!("exercise weight provenance read failed: {error}");
                None
            }
        }
    } else {
        None
    };

    let history = format!(
        "{} {} across {} {}, {} through {}",
        profile.set_count,
        plural(profile.set_count, "set", "sets"),
        profile.workout_count,
        plural(profile.workout_count, "workout", "workouts"),
        profile.first_date,
        profile.last_date,
    );
    let log_href = format!("{LOG_PATH}?exercise={}#set-log", urlencode(name));
    let title = format!("{name} · Fitness");

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: crate::components::PageMeta::new(title.as_str()).description(history.as_str()),
            active: "",
            runtime: false,
            fitness_pwa: true,
            <header class="rail-row mt-16">
                <p class="rail-stamp rail-stamp-label">"exercise"</p>
                <div class="min-w-0">
                    <h1 class="font-display text-4xl font-bold tracking-tight break-words">
                        (name)
                    </h1>
                    <p class="mt-2 font-meta text-[0.72rem] text-muted">
                        (history.as_str())
                        " · "
                        <a
                            class="text-oxide underline decoration-oxide/35 \
                                 underline-offset-[0.18em]"
                            href=(log_href.as_str())
                        >"view in log"</a>
                    </p>
                    if !tags.is_empty() {
                        <div class="mt-3 flex flex-wrap gap-[0.45rem]">
                            for (kind, value) in &tags {
                                <a
                                    class="inline-flex items-center rounded-full border \
                                         border-hairline px-[0.7rem] py-1 font-meta \
                                         text-[0.7rem] leading-none text-ink2 \
                                         hover:border-oxide hover:text-oxide"
                                    href=(format!(
                                        "{LOG_PATH}?{}={}#set-log",
                                        urlencode(kind),
                                        urlencode(value)
                                    ))
                                >
                                    (value.as_str())
                                </a>
                            }
                        </div>
                    }
                </div>
            </header>
            if let Some(message) = notice {
                <p class="mt-6 max-w-prose border-l-2 border-oxide pl-3 font-meta text-sm text-ink2">
                    (message)
                </p>
            }
            <div class="mt-10 flex flex-wrap items-start gap-x-12 gap-y-8">
                <div class="min-w-0 max-w-[26rem] flex-1">
                    <p class=(META_LABEL)>"muscles worked"</p>
                    if involvement.is_empty() {
                        <p class="mt-2 max-w-prose text-sm text-muted">
                            "This exercise has no muscle weights yet."
                        </p>
                    } else {
                        <div class="mt-3 flex items-start gap-x-6">
                            muscles::muscle_figure(
                                paths: muscles::FRONT_PATHS,
                                caption: "front",
                                involvement: &involvement,
                                compact: false
                            )
                            muscles::muscle_figure(
                                paths: muscles::BACK_PATHS,
                                caption: "back",
                                involvement: &involvement,
                                compact: false
                            )
                        </div>
                    }
                </div>
                <div class="min-w-[18rem] max-w-[30rem] flex-1">
                    if can_edit {
                        <p class=(META_LABEL)>"volume ratios · edit"</p>
                        if let Some(line) = &provenance {
                            <p class="mt-1 font-meta text-[0.68rem] text-muted">(line.as_str())</p>
                        }
                        weight_form(name: name, weights: &weights)
                    } else {
                        <p class=(META_LABEL)>"volume ratios"</p>
                        weight_bars(weights: &weights)
                    }
                </div>
            </div>
        )
    }
}

#[route(GET "/lifting/exercise/{exercise_name}")]
async fn legacy_exercise_page(cx: &Cx) -> Result {
    let name = path_param::<ExerciseName>(cx);
    if !plausible_exercise_name(name) {
        return Err(not_found().into());
    }
    let target = with_raw_query(cx, &page_url(name));
    Err(redirect_permanent(&target).into())
}

/// The read-only ratio list: group headers, one bar per weighted muscle.
#[component]
async fn weight_bars(weights: &[(&'static str, u32)]) -> Result {
    let groups = grouped(weights, false);
    view! {
        if weights.is_empty() {
            <p class="mt-2 max-w-prose text-sm text-muted">
                "No stored weights — sets of this exercise earn no muscle credit."
            </p>
        }
        for group in &groups {
            <p class=(class!(META_LABEL, "mt-4"))>(group.label)</p>
            <ul class="mt-1.5 space-y-1.5">
                for row in &group.rows {
                    <li class="flex items-center gap-3">
                        <span class="w-[8.5rem] flex-none font-meta text-[0.7rem] text-ink2">
                            (row.label)
                        </span>
                        <span
                            class="relative h-1 min-w-0 flex-1 rounded-full bg-hairline"
                            aria-hidden="true"
                        >
                            <span
                                class="absolute inset-y-0 left-0 rounded-full bg-oxide/75"
                                style=(format!("width: {}%", row.ratio))
                            ></span>
                        </span>
                        <span class="w-8 flex-none text-right font-meta text-[0.68rem] text-ink">
                            (format!("{}", row.ratio))
                        </span>
                    </li>
                }
            </ul>
        }
    }
}

/// The admin form: every granular muscle gets a 0–100 input, grouped like
/// the read-only list; 0 or blank means "no connection".
#[component]
async fn weight_form(name: &str, weights: &[(&'static str, u32)]) -> Result {
    let groups = grouped(weights, true);
    let action = page_url(name);
    view! {
        <form method="post" action=(action.as_str()) class="mt-1">
            for group in &groups {
                <p class=(class!(META_LABEL, "mt-4"))>(group.label)</p>
                <ul class="mt-1.5 space-y-1.5">
                    for row in &group.rows {
                        <li class="flex items-center gap-3">
                            <label
                                class="w-[10.5rem] flex-none font-meta text-[0.7rem] text-ink2"
                                for=(format!("ratio-{}", row.id))
                            >
                                (row.label)
                            </label>
                            <input
                                class="w-[4.5rem] flex-none rounded-[0.2rem] border \
                                     border-hairline bg-page px-2 py-1 text-right font-meta \
                                     text-[0.78rem] text-ink outline-none \
                                     focus-visible:outline-solid focus-visible:outline-2 \
                                     focus-visible:outline-oxide focus-visible:outline-offset-2"
                                id=(format!("ratio-{}", row.id))
                                name=(format!("ratio_{}", row.id))
                                type="number"
                                inputmode="numeric"
                                min="0"
                                max="100"
                                step="1"
                                value=(if row.ratio > 0 {
                                    row.ratio.to_string()
                                } else {
                                    String::new()
                                })
                            >
                        </li>
                    }
                </ul>
            }
            <p class="mt-3 max-w-prose font-meta text-[0.65rem] leading-[1.5] text-muted">
                "100 = full volume credit, 50 = half, blank or 0 = none. At least one \
                 muscle must stay above zero."
            </p>
            <button
                type="submit"
                class="mt-3 cursor-pointer rounded-sm border border-oxide px-3 py-2 \
                     font-meta text-xs text-oxide hover:bg-oxide hover:text-card \
                     focus-visible:outline-solid focus-visible:outline-2 \
                     focus-visible:outline-oxide focus-visible:outline-offset-2"
            >"save ratios"</button>
        </form>
    }
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

async fn save_weights_inner(cx: &Cx, body: Body) -> Result<Response> {
    let name = path_param::<ExerciseName>(cx).to_string();
    if !plausible_exercise_name(&name) {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    }
    let ratios = match gate(cx, body).await {
        Ok(ratios) => ratios,
        Err(response) => return Ok(*response),
    };

    // The exercise must exist in the archive; weights for phantom names
    // would be invisible everywhere and only invite typo rows.
    let store = app_context::<FitnessStore>(cx);
    match store.snapshot().await {
        Ok(snapshot) => {
            if snapshot.exercise_profile(&name).is_none() {
                return Ok(plain(StatusCode::NOT_FOUND, "not found"));
            }
        }
        Err(error) => {
            eprintln!("fitness snapshot fetch failed for weight save: {error}");
            return Ok(back(&name, "unavailable"));
        }
    }

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
    parse_weight_form(&bytes).ok_or_else(|| Box::new(plain(StatusCode::BAD_REQUEST, "bad form")))
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

/// Printable, non-empty, and small enough for the schema — the same shape
/// the importer enforces on stored names.
fn plausible_exercise_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && !name.chars().any(char::is_control)
}

struct WeightGroup {
    label: &'static str,
    rows: Vec<WeightRow>,
}

struct WeightRow {
    id: &'static str,
    label: &'static str,
    ratio: u32,
}

/// Group rows in taxonomy display order. The read-only view keeps only
/// weighted muscles; the form lists every muscle so the admin can add one.
fn grouped(weights: &[(&'static str, u32)], include_zero: bool) -> Vec<WeightGroup> {
    muscle_taxonomy::MUSCLE_GROUPS
        .iter()
        .filter_map(|(_, group_label, members)| {
            let rows: Vec<WeightRow> = members
                .iter()
                .filter_map(|(id, label)| {
                    let ratio = weights
                        .iter()
                        .find_map(|(muscle, ratio)| (muscle == id).then_some(*ratio))
                        .unwrap_or(0);
                    (include_zero || ratio > 0).then_some(WeightRow { id, label, ratio })
                })
                .collect();
            (!rows.is_empty()).then_some(WeightGroup {
                label: group_label,
                rows,
            })
        })
        .collect()
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

/// Bounce back to the exercise page with a static notice code — never
/// echoed input, and the name is re-encoded so the `Location` header is
/// always valid ASCII.
fn back(name: &str, notice: &'static str) -> Response {
    see_other(&format!("{}?notice={notice}", page_url(name)))
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
    fn page_urls_reencode_names() {
        assert_eq!(
            page_url("Bench Press (Barbell)"),
            "/fitness/exercise/Bench%20Press%20%28Barbell%29"
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

    /// The exercise pages are dynamic per-name routes like `/fitness/lift/{path}`
    /// permalinks: out of `site_routes()`, like every other public lifting detail.
    #[test]
    fn exercise_pages_stay_out_of_the_route_registry() {
        let sample = page_url("Bench Press");
        assert!(!crate::content::routes::site_routes().contains(&sample));
    }
}
