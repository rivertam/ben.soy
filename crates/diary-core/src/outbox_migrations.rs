//! Ordered, forward-only migrations for the device-local diary store.
//!
//! The page and service worker share one IndexedDB database but may execute
//! different cached wasm builds. Each migration changes the schema and records
//! its epoch in one transaction. Browser glue holds `diary-store` across
//! [`require_current`] and the caller's operation, so an older build pauses
//! before it can interpret a newer canonical row.

use crate::Db;
use crate::contract::CURRENT_SCHEMA_EPOCH;
use crate::outbox::OutboxError;

struct Migration {
    epoch: u16,
    schema_sql: &'static str,
    data_sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    epoch: 1,
    schema_sql: include_str!("outbox_migrations/0001_current_entry_store.surql"),
    // This column existed only on review builds of the superseded adapter
    // ladder. It is retired once, not rewritten across all history per sync.
    data_sql: "\
        DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int>;\n\
        UPDATE diary_entries UNSET entry_version;\n\
        REMOVE FIELD IF EXISTS entry_version ON diary_entries;",
}];

const LEDGER_SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_schema_migrations SCHEMAFULL PERMISSIONS NONE;\n\
    DEFINE FIELD OVERWRITE epoch ON diary_schema_migrations TYPE int \
        ASSERT $value >= 1 AND $value <= 65535;\n\
    DEFINE INDEX OVERWRITE diary_schema_migrations_epoch \
        ON diary_schema_migrations FIELDS epoch UNIQUE;\n";

/// Apply every missing migration, then reconcile only the latest definitions.
/// One-shot data cleanup stays inside its migration and is never O(history) on
/// an ordinary reopen or sync.
pub(crate) async fn apply(db: &Db) -> Result<(), OutboxError> {
    validate_registry()?;
    db.query(LEDGER_SCHEMA)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let applied = applied_epochs(db).await?;
    validate_ledger(&applied)?;
    for migration in MIGRATIONS {
        if applied.contains(&migration.epoch) {
            continue;
        }
        apply_one(db, migration).await?;
    }
    require_current(db).await?;
    db.query(current_schema()?)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    Ok(())
}

/// Cheap check for an already-cached device handle. The browser's shared Web
/// Lock makes this check and the following operation one critical section.
pub(crate) async fn require_current(db: &Db) -> Result<(), OutboxError> {
    let applied = applied_epochs(db).await?;
    validate_ledger(&applied)?;
    if applied.last().copied() == Some(CURRENT_SCHEMA_EPOCH) {
        Ok(())
    } else {
        Err(OutboxError::Db(format!(
            "device diary schema stopped at epoch {:?}, expected {CURRENT_SCHEMA_EPOCH}",
            applied.last()
        )))
    }
}

fn validate_registry() -> Result<(), OutboxError> {
    let expected: Vec<u16> = (1..=CURRENT_SCHEMA_EPOCH).collect();
    let actual: Vec<u16> = MIGRATIONS.iter().map(|migration| migration.epoch).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(OutboxError::Db(format!(
            "device diary migration registry {actual:?} does not cover {expected:?}"
        )))
    }
}

fn validate_ledger(applied: &[u16]) -> Result<(), OutboxError> {
    if let Some(epoch) = applied
        .iter()
        .copied()
        .find(|epoch| *epoch > CURRENT_SCHEMA_EPOCH)
    {
        return Err(OutboxError::Db(format!(
            "device diary schema epoch {epoch} is newer than this worker's {CURRENT_SCHEMA_EPOCH}"
        )));
    }
    let expected: Vec<u16> = (1..=applied.last().copied().unwrap_or(0)).collect();
    if applied == expected {
        Ok(())
    } else {
        Err(OutboxError::Db(format!(
            "device diary migration ledger has a gap: {applied:?}"
        )))
    }
}

fn current_schema() -> Result<&'static str, OutboxError> {
    validate_registry()?;
    MIGRATIONS
        .last()
        .map(|migration| migration.schema_sql)
        .ok_or_else(|| OutboxError::Db("device diary migration registry is empty".to_string()))
}

async fn apply_one(db: &Db, migration: &Migration) -> Result<(), OutboxError> {
    let query = format!(
        "BEGIN TRANSACTION;\n{}\n{}\n\
         UPSERT type::record('diary_schema_migrations', $migration_id) \
             CONTENT {{ epoch: $migration_epoch }} RETURN NONE;\n\
         COMMIT TRANSACTION;",
        migration.schema_sql, migration.data_sql
    );
    db.query(query)
        .bind(("migration_id", format!("{:04}", migration.epoch)))
        .bind(("migration_epoch", i64::from(migration.epoch)))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    Ok(())
}

async fn applied_epochs(db: &Db) -> Result<Vec<u16>, OutboxError> {
    let mut response = db
        .query("SELECT VALUE epoch FROM diary_schema_migrations ORDER BY epoch ASC")
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

fn db_error(error: surrealdb::Error) -> OutboxError {
    OutboxError::Db(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;
    use surrealdb::engine::any;
    use surrealdb::types::SurrealValue;

    use super::*;

    async fn db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("diary").use_db("diary").await.unwrap();
        db
    }

    #[tokio::test]
    async fn migrations_are_idempotent_and_reconcile_the_current_schema() {
        let db = db().await;
        apply(&db).await.unwrap();
        apply(&db).await.unwrap();
        assert_eq!(applied_epochs(&db).await.unwrap(), [1]);
    }

    #[tokio::test]
    async fn first_migration_preserves_rows_and_retires_review_metadata_once() {
        #[derive(Deserialize, SurrealValue)]
        struct TableInfo {
            fields: BTreeMap<String, String>,
        }

        let db = db().await;
        db.query(
            "DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
             DEFINE FIELD written_at ON diary_entries TYPE int;
             DEFINE FIELD body ON diary_entries TYPE string;
             DEFINE FIELD entry_version ON diary_entries TYPE option<int>;
             DEFINE FIELD state ON diary_entries TYPE string;
             DEFINE FIELD reason ON diary_entries TYPE option<string>;
             DEFINE FIELD enqueued_at ON diary_entries TYPE int;
             DEFINE FIELD write_fingerprint ON diary_entries TYPE option<string>;
             CREATE ONLY diary_entries:legacy SET
                 written_at = 1,
                 body = 'preserved',
                 entry_version = 1,
                 state = 'pending',
                 enqueued_at = 7;",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        apply(&db).await.unwrap();

        let mut response = db
            .query("SELECT VALUE body FROM diary_entries:legacy; INFO FOR TABLE diary_entries")
            .await
            .unwrap()
            .check()
            .unwrap();
        let bodies: Vec<String> = response.take(0).unwrap();
        let info: Option<TableInfo> = response.take(1).unwrap();
        assert_eq!(bodies, ["preserved"]);
        assert!(!info.unwrap().fields.contains_key("entry_version"));
    }

    #[tokio::test]
    async fn cached_worker_refuses_a_device_store_migrated_by_a_newer_epoch() {
        let db = db().await;
        apply(&db).await.unwrap();
        db.query(
            "UPSERT type::record('diary_schema_migrations', $id)
             CONTENT { epoch: $epoch } RETURN NONE",
        )
        .bind(("id", format!("{:04}", CURRENT_SCHEMA_EPOCH + 1)))
        .bind(("epoch", i64::from(CURRENT_SCHEMA_EPOCH + 1)))
        .await
        .unwrap()
        .check()
        .unwrap();

        let error = require_current(&db).await.unwrap_err().to_string();
        assert!(error.contains("newer than this worker"), "{error}");
    }
}
