//! Forward-only migrations for site-wide schema work that must run once.
//!
//! Tables and fields remain cheaply reconcilable through `schema.surql`.
//! Index definitions live here because SurrealDB 3.2.3 treats
//! `DEFINE INDEX OVERWRITE` as a destructive replacement and rebuild, even
//! when the physical definition is unchanged.

use super::Db;

const CURRENT_SCHEMA_EPOCH: u16 = 7;

struct Migration {
    epoch: u16,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        epoch: 1,
        sql: include_str!("schema_migrations/0001_indexes.surql"),
    },
    Migration {
        epoch: 2,
        sql: include_str!("schema_migrations/0002_analytics_visitor_days.surql"),
    },
    Migration {
        epoch: 3,
        sql: include_str!("schema_migrations/0003_analytics_pause_dirty_event.surql"),
    },
    Migration {
        epoch: 4,
        sql: include_str!("schema_migrations/0004_drop_analytics.surql"),
    },
    Migration {
        epoch: 5,
        sql: include_str!("schema_migrations/0005_thought_comments.surql"),
    },
    Migration {
        epoch: 6,
        sql: include_str!("schema_migrations/0006_running_activities.surql"),
    },
    Migration {
        epoch: 7,
        sql: include_str!("schema_migrations/0007_exercise_aliases.surql"),
    },
];

const LEDGER_SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE site_schema_migrations SCHEMAFULL PERMISSIONS NONE;
    DEFINE FIELD OVERWRITE epoch ON site_schema_migrations TYPE int
        ASSERT $value >= 1 AND $value <= 65535;";

pub(super) async fn apply(db: &Db) -> Result<(), String> {
    validate_registry()?;
    db.query(LEDGER_SCHEMA)
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;

    let mut applied = applied_epochs(db).await?;
    validate_ledger(&applied)?;
    for migration in MIGRATIONS {
        if applied.contains(&migration.epoch) {
            continue;
        }
        apply_one(db, migration).await?;
        applied = applied_epochs(db).await?;
        validate_ledger(&applied)?;
        if !applied.contains(&migration.epoch) {
            return Err(format!(
                "site schema migration epoch {} lost a concurrent activation race",
                migration.epoch
            ));
        }
    }
    if applied.contains(&CURRENT_SCHEMA_EPOCH) {
        Ok(())
    } else {
        Err(format!(
            "site schema stopped at epoch {:?}, expected {CURRENT_SCHEMA_EPOCH}",
            applied.last()
        ))
    }
}

async fn apply_one(db: &Db, migration: &Migration) -> Result<(), String> {
    let prior: Vec<i64> = (1..migration.epoch).map(i64::from).collect();
    let statement = format!(
        "BEGIN TRANSACTION;
         LET $applied = SELECT VALUE epoch FROM site_schema_migrations ORDER BY epoch ASC;
         IF $applied = $prior {{
             {}
             CREATE ONLY type::record('site_schema_migrations', $id)
                 CONTENT {{ epoch: $epoch }} RETURN NONE;
         }};
         COMMIT TRANSACTION;",
        migration.sql
    );
    let mut response = db
        .query(statement)
        .bind(("prior", prior))
        .bind(("id", format!("{:04}", migration.epoch)))
        .bind(("epoch", i64::from(migration.epoch)))
        .await
        .map_err(migration_error)?;
    let mut errors: Vec<(usize, String)> = response
        .take_errors()
        .into_iter()
        .map(|(index, error)| (index, error.to_string()))
        .collect();
    errors.sort_unstable_by_key(|(index, _)| *index);
    if !errors.is_empty() {
        return Err(format!(
            "site schema migration epoch {} statement errors: {errors:?}",
            migration.epoch
        ));
    }
    Ok(())
}

fn validate_registry() -> Result<(), String> {
    let expected: Vec<u16> = (1..=CURRENT_SCHEMA_EPOCH).collect();
    let actual: Vec<u16> = MIGRATIONS.iter().map(|migration| migration.epoch).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "site schema migration registry {actual:?} does not cover {expected:?}"
        ))
    }
}

pub(super) fn validate_ledger(applied: &[u16]) -> Result<(), String> {
    // Site epochs are additive. An older binary may use a database whose
    // contiguous ledger has newer entries; this is what keeps rollback to the
    // epoch-1 binary possible. Diary epochs remain separately exact-fenced.
    let expected: Vec<u16> = (1..=applied.last().copied().unwrap_or(0)).collect();
    if applied == expected {
        Ok(())
    } else {
        Err(format!(
            "site schema migration ledger has a gap: {applied:?}"
        ))
    }
}

async fn applied_epochs(db: &Db) -> Result<Vec<u16>, String> {
    let mut response = db
        .query("SELECT VALUE epoch FROM site_schema_migrations ORDER BY epoch ASC")
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;
    response.take(0).map_err(migration_error)
}

fn migration_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;
    use crate::data::SCHEMA;

    async fn db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db
    }

    #[tokio::test]
    async fn migrations_create_indexes_once_and_are_idempotent() {
        let db = db().await;
        db.query(SCHEMA).await.unwrap().check().unwrap();

        apply(&db).await.unwrap();
        apply(&db).await.unwrap();

        assert_eq!(
            applied_epochs(&db).await.unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7]
        );
        db.query("INFO FOR INDEX workouts_started_at_utc ON workouts")
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query("INFO FOR INDEX thought_comments_thought_created_at ON thought_comments")
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query("INFO FOR INDEX running_activities_started_at_utc ON running_activities")
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query("INFO FOR INDEX exercise_aliases_alias_name ON exercise_aliases")
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    #[tokio::test]
    async fn exercise_alias_migration_renames_existing_rows() {
        let db = db().await;
        db.query(SCHEMA).await.unwrap().check().unwrap();
        db.query(
            "CREATE ONLY type::record('exercises', $old) CONTENT { name: $old };
             CREATE exercise_tags CONTENT {
                 exercise_name: $old, kind: 'muscle', value: 'core'
             };
             LET $weight_id = crypto::sha256(string::concat($old, '\n', 'abs'));
             CREATE ONLY type::record('exercise_muscles', $weight_id) CONTENT {
                 id: $weight_id,
                 exercise_name: $old,
                 muscle: 'abs',
                 ratio_hundredths: 100,
                 source: 'admin',
                 updated_at: 123
             };
             CREATE ONLY type::record('sets', 'migration-test') CONTENT {
                 id: 'migration-test',
                 workout_id: 'workout',
                 exercise_name: $old,
                 raw_exercise_name: $old,
                 ordinal: 1,
                 exercise_note: NONE,
                 superset_id: NONE,
                 weight_milli: 50000,
                 weight_unit: 'lbs',
                 reps: 8,
                 effort_hundredths: NONE,
                 distance_milli: NONE,
                 set_time_seconds: NONE,
                 set_type: 'NORMAL_SET',
                 incomplete: false
             };",
        )
        .bind(("old", "Barbell Pullover Crunches".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();

        apply(&db).await.unwrap();

        let mut response = db
            .query(
                "RETURN {
                     exercises: (SELECT VALUE name FROM exercises),
                     set_name: (
                         SELECT VALUE exercise_name
                         FROM type::record('sets', 'migration-test')
                     )[0],
                     raw_name: (
                         SELECT VALUE raw_exercise_name
                         FROM type::record('sets', 'migration-test')
                     )[0],
                     tag_names: (SELECT VALUE exercise_name FROM exercise_tags),
                     weight_names: (SELECT VALUE exercise_name FROM exercise_muscles),
                     aliases: (SELECT alias_name, canonical_name FROM exercise_aliases),
                     version: (SELECT VALUE v FROM fitness_meta:version)[0]
                 };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let value: Option<serde_json::Value> = response.take(0).unwrap();
        let value = value.unwrap();
        assert_eq!(
            value["exercises"],
            serde_json::json!(["Barbell Resurrection Lifts"])
        );
        assert_eq!(value["set_name"], "Barbell Resurrection Lifts");
        assert_eq!(value["raw_name"], "Barbell Pullover Crunches");
        assert_eq!(
            value["tag_names"],
            serde_json::json!(["Barbell Resurrection Lifts"])
        );
        assert_eq!(
            value["weight_names"],
            serde_json::json!(["Barbell Resurrection Lifts"])
        );
        assert_eq!(value["aliases"].as_array().unwrap().len(), 3);
        assert_eq!(value["version"], 1);
    }

    #[tokio::test]
    async fn exercise_alias_migration_does_not_silently_merge_existing_press_history() {
        let db = db().await;
        db.query(SCHEMA).await.unwrap().check().unwrap();
        db.query(
            "CREATE ONLY type::record('exercises', 'Barbell Overhead Press')
                 CONTENT { name: 'Barbell Overhead Press' };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        apply(&db).await.unwrap();

        let mut response = db
            .query(
                "SELECT VALUE alias_name FROM exercise_aliases
                 WHERE alias_name = 'Barbell Overhead Press';",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(response.take::<Vec<String>>(0).unwrap().is_empty());
    }

    #[test]
    fn indexes_are_owned_only_by_forward_migrations() {
        assert!(
            !SCHEMA
                .lines()
                .any(|line| line.trim_start().starts_with("DEFINE INDEX"))
        );
        let indexes = MIGRATIONS
            .iter()
            .map(|migration| migration.sql.matches("DEFINE INDEX IF NOT EXISTS").count())
            .sum::<usize>();
        assert_eq!(indexes, 18);
        assert!(MIGRATIONS.iter().all(|migration| {
            !migration
                .sql
                .lines()
                .any(|line| line.trim_start().starts_with("DEFINE INDEX OVERWRITE"))
        }));
    }
}
