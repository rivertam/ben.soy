//! Database models for first-party site analytics.
//!
//! Event rows are deliberately coarse: there is no IP address, raw user agent,
//! arbitrary query string, or external referrer path. Engagement rows only
//! move monotonically toward their final cumulative measurements. Voluntary
//! names live in a separate table that the public dashboard never reads.

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// One browser-reported event, enriched with coarse request metadata.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct AnalyticsEvent {
    /// Client-generated UUID. It makes retries idempotent.
    pub id: String,
    /// SHA-256 digest of the opaque first-party cookie value.
    pub visitor_id: String,
    /// Opaque 30-minute session selected atomically by the database.
    pub session_id: String,
    pub occurred_at: i64,
    pub kind: String,
    pub page_path: String,
    pub referrer_kind: String,
    pub referrer_host: Option<String>,
    /// Stored only for same-site referrers, to support journey aggregates.
    pub referrer_path: Option<String>,
    /// ISO 3166-1 alpha-2 from a trusted platform header, when available.
    pub country_code: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub device_kind: String,
    pub browser: String,
    pub operating_system: String,
    pub viewport_kind: String,
    pub navigation_kind: Option<String>,
    pub local_hour: Option<i64>,
    pub local_weekday: Option<i64>,
    pub engagement_seconds: Option<i64>,
    pub scroll_percent: Option<i64>,
    pub lcp_milliseconds: Option<i64>,
    pub cls_thousandths: Option<i64>,
    pub navigation_milliseconds: Option<i64>,
    pub target_host: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
}

/// A run of otherwise-identical events inside one visitor's UTC day.
///
/// `first` retains the deterministic `(occurred_at, event id)` ordering used
/// for acquisition, while `last_occurred_at` is the exact boundary marker
/// needed to recognize a session crossing midnight. Repeated rows are folded
/// into `count`; the dashboard expands these compact facts only in memory so
/// its established aggregation remains the single semantic implementation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsEventFact {
    pub event: AnalyticsEvent,
    pub last_occurred_at: i64,
    pub count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsSessionFact {
    pub session_id: String,
    pub pageviews: u64,
    pub first_pageview: AnalyticsEvent,
    pub last_pageview_at: i64,
}

/// A count grouped on one public dashboard dimension. `secondary` is used by
/// journeys; local clocks use `weekday:hour` as the key.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsDimensionFact {
    pub dimension: String,
    pub key: String,
    pub secondary: Option<String>,
    pub count: u64,
}

/// Exact per-page sums and explicit denominators for optional engagement
/// measurements. Zero is a sample; only `None` omits one from its denominator.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AnalyticsEngagementFact {
    pub page_path: String,
    pub events: u64,
    pub engagement_seconds_sum: i128,
    pub engagement_seconds_samples: u64,
    pub scroll_percent_sum: i128,
    pub scroll_percent_samples: u64,
    pub finish_count: u64,
    pub lcp_milliseconds_sum: i128,
    pub lcp_milliseconds_samples: u64,
    pub cls_thousandths_sum: i128,
    pub cls_thousandths_samples: u64,
    pub navigation_milliseconds_sum: i128,
    pub navigation_milliseconds_samples: u64,
}

/// Exact, compact read model for one anonymous visitor on one UTC day.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsVisitorDay {
    pub utc_day: i64,
    pub visitor_id: String,
    pub sessions: Vec<AnalyticsSessionFact>,
    pub dimensions: Vec<AnalyticsDimensionFact>,
    pub engagement: Vec<AnalyticsEngagementFact>,
    pub events: Vec<AnalyticsEventFact>,
}

/// Maps a hardened session cookie to the stable anonymous visitor selected
/// during its first event.
///
/// The alias closes a first-load race: pageview and unload beacons can arrive
/// before either response has installed its cookie. Both still converge on
/// the same tab-bootstrap-derived visitor, and whichever cookie wins maps
/// back to it on later requests. Reusing the nonce within one browser tab also
/// closes a rapid-navigation race before the first response installs a cookie.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct AnalyticsVisitorAlias {
    pub token_hash: String,
    pub visitor_id: String,
    pub created_at: i64,
}

/// The current server-owned session cursor for one anonymous visitor.
///
/// Historical session membership remains fixed on event rows. This small state
/// table lets concurrent first events agree on one session and rotate it
/// atomically after thirty minutes without trusting a browser-defined session.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct AnalyticsSession {
    pub visitor_id: String,
    pub session_id: String,
    pub last_seen_at: i64,
}

/// A visitor's voluntary private-ledger entry.
///
/// This table intentionally has no public read path and no relation declared
/// to `AnalyticsEvent`; a dashboard query cannot accidentally eager-load it.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct AnalyticsIdentity {
    pub visitor_id: String,
    pub display_name: String,
    pub note: Option<String>,
    pub first_submitted_at: i64,
    pub updated_at: i64,
}
