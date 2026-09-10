//! Owner-only cloth workbench and the stable public stylesheet.

use base64::{Engine, engine::general_purpose::STANDARD};
use benjisponge::plaid::{
    self, GenerateMode, Pattern, Spec,
    store::{PlaidStore, SaveError},
};
use serde::{Deserialize, de::DeserializeOwned};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, header, page, request::headers,
        response::Response, route, to_bytes,
    },
    view::{Unescaped, view},
};

use super::{login::viewer, not_found::not_found_page};
use crate::{
    components::{page_head, shell},
    content::access::is_admin,
    util::is_same_origin,
};

const EDITOR_JS: Asset = asset!("./plaid/editor.js");
const BODY_LIMIT: usize = 32 * 1024;

#[route(GET "/plaid/current.css")]
async fn current_css(cx: &Cx) -> Result<Response> {
    let loaded = app_context::<PlaidStore>(cx).current(false).await;
    let css = loaded.saved.pattern.stylesheet();
    let etag = format!("\"{}\"", plaid::digest(css.as_bytes()));
    let unchanged = loaded.available
        && headers(cx)
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(',')
                    .any(|v| v.trim().trim_start_matches("W/") == etag)
            });
    Ok(Response::builder()
        .status(if unchanged {
            StatusCode::NOT_MODIFIED
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, etag)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(if unchanged { String::new() } else { css }))
        .expect("generated CSS response"))
}

#[page("/admin/plaid")]
async fn editor(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return view! {
            (StatusCode::SEE_OTHER)
            ((header::LOCATION, HeaderValue::from_static("/login?next=%2Fadmin%2Fplaid")))
            ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        };
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            not_found_page(requested: "/admin/plaid")
        };
    }
    let loaded = app_context::<PlaidStore>(cx).current(true).await;
    let saved = &loaded.saved;
    let css = saved.pattern.preview_stylesheet();
    let png = sample_png(&saved.pattern);
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            page: "Plaid", active: "", runtime: false,
            page_head(stamp: "admin", title: "Cut from the same cloth", lede: "A plaid for the page, and every Thursday lift.")
            <section class="plaid-editor" data-plaid-editor="" data-revision=(saved.revision) data-store-available=(loaded.available.to_string()) data-generator-version=(plaid::GENERATOR_VERSION)
                data-warp=(serde_json::to_string(saved.pattern.warp()).unwrap()) data-weft=(serde_json::to_string(saved.pattern.weft()).unwrap())>
                <style data-plaid-style="">(Unescaped::new_unchecked(css))</style>
                <div class="plaid-workbench">
                    <div class="plaid-preview-column">
                        <div class="plaid-cloth" data-plaid-cloth="" role="img" aria-label="Unshaded plaid cloth"></div>
                        <div class="plaid-preview-switch" role="group" aria-label="Preview">
                            <button type="button" data-plaid-tab="page" aria-pressed="true">"On the page"</button>
                            <button type="button" data-plaid-tab="card" aria-pressed="false">"Workout card"</button>
                        </div>
                        <div class="plaid-page-preview" data-plaid-preview="" data-plaid-panel="page">
                            <p class="plaid-sample-meta">"Thursday, a little differently"</p>
                            <h2>"A good day to pick things up."</h2>
                            <p>"Same familiar site. A fresh set of stripes. The cloth keeps its color; the ink finds its contrast."</p>
                            <p class="plaid-sample-link">"Notes from the lifting archive"</p>
                            <div class="plaid-sample-card">"A quiet surface for the details."</div>
                        </div>
                        <div class="plaid-card-preview" data-plaid-panel="card" hidden="">
                            <img data-plaid-png="" src=(png) width="1200" height="600" alt="Sample Thursday workout card using this draft plaid">
                            <p>"Sample workout. The same renderer makes the archive’s social images."</p>
                        </div>
                        <p class="plaid-caption" data-plaid-finish="">"The page and card adjust their ink and backing for readable contrast."</p>
                    </div>
                    <div class="plaid-controls">
                        <fieldset>
                            <legend>"Try another cloth"</legend>
                            <label>"Seed"<input data-plaid-seed="" value="thursday" maxlength="128" spellcheck="false"></label>
                            <div class="plaid-actions">
                                <button type="button" data-plaid-generate="all">"Random plaid"</button>
                                <button type="button" data-plaid-generate="colors">"Random colors"</button>
                                <button type="button" data-plaid-generate="stripes">"Random stripes"</button>
                                <button type="button" data-plaid-repeat-seed="">"Repeat seed"</button>
                            </div>
                        </fieldset>
                        <fieldset>
                            <legend>"Palette"</legend>
                            <div class="plaid-palette" data-plaid-palette=""></div>
                            <button type="button" data-plaid-add-color="">"Add color"</button>
                        </fieldset>
                        <fieldset>
                            <legend>"Stripe rhythm"</legend>
                            <label class="plaid-check"><input type="checkbox" data-plaid-linked="" checked=(true)>"Use the same bands in both directions"</label>
                            for (axis, title) in [("warp", "Vertical bands"), ("weft", "Horizontal bands")] {
                                <div class="plaid-axis" data-plaid-axis=(axis) hidden=(axis == "weft")>
                                    <div class="plaid-axis-heading">
                                        <h3>(title)</h3>
                                        <select data-plaid-repeat=(axis) aria-label=(format!("{title} repeat"))>
                                            <option value="mirrored">"Mirrored"</option>
                                            <option value="repeating">"Repeating"</option>
                                        </select>
                                    </div>
                                    <div class="plaid-bands" data-plaid-bands=(axis)></div>
                                    <button type="button" data-plaid-add-band=(axis)>"Add stripe"</button>
                                </div>
                            }
                        </fieldset>
                        <fieldset class="plaid-dimensions">
                            <legend>"Cut and scale"</legend>
                            <label>"Repeat size (px)"<input type="number" min="24" max="320" step="1" data-plaid-size="" value=(saved.pattern.spec().repeat_px)></label>
                            <label>"Rotation (degrees)"<input type="number" min="-45" max="45" step="1" data-plaid-angle="" value=(saved.pattern.spec().rotation_deg)></label>
                        </fieldset>
                    </div>
                </div>
                <details class="plaid-document">
                    <summary>"Pattern text"</summary>
                    <p>"Copy this definition to keep a cloth. Letters name palette colors; numbers count threads. / marks mirrored end bands; ... wraps a repeating sequence."</p>
                    <label for="plaid-document">"Plaid definition (JSON)"</label>
                    <textarea id="plaid-document" data-plaid-document="" rows="16" spellcheck="false">(saved.pattern.text())</textarea>
                    <button type="button" data-plaid-copy="">"Copy definition"</button>
                </details>
                <div class="plaid-publish">
                    <div class="plaid-actions">
                        <button type="button" class="plaid-use" data-plaid-save="" disabled=(true)>"Use this plaid"</button>
                        <button type="button" data-plaid-reset="">"Reset to current"</button>
                    </div>
                    <p data-plaid-status="" role="status" aria-live="polite">
                        (if loaded.available { "You’re viewing the current plaid. Changes stay in this editor until you save." } else { "The plaid store is unavailable. You can experiment, but saving needs the store to reconnect." })
                    </p>
                </div>
                <noscript><p>"The visual builder needs JavaScript. The current plaid still appears above."</p></noscript>
                <template data-plaid-color-template="">
                    <div class="plaid-color-row">
                        <label><span data-plaid-color-name=""></span><input type="color" data-plaid-color-value=""></label>
                        <button type="button" data-plaid-remove-color="" aria-label="Remove color">"×"</button>
                    </div>
                </template>
                <template data-plaid-band-template="">
                    <div class="plaid-band-row">
                        <select data-plaid-band-color="" aria-label="Stripe color"></select>
                        <input type="number" min="1" max="512" step="1" data-plaid-band-width="" aria-label="Thread count">
                        <button type="button" data-plaid-move="-1" aria-label="Move stripe up">"↑"</button>
                        <button type="button" data-plaid-move="1" aria-label="Move stripe down">"↓"</button>
                        <button type="button" data-plaid-remove-band="" aria-label="Remove stripe">"×"</button>
                    </div>
                </template>
            </section>
            <script type="module" src=(EDITOR_JS)></script>
        )
    }
}

fn sample_png(pattern: &Pattern) -> String {
    format!(
        "data:image/png;base64,{}",
        STANDARD.encode(super::interests::lifting::social_card::preview_png(pattern))
    )
}

fn preview_value(pattern: &Pattern) -> serde_json::Value {
    serde_json::json!({
        "spec": pattern.spec(), "text": pattern.text(),
        "warp": pattern.warp(), "weft": pattern.weft(),
        "css": pattern.preview_stylesheet(), "png": sample_png(pattern),
        "finish": if pattern.finish().light { "Dark ink on light cloth" } else { "Light ink on dark cloth" },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    spec: Spec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    spec: Spec,
    seed: String,
    generator_version: u8,
    mode: GenerateMode,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    spec: Spec,
    expected_revision: i64,
}

#[route(POST "/admin/plaid/preview")]
async fn preview(cx: &Cx, body: Body) -> Result<Response> {
    let draft: Draft = match json_request(cx, body).await {
        Ok(v) => v,
        Err(r) => return Ok(*r),
    };
    Ok(match Pattern::new(draft.spec) {
        Ok(pattern) => json(StatusCode::OK, preview_value(&pattern)),
        Err(error) => failure(StatusCode::UNPROCESSABLE_ENTITY, &error),
    })
}

#[route(POST "/admin/plaid/generate")]
async fn randomize(cx: &Cx, body: Body) -> Result<Response> {
    let request: Generation = match json_request(cx, body).await {
        Ok(v) => v,
        Err(r) => return Ok(*r),
    };
    let generated = Pattern::new(request.spec).and_then(|pattern| {
        plaid::generate(
            &pattern,
            &request.seed,
            request.generator_version,
            request.mode,
        )
    });
    Ok(match generated {
        Ok(pattern) => json(StatusCode::OK, preview_value(&pattern)),
        Err(error) => failure(StatusCode::UNPROCESSABLE_ENTITY, &error),
    })
}

#[route(POST "/admin/plaid")]
async fn publish(cx: &Cx, body: Body) -> Result<Response> {
    let request: Publication = match json_request(cx, body).await {
        Ok(v) => v,
        Err(r) => return Ok(*r),
    };
    let pattern = match Pattern::new(request.spec) {
        Ok(p) => p,
        Err(error) => return Ok(failure(StatusCode::UNPROCESSABLE_ENTITY, &error)),
    };
    Ok(
        match app_context::<PlaidStore>(cx)
            .save(pattern, request.expected_revision)
            .await
        {
            Ok(saved) => {
                let mut value = preview_value(&saved.pattern);
                value["revision"] = saved.revision.into();
                value["updated_at"] = saved.updated_at.into();
                json(StatusCode::OK, value)
            }
            Err(SaveError::Conflict) => failure(
                StatusCode::CONFLICT,
                "The current plaid changed in another editor. Your draft is safe here. Copy its definition, then reset to current before saving again.",
            ),
            Err(SaveError::Unavailable(error)) => {
                tracing::warn!(%error, "plaid save failed");
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The plaid store did not confirm the save. Your draft is still here; try again.",
                )
            }
        },
    )
}

fn request_gate(admin: Option<bool>, request_headers: &HeaderMap) -> Option<Response> {
    match admin {
        None => {
            return Some(failure(
                StatusCode::UNAUTHORIZED,
                "Sign in again before saving or previewing.",
            ));
        }
        Some(false) => return Some(failure(StatusCode::NOT_FOUND, "Not found.")),
        Some(true) => {}
    }
    if !is_same_origin(request_headers) {
        return Some(failure(
            StatusCode::FORBIDDEN,
            "A same-origin request is required.",
        ));
    }
    let mime = request_headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next());
    if !mime.is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json")) {
        return Some(failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Send an application/json document.",
        ));
    }
    None
}

async fn json_request<T: DeserializeOwned>(
    cx: &Cx,
    body: Body,
) -> std::result::Result<T, Box<Response>> {
    if let Some(response) = request_gate(viewer(cx).map(|v| is_admin(&v.email)), headers(cx)) {
        return Err(Box::new(response));
    }
    let bytes = to_bytes(body, BODY_LIMIT).await.map_err(|_| {
        Box::new(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "The plaid request exceeds 32 KiB.",
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        Box::new(failure(
            StatusCode::BAD_REQUEST,
            &format!("Invalid plaid document: {e}"),
        ))
    })
}

fn failure(status: StatusCode, message: &str) -> Response {
    json(status, serde_json::json!({ "error": message }))
}
fn json(status: StatusCode, value: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(value.to_string()))
        .expect("static JSON headers")
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

    fn test_cookie(email: &str) -> String {
        let (parts, ()) = http::Request::builder().body(()).unwrap().into_parts();
        let cx = CxTestBuilder::new()
            .app_context(test_key())
            .request_context(parts)
            .request_context(CookieJarCell::new())
            .build();
        private_cookies(&cx).override_prefix_host().add((
            "viewer",
            serde_json::json!({
                "sub": "plaid-local-test", "email": email,
                "exp": jiff::Timestamp::now().as_second() + 3600,
            })
            .to_string(),
        ));
        let mut headers = HeaderMap::new();
        write_cookies(&cx, &mut headers);
        headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    fn test_key() -> Key {
        Key::derive_from(b"plaid-local-test-only-key-32-bytes")
    }

    fn test_assets() -> topcoat::asset::AssetConfig {
        use topcoat::asset::{AssetConfig, Manifest, ManifestEntry, RawAsset};
        // Resolve every shell asset without requiring a bundled executable in
        // CI. This test inspects HTML; it never fetches these placeholder URLs.
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

    #[tokio::test]
    async fn real_routes_protect_drafts_and_preserve_their_document() {
        let data = benjisponge::data::Data::new(Err("test database unavailable"));
        let router = Router::builder()
            .cookies()
            .discover_fonts()
            .app_context(test_assets())
            .app_context(test_key())
            .app_context(data.clone())
            .app_context(PlaidStore::new(data))
            .page(editor)
            .route(preview)
            .route(randomize)
            .route(publish)
            .route(current_css)
            .build();
        let admin_cookie = test_cookie(crate::content::access::ADMIN_EMAIL);
        // Optional artifact for the actual app on an isolated local database.
        // This known test key never authenticates to a deployed app.
        if let Ok(path) = std::env::var("PLAID_TEST_COOKIE_PATH") {
            std::fs::write(path, &admin_cookie).unwrap();
        }
        for (cookie, status) in [
            (None, StatusCode::SEE_OTHER),
            (
                Some(test_cookie("visitor@example.com")),
                StatusCode::NOT_FOUND,
            ),
        ] {
            let mut request = Request::builder().uri("http://localhost/admin/plaid");
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let response = router.handle(request.body(Body::empty()).unwrap()).await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let document = serde_json::json!({"spec": Spec::default()}).to_string();
        let post = |path: &str, bytes: String| {
            Request::builder()
                .method("POST")
                .uri(format!("http://localhost{path}"))
                .header(header::COOKIE, &admin_cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .header("sec-fetch-site", "same-origin")
                .body(Body::from(bytes))
                .unwrap()
        };
        let response = router
            .handle(
                Request::builder()
                    .uri("http://localhost/admin/plaid")
                    .header(header::COOKIE, &admin_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&html).contains(&format!(
            "data-generator-version=\"{}\"",
            plaid::GENERATOR_VERSION
        )));
        let generation = serde_json::json!({
            "spec": Spec::default(), "seed": "shirt",
            "generator_version": plaid::GENERATOR_VERSION, "mode": "all",
        });
        let response = router
            .handle(post("/admin/plaid/generate", generation.to_string()))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let generated = plaid::generate(
            &Pattern::default(),
            "shirt",
            plaid::GENERATOR_VERSION,
            GenerateMode::All,
        )
        .unwrap();
        assert_eq!(
            value["spec"],
            serde_json::to_value(generated.spec()).unwrap()
        );
        let response = router
            .handle(post("/admin/plaid/preview", document.clone()))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["spec"],
            serde_json::to_value(Spec::default()).unwrap()
        );
        assert!(!value["css"].as_str().unwrap().contains("data-theme"));
        assert!(
            value["png"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(
            router
                .handle(post("/admin/plaid/preview", "x".repeat(BODY_LIMIT + 1)))
                .await
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let mut invalid = serde_json::json!({"spec": Spec::default()});
        invalid["spec"]["palette"]["K"] = "red;display:none".into();
        assert_eq!(
            router
                .handle(post("/admin/plaid/preview", invalid.to_string()))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let save = serde_json::json!({"spec": Spec::default(), "expected_revision": 0});
        assert_eq!(
            router
                .handle(post("/admin/plaid", save.to_string()))
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let response = router.handle(post("/admin/plaid/preview", document)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a failed save must not prevent previewing the retained draft"
        );
    }
    #[test]
    fn all_admin_posts_require_identity_origin_and_json() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            request_gate(None, &headers).unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request_gate(Some(false), &headers).unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request_gate(Some(true), &headers).unwrap().status(),
            StatusCode::FORBIDDEN
        );
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert_eq!(
            request_gate(Some(true), &headers).unwrap().status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert!(request_gate(Some(true), &headers).is_none());
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert_eq!(
            request_gate(Some(true), &headers).unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
