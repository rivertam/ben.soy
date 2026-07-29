//! `GET /api/podrick/seed` — machine export of every production `podrick_*` row.
//!
//! Local `just dev --podrick-reset` pulls this so a cleared database mirrors
//! production Podrick state (announcements, Pants history, actions, cursors)
//! instead of rebuilding from Discord. Bearer auth only; the hidden `/podrick`
//! page stays the human surface (`docs/podrick.md`).

use serde::Serialize;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{StatusCode, headers, route},
};

use benjisponge::auth::bearer_authorized;
use benjisponge::data::Data;

use super::status;

pub const PODRICK_SYNC_TOKEN_VAR: &str = "PODRICK_SYNC_TOKEN";

type ApiResponse = (StatusCode, [(&'static str, &'static str); 2], String);

const JSON_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "application/json; charset=utf-8"),
    ("Cache-Control", "private, no-store"),
];

fn json(status: StatusCode, body: String) -> ApiResponse {
    (status, JSON_HEADERS, body)
}

fn json_error(status: StatusCode, message: &str) -> ApiResponse {
    json(status, serde_json::json!({ "error": message }).to_string())
}

fn to_body<T: Serialize>(payload: &T) -> String {
    serde_json::to_string(payload).expect("api payloads are plain data")
}

fn log_failure(path: &str, error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "podrick api failed",
            "path": path,
            "error": error.to_string(),
        })
    );
}

#[route(GET "/api/podrick/seed")]
async fn podrick_seed(cx: &Cx) -> Result<ApiResponse> {
    export_seed(cx).await
}

/// Temporary alias while local scripts that still name pants-seed catch up.
#[route(GET "/api/podrick/pants-seed")]
async fn pants_seed_alias(cx: &Cx) -> Result<ApiResponse> {
    export_seed(cx).await
}

async fn export_seed(cx: &Cx) -> Result<ApiResponse> {
    let authorization = headers(cx)
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = std::env::var(PODRICK_SYNC_TOKEN_VAR).ok();
    if !bearer_authorized(authorization, expected.as_deref()) {
        return Ok(json_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }

    let data = app_context::<Data>(cx);
    Ok(match status::export_podrick_seed(data).await {
        Ok(seed) => json(StatusCode::OK, to_body(&seed)),
        Err(error) => {
            log_failure("/api/podrick/seed", error);
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use benjisponge::data::podrick_models::{PantsSeedMessage, PodrickSeed};

    #[test]
    fn seed_payload_round_trips() {
        let seed = PodrickSeed {
            announcements: Vec::new(),
            pants_messages: vec![PantsSeedMessage {
                message_id: "1".into(),
                channel_id: "2".into(),
                author_id: "3".into(),
                posted_at: 1_700_000_000,
            }],
            pants_actions: Vec::new(),
            meta: BTreeMap::from([
                ("pants_cursor".into(), "99".into()),
                ("announce_watermark".into(), "2026-01-01 00:00:00".into()),
            ]),
        };
        let encoded = to_body(&seed);
        let decoded: PodrickSeed = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, seed);
    }
}
