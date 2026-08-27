//! `/api/spire/*` — the run database's JSON API, ported from the old
//! Worker's `spire.ts` (since deleted). The GET endpoints
//! are public; POST is the sync CLI's bearer-authed write path. Bodies,
//! error messages, and headers are contract — the golden fixtures under
//! `tests/fixtures/api` capture the Worker originals verbatim. Spire
//! responses carry no CORS header (only the fitness API ever did).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        StatusCode,
        request::{Bytes, headers},
        route,
    },
};

use benjisponge::auth::bearer_authorized;
use benjisponge::data::{Data, spire_models::SpireRun};

use super::db as spire;
use super::runs as spire_runs;

pub const SPIRE_SYNC_TOKEN_VAR: &str = "SPIRE_SYNC_TOKEN";

type ApiResponse = (StatusCode, [(&'static str, &'static str); 2], String);

const JSON_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "application/json; charset=utf-8"),
    ("Cache-Control", "no-store"),
];

fn json(status: StatusCode, body: String) -> ApiResponse {
    (status, JSON_HEADERS, body)
}

fn json_error(status: StatusCode, message: &str) -> ApiResponse {
    json(status, serde_json::json!({ "error": message }).to_string())
}

fn internal_error(path: &str, error: impl std::fmt::Display) -> ApiResponse {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "spire api failed",
            "path": path,
            "error": error.to_string(),
        })
    );
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// The wire shape of a run: every stored column except `added_at`, in the
/// Worker's column order; `raw` lives in its own table and is never served.
#[derive(Serialize)]
struct ApiRun {
    id: String,
    game: String,
    date: String,
    start_time: i64,
    character: String,
    win: bool,
    abandoned: bool,
    ascension: i64,
    acts: i64,
    floors: i64,
    killed_by: Option<String>,
    kill_kind: Option<String>,
    run_time: i64,
    seed: String,
    game_mode: String,
    build_id: String,
}

impl From<SpireRun> for ApiRun {
    fn from(row: SpireRun) -> Self {
        ApiRun {
            id: row.id,
            game: row.game,
            date: row.date,
            start_time: row.start_time,
            character: row.character,
            win: row.win,
            abandoned: row.abandoned,
            ascension: row.ascension,
            acts: row.acts,
            floors: row.floors,
            killed_by: row.killed_by,
            kill_kind: row.kill_kind,
            run_time: row.run_time,
            seed: row.seed,
            game_mode: row.game_mode,
            build_id: row.build_id,
        }
    }
}

#[derive(Serialize)]
struct RunsEnvelope {
    version: i64,
    count: usize,
    runs: Vec<ApiRun>,
}

#[derive(Serialize)]
struct IdsEnvelope {
    ids: Vec<String>,
}

#[derive(Serialize)]
struct ImportEnvelope {
    received: usize,
    added: usize,
    skipped: usize,
    version: i64,
}

fn to_body<T: Serialize>(payload: &T) -> String {
    serde_json::to_string(payload).expect("api payloads are plain data")
}

#[route(GET "/api/spire/runs")]
async fn list_runs(cx: &Cx) -> Result<ApiResponse> {
    let data = app_context::<Data>(cx);
    let response = async {
        let db = data.db().await?;
        let rows = spire::list_runs(&db).await?;
        let version = spire::current_version(&db).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(RunsEnvelope {
            version,
            count: rows.len(),
            runs: rows.into_iter().map(ApiRun::from).collect(),
        })
    }
    .await;
    Ok(match response {
        Ok(envelope) => json(StatusCode::OK, to_body(&envelope)),
        Err(error) => internal_error("/api/spire/runs", error),
    })
}

#[route(GET "/api/spire/ids")]
async fn list_ids(cx: &Cx) -> Result<ApiResponse> {
    let data = app_context::<Data>(cx);
    let response = async {
        let db = data.db().await?;
        let ids = spire::list_ids(&db).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(IdsEnvelope { ids })
    }
    .await;
    Ok(match response {
        Ok(envelope) => json(StatusCode::OK, to_body(&envelope)),
        Err(error) => internal_error("/api/spire/ids", error),
    })
}

#[route(POST "/api/spire/runs")]
async fn import_runs(cx: &Cx, body: Bytes) -> Result<ApiResponse> {
    let authorization = headers(cx)
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = std::env::var(SPIRE_SYNC_TOKEN_VAR).ok();
    if !bearer_authorized(authorization, expected.as_deref()) {
        return Ok(json_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }

    let incoming = match spire::parse_payload(&body) {
        Ok(runs) => runs,
        Err(message) => return Ok(json_error(StatusCode::BAD_REQUEST, &message)),
    };

    let data = app_context::<Data>(cx);
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let outcome = async {
        let db = data.db().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
            spire::insert_runs(&db, &incoming, added_at).await?,
        )
    }
    .await;

    Ok(match outcome {
        Ok(outcome) => {
            if outcome.added > 0 {
                spire_runs::invalidate();
            }
            json(
                StatusCode::OK,
                to_body(&ImportEnvelope {
                    received: outcome.received,
                    added: outcome.added,
                    skipped: outcome.skipped,
                    version: outcome.version,
                }),
            )
        }
        Err(error) => internal_error("/api/spire/runs", error),
    })
}

/// Unmatched `/api/spire/*` paths stay JSON 404s, like the Worker's router
/// fallthrough — not the site's HTML not-found page.
#[route(GET "/api/spire/{*rest}")]
async fn unknown_get() -> Result<ApiResponse> {
    Ok(json_error(StatusCode::NOT_FOUND, "not found"))
}

#[route(POST "/api/spire/{*rest}")]
async fn unknown_post() -> Result<ApiResponse> {
    Ok(json_error(StatusCode::NOT_FOUND, "not found"))
}
