//! Stored running documents.
//!
//! Running is a sibling fitness model, not a lifting workout/set shape. The
//! committed database definition lives in `src/schema.surql`.

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// One route-free running summary imported from Garmin Connect or entered by
/// hand.
///
/// The public embed page also carries GPS traces, account details, heart-rate
/// samples, and device identifiers. None of those cross this storage seam:
/// the running log keeps only the small summary needed to render distance,
/// time, pace, and ascent, plus an optional canonical source link.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
pub struct RunningActivity {
    pub id: String,
    pub source: String,
    pub source_activity_id: String,
    pub source_url: Option<String>,
    pub title: String,
    pub activity_type: String,
    pub started_at_utc: String,
    pub started_at_local: String,
    pub eastern_offset_minutes: i64,
    pub duration_milliseconds: i64,
    pub moving_duration_milliseconds: Option<i64>,
    pub distance_millimeters: i64,
    pub ascent_millimeters: Option<i64>,
    pub imported_at: i64,
}
