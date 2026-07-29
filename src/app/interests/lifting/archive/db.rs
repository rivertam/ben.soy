//! Fitness archive database IO: one coherent snapshot query and the atomic
//! import write path.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use anyhow::Context;
use benjisponge::data::{
    Db,
    fitness_models::{Exercise, ExerciseTag, LiftSet, Workout},
};
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use super::import::{IncomingTag, Payload, tag_signature};

/// The data version; 0 when the row does not exist yet.
pub async fn current_version(db: &Db) -> surrealdb::Result<i64> {
    let mut response = db
        .query("SELECT VALUE v FROM fitness_meta:version")
        .await?
        .check()?;
    let versions: Vec<i64> = response.take(0)?;
    Ok(versions.into_iter().next().unwrap_or(0))
}

#[derive(Deserialize, SurrealValue)]
struct ArchiveRows {
    version: i64,
    workouts: Vec<Workout>,
    sets: Vec<LiftSet>,
    tags: Vec<ExerciseTag>,
}

/// The version and everything its snapshot needs from one read transaction.
/// Row order is irrelevant because the snapshot sorts.
pub async fn load_archive(
    db: &Db,
) -> anyhow::Result<(i64, Vec<Workout>, Vec<LiftSet>, Vec<ExerciseTag>)> {
    let mut response = db
        .query(
            "RETURN {
                 version: (SELECT VALUE v FROM fitness_meta:version)[0] ?? 0,
                 workouts: (SELECT *, record::id(id) AS id FROM workouts),
                 sets: (SELECT *, record::id(id) AS id FROM sets),
                 tags: (
                     SELECT exercise_name, kind, value FROM exercise_tags
                 )
             };",
        )
        .await?
        .check()?;
    let rows: Option<ArchiveRows> = response.take(0)?;
    let rows = rows.context("fitness archive query returned no snapshot")?;
    Ok((rows.version, rows.workouts, rows.sets, rows.tags))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOutcome {
    pub received: usize,
    pub added: usize,
    pub skipped: usize,
    pub version: i64,
    /// Whether anything was written (sets added or tags replaced) — the
    /// caller rebuilds the snapshot only then.
    pub mutated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualImportOutcome {
    Added,
    Duplicate,
    Conflict,
}

/// Create one owner-submitted workout without inheriting the CSV importer's
/// append semantics. An exact repeat is idempotent; the same deterministic
/// timestamp ID with any different workout/set content is a conflict.
///
/// Existing exercise taxonomy is left untouched. Only exercises that do not
/// exist yet are created and receive the parser's shared taxonomy.
pub async fn create_manual_workout(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<ManualImportOutcome> {
    let Some(incoming_workout) = payload.workouts.first() else {
        return Ok(ManualImportOutcome::Conflict);
    };
    if payload.workouts.len() != 1 || incoming_workout.source != "manual" {
        return Ok(ManualImportOutcome::Conflict);
    }

    // Preflight reads deliberately stay outside the write transaction so the
    // common path remains small. CREATE ONLY and the unique indexes close the
    // race atomically; if another writer wins between preflight and COMMIT,
    // re-read to classify an identical/timestamp-colliding workout, or retry
    // after a shared exercise/version conflict. SurrealDB 3.2.3 can surface a
    // transaction conflict as an earlier NotExecuted statement, so every
    // failed transaction gets the same conservative bounded treatment.
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        if let Some(outcome) = existing_manual_outcome(db, payload).await? {
            return Ok(outcome);
        }
        match create_manual_workout_attempt(db, payload, imported_at).await {
            Ok(()) => return Ok(ManualImportOutcome::Added),
            Err(error) => {
                if let Ok(Some(outcome)) = existing_manual_outcome(db, payload).await {
                    return Ok(outcome);
                }
                if attempt == MAX_ATTEMPTS {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
            }
        }
    }
    unreachable!("the bounded manual-import loop always returns")
}

async fn existing_manual_outcome(
    db: &Db,
    payload: &Payload,
) -> surrealdb::Result<Option<ManualImportOutcome>> {
    let incoming_workout = payload
        .workouts
        .first()
        .expect("manual payload was validated before lookup");
    let mut response = db
        .query(
            "SELECT *, record::id(id) AS id
             FROM workouts
             WHERE record::id(id) = $workout_id;",
        )
        .bind(("workout_id", incoming_workout.id.clone()))
        .await?
        .check()?;
    let mut existing_workouts: Vec<Workout> = response.take(0)?;
    if let Some(existing_workout) = existing_workouts.pop() {
        let mut response = db
            .query(
                "SELECT *, record::id(id) AS id
                 FROM sets
                 WHERE workout_id = $workout_id
                 ORDER BY ordinal ASC;",
            )
            .bind(("workout_id", incoming_workout.id.clone()))
            .await?
            .check()?;
        let existing_sets: Vec<LiftSet> = response.take(0)?;
        return Ok(Some(
            if same_manual_workout(&existing_workout, &existing_sets, payload) {
                ManualImportOutcome::Duplicate
            } else {
                ManualImportOutcome::Conflict
            },
        ));
    }
    Ok(None)
}

async fn create_manual_workout_attempt(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<()> {
    let incoming_workout = payload
        .workouts
        .first()
        .expect("manual payload was validated before write");
    let exercise_names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let mut response = db
        .query("SELECT VALUE name FROM exercises WHERE name IN $names;")
        .bind(("names", exercise_names))
        .await?
        .check()?;
    let existing_exercises: HashSet<String> =
        response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_exercises: Vec<Exercise> = payload
        .exercises
        .iter()
        .filter(|exercise| !existing_exercises.contains(&exercise.name))
        .map(|exercise| Exercise {
            name: exercise.name.clone(),
        })
        .collect();
    let missing_names: HashSet<&str> = missing_exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let added_tags: Vec<ExerciseTag> = payload
        .exercises
        .iter()
        .filter(|exercise| missing_names.contains(exercise.name.as_str()))
        .flat_map(|exercise| {
            exercise.tags.iter().map(|tag| ExerciseTag {
                exercise_name: exercise.name.clone(),
                kind: tag.kind.clone(),
                value: tag.value.clone(),
            })
        })
        .collect();

    let workouts = vec![Workout {
        id: incoming_workout.id.clone(),
        title: incoming_workout.title.clone(),
        raw_title: incoming_workout.raw_title.clone(),
        started_at_utc: incoming_workout.started_at_utc.clone(),
        started_at_local: incoming_workout.started_at_local.clone(),
        eastern_offset_minutes: incoming_workout.eastern_offset_minutes,
        duration_seconds: incoming_workout.duration_seconds,
        duration_suspicious: incoming_workout.duration_suspicious,
        notes: incoming_workout.notes.clone(),
        description: incoming_workout.description.clone(),
        source: incoming_workout.source.clone(),
        imported_at,
    }];
    let sets: Vec<LiftSet> = payload
        .sets
        .iter()
        .map(|set| LiftSet {
            id: set.id.clone(),
            workout_id: set.workout_id.clone(),
            exercise_name: set.exercise_name.clone(),
            raw_exercise_name: set.raw_exercise_name.clone(),
            ordinal: set.ordinal,
            exercise_note: set.exercise_note.clone(),
            superset_id: set.superset_id,
            weight_milli: set.weight_milli,
            weight_unit: set.weight_unit.clone(),
            reps: set.reps,
            effort_hundredths: set.effort_hundredths,
            distance_milli: set.distance_milli,
            set_time_seconds: set.set_time_seconds,
            set_type: set.set_type.clone(),
            incomplete: set.incomplete,
        })
        .collect();

    db.query(
        "BEGIN TRANSACTION;
         FOR $workout IN $workouts {
             CREATE ONLY type::record('workouts', $workout.id) CONTENT $workout;
         };
         FOR $exercise IN $exercises {
             CREATE ONLY type::record('exercises', $exercise.name) CONTENT $exercise;
         };
         FOR $tag IN $tags {
             CREATE exercise_tags CONTENT $tag;
         };
         FOR $set IN $sets {
             CREATE ONLY type::record('sets', $set.id) CONTENT $set;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workouts", workouts))
    .bind(("exercises", missing_exercises))
    .bind(("tags", added_tags))
    .bind(("sets", sets))
    .await?
    .check()?;

    Ok(())
}

fn same_manual_workout(workout: &Workout, sets: &[LiftSet], payload: &Payload) -> bool {
    let Some(incoming) = payload.workouts.first() else {
        return false;
    };
    if payload.workouts.len() != 1
        || workout.id != incoming.id
        || workout.title != incoming.title
        || workout.raw_title != incoming.raw_title
        || workout.started_at_utc != incoming.started_at_utc
        || workout.started_at_local != incoming.started_at_local
        || workout.eastern_offset_minutes != incoming.eastern_offset_minutes
        || workout.duration_seconds != incoming.duration_seconds
        || workout.duration_suspicious != incoming.duration_suspicious
        || workout.notes != incoming.notes
        || workout.description != incoming.description
        || workout.source != incoming.source
        || sets.len() != payload.sets.len()
    {
        return false;
    }

    let mut stored: Vec<&LiftSet> = sets.iter().collect();
    stored.sort_unstable_by_key(|set| set.ordinal);
    let mut incoming: Vec<_> = payload.sets.iter().collect();
    incoming.sort_unstable_by_key(|set| set.ordinal);
    stored.into_iter().zip(incoming).all(|(stored, incoming)| {
        stored.id == incoming.id
            && stored.workout_id == incoming.workout_id
            && stored.exercise_name == incoming.exercise_name
            && stored.raw_exercise_name == incoming.raw_exercise_name
            && stored.ordinal == incoming.ordinal
            && stored.exercise_note == incoming.exercise_note
            && stored.superset_id == incoming.superset_id
            && stored.weight_milli == incoming.weight_milli
            && stored.weight_unit == incoming.weight_unit
            && stored.reps == incoming.reps
            && stored.effort_hundredths == incoming.effort_hundredths
            && stored.distance_milli == incoming.distance_milli
            && stored.set_time_seconds == incoming.set_time_seconds
            && stored.set_type == incoming.set_type
            && stored.incomplete == incoming.incomplete
    })
}

/// What a delete removed, for the caller's receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedWorkout {
    pub workout_id: String,
    pub source: String,
    pub sets_deleted: usize,
    pub version: i64,
}

/// The one row the public path resolves to. `source` rides along because the
/// receipt reports it: deleting a `workout-data-csv` workout is undone by the
/// next `just sync-fitness`, and the caller deserves to be told.
#[derive(Deserialize, SurrealValue)]
struct DeletionTarget {
    id: String,
    source: String,
}

/// Delete one workout and its sets, addressed exactly the way the public API
/// addresses it — by Eastern local start plus offset, the pair `by_path`
/// matches on. `Ok(None)` means no workout is stored there.
///
/// Deliberately narrow: `sets`, the `workouts` row, and the version counter.
/// Exercise and tag rows are left alone even when this was an exercise's last
/// set. That is the same invariant the CSV reset relies on — the snapshot
/// never loads the `exercises` table, and every public count joins through
/// sets, so an orphan is invisible rather than harmless-but-visible. Keeping
/// them also preserves hand-corrected taxonomy across a delete-and-repaste.
///
/// Records need no cleanup: they are derived at snapshot build, so the
/// remaining history re-derives its own podium on the next rebuild.
pub async fn delete_workout_by_path(
    db: &Db,
    local: &str,
    offset_minutes: i32,
) -> surrealdb::Result<Option<DeletedWorkout>> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, source
             FROM workouts
             WHERE started_at_local = $local AND eastern_offset_minutes = $offset;",
        )
        .bind(("local", local.to_string()))
        .bind(("offset", i64::from(offset_minutes)))
        .await?
        .check()?;
    let mut targets: Vec<DeletionTarget> = response.take(0)?;
    let Some(target) = targets.pop() else {
        return Ok(None);
    };

    // Counted before the delete rather than returned from it: `DELETE` yields
    // nothing by default, and `RETURN BEFORE` would hand back records whose
    // `id` is a record id the model cannot deserialize.
    let mut response = db
        .query("SELECT VALUE record::id(id) FROM sets WHERE workout_id = $workout_id;")
        .bind(("workout_id", target.id.clone()))
        .await?
        .check()?;
    let sets_deleted = response.take::<Vec<String>>(0)?.len();

    // `=` on the whole predicate, never `IN [..]`: on a table carrying a
    // compound unique index an `IN` delete can match nothing and still report
    // success (see docs/surrealdb-notes.md).
    db.query(
        "BEGIN TRANSACTION;
         DELETE sets WHERE workout_id = $workout_id;
         DELETE type::record('workouts', $workout_id);
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workout_id", target.id.clone()))
    .await?
    .check()?;

    Ok(Some(DeletedWorkout {
        workout_id: target.id,
        source: target.source,
        sets_deleted,
        version: current_version(db).await?,
    }))
}

pub async fn apply_import(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<ImportOutcome> {
    let set_ids: Vec<String> = payload.sets.iter().map(|set| set.id.clone()).collect();
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM sets
             WHERE record::id(id) IN $set_ids;",
        )
        .bind(("set_ids", set_ids))
        .await?
        .check()?;
    let existing_sets: HashSet<String> = response.take::<Vec<String>>(0)?.into_iter().collect();
    let candidates: Vec<_> = payload
        .sets
        .iter()
        .filter(|set| !existing_sets.contains(&set.id))
        .collect();

    let exercise_names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let stored_tags: HashMap<String, Vec<IncomingTag>> = {
        let mut response = db
            .query(
                "SELECT exercise_name, kind, value
                 FROM exercise_tags
                 WHERE exercise_name IN $exercise_names;",
            )
            .bind(("exercise_names", exercise_names))
            .await?
            .check()?;
        let rows: Vec<ExerciseTag> = response.take(0)?;
        let mut by_exercise: HashMap<String, Vec<IncomingTag>> = HashMap::new();
        for row in rows {
            by_exercise
                .entry(row.exercise_name)
                .or_default()
                .push(IncomingTag {
                    kind: row.kind,
                    value: row.value,
                });
        }
        by_exercise
    };
    let changed_exercises: HashSet<&str> = payload
        .exercises
        .iter()
        .filter(|exercise| {
            let stored = stored_tags
                .get(&exercise.name)
                .map(Vec::as_slice)
                .unwrap_or_default();
            tag_signature(&exercise.tags) != tag_signature(stored)
        })
        .map(|exercise| exercise.name.as_str())
        .collect();

    let received = payload.sets.len();
    if candidates.is_empty() && changed_exercises.is_empty() {
        return Ok(ImportOutcome {
            received,
            added: 0,
            skipped: received,
            version: current_version(db).await?,
            mutated: false,
        });
    }

    let workout_ids: Vec<String> = payload
        .workouts
        .iter()
        .map(|workout| workout.id.clone())
        .collect();
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM workouts
             WHERE record::id(id) IN $workout_ids;",
        )
        .bind(("workout_ids", workout_ids))
        .await?
        .check()?;
    let existing_workouts: HashSet<String> = response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_workouts: Vec<Workout> = payload
        .workouts
        .iter()
        .filter(|workout| !existing_workouts.contains(&workout.id))
        .map(|workout| Workout {
            id: workout.id.clone(),
            title: workout.title.clone(),
            raw_title: workout.raw_title.clone(),
            started_at_utc: workout.started_at_utc.clone(),
            started_at_local: workout.started_at_local.clone(),
            eastern_offset_minutes: workout.eastern_offset_minutes,
            duration_seconds: workout.duration_seconds,
            duration_suspicious: workout.duration_suspicious,
            notes: workout.notes.clone(),
            description: workout.description.clone(),
            source: workout.source.clone(),
            imported_at,
        })
        .collect();

    let names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let mut response = db
        .query("SELECT VALUE name FROM exercises WHERE name IN $names;")
        .bind(("names", names))
        .await?
        .check()?;
    let existing_exercises: HashSet<String> =
        response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_exercises: Vec<Exercise> = payload
        .exercises
        .iter()
        .filter(|exercise| !existing_exercises.contains(&exercise.name))
        .map(|exercise| Exercise {
            name: exercise.name.clone(),
        })
        .collect();

    let mut removed_tags = Vec::new();
    let mut added_tags = Vec::new();
    for exercise in &payload.exercises {
        if !changed_exercises.contains(exercise.name.as_str()) {
            continue;
        }
        let stored = stored_tags
            .get(&exercise.name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let incoming_keys: HashSet<_> = exercise
            .tags
            .iter()
            .map(|tag| (tag.kind.as_str(), tag.value.as_str()))
            .collect();
        for tag in stored {
            if !incoming_keys.contains(&(tag.kind.as_str(), tag.value.as_str())) {
                removed_tags.push(ExerciseTag {
                    exercise_name: exercise.name.clone(),
                    kind: tag.kind.clone(),
                    value: tag.value.clone(),
                });
            }
        }
        let stored_keys: HashSet<_> = stored
            .iter()
            .map(|tag| (tag.kind.as_str(), tag.value.as_str()))
            .collect();
        for tag in &exercise.tags {
            if !stored_keys.contains(&(tag.kind.as_str(), tag.value.as_str())) {
                added_tags.push(ExerciseTag {
                    exercise_name: exercise.name.clone(),
                    kind: tag.kind.clone(),
                    value: tag.value.clone(),
                });
            }
        }
    }

    let candidate_sets: Vec<LiftSet> = candidates
        .iter()
        .map(|set| LiftSet {
            id: set.id.clone(),
            workout_id: set.workout_id.clone(),
            exercise_name: set.exercise_name.clone(),
            raw_exercise_name: set.raw_exercise_name.clone(),
            ordinal: set.ordinal,
            exercise_note: set.exercise_note.clone(),
            superset_id: set.superset_id,
            weight_milli: set.weight_milli,
            weight_unit: set.weight_unit.clone(),
            reps: set.reps,
            effort_hundredths: set.effort_hundredths,
            distance_milli: set.distance_milli,
            set_time_seconds: set.set_time_seconds,
            set_type: set.set_type.clone(),
            incomplete: set.incomplete,
        })
        .collect();
    db.query(
        "BEGIN TRANSACTION;
         FOR $workout IN $workouts {
             CREATE ONLY type::record('workouts', $workout.id) CONTENT $workout;
         };
         FOR $exercise IN $exercises {
             CREATE ONLY type::record('exercises', $exercise.name) CONTENT $exercise;
         };
         FOR $tag IN $removed_tags {
             DELETE exercise_tags
                 WHERE exercise_name = $tag.exercise_name
                     AND kind = $tag.kind
                     AND value = $tag.value;
         };
         FOR $tag IN $added_tags {
             CREATE exercise_tags CONTENT $tag;
         };
         FOR $set IN $sets {
             CREATE ONLY type::record('sets', $set.id) CONTENT $set;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workouts", missing_workouts))
    .bind(("exercises", missing_exercises))
    .bind(("removed_tags", removed_tags))
    .bind(("added_tags", added_tags))
    .bind(("sets", candidate_sets))
    .await?
    .check()?;

    let added = candidates.len();
    Ok(ImportOutcome {
        received,
        added,
        skipped: received - added,
        version: current_version(db).await?,
        mutated: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::lifting::archive::import::{
        IncomingExercise, IncomingSet, IncomingWorkout,
    };

    fn payload() -> Payload {
        Payload {
            workouts: vec![IncomingWorkout {
                id: "fitness:2026-07-24T14:38:00".into(),
                title: "Arms".into(),
                raw_title: "Arms".into(),
                started_at_utc: "2026-07-24 14:38:00".into(),
                started_at_local: "2026-07-24 10:38:00".into(),
                eastern_offset_minutes: -240,
                duration_seconds: 720,
                duration_suspicious: false,
                notes: None,
                description: None,
                source: "manual".into(),
            }],
            exercises: vec![IncomingExercise {
                name: "Incline Bench Press".into(),
                tags: Vec::new(),
            }],
            sets: vec![IncomingSet {
                id: "fitness:2026-07-24T14:38:00:0001".into(),
                workout_id: "fitness:2026-07-24T14:38:00".into(),
                ordinal: 1,
                exercise_name: "Incline Bench Press".into(),
                raw_exercise_name: "Incline Bench Press".into(),
                exercise_note: None,
                superset_id: None,
                weight_milli: Some(135_000),
                weight_unit: "lbs".into(),
                reps: Some(6),
                effort_hundredths: Some(900),
                distance_milli: None,
                set_time_seconds: None,
                set_type: "NORMAL_SET".into(),
                incomplete: false,
            }],
        }
    }

    fn stored(payload: &Payload) -> (Workout, Vec<LiftSet>) {
        let workout = &payload.workouts[0];
        let set = &payload.sets[0];
        (
            Workout {
                id: workout.id.clone(),
                title: workout.title.clone(),
                raw_title: workout.raw_title.clone(),
                started_at_utc: workout.started_at_utc.clone(),
                started_at_local: workout.started_at_local.clone(),
                eastern_offset_minutes: workout.eastern_offset_minutes,
                duration_seconds: workout.duration_seconds,
                duration_suspicious: workout.duration_suspicious,
                notes: workout.notes.clone(),
                description: workout.description.clone(),
                source: workout.source.clone(),
                imported_at: 123,
            },
            vec![LiftSet {
                id: set.id.clone(),
                workout_id: set.workout_id.clone(),
                exercise_name: set.exercise_name.clone(),
                raw_exercise_name: set.raw_exercise_name.clone(),
                ordinal: set.ordinal,
                exercise_note: set.exercise_note.clone(),
                superset_id: set.superset_id,
                weight_milli: set.weight_milli,
                weight_unit: set.weight_unit.clone(),
                reps: set.reps,
                effort_hundredths: set.effort_hundredths,
                distance_milli: set.distance_milli,
                set_time_seconds: set.set_time_seconds,
                set_type: set.set_type.clone(),
                incomplete: set.incomplete,
            }],
        )
    }

    #[test]
    fn exact_manual_repeat_is_idempotent() {
        let payload = payload();
        let (workout, sets) = stored(&payload);
        assert!(same_manual_workout(&workout, &sets, &payload));
    }

    #[test]
    fn timestamp_collision_with_different_content_or_source_conflicts() {
        let payload = payload();
        let (workout, mut sets) = stored(&payload);
        sets[0].reps = Some(7);
        assert!(!same_manual_workout(&workout, &sets, &payload));

        let (mut workout, sets) = stored(&payload);
        workout.source = "workout-data-csv".into();
        assert!(!same_manual_workout(&workout, &sets, &payload));
    }
}
