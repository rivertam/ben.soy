//! Garmin Connect's public-share adapter.
//!
//! This is deliberately a narrow convenience seam, not Garmin account
//! automation. A shared URL yields only a numeric activity id; the server
//! reconstructs Garmin's public embed URL, fetches it without credentials,
//! and retains only a route-free running summary plus the canonical activity
//! link. The response also contains GPS traces and account/device metadata,
//! which are ignored and never logged.

use std::time::Duration;

use benjisponge::data::running_models::RunningActivity;
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::app::interests::lifting::archive::eastern;

const SOURCE: &str = "garmin-connect";
const EMBED_ORIGIN: &str = "https://connect.garmin.com";
const MAX_BODY_BYTES: usize = 1_000_000;
const MAX_SHARED_FIELD_BYTES: usize = 4_096;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GarminError {
    MissingLink,
    AmbiguousLink,
    BadLink,
    NotPublic,
    WrongActivity,
    InvalidSummary,
    Unavailable,
}

impl GarminError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::MissingLink => "No Garmin Connect activity link was included in the share.",
            Self::AmbiguousLink => "The share included more than one Garmin activity.",
            Self::BadLink => "That is not a supported Garmin Connect activity link.",
            Self::NotPublic => {
                "Garmin did not expose that activity. Set its privacy to Everyone, then share it again."
            }
            Self::WrongActivity => "That Garmin activity is not a run.",
            Self::InvalidSummary => {
                "Garmin's shared activity did not contain a complete running summary."
            }
            Self::Unavailable => {
                "Garmin Connect is unavailable right now. Try sharing the run again."
            }
        }
    }
}

impl std::fmt::Display for GarminError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for GarminError {}

/// Locate one Garmin activity across Android's inconsistently populated
/// `title`, `text`, and `url` share fields. Duplicate copies of the same link
/// are fine; distinct activity ids are rejected so preview can never select
/// one implicitly.
pub fn shared_activity_id(fields: &[&str]) -> Result<String, GarminError> {
    let mut found: Option<String> = None;
    let mut saw_garmin_url = false;
    for field in fields {
        if field.len() > MAX_SHARED_FIELD_BYTES {
            return Err(GarminError::BadLink);
        }
        for token in field.split_whitespace() {
            let token = token.trim_matches(|character: char| {
                matches!(
                    character,
                    '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                )
            });
            if !token.starts_with("https://") {
                continue;
            }
            let Ok(url) = Url::parse(token) else {
                continue;
            };
            if url.host_str() != Some("connect.garmin.com") {
                continue;
            }
            saw_garmin_url = true;
            // `url::Url::port()` normalizes an explicitly written `:443`
            // away. Check the raw authority as well so accepted links have
            // exactly the one intended origin spelling and no userinfo/port.
            let authority = token
                .strip_prefix("https://")
                .and_then(|rest| rest.split(['/', '?', '#']).next());
            if authority != Some("connect.garmin.com") {
                continue;
            }
            let Some(activity_id) = activity_id_from_url(&url) else {
                continue;
            };
            match &found {
                Some(existing) if existing != activity_id => {
                    return Err(GarminError::AmbiguousLink);
                }
                Some(_) => {}
                None => found = Some(activity_id.to_string()),
            }
        }
    }
    found.ok_or(if saw_garmin_url {
        GarminError::BadLink
    } else {
        GarminError::MissingLink
    })
}

fn activity_id_from_url(url: &Url) -> Option<&str> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.collect();
    let activity_id = match segments.as_slice() {
        ["activity", activity_id]
        | ["app", "activity", activity_id]
        | ["modern", "activity", activity_id]
        | ["app", "activity", activity_id, "share", "0" | "1"]
        | ["modern", "activity", activity_id, "share", "0" | "1"] => *activity_id,
        _ => return None,
    };
    ((1..=20).contains(&activity_id.len()) && activity_id.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(activity_id)
}

pub fn storage_id(activity_id: &str) -> String {
    sha256_hex(format!("{SOURCE}\n{activity_id}"))
}

/// Bind the confirmation click to the exact normalized summary the owner
/// reviewed. `imported_at` is deliberately excluded: the commit re-fetch gets
/// a later ingestion timestamp even when every Garmin field is unchanged.
pub fn summary_digest(activity: &RunningActivity) -> String {
    let summary = (
        &activity.id,
        &activity.source,
        &activity.source_activity_id,
        &activity.source_url,
        &activity.title,
        &activity.activity_type,
        &activity.started_at_utc,
        &activity.started_at_local,
        activity.eastern_offset_minutes,
        activity.duration_milliseconds,
        activity.moving_duration_milliseconds,
        activity.distance_millimeters,
        activity.ascent_millimeters,
    );
    let encoded = serde_json::to_vec(&summary).expect("running summary fields serialize");
    sha256_hex(encoded)
}

fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    Sha256::digest(value.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub async fn fetch(activity_id: &str, imported_at: i64) -> Result<RunningActivity, GarminError> {
    if !(1..=20).contains(&activity_id.len())
        || !activity_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(GarminError::BadLink);
    }
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .user_agent("ben.soy running-share importer")
        .build()
        .map_err(|_| GarminError::Unavailable)?;
    let mut response = client
        .get(format!("{EMBED_ORIGIN}/embed/activity/{activity_id}"))
        .header("Accept", "text/html")
        .header("Accept-Language", "en-US,en;q=0.8")
        .send()
        .await
        .map_err(|_| GarminError::Unavailable)?;
    if response.status().is_redirection() || response.status().as_u16() == 404 {
        return Err(GarminError::NotPublic);
    }
    if !response.status().is_success() {
        return Err(GarminError::Unavailable);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BODY_BYTES as u64)
    {
        return Err(GarminError::InvalidSummary);
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"))
    {
        return Err(GarminError::InvalidSummary);
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| GarminError::Unavailable)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(GarminError::InvalidSummary);
        }
        bytes.extend_from_slice(&chunk);
    }
    let html = std::str::from_utf8(&bytes).map_err(|_| GarminError::InvalidSummary)?;
    parse_embed(activity_id, html, imported_at)
}

#[derive(Debug, Deserialize)]
struct ActivityData {
    #[serde(rename = "activityId")]
    activity_id: u64,
    #[serde(rename = "activityName")]
    activity_name: String,
    #[serde(rename = "activityTypeDTO")]
    activity_type_dto: ActivityType,
    #[serde(rename = "accessControlRuleDTO")]
    access_control_rule_dto: AccessControl,
    #[serde(rename = "summaryDTO")]
    summary_dto: Summary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityType {
    type_id: i64,
    type_key: String,
    parent_type_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessControl {
    type_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    #[serde(rename = "startTimeGMT")]
    start_time_gmt: String,
    distance: f64,
    duration: f64,
    moving_duration: Option<f64>,
    elevation_gain: Option<f64>,
}

fn parse_embed(
    expected_activity_id: &str,
    html: &str,
    imported_at: i64,
) -> Result<RunningActivity, GarminError> {
    let activity = activity_data(html)?;
    if activity.activity_id.to_string() != expected_activity_id {
        return Err(GarminError::InvalidSummary);
    }
    if activity.access_control_rule_dto.type_key != "public" {
        return Err(GarminError::NotPublic);
    }
    if activity.activity_type_dto.type_id != 1
        && activity.activity_type_dto.parent_type_id != Some(1)
    {
        return Err(GarminError::WrongActivity);
    }
    let title = normalize_text(&activity.activity_name, 200)?;
    let activity_type = normalize_activity_type(&activity.activity_type_dto.type_key)?;
    let started_at_utc = normalize_garmin_utc(&activity.summary_dto.start_time_gmt)?;
    let eastern =
        eastern::eastern_instant(&started_at_utc, 0).map_err(|_| GarminError::InvalidSummary)?;
    let duration_milliseconds = scale_metric(activity.summary_dto.duration, 1_000, 1, 604_800_000)?;
    let moving_duration_milliseconds = activity
        .summary_dto
        .moving_duration
        .map(|value| scale_metric(value, 1_000, 1, 604_800_000))
        .transpose()?;
    let distance_millimeters =
        scale_metric(activity.summary_dto.distance, 1_000, 1, 1_000_000_000)?;
    let ascent_millimeters = activity
        .summary_dto
        .elevation_gain
        .map(|value| scale_metric(value, 1_000, 0, 100_000_000))
        .transpose()?;

    Ok(RunningActivity {
        id: storage_id(expected_activity_id),
        source: SOURCE.to_string(),
        source_activity_id: expected_activity_id.to_string(),
        source_url: Some(format!(
            "{EMBED_ORIGIN}/app/activity/{expected_activity_id}"
        )),
        title,
        activity_type,
        started_at_utc,
        started_at_local: eastern.local,
        eastern_offset_minutes: i64::from(eastern.offset_minutes),
        duration_milliseconds,
        moving_duration_milliseconds,
        distance_millimeters,
        ascent_millimeters,
        imported_at,
    })
}

/// Decode each inert Next.js RSC string, concatenate the streamed records,
/// then deserialize only the `activityData` object. Unknown fields—including
/// the polyline and profile/device objects—are skipped by Serde.
fn activity_data(html: &str) -> Result<ActivityData, GarminError> {
    const PUSH: &str = "self.__next_f.push(";
    let mut records = String::new();
    let mut rest = html;
    while let Some(start) = rest.find(PUSH) {
        let after = &rest[start + PUSH.len()..];
        let Some(end) = after.find(")</script>") else {
            break;
        };
        let candidate = &after[..end];
        if let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(candidate)
            && parts.first().and_then(Value::as_i64) == Some(1)
            && let Some(payload) = parts.get(1).and_then(Value::as_str)
        {
            records.push_str(payload);
        }
        rest = &after[end + ")</script>".len()..];
    }
    let marker = "\"activityData\":";
    let start = records
        .find(marker)
        .map(|index| index + marker.len())
        .ok_or(GarminError::InvalidSummary)?;
    let mut deserializer = serde_json::Deserializer::from_str(&records[start..]);
    ActivityData::deserialize(&mut deserializer).map_err(|_| GarminError::InvalidSummary)
}

fn normalize_text(value: &str, max: usize) -> Result<String, GarminError> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty()
        || normalized.len() > max
        || normalized.chars().any(|character| character.is_control())
    {
        return Err(GarminError::InvalidSummary);
    }
    Ok(normalized)
}

fn normalize_activity_type(value: &str) -> Result<String, GarminError> {
    if value.is_empty()
        || value.len() > 80
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(GarminError::InvalidSummary);
    }
    Ok(value.to_string())
}

fn normalize_garmin_utc(value: &str) -> Result<String, GarminError> {
    if value.len() < 19 || value.as_bytes().get(10) != Some(&b'T') {
        return Err(GarminError::InvalidSummary);
    }
    let suffix = &value[19..];
    if !suffix.is_empty() {
        let fraction = suffix
            .strip_prefix('.')
            .ok_or(GarminError::InvalidSummary)?;
        if fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(GarminError::InvalidSummary);
        }
    }
    let normalized = value[..19].replacen('T', " ", 1);
    eastern::eastern_instant(&normalized, 0).map_err(|_| GarminError::InvalidSummary)?;
    Ok(normalized)
}

fn scale_metric(value: f64, scale: i64, minimum: i64, maximum: i64) -> Result<i64, GarminError> {
    if !value.is_finite() || value < 0.0 {
        return Err(GarminError::InvalidSummary);
    }
    let scaled = (value * scale as f64).round();
    if scaled < minimum as f64 || scaled > maximum as f64 {
        return Err(GarminError::InvalidSummary);
    }
    Ok(scaled as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embed(activity: Value) -> String {
        let record = format!(
            "6:[\"$\",\"x\",null,{{\"activityData\":{activity},\"hereToken\":\"discarded\"}}]\n"
        );
        let push = serde_json::json!([1, record]);
        format!("<html><script>self.__next_f.push({push})</script></html>")
    }

    fn activity() -> Value {
        serde_json::json!({
            "activityId": 24065766206_u64,
            "activityName": "New York Running",
            "activityTypeDTO": {
                "typeId": 1,
                "typeKey": "running",
                "parentTypeId": 17
            },
            "accessControlRuleDTO": { "typeKey": "public" },
            "summaryDTO": {
                "startTimeGMT": "2026-08-21T21:22:15.0",
                "distance": 6480.75,
                "duration": 2613.929,
                "movingDuration": 2571.0,
                "elevationGain": 6.76,
                "startLatitude": 40.7,
                "startLongitude": -73.9
            },
            "activityDetailMetrics": [{"metrics": [1, 2, 3]}]
        })
    }

    #[test]
    fn finds_one_activity_across_android_share_fields() {
        assert_eq!(
            shared_activity_id(&[
                "Check out my run",
                "#beatyesterday https://connect.garmin.com/modern/activity/24065766206/share/0?lang=en",
                "https://connect.garmin.com/app/activity/24065766206",
            ]),
            Ok("24065766206".to_string())
        );
    }

    #[test]
    fn rejects_ssrf_and_ambiguous_share_shapes() {
        for unsupported in [
            "http://connect.garmin.com/app/activity/1",
            "https://connect.garmin.com:443/app/activity/1",
            "https://someone@connect.garmin.com/app/activity/1",
            "https://connect.garmin.com.evil.test/app/activity/1",
            "https://connect.garmin.com/app/profile/1",
            "https://connect.garmin.com/app/activity/not-digits",
        ] {
            assert!(shared_activity_id(&[unsupported]).is_err(), "{unsupported}");
        }
        assert_eq!(
            shared_activity_id(&[
                "https://connect.garmin.com/app/activity/1",
                "https://connect.garmin.com/app/activity/2",
            ]),
            Err(GarminError::AmbiguousLink)
        );
    }

    #[test]
    fn parses_only_the_route_free_running_summary() {
        let parsed = parse_embed("24065766206", &embed(activity()), 1_787_366_800).unwrap();
        assert_eq!(parsed.source_activity_id, "24065766206");
        assert_eq!(
            parsed.source_url.as_deref(),
            Some("https://connect.garmin.com/app/activity/24065766206")
        );
        assert_eq!(parsed.title, "New York Running");
        assert_eq!(parsed.started_at_utc, "2026-08-21 21:22:15");
        assert_eq!(parsed.started_at_local, "2026-08-21 17:22:15");
        assert_eq!(parsed.eastern_offset_minutes, -240);
        assert_eq!(parsed.distance_millimeters, 6_480_750);
        assert_eq!(parsed.duration_milliseconds, 2_613_929);
        assert_eq!(parsed.moving_duration_milliseconds, Some(2_571_000));
        assert_eq!(parsed.ascent_millimeters, Some(6_760));
    }

    #[test]
    fn rejects_private_non_running_and_malformed_payloads() {
        let mut private = activity();
        private["accessControlRuleDTO"]["typeKey"] = Value::String("private".to_string());
        assert_eq!(
            parse_embed("24065766206", &embed(private), 0),
            Err(GarminError::NotPublic)
        );

        let mut cycling = activity();
        cycling["activityTypeDTO"]["typeId"] = Value::from(2);
        assert_eq!(
            parse_embed("24065766206", &embed(cycling), 0),
            Err(GarminError::WrongActivity)
        );

        assert_eq!(
            parse_embed("24065766206", "<html>changed</html>", 0),
            Err(GarminError::InvalidSummary)
        );

        let mut bad_time = activity();
        bad_time["summaryDTO"]["startTimeGMT"] = Value::String("2026-08-21T21:22:15.".to_string());
        assert_eq!(
            parse_embed("24065766206", &embed(bad_time), 0),
            Err(GarminError::InvalidSummary)
        );
    }

    #[test]
    fn confirmation_digest_tracks_the_reviewed_summary_not_import_time() {
        let mut parsed = parse_embed("24065766206", &embed(activity()), 1).unwrap();
        let reviewed = summary_digest(&parsed);
        assert_eq!(reviewed.len(), 64);
        assert!(reviewed.bytes().all(|byte| byte.is_ascii_hexdigit()));

        parsed.imported_at = 2;
        assert_eq!(summary_digest(&parsed), reviewed);

        parsed.title = "Edited after review".to_string();
        assert_ne!(summary_digest(&parsed), reviewed);

        parsed.title = "New York Running".to_string();
        parsed.source_url = Some("https://connect.garmin.com/app/activity/1".to_string());
        assert_ne!(summary_digest(&parsed), reviewed);
    }
}
