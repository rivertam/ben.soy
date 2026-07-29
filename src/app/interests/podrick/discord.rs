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
use url::Url;

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

    /// Discord's requested delay when this is a rate-limit response.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            DiscordError::RateLimited(after) => Some(*after),
            _ => None,
        }
    }
}

/// Discord's response to a created message. It sends far more than this;
/// unknown fields are ignored, like the Spire importer's handling of run JSON.
#[derive(Debug, Deserialize)]
struct PostedMessage {
    id: String,
}

/// The portion of a channel message Podrick needs when polling history.
///
/// Discord returns many more fields; serde deliberately ignores them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ChannelMessage {
    pub id: String,
    pub author: ChannelMessageAuthor,
    pub timestamp: String,
    /// Present on messages created with a nonce. Numeric client nonces are
    /// normalized to strings so Podrick can compare its own stable nonce
    /// without making unrelated source messages fail deserialization.
    #[serde(default, deserialize_with = "deserialize_nonce")]
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ChannelMessageAuthor {
    pub id: String,
}

#[derive(Clone)]
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
        self.post_message_inner(channel_id, content, None).await
    }

    /// Post a plain-text message with Discord's duplicate-nonce protection.
    ///
    /// `nonce` is a caller-owned stable key and must be at most 25 characters,
    /// per Discord's Create Message contract.
    pub async fn post_message_idempotent(
        &self,
        channel_id: &str,
        content: &str,
        nonce: &str,
    ) -> Result<String, DiscordError> {
        self.post_message_inner(channel_id, content, Some(nonce))
            .await
    }

    async fn post_message_inner(
        &self,
        channel_id: &str,
        content: &str,
        nonce: Option<&str>,
    ) -> Result<String, DiscordError> {
        let response = self
            .client
            .post(api_url(&["channels", channel_id, "messages"]))
            .header(reqwest::header::AUTHORIZATION, self.authorization())
            .json(&create_message_body(content, nonce))
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

    /// Fetch up to 100 channel messages, newest first.
    ///
    /// Passing `before` walks backward from that message's snowflake. Discord
    /// already returns this endpoint in newest-to-oldest order.
    pub async fn messages(
        &self,
        channel_id: &str,
        before: Option<&str>,
    ) -> Result<Vec<ChannelMessage>, DiscordError> {
        let response = self
            .client
            .get(channel_messages_url(channel_id, before))
            .header(reqwest::header::AUTHORIZATION, self.authorization())
            .send()
            .await
            .map_err(|error| DiscordError::Transport(error.to_string()))?;
        check(response)
            .await?
            .json()
            .await
            .map_err(|error| DiscordError::Transport(error.to_string()))
    }

    /// Add the bot's own reaction to a message.
    ///
    /// Discord makes this endpoint naturally idempotent: repeating the same
    /// `PUT` leaves one reaction by the bot and returns 204.
    pub async fn add_own_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), DiscordError> {
        let response = self
            .client
            .put(reaction_url(channel_id, message_id, emoji))
            .header(reqwest::header::AUTHORIZATION, self.authorization())
            .send()
            .await
            .map_err(|error| DiscordError::Transport(error.to_string()))?;
        check(response).await?;
        Ok(())
    }
}

fn create_message_body(content: &str, nonce: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "content": content,
        "allowed_mentions": { "parse": [] },
    });
    if let Some(nonce) = nonce {
        body["nonce"] = nonce.into();
        body["enforce_nonce"] = true.into();
    }
    body
}

fn channel_messages_url(channel_id: &str, before: Option<&str>) -> Url {
    let mut url = api_url(&["channels", channel_id, "messages"]);
    let mut query = url.query_pairs_mut();
    query.append_pair("limit", "100");
    if let Some(before) = before {
        query.append_pair("before", before);
    }
    drop(query);
    url
}

fn reaction_url(channel_id: &str, message_id: &str, emoji: &str) -> Url {
    api_url(&[
        "channels",
        channel_id,
        "messages",
        message_id,
        "reactions",
        emoji,
        "@me",
    ])
}

fn api_url(path: &[&str]) -> Url {
    let mut url = Url::parse(API_BASE).expect("valid Discord API base URL");
    url.path_segments_mut()
        .expect("Discord API base URL can hold path segments")
        .extend(path);
    url
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response, DiscordError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let header_retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<f64>().ok())
            .and_then(retry_duration);
        let body = response.text().await.unwrap_or_default();
        let body_retry_after = serde_json::from_str::<RateLimitBody>(&body)
            .ok()
            .and_then(|body| body.retry_after)
            .and_then(retry_duration);
        let retry_after = header_retry_after
            .or(body_retry_after)
            .unwrap_or(Duration::from_secs(1));
        return Err(DiscordError::RateLimited(retry_after));
    }
    let body = response.text().await.unwrap_or_default();
    Err(match status {
        reqwest::StatusCode::UNAUTHORIZED => DiscordError::Unauthorized,
        reqwest::StatusCode::FORBIDDEN => DiscordError::Forbidden(truncate(&body, 300)),
        reqwest::StatusCode::NOT_FOUND => DiscordError::NotFound,
        other => DiscordError::Api(other.as_u16(), truncate(&body, 500)),
    })
}

#[derive(Deserialize)]
struct RateLimitBody {
    retry_after: Option<f64>,
}

fn retry_duration(seconds: f64) -> Option<Duration> {
    (seconds.is_finite() && seconds >= 0.0 && seconds <= Duration::MAX.as_secs_f64())
        .then(|| Duration::from_secs_f64(seconds))
}

fn deserialize_nonce<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        Some(serde_json::Value::Number(value)) => Ok(Some(value.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "Discord message nonce must be a string, number, or null, got {other}"
        ))),
    }
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
        let limited = DiscordError::RateLimited(Duration::from_secs(1_337));
        assert!(limited.is_retryable());
        assert_eq!(limited.retry_after(), Some(Duration::from_secs(1_337)));
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
    fn retry_durations_keep_large_discord_hints_and_reject_nonsense() {
        assert_eq!(
            retry_duration(1_336.57),
            Some(Duration::from_secs_f64(1_336.57))
        );
        assert_eq!(retry_duration(-1.0), None);
        assert_eq!(retry_duration(f64::NAN), None);
        assert_eq!(retry_duration(f64::INFINITY), None);
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

    #[test]
    fn channel_messages_parse_only_the_history_fields_we_need() {
        let raw = r#"[
            {
                "id": "1234567890",
                "content": "6:07",
                "timestamp": "2026-07-28T10:07:00.000000+00:00",
                "author": {
                    "id": "42",
                    "username": "Captain Beyond Beefheart",
                    "avatar": null
                },
                "some_future_field": {"nested": true}
            }
        ]"#;
        let messages: Vec<ChannelMessage> = serde_json::from_str(raw).unwrap();
        assert_eq!(
            messages,
            vec![ChannelMessage {
                id: "1234567890".into(),
                author: ChannelMessageAuthor { id: "42".into() },
                timestamp: "2026-07-28T10:07:00.000000+00:00".into(),
                nonce: None,
            }]
        );
    }

    #[test]
    fn string_and_numeric_message_nonces_normalize_for_recovery() {
        let string: ChannelMessage = serde_json::from_str(
            r#"{
                "id":"1",
                "author":{"id":"2"},
                "timestamp":"2026-07-28T10:07:00Z",
                "nonce":"i123"
            }"#,
        )
        .unwrap();
        let numeric: ChannelMessage = serde_json::from_str(
            r#"{
                "id":"1",
                "author":{"id":"2"},
                "timestamp":"2026-07-28T10:07:00Z",
                "nonce":123
            }"#,
        )
        .unwrap();
        assert_eq!(string.nonce.as_deref(), Some("i123"));
        assert_eq!(numeric.nonce.as_deref(), Some("123"));
    }

    #[test]
    fn history_url_always_requests_one_hundred_and_optionally_pages_before() {
        assert_eq!(
            channel_messages_url("883473115085164544", None).as_str(),
            "https://discord.com/api/v10/channels/883473115085164544/messages?limit=100"
        );
        assert_eq!(
            channel_messages_url("883473115085164544", Some("1234567890")).as_str(),
            "https://discord.com/api/v10/channels/883473115085164544/messages?limit=100&before=1234567890"
        );
    }

    #[test]
    fn reaction_url_percent_encodes_the_emoji_path_segment() {
        assert_eq!(
            reaction_url("883473115085164544", "1234567890", "🐛/worm").as_str(),
            "https://discord.com/api/v10/channels/883473115085164544/messages/1234567890/reactions/%F0%9F%90%9B%2Fworm/@me"
        );
    }

    #[test]
    fn message_bodies_disable_mentions_and_enable_nonce_enforcement_only_when_used() {
        assert_eq!(
            create_message_body("@everyone", None),
            serde_json::json!({
                "content": "@everyone",
                "allowed_mentions": { "parse": [] },
            })
        );
        assert_eq!(
            create_message_body("🐛", Some("kwerm:2026-07-28:am")),
            serde_json::json!({
                "content": "🐛",
                "allowed_mentions": { "parse": [] },
                "nonce": "kwerm:2026-07-28:am",
                "enforce_nonce": true,
            })
        );
    }
}
