//! A small Discord REST client.
//!
//! Deliberately not serenity/twilight and deliberately not the gateway. Both
//! of Podrick's jobs are REST shaped: announcing is one `POST
//! /channels/{id}/messages`, and reading a channel — live *and* when seeding
//! from history — is `GET /channels/{id}/messages` walked by snowflake. A
//! gateway socket would buy sub-second latency in exchange for a large
//! dependency tree and a heartbeat/resume lifecycle to babysit, and it would
//! split the backfill and the live tail into two code paths that could drift.
//! Here they are the same function.
//!
//! Intents are a gateway concept and do not gate these endpoints; the bot
//! needs channel permissions (View Channel, Send Messages, Read Message
//! History), not privileged intents. See `docs/podrick.md`.

use std::time::Duration;

use serde::Deserialize;

/// Pinned so a future default-version change cannot silently alter payloads.
const API_BASE: &str = "https://discord.com/api/v10";
const USER_AGENT: &str = concat!(
    "DiscordBot (https://benjisponge.com/podrick, ",
    env!("CARGO_PKG_VERSION"),
    ")"
);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Discord rejects messages over 2000 characters. Podrick renders well under
/// this; the constant exists so truncation is a decision, not a 400.
pub const MAX_MESSAGE_CHARS: usize = 2000;

#[derive(Debug)]
pub enum DiscordError {
    /// The token was rejected. Retrying will not help.
    Unauthorized,
    /// The bot cannot see or post in that channel. Retrying will not help.
    Forbidden(String),
    /// Channel or message does not exist.
    NotFound,
    /// Rate limited; the value is Discord's own retry hint.
    RateLimited(Duration),
    /// Any other API-level rejection: status plus body.
    Api(u16, String),
    /// Transport failure, timeout, or an unreadable body.
    Transport(String),
}

impl std::fmt::Display for DiscordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscordError::Unauthorized => write!(f, "discord rejected the bot token"),
            DiscordError::Forbidden(detail) => write!(f, "discord forbade the request: {detail}"),
            DiscordError::NotFound => write!(f, "discord channel or message not found"),
            DiscordError::RateLimited(after) => {
                write!(
                    f,
                    "discord rate limited, retry in {:.1}s",
                    after.as_secs_f64()
                )
            }
            DiscordError::Api(status, body) => write!(f, "discord returned {status}: {body}"),
            DiscordError::Transport(error) => write!(f, "discord request failed: {error}"),
        }
    }
}

impl std::error::Error for DiscordError {}

impl DiscordError {
    /// Whether another attempt could plausibly succeed. A bad token or a
    /// missing permission is operator error and must not be retried in a
    /// loop; a 5xx or a timeout is worth another tick.
    pub fn is_retryable(&self) -> bool {
        match self {
            DiscordError::Unauthorized | DiscordError::Forbidden(_) | DiscordError::NotFound => {
                false
            }
            DiscordError::RateLimited(_) | DiscordError::Transport(_) => true,
            DiscordError::Api(status, _) => *status >= 500,
        }
    }
}

/// Discord's response to a created message. It sends far more than this;
/// unknown fields are ignored, like the Spire importer's handling of run JSON.
#[derive(Debug, Deserialize)]
struct PostedMessage {
    id: String,
}

pub struct Discord {
    client: reqwest::Client,
    token: String,
}

impl Discord {
    pub fn new(token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .expect("reqwest client");
        Discord { client, token }
    }

    fn authorization(&self) -> String {
        format!("Bot {}", self.token)
    }

    /// Post a plain-text message and return its id.
    ///
    /// `allowed_mentions` is empty on purpose: Podrick posts workout titles it
    /// did not author, and an `@everyone` that happened to appear in one must
    /// never actually ping a server that is not mine.
    pub async fn post_message(
        &self,
        channel_id: &str,
        content: &str,
    ) -> Result<String, DiscordError> {
        let body = serde_json::json!({
            "content": content,
            "allowed_mentions": { "parse": [] },
        });
        let response = self
            .client
            .post(format!("{API_BASE}/channels/{channel_id}/messages"))
            .header(reqwest::header::AUTHORIZATION, self.authorization())
            .json(&body)
            .send()
            .await
            .map_err(|error| DiscordError::Transport(error.to_string()))?;
        let response = check(response).await?;
        let message: PostedMessage = response
            .json()
            .await
            .map_err(|error| DiscordError::Transport(error.to_string()))?;
        Ok(message.id)
    }
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response, DiscordError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        return Err(DiscordError::RateLimited(Duration::from_secs_f64(
            retry_after.clamp(0.0, 300.0),
        )));
    }
    let body = response.text().await.unwrap_or_default();
    Err(match status {
        reqwest::StatusCode::UNAUTHORIZED => DiscordError::Unauthorized,
        reqwest::StatusCode::FORBIDDEN => DiscordError::Forbidden(truncate(&body, 300)),
        reqwest::StatusCode::NOT_FOUND => DiscordError::NotFound,
        other => DiscordError::Api(other.as_u16(), truncate(&body, 500)),
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    match value.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &value[..index]),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transport_and_server_errors_are_retried() {
        assert!(!DiscordError::Unauthorized.is_retryable());
        assert!(!DiscordError::Forbidden("no".into()).is_retryable());
        assert!(!DiscordError::NotFound.is_retryable());
        assert!(!DiscordError::Api(400, "bad".into()).is_retryable());
        assert!(DiscordError::Api(503, "down".into()).is_retryable());
        assert!(DiscordError::RateLimited(Duration::from_secs(2)).is_retryable());
        assert!(DiscordError::Transport("timeout".into()).is_retryable());
    }

    #[test]
    fn truncate_respects_character_boundaries() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 3), "hel…");
        // A multi-byte tail must not be sliced mid-character.
        assert_eq!(truncate("héllo wörld", 4), "héll…");
    }

    #[test]
    fn a_created_message_parses_past_the_fields_we_ignore() {
        let raw = r#"{
            "id": "1234567890",
            "content": "hi",
            "timestamp": "2026-07-28T12:00:00.000000+00:00",
            "author": {"id": "42", "username": "podrick", "bot": true},
            "some_future_field": {"nested": true}
        }"#;
        let message: PostedMessage = serde_json::from_str(raw).unwrap();
        assert_eq!(message.id, "1234567890");
    }
}
