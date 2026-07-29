//! The public JSON wire shapes, shared by the API routes (serialize) and
//! the site's own pages (consume directly, no HTTP round trip).
//!
//! Struct field order IS the response key order — the golden fixtures pin
//! it. Integers use the reader-friendly unsigned types the pages always
//! assumed; every value is range-checked at import so the casts from the
//! stored `i64`s are total.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetPage {
    pub version: i64,
    pub page: usize,
    pub per_page: usize,
    pub total_sets: u64,
    pub total_workouts: u64,
    pub workouts: Vec<Workout>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workout {
    pub id: String,
    /// The canonical public path segment (`YYYY-MM-DDThh-mm-ss±HH-MM`).
    pub path: String,
    pub title: String,
    pub raw_title: String,
    pub started_at_local: String,
    pub ended_at_local: String,
    pub eastern_offset_minutes: i32,
    pub end_eastern_offset_minutes: i32,
    pub duration_seconds: u64,
    pub duration_suspicious: bool,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub sets: Vec<Set>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Set {
    pub id: String,
    pub ordinal: u32,
    pub exercise_name: String,
    pub raw_exercise_name: String,
    pub exercise_note: Option<String>,
    pub superset_id: Option<u64>,
    pub weight_milli: Option<u64>,
    pub weight_unit: String,
    pub reps: Option<u64>,
    pub effort_hundredths: Option<u64>,
    pub distance_milli: Option<u64>,
    pub set_time_seconds: Option<u64>,
    pub set_type: String,
    pub records: Vec<Record>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub level: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Facets {
    pub version: i64,
    pub summary: Summary,
    pub exercises: Vec<Facet>,
    pub tags: TagFacets,
    pub set_types: Vec<Facet>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub sets: u64,
    pub workouts: u64,
    pub min_date: Option<String>,
    pub max_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Facet {
    pub value: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TagFacets {
    pub movement: Vec<Facet>,
    pub muscle: Vec<Facet>,
    pub equipment: Vec<Facet>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calendar {
    pub version: i64,
    pub days: Vec<CalendarDay>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalendarDay {
    pub date: String,
    pub volume_points: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkoutDetail {
    pub version: i64,
    pub workout: Option<Workout>,
    pub newer_workout_path: Option<String>,
    pub older_workout_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetIds {
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportReceipt {
    pub received: usize,
    pub added: usize,
    pub skipped: usize,
    pub version: i64,
}

/// What `DELETE /api/fitness/workouts/by-path/{path}` reports. `source` is
/// the deleted workout's, so a caller who removes a `workout-data-csv`
/// workout can see that the next sync will bring it straight back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteReceipt {
    pub path: String,
    pub workout_id: String,
    pub source: String,
    pub sets_deleted: usize,
    pub version: i64,
}
