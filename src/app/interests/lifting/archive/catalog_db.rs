//! Atomic owner-managed exercise creation and definition editing.
use super::super::exercise_definition::Definition;
use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct SavedExercise {
    pub name: String,
    pub created: bool,
}

#[derive(Deserialize, SurrealValue)]
struct Identities {
    version: i64,
    exercises: Vec<String>,
    aliases: Vec<ExerciseAlias>,
}

/// A version fence serializes case-folded name/alias checks with all fitness writes.
/// A replay finds the committed identity and never replaces its definition.
pub async fn save(
    db: &Db,
    definition: &Definition,
    editing: Option<&str>,
    now: i64,
) -> anyhow::Result<SavedExercise> {
    let mut definition = definition.clone().validate().map_err(anyhow::Error::msg)?;
    for attempt in 0..5 {
        let mut response = db.query("RETURN { version: (SELECT VALUE v FROM fitness_meta:version)[0] ?? 0, exercises: (SELECT VALUE name FROM exercises), aliases: (SELECT alias_name, canonical_name FROM exercise_aliases) };").await?.check()?;
        let scan: Identities = response
            .take::<Option<Identities>>(0)?
            .context("missing exercise identities")?;
        let aliases = AliasMap::new(scan.aliases.clone());
        let folded = definition.name.to_lowercase();
        let existing = scan
            .exercises
            .iter()
            .find(|name| name.to_lowercase() == folded)
            .map(|name| aliases.resolve(name))
            .or_else(|| {
                scan.aliases
                    .iter()
                    .find(|alias| alias.alias_name.to_lowercase() == folded)
                    .map(|alias| aliases.resolve(&alias.alias_name))
            });
        if editing.is_none()
            && let Some(name) = &existing
        {
            if scan
                .exercises
                .iter()
                .any(|stored| aliases.resolve(stored) == *name)
            {
                return Ok(SavedExercise {
                    name: name.clone(),
                    created: false,
                });
            }
            // Pre-import aliases reserve a canonical target before it has a row.
            definition.name = name.clone();
        }
        if let Some(name) = editing {
            anyhow::ensure!(
                existing.as_deref() == Some(name) && definition.name == name,
                "Exercise identity changed; reload its page."
            );
        }
        let weights: Vec<WeightWrite> = definition
            .weights
            .iter()
            .map(|(muscle, ratio)| WeightWrite {
                id: exercise_muscle_id(&definition.name, muscle),
                exercise_name: definition.name.clone(),
                muscle: muscle.clone(),
                ratio_hundredths: i64::from(*ratio),
                source: "admin".into(),
                updated_at: now,
            })
            .collect();
        let tags: Vec<ExerciseTag> = definition
            .tags()
            .into_iter()
            .map(|(kind, value)| ExerciseTag {
                exercise_name: definition.name.clone(),
                kind,
                value,
            })
            .collect();
        let result = db.query("BEGIN TRANSACTION;
            IF ((SELECT VALUE v FROM fitness_meta:version)[0] ?? 0) != $version { THROW 'exercise-definition-stale'; };
            IF $editing {
                DELETE exercise_tags WHERE exercise_name = $name RETURN NONE;
                LET $muscles = SELECT VALUE muscle FROM exercise_muscles WHERE exercise_name = $name;
                FOR $muscle IN $muscles { DELETE exercise_muscles WHERE exercise_name = $name AND muscle = $muscle RETURN NONE; };
                UPDATE ONLY type::record('exercises', $name) SET admin_managed = true RETURN NONE;
            } ELSE {
                CREATE ONLY type::record('exercises', $name) CONTENT { name: $name, admin_managed: true } RETURN NONE;
            };
            FOR $tag IN $tags { CREATE exercise_tags CONTENT $tag RETURN NONE; };
            FOR $weight IN $weights { CREATE ONLY type::record('exercise_muscles', $weight.id) CONTENT $weight RETURN NONE; };
            UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1 RETURN NONE;
            COMMIT TRANSACTION;")
            .bind(("version", scan.version)).bind(("editing", editing.is_some())).bind(("name", definition.name.clone())).bind(("tags", tags)).bind(("weights", weights)).await.and_then(|response| response.check());
        match result {
            Ok(_) => {
                return Ok(SavedExercise {
                    name: definition.name.clone(),
                    created: editing.is_none(),
                });
            }
            Err(error) if attempt == 4 => return Err(error.into()),
            Err(_) => tokio::time::sleep(Duration::from_millis(15 * (attempt + 1))).await,
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{database, payload};
    use super::*;
    use crate::app::interests::lifting::archive::{
        exercise_definition::references, native_entry::build_native_entry, snapshot,
    };

    async fn snapshot(db: &Db) -> snapshot::Snapshot {
        let (version, workouts, sets, aliases, tags, weights, interruptions, exercises, references) =
            load_archive(db).await.unwrap();
        snapshot::build(
            version,
            workouts,
            sets,
            aliases,
            tags,
            weights,
            interruptions,
        )
        .unwrap()
        .with_catalog(exercises, references)
    }
    fn definition(name: &str) -> Definition {
        Definition {
            name: name.into(),
            movements: vec!["elbow-flexion".into()],
            equipment: vec!["dumbbell".into()],
            weights: [("biceps".into(), 100), ("brachialis".into(), 45)].into(),
        }
    }

    #[tokio::test]
    async fn a_preimport_alias_creates_its_canonical_target() {
        let db = database().await;
        db.query("CREATE exercise_aliases CONTENT {alias_name: 'Old Curl', canonical_name: 'New Curl', updated_at: 1};").await.unwrap().check().unwrap();
        let saved = save(&db, &definition("old curl"), None, 2).await.unwrap();
        assert!(saved.created);
        assert_eq!(saved.name, "New Curl");
        assert_eq!(snapshot(&db).await.exercise_names(), ["New Curl"]);
    }

    #[tokio::test]
    async fn three_created_exercises_publish_together_without_prior_history() {
        let db = database().await;
        let names = ["New Dumbbell Curl", "New Cable Curl", "Arnold Press"];
        for (index, name) in names.iter().enumerate() {
            let mut value = definition(name);
            if index == 2 {
                value = Definition {
                    name: (*name).into(),
                    ..Definition::default()
                };
            }
            assert!(save(&db, &value, None, 5).await.unwrap().created);
        }
        reconcile_muscle_weights(&db, 6).await.unwrap();
        let before = snapshot(&db).await;
        assert_eq!(before.exercise_names().len(), 3);
        assert_eq!(before.facets().summary.sets, 0);
        assert!(before.exercise_weight_map().get("Arnold Press").is_none());
        let input = fitness_entry_core::FinalizedWorkout {
            started_at_utc: "2026-09-10 14:00:00".into(),
            ended_at_utc: "2026-09-10 15:00:00".into(),
            title: "Three creations".into(),
            notes: None,
            exercises: names
                .iter()
                .map(|name| fitness_entry_core::FinalizedExercise {
                    name: (*name).into(),
                    sets: vec![fitness_entry_core::FinalizedSet {
                        weight_milli: Some(20_000),
                        reps: 8,
                        effort_hundredths: Some(900),
                        failure: false,
                        set_type: "NORMAL_SET".into(),
                    }],
                })
                .collect(),
        };
        let built = build_native_entry(input, &before).unwrap();
        assert_eq!(
            create_manual_workout(&db, &built.payload, 7).await.unwrap(),
            ManualImportOutcome::Added
        );
        assert_eq!(
            create_manual_workout(&db, &built.payload, 8).await.unwrap(),
            ManualImportOutcome::Duplicate
        );
        let after = snapshot(&db).await;
        assert_eq!(after.facets().summary.workouts, 1);
        assert_eq!(after.facets().summary.sets, 3);
        for name in names {
            assert_eq!(after.exercise_profile(name).unwrap().set_count, 1);
        }
    }

    #[tokio::test]
    async fn managed_definition_survives_csv_and_an_empty_managed_rename() {
        let db = database().await;
        let original = definition("Incline Bench Press");
        save(&db, &original, None, 1).await.unwrap();
        let mut csv = payload();
        csv.workouts[0].source = "workout-data-csv".into();
        csv.exercises[0].tags = vec![IncomingTag {
            kind: "muscle".into(),
            value: "chest".into(),
        }];
        apply_import(&db, &csv, 2).await.unwrap();
        assert_eq!(
            Definition::from_snapshot(&snapshot(&db).await, &original.name),
            original
        );
        let empty = Definition {
            name: original.name.clone(),
            ..Definition::default()
        };
        save(&db, &empty, Some(&original.name), 3).await.unwrap();
        apply_import(&db, &csv, 4).await.unwrap();
        reconcile_muscle_weights(&db, 5).await.unwrap();
        assert_eq!(
            Definition::from_snapshot(&snapshot(&db).await, &original.name),
            empty
        );
        let plan = plan_exercise_identity(&db, &original.name, "New Bench", &[])
            .await
            .unwrap()
            .unwrap();
        replace_exercise_identity(&db, &plan, 6).await.unwrap();
        let mut response = db
            .query("SELECT admin_managed FROM exercises WHERE name = 'New Bench';")
            .await
            .unwrap()
            .check()
            .unwrap();
        let rows: Vec<serde_json::Value> = response.take(0).unwrap();
        assert_eq!(rows[0]["admin_managed"], true);
        assert!(
            snapshot(&db)
                .await
                .exercise_weight_map()
                .get("New Bench")
                .is_none()
        );
    }

    #[tokio::test]
    async fn case_alias_and_concurrent_replays_never_replace_definitions() {
        let db = database().await;
        let original = definition("Dumbbell Curl");
        let alternate = Definition {
            name: "dumbbell curl".into(),
            ..Definition::default()
        };
        let (first, second) = tokio::join!(
            save(&db, &original, None, 1),
            save(&db, &alternate, None, 1)
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.name, second.name);
        assert_ne!(first.created, second.created);
        let before = snapshot(&db).await;
        let mut replay = definition(&first.name);
        replay.weights.clear();
        assert!(!save(&db, &replay, None, 2).await.unwrap().created);
        assert_eq!(
            Definition::from_snapshot(&snapshot(&db).await, &first.name),
            Definition::from_snapshot(&before, &first.name)
        );
        db.query("CREATE exercise_aliases CONTENT {alias_name: 'Old Curl', canonical_name: $name, updated_at: 1};").bind(("name", first.name.clone())).await.unwrap().check().unwrap();
        assert_eq!(
            save(&db, &definition("old curl"), None, 3)
                .await
                .unwrap()
                .name,
            first.name
        );
    }

    #[tokio::test]
    async fn suggestions_prefer_matching_equipment_and_do_not_invent_weights() {
        let db = database().await;
        let dumbbell = definition("Dumbbell Curl");
        let mut cable = definition("Cable Curl");
        cable.equipment = vec!["cable".into()];
        save(&db, &dumbbell, None, 1).await.unwrap();
        save(&db, &cable, None, 2).await.unwrap();
        let snap = snapshot(&db).await;
        assert_eq!(
            references(&snap, &definition("Another Curl")),
            ["Dumbbell Curl", "Cable Curl"]
        );
        let unrelated = Definition {
            name: "Unknown".into(),
            movements: vec!["hinge".into()],
            ..Definition::default()
        };
        assert!(references(&snap, &unrelated).is_empty());
    }
}
