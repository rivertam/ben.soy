//! Owner-only weekly target editor; the resulting goals are public on Fitness.

use std::collections::{HashMap, HashSet};

use benjisponge::data::{
    Data,
    fitness_models::{MAX_MUSCLE_TARGET_CENTI_POINTS, MuscleTarget},
};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, header, page, query_params,
        request::headers,
        response::{IntoResponse, Response},
        route, to_bytes,
    },
    view::{component, view},
};

use super::{
    archive::{eastern, store::FitnessStore, targets as store},
    muscle_taxonomy,
    training_focus::{BASELINE_WEEKS, format_ratio},
};
use crate::{
    app::{login::viewer, not_found::not_found_page},
    components::{page_head, shell},
    content::access::is_admin,
    util::is_same_origin,
};

const PATH: &str = "/admin/fitness-targets";
const LOGIN: &str = "/login?next=%2Fadmin%2Ffitness-targets";
const BODY_LIMIT: usize = 16 * 1024;
const NO_STORE: &str = "no-store";
const TARGETS_JS: Asset = asset!("./targets.js");

#[query_params(error = redirect("?"))]
struct TargetQuery {
    notice: Option<String>,
}

#[page("/admin/fitness-targets")]
async fn editor(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return view! {
            (StatusCode::SEE_OTHER)
            ((header::LOCATION, HeaderValue::from_static(LOGIN)))
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        };
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: PATH)
        };
    }
    let query = query_params::<TargetQuery>(cx)?;
    let loaded = async {
        let db = app_context::<Data>(cx).db().await?;
        store::read(&db).await
    }
    .await;
    let (form, enabled, notice) = match loaded {
        Ok(rows) => (
            TargetForm::saved(&rows),
            true,
            match query.notice.as_deref() {
                Some("saved") => {
                    Some("Targets saved. They are visible on Fitness and guide your next focus.")
                }
                Some("refresh-delayed") => Some(
                    "Targets saved. The load panel is still refreshing; your changes are stored.",
                ),
                _ => None,
            },
        ),
        Err(error) => {
            eprintln!("fitness targets load failed: {error}");
            (
                TargetForm::default(),
                false,
                Some("Targets are unavailable right now. Reload this page to try again."),
            )
        }
    };
    view! { editor_document(form: &form, enabled: enabled, notice: notice) }
}

#[component]
async fn editor_document(
    cx: &Cx,
    form: &TargetForm,
    enabled: bool,
    notice: Option<&str>,
) -> Result {
    let focus = app_context::<FitnessStore>(cx)
        .snapshot()
        .await
        .ok()
        .map(|snapshot| snapshot.training_focus(eastern::eastern_date(jiff::Timestamp::now())));
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: "Muscle targets", active: "", runtime: false,
            page_head(stamp: "admin", title: "Muscle targets", lede: "Weekly volume-point goals for my training. Targets are visible to everyone on Fitness.")
            <div class="mt-6 flex flex-wrap gap-5 font-meta text-xs">
                <a href="/fitness" class="text-oxide underline underline-offset-4">"View muscle loads"</a>
                <a href="/admin" class="text-ink2 underline underline-offset-4">"Admin"</a>
            </div>
            if let Some(message) = notice {
                <p class="mt-6 border-l-2 border-oxide pl-3 text-sm text-ink2" role="status">(message)</p>
            }
            <p id="target-help" class="mt-8 max-w-prose text-sm leading-relaxed text-ink2">
                "Drag a diamond to set a weekly target, or type an exact value. Use usual clears the target; 0 sets no work to catch up on. Save when you’re ready."
            </p>
            <p class="mt-3 max-w-prose font-meta text-xs leading-relaxed text-muted">
                "Now is the last seven days; usual is the weekly average over the preceding eight weeks. Targets allow 0–10,000 points in steps of 0.1. Muscles trained today or yesterday still wait for recovery."
            </p>
            if focus.is_none() && enabled {
                <p class="mt-3 text-sm text-muted">"Current loads are unavailable; your saved targets are shown."</p>
            }
            <form action=(PATH) method="post" class="muscle-targets mt-8" aria-describedby="target-help" data-muscle-targets="">
                <fieldset disabled=(!enabled)>
                    <legend class="sr-only">"Weekly muscle targets"</legend>
                    <div class="muscle-targets-legend" aria-label="Muscle load legend">
                        <span><span class="muscle-targets-key muscle-targets-key-now" aria-hidden="true"></span>"now"</span>
                        <span><span class="muscle-targets-key muscle-targets-key-usual" aria-hidden="true"></span>"usual"</span>
                        <span><span class="muscle-targets-key muscle-targets-key-target" aria-hidden="true"></span>"target"</span>
                        <span data-target-scale="" hidden=""></span>
                    </div>
                    <div class="grid gap-x-10 gap-y-8 sm:grid-cols-2">
                        for (_, group, members) in muscle_taxonomy::MUSCLE_GROUPS {
                            <fieldset class="min-w-0">
                                <legend class="w-full border-b border-hairline pb-2 font-meta text-xs uppercase tracking-wider text-muted">(*group)</legend>
                                <p class="muscle-targets-columns" aria-hidden="true">"now / usual / target"</p>
                                for (id, label) in *members {
                                    let load = focus.as_ref().and_then(|focus| focus.muscles.iter().find(|muscle| muscle.id == *id));
                                    let recent_centi = load.map_or(0, |muscle| muscle.recent_centi_points);
                                    let baseline_centi = load.map_or(0, |muscle| muscle.baseline_centi_points);
                                    let recent = focus.as_ref().map(|_| format_ratio(recent_centi, 100));
                                    let usual = focus.as_ref().map(|_| format_ratio(baseline_centi, BASELINE_WEEKS * 100));
                                    let invalid = form.invalid.contains(id);
                                    <div class="muscle-target-row" data-target-row="" data-target-muscle=(id)
                                        data-recent-scaled=(u64::from(recent_centi) * u64::from(BASELINE_WEEKS))
                                        data-usual-scaled=(baseline_centi)>
                                        <div class="muscle-target-heading">
                                            <label id=(format!("target-label-{id}")) for=(format!("target-{id}")) class="font-meta text-sm text-ink">(*label)</label>
                                            <span class="muscle-target-numbers">
                                                <span class="sr-only">"Current load "</span>
                                                <span class="text-ink">(recent.as_deref().unwrap_or("–"))</span>
                                                <span aria-hidden="true">" / "</span>
                                                <span class="sr-only">"; usual weekly pace "</span>
                                                <span class="text-patina">(usual.as_deref().unwrap_or("–"))</span>
                                                <span aria-hidden="true">" / "</span>
                                            <input
                                                id=(format!("target-{id}")) name=(format!("target_{id}"))
                                                type="text" inputmode="decimal" autocomplete="off" maxlength="32"
                                                data-target-value="" placeholder="–"
                                                aria-label=(format!("{label} target in weekly points"))
                                                value=(form.values.get(id).map(String::as_str).unwrap_or(""))
                                                aria-invalid=(if invalid { "true" } else { "false" })
                                                aria-describedby=(if invalid { format!("target-help target-error-{id}") } else { "target-help".into() })
                                                class="muscle-target-value"
                                            >
                                            </span>
                                        </div>
                                        <div class="muscle-target-slider" data-target-slider="" hidden="">
                                            <div class="muscle-target-track" aria-hidden="true">
                                                <span class="muscle-target-now"></span>
                                                <span class="muscle-target-usual" hidden=(baseline_centi == 0)></span>
                                            </div>
                                            <input type="range" min="0" max="10000" step="0.1" value="0"
                                                class="muscle-target-range" data-target-range=""
                                                aria-label=(format!("{label} weekly target")) aria-describedby="target-help">
                                        </div>
                                        <div class="muscle-target-actions" data-target-actions="" hidden="">
                                            <span class="muscle-target-state" data-target-state=""></span>
                                            <button type="button" data-target-clear="" aria-label=(format!("Use usual pace for {label}"))>"Use usual"</button>
                                        </div>
                                        if invalid {
                                            <p id=(format!("target-error-{id}")) data-target-error="" class="mt-2 text-xs text-oxide">"Use 0–10,000 with at most one decimal, or leave blank."</p>
                                        }
                                    </div>
                                }
                            </fieldset>
                        }
                    </div>
                    <button type="submit" class="mt-10 min-h-11 cursor-pointer rounded-sm border border-oxide bg-oxide px-5 py-2.5 font-meta text-sm text-card hover:bg-oxide-hot focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2 disabled:cursor-default disabled:opacity-50">
                        "Save targets"
                    </button>
                </fieldset>
            </form>
            <script type="module" src=(TARGETS_JS)></script>
        )
    }
}

#[route(POST "/admin/fitness-targets")]
async fn save_targets(cx: &Cx, body: Body) -> Result<Response> {
    if let Some(response) = request_gate(
        viewer(cx).map(|current| is_admin(&current.email)),
        headers(cx),
    ) {
        return Ok(response);
    }
    let bytes = match to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(plain(StatusCode::PAYLOAD_TOO_LARGE, "form is too large")),
    };
    let form = TargetForm::parse(&bytes);
    if form.malformed || !form.invalid.is_empty() {
        return failed_form(
            cx,
            &form,
            StatusCode::BAD_REQUEST,
            "Check the target values below. Every muscle must appear once; nothing was saved.",
        )
        .await;
    }
    let rows = form.rows();
    let result = async {
        let db = app_context::<Data>(cx).db().await?;
        store::save(&db, &rows, jiff::Timestamp::now().as_second()).await
    }
    .await;
    if let Err(error) = result {
        eprintln!("fitness targets save failed: {error}");
        return failed_form(
            cx,
            &form,
            StatusCode::SERVICE_UNAVAILABLE,
            "Targets could not be saved. Your entries are still here; try saving again.",
        )
        .await;
    }
    let notice = match app_context::<FitnessStore>(cx).rebuild().await {
        Ok(()) => "saved",
        Err(error) => {
            eprintln!("fitness targets snapshot refresh failed: {error}");
            "refresh-delayed"
        }
    };
    Ok(see_other(&format!("{PATH}?notice={notice}")))
}

async fn failed_form(
    cx: &Cx,
    form: &TargetForm,
    status: StatusCode,
    notice: &str,
) -> Result<Response> {
    let __cx = cx;
    let document = view! {
        (status)
        editor_document(form: form, enabled: true, notice: Some(notice))
    }?;
    document.into_response(cx)
}

fn request_gate(admin: Option<bool>, headers: &HeaderMap) -> Option<Response> {
    match admin {
        None => return Some(see_other(LOGIN)),
        Some(false) => return Some(plain(StatusCode::NOT_FOUND, "not found")),
        Some(true) => (),
    }
    if !is_same_origin(headers) {
        return Some(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    let form_content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        });
    (!form_content_type).then(|| {
        plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded",
        )
    })
}

#[derive(Default)]
struct TargetForm {
    values: HashMap<&'static str, String>,
    invalid: HashSet<&'static str>,
    malformed: bool,
}

impl TargetForm {
    fn saved(rows: &[MuscleTarget]) -> Self {
        Self {
            values: rows
                .iter()
                .map(|row| {
                    (
                        muscle_taxonomy::canonical_muscle(&row.muscle)
                            .expect("stored targets validated"),
                        format_ratio(row.weekly_centi_points as u32, 100),
                    )
                })
                .collect(),
            ..Self::default()
        }
    }

    fn parse(bytes: &[u8]) -> Self {
        let mut form = Self::default();
        for (key, value) in form_urlencoded::parse(bytes) {
            let Some(id) = key
                .strip_prefix("target_")
                .and_then(muscle_taxonomy::canonical_muscle)
            else {
                form.malformed = true;
                continue;
            };
            if form.values.contains_key(id) {
                form.malformed = true;
            } else {
                if parse_points(&value).is_none() {
                    form.invalid.insert(id);
                }
                form.values.insert(id, value.into_owned());
            }
        }
        form.malformed |= form.values.len() != muscle_taxonomy::muscles().count();
        form
    }

    fn rows(&self) -> Vec<MuscleTarget> {
        muscle_taxonomy::muscles()
            .filter_map(|(id, _)| {
                let weekly_centi_points = parse_points(self.values.get(id)?).flatten()?;
                Some(MuscleTarget {
                    muscle: id.into(),
                    weekly_centi_points,
                })
            })
            .collect()
    }
}

/// Parse at most one decimal exactly, preserving blank versus literal zero.
fn parse_points(value: &str) -> Option<Option<i64>> {
    let value = value.trim();
    if value.is_empty() {
        return Some(None);
    }
    let (whole, decimal) = value
        .split_once('.')
        .map_or((value, None), |(whole, decimal)| (whole, Some(decimal)));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let fractional = match decimal {
        None => 0,
        Some(decimal) if decimal.len() == 1 && decimal.as_bytes()[0].is_ascii_digit() => {
            i64::from(decimal.as_bytes()[0] - b'0') * 10
        }
        _ => return None,
    };
    let centi = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fractional)?;
    (centi <= MAX_MUSCLE_TARGET_CENTI_POINTS).then_some(Some(centi))
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("static location")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(message))
        .expect("static response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::{
        context::CxTestBuilder,
        cookie::{
            CookieJarCell, Cookies, Key, RouterBuilderCookieExt, private_cookies, write_cookies,
        },
        font::RouterBuilderFontExt,
        router::{Router, request::Request},
    };

    fn form_body(overrides: &[(&str, &str)]) -> String {
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(muscle_taxonomy::muscles().map(|(id, _)| {
                (
                    format!("target_{id}"),
                    overrides
                        .iter()
                        .find(|(muscle, _)| *muscle == id)
                        .map_or("", |(_, value)| *value),
                )
            }))
            .finish()
    }

    #[test]
    fn exact_decimal_validation_preserves_blank_and_zero() {
        for (value, expected) in [
            ("", None),
            ("  ", None),
            ("0", Some(0)),
            ("0.0", Some(0)),
            ("12", Some(1200)),
            (" 12.5 ", Some(1250)),
            ("10000.0", Some(1_000_000)),
        ] {
            assert_eq!(parse_points(value), Some(expected), "{value}");
        }
        for value in [
            "-1",
            "+1",
            "NaN",
            "inf",
            "1e2",
            "12.50",
            "1.",
            ".5",
            "10000.1",
            "10001",
            "999999999999999999999999",
            "1,000",
            "１",
            "1.2.3",
        ] {
            assert_eq!(parse_points(value), None, "{value}");
        }
        let form = TargetForm::parse(form_body(&[("biceps", "12.5"), ("abs", "0")]).as_bytes());
        assert!(!form.malformed);
        assert!(form.invalid.is_empty());
        assert_eq!(
            form.rows(),
            [
                MuscleTarget {
                    muscle: "biceps".into(),
                    weekly_centi_points: 1250
                },
                MuscleTarget {
                    muscle: "abs".into(),
                    weekly_centi_points: 0
                },
            ]
        );
        assert!(
            TargetForm::parse(form_body(&[]).as_bytes())
                .rows()
                .is_empty()
        );
    }

    #[test]
    fn forms_reject_missing_unknown_and_duplicate_fields_and_retain_invalid_text() {
        let body = form_body(&[]);
        for invalid in [
            String::new(),
            format!("{body}&target_abs=1"),
            format!("{body}&target_chest=1"),
            format!("{body}&unexpected=1"),
        ] {
            assert!(TargetForm::parse(invalid.as_bytes()).malformed);
        }
        let form =
            TargetForm::parse(form_body(&[("biceps", "12.5"), ("abs", "bad input")]).as_bytes());
        assert!(!form.malformed);
        assert!(form.invalid.contains("abs"));
        assert_eq!(form.values["abs"], "bad input");
        assert_eq!(form.values["biceps"], "12.5");
    }

    fn test_key() -> Key {
        Key::derive_from(b"fitness-target-local-test-key-32-bytes")
    }

    fn test_cookie(email: &str) -> String {
        let (parts, ()) = http::Request::builder().body(()).unwrap().into_parts();
        let cx = CxTestBuilder::new()
            .app_context(test_key())
            .request_context(parts)
            .request_context(CookieJarCell::new())
            .build();
        private_cookies(&cx).override_prefix_host().add(("viewer", serde_json::json!({
            "sub": "fitness-target-test", "email": email, "exp": jiff::Timestamp::now().as_second() + 3600,
        }).to_string()));
        let mut headers = HeaderMap::new();
        write_cookies(&cx, &mut headers);
        headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    fn test_assets() -> topcoat::asset::AssetConfig {
        use topcoat::asset::{AssetConfig, Manifest, ManifestEntry, RawAsset};
        let binary = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let manifest = Manifest {
            version: 1,
            assets: RawAsset::find_in_binary(&binary)
                .into_iter()
                .map(|asset| ManifestEntry {
                    id: asset.id(),
                    file: format!("{}.asset", asset.id().as_u64()),
                    hash: "test".into(),
                    content_type: "application/octet-stream".into(),
                })
                .collect(),
        };
        AssetConfig::hosted_at("/test-assets", manifest)
    }

    fn router(data: Data, fitness: FitnessStore) -> Router {
        Router::builder()
            .cookies()
            .discover_fonts()
            .app_context(test_assets())
            .app_context(test_key())
            .app_context(data)
            .app_context(fitness)
            .page(editor)
            .route(save_targets)
            .page(public_panel)
            .build()
    }

    // Exercises the exact shared public component with authenticated and
    // anonymous requests, without loading unrelated activity streams.
    #[page("/test-muscle-loads")]
    async fn public_panel(cx: &Cx) -> Result {
        let snapshot = app_context::<FitnessStore>(cx).snapshot().await?;
        let focus = snapshot.training_focus(eastern::eastern_date(jiff::Timestamp::now()));
        let can_edit = viewer(cx).is_some_and(|current| is_admin(&current.email));
        view! { super::super::training_focus::panel(focus: &focus, heading_id: "test-focus", can_edit: can_edit) }
    }

    async fn body_text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn targets_routes_save_refresh_and_protect_public_and_admin_views() {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("targets").use_db("routes").await.unwrap();
        db.query(include_str!("../../../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        let data = Data::from_initialized_db(db);
        let fitness = FitnessStore::new(data.clone());
        let app = router(data.clone(), fitness.clone());
        let admin_cookie = test_cookie(crate::content::access::ADMIN_EMAIL);
        // Optional cookie for a throwaway local app using this exact test key.
        if let Ok(path) = std::env::var("FITNESS_TARGET_TEST_COOKIE_PATH") {
            std::fs::write(path, &admin_cookie).unwrap();
        }
        for method in ["GET", "POST"] {
            for (cookie, status) in [
                (None, StatusCode::SEE_OTHER),
                (
                    Some(test_cookie("visitor@example.com")),
                    StatusCode::NOT_FOUND,
                ),
            ] {
                let mut request = Request::builder()
                    .method(method)
                    .uri(format!("http://localhost{PATH}"));
                if let Some(cookie) = cookie {
                    request = request.header(header::COOKIE, cookie);
                }
                let response = app.handle(request.body(Body::empty()).unwrap()).await;
                assert_eq!(response.status(), status);
                assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
            }
        }
        let request = |method: &str, path: &str, body: String| {
            Request::builder()
                .method(method)
                .uri(format!("http://localhost{path}"))
                .header(header::COOKIE, &admin_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin")
                .body(Body::from(body))
                .unwrap()
        };
        let response = app.handle(request("GET", PATH, String::new())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        let html = body_text(response).await;
        assert_eq!(html.matches("inputmode=\"decimal\"").count(), 28);
        assert!(!html.contains("<fieldset disabled"));
        let guide_before = super::super::entry::entry_guide(&fitness).await.unwrap();
        assert!(guide_before.muscle_needs.is_empty());
        let response = app
            .handle(request(
                "POST",
                PATH,
                form_body(&[("biceps", "12.5"), ("quads", "8"), ("abs", "0")]),
            ))
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            "/admin/fitness-targets?notice=saved"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        let guide_after = super::super::entry::entry_guide(&fitness).await.unwrap();
        assert_eq!(guide_after.version, guide_before.version + 1);
        assert_eq!(guide_after.muscle_needs.len(), 3);
        assert_eq!(guide_after.muscle_needs["biceps"], 10_000);
        assert_eq!(guide_after.muscle_needs["quads"], 6400);
        assert_eq!(guide_after.muscle_needs["abs"], 0);

        let response = app
            .handle(
                Request::builder()
                    .uri("http://localhost/test-muscle-loads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let public = body_text(response).await;
        assert!(public.contains("now / usual / target"));
        assert!(public.contains("weekly target 12.5 points"));
        assert!(public.contains("weekly target 0 points"));
        assert!(public.contains("below its weekly target"));
        assert!(!public.contains("Edit targets"));
        let admin_panel = body_text(
            app.handle(request("GET", "/test-muscle-loads", String::new()))
                .await,
        )
        .await;
        assert!(admin_panel.contains("Edit targets"));
        let invalid = body_text(
            app.handle(request(
                "POST",
                PATH,
                form_body(&[("biceps", "23.5"), ("abs", "bad input")]),
            ))
            .await,
        )
        .await;
        assert!(invalid.contains("value=\"23.5\""));
        assert!(invalid.contains("value=\"bad input\""));
        assert_eq!(
            fitness.snapshot().await.unwrap().version,
            guide_after.version
        );
        assert_eq!(
            app.handle(request("POST", PATH, "x".repeat(BODY_LIMIT + 1)))
                .await
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let mut untrusted = request("POST", PATH, form_body(&[]));
        untrusted.headers_mut().remove("sec-fetch-site");
        assert_eq!(app.handle(untrusted).await.status(), StatusCode::FORBIDDEN);
        let mut wrong_type = request("POST", PATH, "{}".into());
        wrong_type.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert_eq!(
            app.handle(wrong_type).await.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            app.handle(request("POST", PATH, form_body(&[])))
                .await
                .status(),
            StatusCode::SEE_OTHER
        );
        assert!(
            fitness
                .snapshot()
                .await
                .unwrap()
                .muscle_targets()
                .is_empty()
        );
        assert!(
            super::super::entry::entry_guide(&fitness)
                .await
                .unwrap()
                .muscle_needs
                .is_empty()
        );
    }

    #[tokio::test]
    async fn targets_outage_disables_empty_editor_and_retains_failed_submission() {
        let data = Data::new(Err("test unavailable"));
        let app = router(data.clone(), FitnessStore::new(data));
        let cookie = test_cookie(crate::content::access::ADMIN_EMAIL);
        let request = |method: &str, body: String| {
            Request::builder()
                .method(method)
                .uri(format!("http://localhost{PATH}"))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin")
                .body(Body::from(body))
                .unwrap()
        };
        let html = body_text(app.handle(request("GET", String::new())).await).await;
        assert!(html.contains("<fieldset disabled"));
        assert!(html.contains("Targets are unavailable"));
        let response = app
            .handle(request("POST", form_body(&[("abs", "15.5")])))
            .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        let html = body_text(response).await;
        assert!(html.contains("value=\"15.5\""));
        assert!(html.contains("Targets could not be saved"));
    }
}
