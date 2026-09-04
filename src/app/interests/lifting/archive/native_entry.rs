//! Strict adapter from the owner-only native workout-entry wire shape to the
//! archive's existing create-only manual-workout payload.
//!
//! Drafts do not belong in the immutable `workouts`/`sets` tables. The entry
//! UI keeps its mutable draft separately and sends one complete submission;
//! this module derives every stored identity and projection rather than
//! accepting them from the browser.

use std::collections::BTreeMap;

use fitness_entry_core::{FinalizedWorkout, MAX_SETS, validate_finalized};

use super::{
    eastern,
    import::{IncomingExercise, IncomingSet, IncomingTag, IncomingWorkout, Payload},
    snapshot::Snapshot,
};

const SOURCE: &str = "manual";

/// The database-ready payload plus the permalink segment implied by its
/// server-derived Eastern projection.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuiltNativeEntry {
    pub(crate) payload: Payload,
    pub(crate) public_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeEntryError(String);

impl NativeEntryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for NativeEntryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NativeEntryError {}

/// Validate one finalized native workout and build the same typed payload the
/// Lyfta path passes to `db::create_manual_workout`.
pub(crate) fn build_native_entry(
    input: FinalizedWorkout,
    snapshot: &Snapshot,
) -> Result<BuiltNativeEntry, NativeEntryError> {
    validate_finalized(&input).map_err(NativeEntryError::new)?;
    let started = eastern::utc_timestamp(&input.started_at_utc).map_err(|_| {
        NativeEntryError::new("started_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time")
    })?;
    let ended = eastern::utc_timestamp(&input.ended_at_utc).map_err(|_| {
        NativeEntryError::new("ended_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time")
    })?;
    let duration_seconds = ended
        .as_second()
        .checked_sub(started.as_second())
        .expect("shared finalized-workout validation checked duration");

    let projection = eastern::eastern_instant(&input.started_at_utc, 0)
        .map_err(|_| NativeEntryError::new("started_at_utc could not be projected"))?;
    let workout_id = format!("fitness:{}", input.started_at_utc.replacen(' ', "T", 1));
    let mut incoming_sets = Vec::new();
    let mut exercises = BTreeMap::<String, IncomingExercise>::new();

    for exercise in input.exercises {
        let canonical_name = snapshot
            .canonical_exercise_name(&exercise.name)
            .ok_or_else(|| {
                NativeEntryError::new(format!(
                    "exercise {:?} is not an existing canonical exercise",
                    exercise.name
                ))
            })?;

        exercises.entry(canonical_name.clone()).or_insert_with(|| {
            let tags = snapshot
                .exercise_tag_map()
                .get(&canonical_name)
                .into_iter()
                .flatten()
                .map(|(kind, value)| IncomingTag {
                    kind: kind.clone(),
                    value: value.clone(),
                })
                .collect();
            IncomingExercise {
                name: canonical_name.clone(),
                tags,
            }
        });

        for set in exercise.sets {
            if incoming_sets.len() == MAX_SETS {
                return Err(NativeEntryError::new(format!(
                    "a workout may contain at most {MAX_SETS} sets"
                )));
            }
            let ordinal = incoming_sets.len() + 1;
            incoming_sets.push(IncomingSet {
                id: format!("{workout_id}:{ordinal:04}"),
                workout_id: workout_id.clone(),
                ordinal: ordinal as i64,
                exercise_name: canonical_name.clone(),
                raw_exercise_name: exercise.name.clone(),
                exercise_note: None,
                superset_id: None,
                weight_milli: set.weight_milli,
                weight_unit: "lbs".to_string(),
                reps: Some(set.reps as i64),
                effort_hundredths: set.effort_hundredths.map(|value| value as i64),
                distance_milli: None,
                set_time_seconds: None,
                set_type: set.set_type,
                incomplete: false,
            });
        }
    }
    let workout = IncomingWorkout {
        id: workout_id,
        title: input.title.clone(),
        raw_title: input.title,
        started_at_utc: input.started_at_utc,
        started_at_local: projection.local.clone(),
        eastern_offset_minutes: i64::from(projection.offset_minutes),
        duration_seconds,
        duration_suspicious: duration_seconds == 0 || duration_seconds >= 14_400,
        notes: input.notes,
        description: None,
        source: SOURCE.to_string(),
    };

    Ok(BuiltNativeEntry {
        public_path: eastern::public_path(&projection),
        payload: Payload {
            workouts: vec![workout],
            exercises: exercises.into_values().collect(),
            sets: incoming_sets,
        },
    })
}

#[cfg(test)]
mod tests {
    use benjisponge::data::fitness_models::{ExerciseAlias, ExerciseTag, LiftSet, Workout};
    use fitness_entry_core::{FinalizedExercise, FinalizedSet};

    use super::*;
    use crate::app::interests::lifting::archive::snapshot;

    fn fixture_snapshot() -> Snapshot {
        snapshot::build(
            7,
            vec![Workout {
                id: "fitness:2026-07-20T14:00:00".into(),
                title: "Leg day".into(),
                raw_title: "Leg day".into(),
                started_at_utc: "2026-07-20 14:00:00".into(),
                started_at_local: "2026-07-20 10:00:00".into(),
                eastern_offset_minutes: -240,
                duration_seconds: 3600,
                duration_suspicious: false,
                notes: None,
                description: None,
                source: "workout-data-csv".into(),
                imported_at: 0,
            }],
            vec![LiftSet {
                id: "fitness:2026-07-20T14:00:00:0001".into(),
                workout_id: "fitness:2026-07-20T14:00:00".into(),
                exercise_name: "Squat (Barbell)".into(),
                raw_exercise_name: "Squat (Barbell)".into(),
                ordinal: 1,
                exercise_note: None,
                superset_id: None,
                weight_milli: Some(225_000),
                weight_unit: "lbs".into(),
                reps: Some(5),
                effort_hundredths: Some(800),
                distance_milli: None,
                set_time_seconds: None,
                set_type: "NORMAL_SET".into(),
                incomplete: false,
            }],
            vec![ExerciseAlias {
                alias_name: "Back Squat".into(),
                canonical_name: "Squat (Barbell)".into(),
            }],
            vec![ExerciseTag {
                exercise_name: "Squat (Barbell)".into(),
                kind: "movement".into(),
                value: "squat-type".into(),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn set() -> FinalizedSet {
        FinalizedSet {
            weight_milli: Some(235_000),
            reps: 5,
            effort_hundredths: Some(900),
            set_type: "NORMAL_SET".into(),
        }
    }

    fn input() -> FinalizedWorkout {
        FinalizedWorkout {
            started_at_utc: "2026-07-21 14:39:04".into(),
            ended_at_utc: "2026-07-21 15:40:05".into(),
            title: "Lunch lift".into(),
            notes: Some("Felt good".into()),
            exercises: vec![FinalizedExercise {
                name: "Back Squat".into(),
                sets: vec![set()],
            }],
        }
    }

    #[test]
    fn builds_the_existing_manual_payload_and_derives_all_identity_fields() {
        let built = build_native_entry(input(), &fixture_snapshot()).unwrap();
        assert_eq!(built.public_path, "2026-07-21T10-39-04-04-00");

        let workout = &built.payload.workouts[0];
        assert_eq!(workout.id, "fitness:2026-07-21T14:39:04");
        assert_eq!(workout.started_at_local, "2026-07-21 10:39:04");
        assert_eq!(workout.eastern_offset_minutes, -240);
        assert_eq!(workout.duration_seconds, 3661);
        assert!(!workout.duration_suspicious);
        assert_eq!(workout.source, "manual");

        assert_eq!(built.payload.exercises.len(), 1);
        assert_eq!(built.payload.exercises[0].name, "Squat (Barbell)");
        assert_eq!(built.payload.exercises[0].tags.len(), 1);
        assert_eq!(built.payload.exercises[0].tags[0].value, "squat-type");

        let set = &built.payload.sets[0];
        assert_eq!(set.id, "fitness:2026-07-21T14:39:04:0001");
        assert_eq!(set.ordinal, 1);
        assert_eq!(set.exercise_name, "Squat (Barbell)");
        assert_eq!(set.raw_exercise_name, "Back Squat");
        assert_eq!(set.weight_milli, Some(235_000));
        assert_eq!(set.reps, Some(5));
        assert_eq!(set.effort_hundredths, Some(900));
        assert_eq!(set.weight_unit, "lbs");
        assert!(!set.incomplete);
    }

    #[test]
    fn serde_rejects_unknown_fields_at_every_level() {
        let top_level = serde_json::json!({
            "started_at_utc": "2026-07-21 14:39:04",
            "ended_at_utc": "2026-07-21 15:39:04",
            "title": "Lift",
            "notes": null,
            "exercises": [],
            "records": []
        });
        assert!(serde_json::from_value::<FinalizedWorkout>(top_level).is_err());

        let set_level = serde_json::json!({
            "started_at_utc": "2026-07-21 14:39:04",
            "ended_at_utc": "2026-07-21 15:39:04",
            "title": "Lift",
            "notes": null,
            "exercises": [{
                "name": "Back Squat",
                "sets": [{
                    "weight_milli": 225000,
                    "reps": 5,
                    "effort_hundredths": 900,
                    "set_type": "NORMAL_SET",
                    "records": []
                }]
            }]
        });
        assert!(serde_json::from_value::<FinalizedWorkout>(set_level).is_err());
    }

    #[test]
    fn rejects_bad_times_unknown_exercises_and_out_of_range_sets() {
        let snapshot = fixture_snapshot();

        let mut bad = input();
        bad.ended_at_utc = "2026-07-21 14:39:03".into();
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].name = "Imaginary Press".into();
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].sets[0].reps = fitness_entry_core::MAX_REPS + 1;
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].sets[0].effort_hundredths =
            Some(fitness_entry_core::MIN_EFFORT_HUNDREDTHS - 1);
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].sets[0].effort_hundredths =
            Some(fitness_entry_core::MAX_EFFORT_HUNDREDTHS + 1);
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].sets[0].set_type = "CHEAT_SET".into();
        assert!(build_native_entry(bad, &snapshot).is_err());
    }

    #[test]
    fn enforces_exercise_and_total_set_bounds() {
        let snapshot = fixture_snapshot();

        let mut bad = input();
        bad.exercises = Vec::new();
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises = vec![
            FinalizedExercise {
                name: "Back Squat".into(),
                sets: vec![set()],
            };
            fitness_entry_core::MAX_EXERCISES + 1
        ];
        assert!(build_native_entry(bad, &snapshot).is_err());

        let mut bad = input();
        bad.exercises[0].sets = vec![set(); MAX_SETS + 1];
        assert!(build_native_entry(bad, &snapshot).is_err());
    }
}
