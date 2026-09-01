//! Stored fitness documents.
//!
//! The committed database definitions live in `src/schema.surql`. These plain
//! data types stay separate from the public API types and retain string IDs so
//! archive/snapshot code does not depend on SurrealDB record-ID formatting.

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// A lifting workout. `started_at_utc` is the Strong-export source instant
/// and the identity anchor; `started_at_local`/`eastern_offset_minutes` are
/// its America/New_York projection, derived server-side at import.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct Workout {
    pub id: String,
    pub title: String,
    pub raw_title: String,
    pub started_at_utc: String,
    pub started_at_local: String,
    pub eastern_offset_minutes: i64,
    pub duration_seconds: i64,
    pub duration_suspicious: bool,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub source: String,
    pub imported_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct Exercise {
    pub name: String,
}

/// An alternate imported/display name that resolves to one canonical
/// exercise. Alias rows are direct (never chained) and survive fitness-data
/// resets so an old export keeps landing on the renamed exercise.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
pub struct ExerciseAlias {
    pub alias_name: String,
    pub canonical_name: String,
}

/// One taxonomy tag on an exercise; an exercise carries several per facet.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct ExerciseTag {
    pub exercise_name: String,
    pub kind: String,
    pub value: String,
}

/// One weighted exercise↔muscle connection: `ratio_hundredths` (1..=100)
/// scales a set's volume points into that muscle's credit. Absence of a row
/// means no credit.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct ExerciseMuscle {
    pub exercise_name: String,
    pub muscle: String,
    pub ratio_hundredths: i64,
}

/// One performed set. There is deliberately no stored records table: badges
/// are derived from the full set history (`archive/records.rs`), so this
/// stays the only source of truth a future manual-logging write path needs.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct LiftSet {
    pub id: String,
    pub workout_id: String,
    pub exercise_name: String,
    pub raw_exercise_name: String,
    pub ordinal: i64,
    pub exercise_note: Option<String>,
    pub superset_id: Option<i64>,
    pub weight_milli: Option<i64>,
    pub weight_unit: String,
    pub reps: Option<i64>,
    pub effort_hundredths: Option<i64>,
    pub distance_milli: Option<i64>,
    pub set_time_seconds: Option<i64>,
    pub set_type: String,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct FitnessMeta {
    pub k: String,
    pub v: i64,
}

/// Inclusive Eastern date range explaining a gap in training. Annotate-only:
/// never feeds volume points, records, or training-focus pace. `to_date` is
/// `None` for an open (still ongoing) interruption.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
pub struct Interruption {
    pub id: String,
    pub from_date: String,
    pub to_date: Option<String>,
    pub note: String,
    /// Heatmap marker; one of the curated choices in `interruptions::EMOJI_CHOICES`.
    pub emoji: String,
    pub updated_at: i64,
}
