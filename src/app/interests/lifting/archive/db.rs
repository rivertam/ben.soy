//! Fitness archive database IO: one coherent snapshot query and the atomic
//! import write path.

use std::collections::{HashMap, HashSet};

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
