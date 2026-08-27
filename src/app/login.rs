//! Google sign-in for comments, hidden pages, and admin controls.
//!
//! Authorization-code flow with PKCE and a `state` check, both stashed in a
//! short-lived encrypted cookie while the browser round-trips through Google.
//! The callback exchanges the code server-side and trusts the resulting
//! `id_token`'s claims without checking its signature — it arrived directly
//! from Google's token endpoint over TLS, which OIDC permits — but still
//! validates issuer, audience, expiry, and `email_verified`. Identity then
//! lives in an encrypted `__Host-viewer` cookie. Any verified Google account
//! may hold that identity so it can author comments; what hidden pages it may
//! see is still decided per request by `content::access`, so revoking a hidden
//! page is an allowlist edit, not a session hunt.
//!
//! Every route here that touches the cookie jar returns a hand-built
//! `Ok(Response)`: the cookie layer only flushes `Set-Cookie` on `Ok`, so the
//! `Err(redirect(…))` idiom would silently drop the jar delta.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use topcoat::{
    Result,
    context::Cx,
    cookie::{Cookie, Cookies, SameSite, private_cookies, time},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, header, page, query_params,
        request::{headers, uri},
        response::Response,
        route,
    },
    view::view,
};

use crate::components::shell;
use crate::util::urlencode;
use benjisponge::auth::secrets_match;

const VIEWER_COOKIE: &str = "viewer";
/// What the browser calls the viewer cookie: `jar()`'s `override_prefix_host`
/// prepends `__Host-`. The response layer greps raw `Cookie` headers for
/// this name to decide cacheability before any decryption happens.
pub(crate) const VIEWER_COOKIE_BROWSER_NAME: &str = "__Host-viewer";
const FLIGHT_COOKIE: &str = "google-flight";
pub(crate) const POPUP_ERROR_PARAM: &str = "auth_error";
/// Maximum local return target carried through the OAuth round trip. Features
/// that append a fragment before login derive their own raw-path ceiling from
/// this value so a valid target is not later collapsed to `/`.
pub(crate) const MAX_AUTH_RETURN_BYTES: usize = 512;
const VIEWER_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;
const FLIGHT_TTL_SECONDS: i64 = 10 * 60;
const NO_STORE: &str = "no-store";

/// The signed-in identity, decrypted from the viewer cookie. Holding one
/// proves who the visitor is, never what they may see — pages check
/// `content::access` on every request. Google's stable `sub` is the ownership
/// identity for authored records; `name` is optional display metadata and must
/// never be used for authorization.
pub struct Viewer {
    pub sub: String,
    pub email: String,
    pub name: Option<String>,
}

/// The current request's verified viewer, if any. Any parse or expiry
/// failure reads as signed-out.
pub fn viewer(cx: &Cx) -> Option<Viewer> {
    let cookie = jar(cx).get(VIEWER_COOKIE)?;
    parse_viewer(cookie.value(), Timestamp::now().as_second())
}

/// The cookie adapter stack shared by every read and write, so names,
/// encryption, and attributes always line up: encrypted, `__Host-` prefixed
/// (forces `Secure` + `Path=/`), `HttpOnly`, `SameSite=Lax` (the Google
/// callback is a top-level cross-site navigation, which Lax permits).
fn jar(cx: &Cx) -> impl Cookies + '_ {
    private_cookies(cx)
        .override_prefix_host()
        .default_http_only(true)
        .default_same_site(SameSite::Lax)
}

#[derive(Serialize, Deserialize)]
struct ViewerClaims {
    sub: String,
    email: String,
    /// Added after viewer cookies first shipped; old cookies remain readable.
    #[serde(default)]
    name: Option<String>,
    exp: i64,
}

fn parse_viewer(value: &str, now: i64) -> Option<Viewer> {
    let claims: ViewerClaims = serde_json::from_str(value).ok()?;
    (claims.exp > now).then_some(Viewer {
        sub: claims.sub,
        email: claims.email,
        name: claims.name,
    })
}

/// State parked in a cookie while the browser visits Google.
#[derive(Serialize, Deserialize)]
struct Flight {
    state: String,
    verifier: String,
    next: String,
    /// Only the progressively enhanced shell dialog requests an in-place
    /// error return. Old in-flight cookies deserialize as the fallback flow.
    #[serde(default)]
    popup: bool,
    exp: i64,
}

fn parse_flight(value: &str, now: i64) -> Option<Flight> {
    let flight: Flight = serde_json::from_str(value).ok()?;
    (flight.exp > now).then_some(flight)
}

#[query_params(error = redirect("?"))]
struct LoginQuery {
    next: Option<String>,
    error: Option<String>,
}

#[page("/login")]
async fn login(cx: &Cx) -> Result {
    let query = query_params::<LoginQuery>(cx)?;
    let next = sanitize_next(query.next.as_deref());
    let google_href = format!("/auth/google?next={}", urlencode(&next));
    let notice = query.error.as_deref().map(login_notice);
    let current = viewer(cx);
    let configured = login_configured();
    let logout_action = format!("/logout?next={}", urlencode(&next));
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: "Sign in",
            active: "",
            runtime: false,
            <section class="mt-16 sm:mt-24">
                <header class="rail-row">
                    <p class="rail-stamp">"sign in"</p>
                    <div class="min-w-0">
                        <h1 class="font-display text-4xl font-bold tracking-tight">
                            "Sign in."
                        </h1>
                        if let Some(message) = notice {
                            <p class="mt-4 max-w-prose text-ink2">(message)</p>
                        }
                        if let Some(current) = current.as_ref() {
                            <p class="mt-4 max-w-prose text-ink2">
                                "Signed in as "
                                <span class="font-meta">(current.email.as_str())</span>
                                "."
                            </p>
                            <form method="post" action=(logout_action.as_str()) class="mt-6">
                                <button type="submit" class="oxlink cursor-pointer font-meta text-sm">
                                    "sign out"
                                </button>
                            </form>
                        } else if configured {
                            <p class="mt-4 max-w-prose text-ink2">
                                "Sign in with Google to join the conversation on thoughts. "
                                "A few private pages are also shared with particular people; "
                                "signing in does not grant access to those."
                            </p>
                            <p class="mt-6">
                                <a class="oxlink font-meta" href=(google_href.as_str())>
                                    "continue with Google →"
                                </a>
                            </p>
                        } else {
                            <p class="mt-4 max-w-prose text-ink2">
                                "Sign-in isn't configured in this environment."
                            </p>
                        }
                    </div>
                </header>
            </section>
        )
    }
}

#[route(GET "/auth/google")]
async fn google_start(cx: &Cx) -> Result<Response> {
    let Some((client_id, _)) = google_env() else {
        return Ok(plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "sign-in is not configured",
        ));
    };
    let next = sanitize_next(query_value(cx, "next").as_deref());
    let state = random_token();
    let verifier = random_token();
    let flight = serde_json::to_string(&Flight {
        state: state.clone(),
        verifier: verifier.clone(),
        next,
        popup: query_value(cx, "popup").as_deref() == Some("1"),
        exp: Timestamp::now().as_second() + FLIGHT_TTL_SECONDS,
    })
    .expect("flight serializes");
    jar(cx).add(
        Cookie::build((FLIGHT_COOKIE, flight))
            .max_age(time::Duration::seconds(FLIGHT_TTL_SECONDS))
            .build(),
    );
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
         ?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile\
         &state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        urlencode(&client_id),
        urlencode(&redirect_uri()),
        urlencode(&state),
        pkce_challenge(&verifier),
    );
    Ok(see_other(&auth_url))
}

#[route(GET "/auth/google/callback")]
async fn google_callback(cx: &Cx) -> Result<Response> {
    let Some((client_id, client_secret)) = google_env() else {
        return Ok(plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "sign-in is not configured",
        ));
    };
    let now = Timestamp::now().as_second();
    let flight = jar(cx)
        .get(FLIGHT_COOKIE)
        .and_then(|cookie| parse_flight(cookie.value(), now));
    let Some(flight) = flight else {
        return Ok(see_other("/login?error=expired"));
    };
    // Consume the flight cookie only after this callback proves ownership via
    // `state`. A second tab's sign-in overwrites the cookie, so the first
    // tab's stale callback — or any forged cross-site GET here — must not
    // destroy the flow that now owns it. Unconsumed flights expire by TTL.
    let state_ok =
        query_value(cx, "state").is_some_and(|state| secrets_match(&state, &flight.state));
    if !state_ok {
        return Ok(callback_error(&flight, "expired"));
    }
    jar(cx).remove((FLIGHT_COOKIE, ""));
    if query_value(cx, "error").is_some() {
        return Ok(callback_error(&flight, "denied"));
    }
    let Some(code) = query_value(cx, "code") else {
        return Ok(callback_error(&flight, "failed"));
    };
    let id_token = match exchange_code(&code, &client_id, &client_secret, &flight.verifier).await {
        Ok(token) => token,
        Err(error) => {
            log_failure("token exchange", &error.to_string());
            return Ok(callback_error(&flight, "failed"));
        }
    };
    let claims = match validate_id_token(&id_token, &client_id, now) {
        Ok(claims) => claims,
        Err(reason) => {
            log_failure("id_token validation", reason);
            return Ok(callback_error(&flight, "failed"));
        }
    };
    let email = claims.email.to_ascii_lowercase();
    let viewer_value = serde_json::to_string(&ViewerClaims {
        sub: claims.sub,
        email,
        name: claims.name,
        exp: now + VIEWER_TTL_SECONDS,
    })
    .expect("viewer serializes");
    jar(cx).add(
        Cookie::build((VIEWER_COOKIE, viewer_value))
            .max_age(time::Duration::seconds(VIEWER_TTL_SECONDS))
            .build(),
    );
    Ok(see_other(&sanitize_next(Some(&flight.next))))
}

#[route(POST "/logout")]
async fn logout(cx: &Cx) -> Result<Response> {
    if cross_site(headers(cx)) {
        return Ok(plain(StatusCode::FORBIDDEN, "forbidden"));
    }
    let next = sanitize_next(query_value(cx, "next").as_deref());
    jar(cx).remove((VIEWER_COOKIE, ""));
    Ok(see_other(&next))
}

/// A logout POST forged from another site is only a nuisance, but the header
/// check is one line: reject when the browser says the request is cross-site.
fn cross_site(headers: &HeaderMap) -> bool {
    matches!(
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
        Some(site) if site != "same-origin" && site != "none"
    )
}

/// Only ever redirect back to a local path: anything absolute, scheme-ful,
/// protocol-relative, or containing oddball bytes collapses to `/`.
fn sanitize_next(raw: Option<&str>) -> String {
    let fallback = || "/".to_string();
    let Some(raw) = raw else { return fallback() };
    let ok = raw.starts_with('/')
        && !raw.starts_with("//")
        && !raw.contains('\\')
        && raw.len() <= MAX_AUTH_RETURN_BYTES
        && raw.bytes().all(|b| (0x21..0x7f).contains(&b));
    if ok { raw.to_string() } else { fallback() }
}

fn login_notice(code: &str) -> &'static str {
    match code {
        "denied" => "Google sign-in was cancelled.",
        "expired" => "That sign-in attempt expired — try again.",
        _ => "Sign-in failed — try again.",
    }
}

/// A callback from the enhanced dialog returns to the original document and
/// asks that document to reopen the dialog with this generic notice.
pub(crate) fn popup_notice(cx: &Cx) -> Option<&'static str> {
    query_value(cx, POPUP_ERROR_PARAM)
        .as_deref()
        .map(login_notice)
}

/// The local destination represented by this document. On the fallback login
/// page, the intended destination is its sanitized `next`; elsewhere it is
/// the live path/query with our one-shot popup marker removed.
pub(crate) fn auth_return_target(cx: &Cx) -> String {
    if uri(cx).path() == "/login" {
        return sanitize_next(query_value(cx, "next").as_deref());
    }
    let target = uri(cx).path_and_query().map_or("/", |value| value.as_str());
    sanitize_next(Some(&without_popup_error(target)))
}

fn without_popup_error(target: &str) -> String {
    let (path_query, fragment) = target
        .split_once('#')
        .map_or((target, None), |(before, after)| (before, Some(after)));
    let (path, query) = path_query
        .split_once('?')
        .map_or((path_query, None), |(path, query)| (path, Some(query)));
    let kept = query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter(|pair| pair.split('=').next() != Some(POPUP_ERROR_PARAM))
        .collect::<Vec<_>>()
        .join("&");
    let mut clean = path.to_string();
    if !kept.is_empty() {
        clean.push('?');
        clean.push_str(&kept);
    }
    if let Some(fragment) = fragment {
        clean.push('#');
        clean.push_str(fragment);
    }
    clean
}

fn popup_error_location(next: &str, code: &str) -> String {
    let next = without_popup_error(&sanitize_next(Some(next)));
    let (base, fragment) = next
        .split_once('#')
        .map_or((next.as_str(), None), |(before, after)| {
            (before, Some(after))
        });
    let separator = if !base.contains('?') {
        "?"
    } else if base.ends_with('?') || base.ends_with('&') {
        ""
    } else {
        "&"
    };
    let fragment = fragment.map_or_else(String::new, |value| format!("#{value}"));
    format!(
        "{base}{separator}{POPUP_ERROR_PARAM}={}{}",
        urlencode(code),
        fragment
    )
}

fn callback_error(flight: &Flight, code: &str) -> Response {
    if flight.popup {
        see_other(&popup_error_location(&flight.next, code))
    } else {
        login_error(&flight.next, code)
    }
}

/// Keep OAuth failures attached to the flight's original destination. A
/// retry from the fallback login page therefore still lands where the user
/// began instead of silently collapsing to the homepage.
fn login_error(next: &str, code: &str) -> Response {
    let next = sanitize_next(Some(next));
    let location = format!("/login?next={}&error={}", urlencode(&next), urlencode(code));
    see_other(&location)
}

/// The shell uses this only to decide whether its account dialog should show
/// the Google action. Secrets never leave this module or enter the page.
pub(crate) fn login_configured() -> bool {
    google_env().is_some()
}

fn google_env() -> Option<(String, String)> {
    let id = std::env::var("GOOGLE_OAUTH_CLIENT_ID")
        .ok()
        .filter(|v| !v.is_empty())?;
    let secret = std::env::var("GOOGLE_OAUTH_CLIENT_SECRET")
        .ok()
        .filter(|v| !v.is_empty())?;
    Some((id, secret))
}

/// Mirrors `feed::origin`: prod sets `SITE_ORIGIN`, dev gets it from
/// `scripts/dev.sh`, and the fallback keeps release builds sane.
fn origin() -> String {
    std::env::var("SITE_ORIGIN").unwrap_or_else(|_| "https://ben.soy".to_string())
}

fn redirect_uri() -> String {
    format!("{}/auth/google/callback", origin())
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Two v4 UUIDs, hex-concatenated: 244 bits of entropy and 64 chars, which
/// also satisfies the PKCE verifier's 43–128 char, unreserved-charset rules.
fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn query_value(cx: &Cx, key: &str) -> Option<String> {
    let query = uri(cx).query()?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

async fn exchange_code(
    code: &str,
    client_id: &str,
    client_secret: &str,
    verifier: &str,
) -> anyhow::Result<String> {
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("code", code)
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("redirect_uri", &redirect_uri())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code_verifier", verifier)
        .finish();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .post("https://oauth2.googleapis.com/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("token endpoint returned {}", response.status());
    }
    #[derive(Deserialize)]
    struct TokenResponse {
        id_token: String,
    }
    Ok(response.json::<TokenResponse>().await?.id_token)
}

#[derive(Deserialize)]
struct GoogleClaims {
    iss: String,
    aud: String,
    exp: i64,
    sub: String,
    email: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

fn validate_id_token(
    id_token: &str,
    client_id: &str,
    now: i64,
) -> std::result::Result<GoogleClaims, &'static str> {
    let mut segments = id_token.split('.');
    let (Some(_), Some(payload), Some(_), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err("malformed id_token");
    };
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "undecodable payload")?;
    let claims: GoogleClaims =
        serde_json::from_slice(&payload).map_err(|_| "unparseable claims")?;
    if claims.iss != "https://accounts.google.com" && claims.iss != "accounts.google.com" {
        return Err("wrong issuer");
    }
    if !secrets_match(&claims.aud, client_id) {
        return Err("wrong audience");
    }
    if claims.exp <= now {
        return Err("expired id_token");
    }
    if !claims.email_verified {
        return Err("unverified email");
    }
    if claims.email.is_empty() || claims.sub.is_empty() {
        return Err("missing identity claims");
    }
    Ok(claims)
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from("see other"))
        .expect("sanitized location is a valid header")
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .body(Body::from(message))
        .expect("static headers")
}

fn log_failure(step: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "google sign-in failed",
            "step": step,
            "error": error,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_cookie_name_matches_the_jar_prefix() {
        assert_eq!(
            VIEWER_COOKIE_BROWSER_NAME,
            format!("__Host-{VIEWER_COOKIE}")
        );
    }

    #[test]
    fn sanitize_next_keeps_local_paths_only() {
        assert_eq!(sanitize_next(Some("/motorcycles")), "/motorcycles");
        assert_eq!(sanitize_next(Some("/a/b?c=d")), "/a/b?c=d");
        assert_eq!(sanitize_next(Some("/a/b?c=d#notes")), "/a/b?c=d#notes");
        assert_eq!(sanitize_next(None), "/");
        assert_eq!(sanitize_next(Some("")), "/");
        assert_eq!(sanitize_next(Some("https://evil.example")), "/");
        assert_eq!(sanitize_next(Some("//evil.example")), "/");
        assert_eq!(sanitize_next(Some("/\\evil.example")), "/");
        assert_eq!(sanitize_next(Some("/a\r\nSet-Cookie: x")), "/");
        let boundary = format!("/{}", "a".repeat(MAX_AUTH_RETURN_BYTES - 1));
        assert_eq!(sanitize_next(Some(&boundary)), boundary);
        assert_eq!(
            sanitize_next(Some(&format!("/{}", "a".repeat(MAX_AUTH_RETURN_BYTES)))),
            "/"
        );
    }

    #[test]
    fn fallback_oauth_error_keeps_the_original_destination() {
        let response = login_error("/thoughts/a-post?view=wide#notes", "denied");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login?next=%2Fthoughts%2Fa-post%3Fview%3Dwide%23notes&error=denied"
        );
    }

    #[test]
    fn popup_oauth_error_returns_to_the_document_and_keeps_its_fragment() {
        assert_eq!(
            popup_error_location("/thoughts/a-post?view=wide#notes", "denied"),
            "/thoughts/a-post?view=wide&auth_error=denied#notes"
        );
        assert_eq!(
            without_popup_error("/thoughts/a-post?view=wide&auth_error=denied&mode=full#notes"),
            "/thoughts/a-post?view=wide&mode=full#notes"
        );
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_token_is_a_valid_pkce_verifier() {
        let token = random_token();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn viewer_claims_round_trip_and_expire() {
        let value = serde_json::to_string(&ViewerClaims {
            sub: "google-sub-1".into(),
            email: "friend@example.com".into(),
            name: Some("Friendly Person".into()),
            exp: 1_000,
        })
        .unwrap();
        let viewer = parse_viewer(&value, 999).expect("not yet expired");
        assert_eq!(viewer.sub, "google-sub-1");
        assert_eq!(viewer.email, "friend@example.com");
        assert_eq!(viewer.name.as_deref(), Some("Friendly Person"));
        assert!(parse_viewer(&value, 1_000).is_none());
        assert!(parse_viewer("not json", 0).is_none());

        let old_value = r#"{"sub":"google-sub-2","email":"old@example.com","exp":1000}"#;
        let old_viewer = parse_viewer(old_value, 999).expect("old cookies remain readable");
        assert_eq!(old_viewer.sub, "google-sub-2");
        assert_eq!(old_viewer.name, None);
    }

    #[test]
    fn flight_round_trips_and_expires() {
        let value = serde_json::to_string(&Flight {
            state: "s".into(),
            verifier: "v".into(),
            next: "/motorcycles".into(),
            popup: true,
            exp: 500,
        })
        .unwrap();
        let flight = parse_flight(&value, 499).unwrap();
        assert_eq!(flight.next, "/motorcycles");
        assert!(flight.popup);
        assert!(parse_flight(&value, 500).is_none());

        let old_cookie = r#"{"state":"s","verifier":"v","next":"/","exp":500}"#;
        assert!(!parse_flight(old_cookie, 499).unwrap().popup);
    }

    fn token_with(claims: &serde_json::Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("header.{payload}.signature")
    }

    fn good_claims() -> serde_json::Value {
        serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": "client-123",
            "exp": 2_000,
            "sub": "google-sub-1",
            "email": "Friend@Example.com",
            "name": "Friendly Person",
            "email_verified": true,
        })
    }

    #[test]
    fn id_token_validation_accepts_the_good_case() {
        let claims = validate_id_token(&token_with(&good_claims()), "client-123", 1_000).unwrap();
        assert_eq!(claims.sub, "google-sub-1");
        assert_eq!(claims.email, "Friend@Example.com");
        assert_eq!(claims.name.as_deref(), Some("Friendly Person"));

        let mut without_profile = good_claims();
        without_profile.as_object_mut().unwrap().remove("name");
        let claims = validate_id_token(&token_with(&without_profile), "client-123", 1_000).unwrap();
        assert_eq!(claims.name, None);
    }

    #[test]
    fn id_token_validation_rejects_each_bad_claim() {
        let cases: [(&str, serde_json::Value); 5] = [
            ("wrong issuer", {
                let mut c = good_claims();
                c["iss"] = "https://evil.example".into();
                c
            }),
            ("wrong audience", {
                let mut c = good_claims();
                c["aud"] = "other-client".into();
                c
            }),
            ("expired id_token", {
                let mut c = good_claims();
                c["exp"] = 999.into();
                c
            }),
            ("unverified email", {
                let mut c = good_claims();
                c["email_verified"] = false.into();
                c
            }),
            ("unverified email", {
                let mut c = good_claims();
                c.as_object_mut().unwrap().remove("email_verified");
                c
            }),
        ];
        for (expected, claims) in cases {
            assert_eq!(
                validate_id_token(&token_with(&claims), "client-123", 1_000).err(),
                Some(expected),
                "claims: {claims}"
            );
        }
        assert!(validate_id_token("nonsense", "client-123", 1_000).is_err());
        assert!(validate_id_token("a.b.c.d", "client-123", 1_000).is_err());
    }

    #[test]
    fn cross_site_flags_only_cross_site_requests() {
        let mut headers = HeaderMap::new();
        assert!(!cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(!cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("none"));
        assert!(!cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));
        assert!(cross_site(&headers));
    }
}
