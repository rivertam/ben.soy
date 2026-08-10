//! Forward-only migrations for site-wide schema work that must run once.
//!
//! Tables and fields remain cheaply reconcilable through `schema.surql`.
//! Index definitions live here because SurrealDB 3.2.3 treats
//! `DEFINE INDEX OVERWRITE` as a destructive replacement and rebuild, even
//! when the physical definition is unchanged.

use super::Db;

const CURRENT_SCHEMA_EPOCH: u16 = 3;

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
    db.query(statement)
        .bind(("prior", prior))
        .bind(("id", format!("{:04}", migration.epoch)))
        .bind(("epoch", i64::from(migration.epoch)))
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;
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

        assert_eq!(applied_epochs(&db).await.unwrap(), vec![1, 2, 3]);
        db.query("INFO FOR INDEX analytics_events_kind_time ON analytics_events")
            .await
            .unwrap()
            .check()
            .unwrap();
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
        assert_eq!(indexes, 19);
        assert!(MIGRATIONS.iter().all(|migration| {
            !migration
                .sql
                .lines()
                .any(|line| line.trim_start().starts_with("DEFINE INDEX OVERWRITE"))
        }));
    }
}
