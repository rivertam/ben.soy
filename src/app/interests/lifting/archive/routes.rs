//! `/api/fitness/*` — the public fitness archive and private bounded
//! import paths. The lifting routes were ported from the old Worker's
//! `fitness.ts` (since deleted); the Health Connect step resource is native to
//! this service. Bodies, error
//! messages, status codes, and headers are contract (golden fixtures in
//! `tests/fixtures/api`). Public GET reads carry `Access-Control-Allow-
//! Origin: *`; import responses never did and still don't.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        StatusCode, path_param,
        request::{Bytes, headers, uri},
        route,
    },
};

use benjisponge::auth::bearer_authorized;
use benjisponge::data::Data;

use crate::app::login::viewer;
use crate::content::access::is_admin;
use crate::util::is_same_origin;

use super::db;
use super::eastern;
use super::filters::parse_filters;
use super::import::{BODY_LIMIT_BYTES, parse_import_payload};
use super::steps;
use super::store::FitnessStore;

pub const FITNESS_SYNC_TOKEN_VAR: &str = "FITNESS_SYNC_TOKEN";
pub const STEPS_SYNC_TOKEN_VAR: &str = "STEPS_SYNC_TOKEN";

type PublicResponse = (StatusCode, [(&'static str, &'static str); 3], String);
type PrivateResponse = (StatusCode, [(&'static str, &'static str); 2], String);

const PUBLIC_HEADERS: [(&str, &str); 3] = [
    ("Content-Type", "application/json; charset=utf-8"),
    ("Cache-Control", "no-store"),
    ("Access-Control-Allow-Origin", "*"),
];

const PRIVATE_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "application/json; charset=utf-8"),
    ("Cache-Control", "no-store"),
];

fn public(status: StatusCode, body: String) -> PublicResponse {
    (status, PUBLIC_HEADERS, body)
}

fn public_error(status: StatusCode, message: &str) -> PublicResponse {
    public(status, serde_json::json!({ "error": message }).to_string())
}

fn private(status: StatusCode, body: String) -> PrivateResponse {
    (status, PRIVATE_HEADERS, body)
}

fn private_error(status: StatusCode, message: &str) -> PrivateResponse {
    private(status, serde_json::json!({ "error": message }).to_string())
}

fn log_failure(path: &str, error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "fitness api failed",
            "path": path,
            "error": error.to_string(),
        })
    );
}

fn is_healthmd_client(cx: &Cx) -> bool {
    headers(cx)
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "Health.md Android API Export")
}

fn log_steps_rejection(
    stage: &str,
    status: StatusCode,
    body_bytes: usize,
    healthmd_client: bool,
    detail: &str,
) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "fitness steps import rejected",
            "path": "/api/fitness/steps",
            "stage": stage,
            "status": status.as_u16(),
            "body_bytes": body_bytes,
            "healthmd_client": healthmd_client,
            "detail": detail,
        })
    );
}

fn log_steps_success(body_bytes: usize, healthmd_client: bool, receipt: &steps::StepImportReceipt) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "fitness steps import accepted",
            "path": "/api/fitness/steps",
            "status": StatusCode::OK.as_u16(),
            "body_bytes": body_bytes,
            "healthmd_client": healthmd_client,
            "receipt": receipt,
        })
    );
}

fn to_body<T: Serialize>(payload: &T) -> String {
    serde_json::to_string(payload).expect("api payloads are plain data")
}

/// `url.search !== ""` — a bare trailing `?` produces an empty search in
/// the Worker too, so only a non-empty query trips the rejection.
fn has_query(cx: &Cx) -> bool {
    uri(cx).query().is_some_and(|query| !query.is_empty())
}

fn query_pairs(cx: &Cx) -> Vec<(String, String)> {
    form_urlencoded::parse(uri(cx).query().unwrap_or("").as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

#[route(GET "/api/fitness/sets")]
async fn list_sets(cx: &Cx) -> Result<PublicResponse> {
    let filters = match parse_filters(&query_pairs(cx)) {
        Ok(filters) => filters,
        Err(message) => return Ok(public_error(StatusCode::BAD_REQUEST, &message)),
    };
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => public(StatusCode::OK, to_body(&snapshot.sets_page(&filters))),
        Err(error) => {
            log_failure("/api/fitness/sets", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

#[route(GET "/api/fitness/facets")]
async fn list_facets(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "facets does not accept filters",
        ));
    }
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => public(StatusCode::OK, to_body(&snapshot.facets())),
        Err(error) => {
            log_failure("/api/fitness/facets", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

#[route(GET "/api/fitness/calendar")]
async fn list_calendar(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "calendar does not accept filters",
        ));
    }
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => public(StatusCode::OK, to_body(&snapshot.calendar())),
        Err(error) => {
            log_failure("/api/fitness/calendar", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

/// Recent daily totals are a direct small-table read rather than part of the
/// lifting snapshot: step rows change independently and never affect records,
/// filters, volume, or the fitness version.
#[route(GET "/api/fitness/steps")]
async fn list_steps(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "steps does not accept filters",
        ));
    }
    Ok(
        match steps::load(app_context::<Data>(cx), steps::RECENT_DAYS_LIMIT).await {
            Ok(days) => public(StatusCode::OK, to_body(&steps::StepSeries { days })),
            Err(error) => {
                log_failure("/api/fitness/steps", error);
                public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        },
    )
}

#[route(GET "/api/fitness/workouts/latest")]
async fn latest_workout(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "latest workout does not accept filters",
        ));
    }
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => public(StatusCode::OK, to_body(&snapshot.latest())),
        Err(error) => {
            log_failure("/api/fitness/workouts/latest", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

path_param!(public_workout_path);

#[route(GET "/api/fitness/workouts/by-path/{public_workout_path}")]
async fn workout_by_path(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "workout does not accept filters",
        ));
    }
    let segment = path_param::<PublicWorkoutPath>(cx);
    let Some(instant) = eastern::parse_public_path(segment) else {
        return Ok(public_error(StatusCode::NOT_FOUND, "not found"));
    };
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => match snapshot.by_path(&instant) {
            Some(detail) => public(StatusCode::OK, to_body(&detail)),
            None => public_error(StatusCode::NOT_FOUND, "not found"),
        },
        Err(error) => {
            log_failure("/api/fitness/workouts/by-path", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

/// Who may delete a lift. Two callers, deliberately: the sync token for
/// scripts (`just delete-lift`), and the signed-in admin for a same-origin
/// browser fetch. A cookie is ambient authority, so that path additionally
/// demands same-origin evidence; a bearer token is not, so it does not.
///
/// Topcoat's session extension already runs `verify_origin` over every
/// non-safe method, so a cross-site DELETE is refused before this function
/// sees it. That layer deliberately passes requests carrying neither
/// `Origin` nor `Sec-Fetch-Site` ("not a browser, so no ambient cookies to
/// forge with") — which is exactly the shape a cookie replayed by a
/// non-browser client has, so the check below stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeleteAuth {
    Allowed,
    /// No usable credential at all.
    Unauthorized,
    /// A signed-in visitor who is not the admin — answered like a path that
    /// does not exist, matching `/lifting/upload` and `/diary`.
    NotFound,
    /// The admin, but the request did not prove it came from this site.
    Forbidden,
}

fn delete_authorized(cx: &Cx) -> DeleteAuth {
    let authorization = headers(cx)
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = std::env::var(FITNESS_SYNC_TOKEN_VAR).ok();
    authorize_delete(
        bearer_authorized(authorization, expected.as_deref()),
        viewer(cx).as_ref().map(|current| current.email.as_str()),
        is_same_origin(headers(cx)),
    )
}

/// The decision behind [`delete_authorized`], over the request's relevant
/// parts. Split out so the matrix is testable without a live `Cx`.
fn authorize_delete(token_ok: bool, viewer_email: Option<&str>, same_origin: bool) -> DeleteAuth {
    if token_ok {
        return DeleteAuth::Allowed;
    }
    match viewer_email {
        None => DeleteAuth::Unauthorized,
        Some(email) if !is_admin(email) => DeleteAuth::NotFound,
        Some(_) if !same_origin => DeleteAuth::Forbidden,
        Some(_) => DeleteAuth::Allowed,
    }
}

/// `DELETE` on the workout resource the GET above serves.
///
/// Not idempotent-by-404: deleting an absent workout answers 404, the same
/// as reading one. The alternative — always 204 — would make a typo'd path
/// indistinguishable from a real delete, and this is the one operation where
/// "nothing happened" must be visible.
///
/// The response carries no `Access-Control-Allow-Origin`, unlike the GET on
/// the same URL — a wildcard CORS header on a credentialed destructive verb
/// is exactly the mistake to avoid, and its absence also means no
/// cross-origin preflight can succeed here.
#[route(DELETE "/api/fitness/workouts/by-path/{public_workout_path}")]
async fn delete_workout_by_path(cx: &Cx) -> Result<PrivateResponse> {
    match delete_authorized(cx) {
        DeleteAuth::Allowed => {}
        DeleteAuth::Unauthorized => {
            return Ok(private_error(StatusCode::UNAUTHORIZED, "unauthorized"));
        }
        DeleteAuth::NotFound => return Ok(private_error(StatusCode::NOT_FOUND, "not found")),
        DeleteAuth::Forbidden => return Ok(private_error(StatusCode::FORBIDDEN, "forbidden")),
    }
    if has_query(cx) {
        return Ok(private_error(
            StatusCode::BAD_REQUEST,
            "delete does not accept filters",
        ));
    }

    let segment = path_param::<PublicWorkoutPath>(cx);
    let Some(instant) = eastern::parse_public_path(segment) else {
        return Ok(private_error(StatusCode::NOT_FOUND, "not found"));
    };

    let data = app_context::<Data>(cx);
    let outcome = async {
        let handle = data.db().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
            db::delete_workout_by_path(&handle, &instant.local, instant.offset_minutes).await?,
        )
    }
    .await;

    Ok(match outcome {
        Ok(Some(deleted)) => {
            if let Err(error) = app_context::<FitnessStore>(cx).rebuild().await {
                // The delete committed. A stale snapshot only delays the
                // workout disappearing from the pages until the next read's
                // version check; reporting failure would invite a retry that
                // now 404s.
                log_failure("/api/fitness/workouts/by-path", error);
            }
            private(
                StatusCode::OK,
                to_body(&super::api::DeleteReceipt {
                    path: segment.to_string(),
                    workout_id: deleted.workout_id,
                    source: deleted.source,
                    sets_deleted: deleted.sets_deleted,
                    version: deleted.version,
                }),
            )
        }
        Ok(None) => private_error(StatusCode::NOT_FOUND, "not found"),
        Err(error) => {
            log_failure("/api/fitness/workouts/by-path", error);
            private_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

#[route(GET "/api/fitness/ids")]
async fn list_ids(cx: &Cx) -> Result<PublicResponse> {
    if has_query(cx) {
        return Ok(public_error(
            StatusCode::BAD_REQUEST,
            "ids does not accept filters",
        ));
    }
    Ok(match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => public(StatusCode::OK, to_body(&snapshot.ids())),
        Err(error) => {
            log_failure("/api/fitness/ids", error);
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

/// The Worker's `readJson` Content-Length handling: `Number(header)` must
/// be a non-negative integer, and anything over the limit is a 413 before
/// the body is even considered.
fn content_length_error(cx: &Cx, limit: usize) -> Option<PrivateResponse> {
    let declared = headers(cx)
        .get("content-length")
        .and_then(|value| value.to_str().ok())?;
    let parsed: Option<f64> = {
        let trimmed = declared.trim();
        if trimmed.is_empty() {
            Some(0.0)
        } else {
            trimmed.parse().ok()
        }
    };
    let Some(length) = parsed.filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0) else {
        return Some(private_error(StatusCode::BAD_REQUEST, "bad Content-Length"));
    };
    if length > limit as f64 {
        return Some(private_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("body exceeds {limit} bytes"),
        ));
    }
    None
}

#[route(POST "/api/fitness/import")]
async fn import_chunk(cx: &Cx, body: Bytes) -> Result<PrivateResponse> {
    let authorization = headers(cx)
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = std::env::var(FITNESS_SYNC_TOKEN_VAR).ok();
    if !bearer_authorized(authorization, expected.as_deref()) {
        return Ok(private_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }

    let media_type = headers(cx)
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    if media_type.as_deref() != Some("application/json") {
        return Ok(private_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }

    if let Some(response) = content_length_error(cx, BODY_LIMIT_BYTES) {
        return Ok(response);
    }
    if body.len() > BODY_LIMIT_BYTES {
        return Ok(private_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("body exceeds {BODY_LIMIT_BYTES} bytes"),
        ));
    }
    let Ok(decoded) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Ok(private_error(StatusCode::BAD_REQUEST, "body must be JSON"));
    };
    let payload = match parse_import_payload(&decoded) {
        Ok(payload) => payload,
        Err(message) => return Ok(private_error(StatusCode::BAD_REQUEST, &message)),
    };

    let imported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let data = app_context::<Data>(cx);
    let outcome = async {
        let handle = data.db().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
            db::apply_import(&handle, &payload, imported_at).await?,
        )
    }
    .await;

    Ok(match outcome {
        Ok(outcome) => {
            if outcome.mutated
                && let Err(error) = app_context::<FitnessStore>(cx).rebuild().await
            {
                // The commit landed; a rebuild failure only delays
                // freshness until the next read's version check.
                log_failure("/api/fitness/import", error);
            }
            private(
                StatusCode::OK,
                to_body(&super::api::ImportReceipt {
                    received: outcome.received,
                    added: outcome.added,
                    skipped: outcome.skipped,
                    version: outcome.version,
                }),
            )
        }
        Err(error) => {
            log_failure("/api/fitness/import", error);
            private_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

/// Health.md compatibility exports. This token is deliberately narrower than
/// `FITNESS_SYNC_TOKEN`: a secret stored on the phone can update only one
/// bounded daily step total, never lifts, runs, taxonomy, or deletions.
#[route(POST "/api/fitness/steps")]
async fn import_steps(cx: &Cx, body: Bytes) -> Result<PrivateResponse> {
    let body_bytes = body.len();
    let healthmd_client = is_healthmd_client(cx);
    let authorization = headers(cx)
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = std::env::var(STEPS_SYNC_TOKEN_VAR).ok();
    if !bearer_authorized(authorization, expected.as_deref()) {
        let detail = if expected.is_some() {
            "bearer token missing or mismatched"
        } else {
            "STEPS_SYNC_TOKEN is not configured"
        };
        log_steps_rejection(
            "authorization",
            StatusCode::UNAUTHORIZED,
            body_bytes,
            healthmd_client,
            detail,
        );
        return Ok(private_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }

    let media_type = headers(cx)
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    if media_type.as_deref() != Some("application/json") {
        log_steps_rejection(
            "content-type",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            body_bytes,
            healthmd_client,
            media_type
                .as_deref()
                .unwrap_or("missing or invalid Content-Type"),
        );
        return Ok(private_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }
    if let Some(response) = content_length_error(cx, steps::BODY_LIMIT_BYTES) {
        let detail = if response.0 == StatusCode::PAYLOAD_TOO_LARGE {
            "declared Content-Length exceeds the steps body limit"
        } else {
            "invalid Content-Length"
        };
        log_steps_rejection(
            "content-length",
            response.0,
            body_bytes,
            healthmd_client,
            detail,
        );
        return Ok(response);
    }
    if body_bytes > steps::BODY_LIMIT_BYTES {
        log_steps_rejection(
            "body-size",
            StatusCode::PAYLOAD_TOO_LARGE,
            body_bytes,
            healthmd_client,
            "received body exceeds the steps body limit",
        );
        return Ok(private_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("body exceeds {} bytes", steps::BODY_LIMIT_BYTES),
        ));
    }
    let Ok(decoded) = serde_json::from_slice::<serde_json::Value>(&body) else {
        log_steps_rejection(
            "json",
            StatusCode::BAD_REQUEST,
            body_bytes,
            healthmd_client,
            "body must be JSON",
        );
        return Ok(private_error(StatusCode::BAD_REQUEST, "body must be JSON"));
    };
    let payload = match steps::parse_healthmd_payload(&decoded) {
        Ok(payload) => payload,
        Err(message) => {
            log_steps_rejection(
                "payload",
                StatusCode::BAD_REQUEST,
                body_bytes,
                healthmd_client,
                &message,
            );
            return Ok(private_error(StatusCode::BAD_REQUEST, &message));
        }
    };

    let imported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let outcome = async {
        let handle = app_context::<Data>(cx).db().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
            steps::apply_import(&handle, &payload, imported_at).await?,
        )
    }
    .await;

    Ok(match outcome {
        Ok(outcome) => {
            log_steps_success(body_bytes, healthmd_client, &outcome.receipt);
            private(StatusCode::OK, to_body(&outcome.receipt))
        }
        Err(error) => {
            log_steps_rejection(
                "database",
                StatusCode::INTERNAL_SERVER_ERROR,
                body_bytes,
                healthmd_client,
                "internal error",
            );
            log_failure("/api/fitness/steps", error);
            private_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

/// Unmatched `/api/fitness/*` paths mirror the Worker's fallthrough:
/// JSON 404s, with CORS only on GET (`isPublicRead`).
#[route(GET "/api/fitness/{*rest}")]
async fn unknown_get() -> Result<PublicResponse> {
    Ok(public_error(StatusCode::NOT_FOUND, "not found"))
}

#[route(POST "/api/fitness/{*rest}")]
async fn unknown_post() -> Result<PrivateResponse> {
    Ok(private_error(StatusCode::NOT_FOUND, "not found"))
}

/// Unauthenticated on purpose, like its GET and POST siblings: it answers
/// only for paths that have no delete handler, so there is nothing behind it
/// to protect and a credential check would just confirm which paths exist.
#[route(DELETE "/api/fitness/{*rest}")]
async fn unknown_delete() -> Result<PrivateResponse> {
    Ok(private_error(StatusCode::NOT_FOUND, "not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::access::ADMIN_EMAIL;

    const STRANGER: &str = "someone.else@example.com";

    #[test]
    fn the_sync_token_deletes_without_a_browser() {
        // Scripts (`just delete-lift`) send a bearer token and no origin
        // evidence at all. A token is not ambient authority, so that is
        // enough on its own.
        assert_eq!(authorize_delete(true, None, false), DeleteAuth::Allowed);
        // A token still wins when a cookie is also present, whoever it names.
        assert_eq!(
            authorize_delete(true, Some(STRANGER), false),
            DeleteAuth::Allowed
        );
    }

    #[test]
    fn the_admin_deletes_only_from_this_site() {
        assert_eq!(
            authorize_delete(false, Some(ADMIN_EMAIL), true),
            DeleteAuth::Allowed
        );
        // A cookie IS ambient authority: without positive same-origin
        // evidence the request could have been made by someone else's page.
        assert_eq!(
            authorize_delete(false, Some(ADMIN_EMAIL), false),
            DeleteAuth::Forbidden
        );
        assert_eq!(
            authorize_delete(false, Some(&ADMIN_EMAIL.to_uppercase()), true),
            DeleteAuth::Allowed
        );
    }

    #[test]
    fn everyone_else_is_refused() {
        assert_eq!(
            authorize_delete(false, None, true),
            DeleteAuth::Unauthorized
        );
        assert_eq!(
            authorize_delete(false, None, false),
            DeleteAuth::Unauthorized
        );
        // A signed-in stranger learns nothing about the endpoint — hidden
        // pages and `/lifting/upload` answer the same way.
        assert_eq!(
            authorize_delete(false, Some(STRANGER), true),
            DeleteAuth::NotFound
        );
        // Identity is checked before origin, so a stranger never gets the
        // 403 that would confirm the admin's cookie is the missing piece.
        assert_eq!(
            authorize_delete(false, Some(STRANGER), false),
            DeleteAuth::NotFound
        );
    }

    /// A grant on a hidden page is not a grant to rewrite the archive: only
    /// `ADMIN_EMAIL` passes, exactly as `/lifting/upload` requires.
    #[test]
    fn hidden_page_grantees_cannot_delete() {
        for email in ["guest@example.com", "", "ben.m.berman@gmail.com.evil.test"] {
            assert_eq!(
                authorize_delete(false, Some(email), true),
                DeleteAuth::NotFound,
                "{email} must not be able to delete a lift"
            );
        }
    }

    /// Key order is the response contract, like every other payload in
    /// `api.rs`, and `sets_deleted` must survive as a number.
    #[test]
    fn the_receipt_serializes_the_documented_shape() {
        let body = to_body(&super::super::api::DeleteReceipt {
            path: "2026-07-27T13-42-00-04-00".to_string(),
            workout_id: "fitness:2026-07-27T17:42:00".to_string(),
            source: "manual".to_string(),
            sets_deleted: 18,
            version: 141,
        });
        assert_eq!(
            body,
            r#"{"path":"2026-07-27T13-42-00-04-00","workout_id":"fitness:2026-07-27T17:42:00","source":"manual","sets_deleted":18,"version":141}"#
        );
    }
}
