//! Ordered, forward-only migrations for the diary's server store.
//!
//! Unlike the site's additive schema reconciliation, diary changes share an
//! offline client and therefore need an exact activation point. Each applied
//! epoch is an immutable ledger row. A binary older than the database refuses
//! the handle instead of reapplying an obsolete table permission.

use diary_core::contract::CURRENT_SCHEMA_EPOCH;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use super::Db;

struct Migration {
    epoch: u16,
    sql: &'static str,
    expected_schema: SchemaExpectation,
}

const MIGRATIONS: &[Migration] = &[Migration {
    epoch: 1,
    sql: include_str!("diary_migrations/0001_current_entry_store.surql"),
    expected_schema: SchemaExpectation {
        table: "DEFINE TABLE diary_entries TYPE NORMAL SCHEMAFULL PERMISSIONS FOR select, create WHERE $access = 'diary_sync' AND ($token.diary_schema_epoch = 1 OR $token.diary_schema_epoch = NONE), FOR update, delete NONE",
        fields: &[
            "DEFINE FIELD body ON diary_entries TYPE string ASSERT string::len($value) >= 1 AND string::len($value) <= 65536 PERMISSIONS FULL",
            "DEFINE FIELD id ON diary_entries TYPE string ASSERT string::matches(record::id($value), '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$') PERMISSIONS FULL",
            "DEFINE FIELD written_at ON diary_entries TYPE int ASSERT $value >= 0 AND $value <= 253402300799 PERMISSIONS FULL",
        ],
        indexes: &["DEFINE INDEX diary_entries_written_at ON diary_entries FIELDS written_at"],
    },
}];

struct SchemaExpectation {
    /// SurrealDB 3.2.3's canonical `INFO FOR DATABASE` definition.
    table: &'static str,
    /// Canonical `INFO FOR TABLE` definitions, ordered by schema name.
    fields: &'static [&'static str],
    indexes: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaSnapshot {
    table: String,
    fields: Vec<String>,
    indexes: Vec<String>,
    events: Vec<String>,
    views: Vec<String>,
}

#[derive(Deserialize, SurrealValue)]
struct DatabaseInfo {
    tables: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, SurrealValue)]
struct TableInfo {
    events: std::collections::BTreeMap<String, String>,
    fields: std::collections::BTreeMap<String, String>,
    indexes: std::collections::BTreeMap<String, String>,
    tables: std::collections::BTreeMap<String, String>,
}

const LEDGER_SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_schema_migrations SCHEMAFULL PERMISSIONS NONE;
    DEFINE FIELD OVERWRITE epoch ON diary_schema_migrations TYPE int
        ASSERT $value >= 1 AND $value <= 65535;
    DEFINE INDEX OVERWRITE diary_schema_migrations_epoch
        ON diary_schema_migrations FIELDS epoch UNIQUE;";

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
                "diary migration epoch {} lost a concurrent activation race",
                migration.epoch
            ));
        }
    }

    require_current(db).await
}

/// Activation check for an already-cached server handle. Every diary adapter
/// calls this, so a process from the preceding rollout stops serving diary
/// data as soon as a newer process advances the shared ledger. The schema
/// snapshot also repairs an old global-schema overwrite without mutating a
/// database whose ledger has already advanced beyond this binary.
pub(super) async fn require_current(db: &Db) -> Result<(), String> {
    validate_registry()?;
    let applied = applied_epochs(db).await?;
    validate_ledger(&applied)?;
    if applied.last().copied() != Some(CURRENT_SCHEMA_EPOCH) {
        return Err(format!(
            "diary schema stopped at epoch {:?}, expected {CURRENT_SCHEMA_EPOCH}",
            applied.last()
        ));
    }

    let current = current_migration()?;
    let actual = schema_snapshot(db).await?;
    if schema_matches(current, &actual) {
        return Ok(());
    }

    reconcile_current(db).await?;
    let applied = applied_epochs(db).await?;
    validate_ledger(&applied)?;
    if applied.last().copied() != Some(CURRENT_SCHEMA_EPOCH) {
        return Err(format!(
            "diary schema advanced to {:?} while epoch {CURRENT_SCHEMA_EPOCH} was reconciling",
            applied.last()
        ));
    }
    let actual = schema_snapshot(db).await?;
    if schema_matches(current, &actual) {
        Ok(())
    } else {
        Err(format!(
            "diary schema definitions do not match epoch {CURRENT_SCHEMA_EPOCH}: {actual:?}"
        ))
    }
}

async fn apply_one(db: &Db, migration: &Migration) -> Result<(), String> {
    let prior: Vec<i64> = (1..migration.epoch).map(i64::from).collect();
    let statement = format!(
        "BEGIN TRANSACTION;
         LET $applied = SELECT VALUE epoch FROM diary_schema_migrations ORDER BY epoch ASC;
         IF $applied = $prior {{
             {}
             CREATE ONLY type::record('diary_schema_migrations', $id)
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

async fn reconcile_current(db: &Db) -> Result<(), String> {
    let current = current_migration()?;
    let expected: Vec<i64> = (1..=current.epoch).map(i64::from).collect();
    let statement = format!(
        "BEGIN TRANSACTION;
         LET $applied = SELECT VALUE epoch FROM diary_schema_migrations ORDER BY epoch ASC;
         IF $applied = $expected {{
             {}
         }};
         COMMIT TRANSACTION;",
        current.sql
    );
    db.query(statement)
        .bind(("expected", expected))
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;
    Ok(())
}

fn current_migration() -> Result<&'static Migration, String> {
    MIGRATIONS
        .last()
        .ok_or_else(|| "diary migration registry is empty".to_string())
}

fn schema_matches(migration: &Migration, actual: &SchemaSnapshot) -> bool {
    actual.table == migration.expected_schema.table
        && actual.fields.iter().map(String::as_str).eq(migration
            .expected_schema
            .fields
            .iter()
            .copied())
        && actual.indexes.iter().map(String::as_str).eq(migration
            .expected_schema
            .indexes
            .iter()
            .copied())
        && actual.events.is_empty()
        && actual.views.is_empty()
}

async fn schema_snapshot(db: &Db) -> Result<SchemaSnapshot, String> {
    let mut response = db
        .query("INFO FOR DB; INFO FOR TABLE diary_entries")
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;
    let database: Option<DatabaseInfo> = response.take(0).map_err(migration_error)?;
    let table: Option<TableInfo> = response.take(1).map_err(migration_error)?;
    let database = database.ok_or_else(|| "diary database schema info was empty".to_string())?;
    let table = table.ok_or_else(|| "diary table schema info was empty".to_string())?;
    let definition = database
        .tables
        .get("diary_entries")
        .cloned()
        .ok_or_else(|| "diary_entries is not defined".to_string())?;
    Ok(SchemaSnapshot {
        table: definition,
        fields: table.fields.into_values().collect(),
        indexes: table.indexes.into_values().collect(),
        events: table.events.into_values().collect(),
        views: table.tables.into_values().collect(),
    })
}

fn validate_registry() -> Result<(), String> {
    let expected: Vec<u16> = (1..=CURRENT_SCHEMA_EPOCH).collect();
    let actual: Vec<u16> = MIGRATIONS.iter().map(|migration| migration.epoch).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "diary migration registry {actual:?} does not cover {expected:?}"
        ))
    }
}

fn validate_ledger(applied: &[u16]) -> Result<(), String> {
    if let Some(epoch) = applied
        .iter()
        .copied()
        .find(|epoch| *epoch > CURRENT_SCHEMA_EPOCH)
    {
        return Err(format!(
            "database diary schema epoch {epoch} is newer than this binary's {CURRENT_SCHEMA_EPOCH}"
        ));
    }
    let expected: Vec<u16> = (1..=applied.last().copied().unwrap_or(0)).collect();
    if applied == expected {
        Ok(())
    } else {
        Err(format!("diary migration ledger has a gap: {applied:?}"))
    }
}

async fn applied_epochs(db: &Db) -> Result<Vec<u16>, String> {
    let mut response = db
        .query("SELECT VALUE epoch FROM diary_schema_migrations ORDER BY epoch ASC")
        .await
        .map_err(migration_error)?
        .check()
        .map_err(migration_error)?;
    response.take(0).map_err(migration_error)
}

#[cfg(test)]
async fn record_applied(db: &Db, epoch: u16) -> Result<(), String> {
    db.query(
        "UPSERT type::record('diary_schema_migrations', $id)
         CONTENT { epoch: $epoch } RETURN NONE",
    )
    .bind(("id", format!("{epoch:04}")))
    .bind(("epoch", i64::from(epoch)))
    .await
    .map_err(migration_error)?
    .check()
    .map_err(migration_error)?;
    Ok(())
}

fn migration_error(error: surrealdb::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::any;

    const TEST_ACCESS_KEY: &str = "diary-schema-epoch-test-key-that-is-deliberately-long-for-hs512";
    const OLD_GLOBAL_DIARY_SCHEMA: &str = "
        DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL
            PERMISSIONS
            FOR select WHERE $access = 'diary_sync'
                AND (entry_version = NONE
                    OR $token.diary_wire_version >= entry_version)
            FOR create WHERE $access = 'diary_sync'
                AND (($token.diary_wire_version = NONE
                        AND entry_version = NONE)
                    OR ($token.diary_wire_version IS NOT NONE
                        AND entry_version IS NOT NONE
                        AND $token.diary_wire_version = entry_version));
        DEFINE FIELD OVERWRITE id ON diary_entries TYPE string
            ASSERT string::matches(record::id($value),
                '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
        DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int
            ASSERT $value >= 0 AND $value <= 253402300799;
        DEFINE FIELD OVERWRITE body ON diary_entries TYPE string
            ASSERT string::len($value) >= 1 AND string::len($value) <= 65536;
        DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int>
            ASSERT $value IS NONE OR ($value >= 1 AND $value <= 65535);
        DEFINE INDEX OVERWRITE diary_entries_written_at
            ON diary_entries FIELDS written_at;";

    async fn db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("site").use_db("site").await.unwrap();
        db
    }

    fn direct_token(epoch: Option<u16>) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut claims = serde_json::json!({
            "ns": "site",
            "db": "site",
            "ac": diary_core::contract::DIRECT_ACCESS,
            "id": "diary_device:admin",
            "iat": now,
            "exp": now + 300,
        });
        if let Some(epoch) = epoch {
            claims["diary_schema_epoch"] = serde_json::json!(epoch);
        }
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS512),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_ACCESS_KEY.as_bytes()),
        )
        .unwrap()
    }

    async fn define_test_access(db: &Db) {
        db.query(
            "DEFINE ACCESS OVERWRITE diary_sync ON DATABASE TYPE RECORD
             WITH JWT ALGORITHM HS512 KEY $key DURATION FOR SESSION 15m",
        )
        .bind(("key", TEST_ACCESS_KEY.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    }

    async fn direct_session(db: &Db, epoch: Option<u16>) -> Db {
        let session = db.clone();
        session.authenticate(direct_token(epoch)).await.unwrap();
        session
    }

    async fn create_as(db: &Db, epoch: Option<u16>, id: &str) -> Option<String> {
        let session = direct_session(db, epoch).await;
        let mut response = session
            .query(
                "CREATE ONLY type::record('diary_entries', $id)
                 SET written_at = $written_at, body = $body
                 RETURN VALUE record::id(id)",
            )
            .bind(("id", id.to_string()))
            .bind(("written_at", 1_i64))
            .bind(("body", id.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        response.take(0).unwrap()
    }

    async fn ids_as(db: &Db, epoch: Option<u16>) -> Vec<String> {
        let session = direct_session(db, epoch).await;
        let mut response = session
            .query("SELECT VALUE record::id(id) FROM diary_entries ORDER BY written_at ASC")
            .await
            .unwrap()
            .check()
            .unwrap();
        response.take(0).unwrap()
    }

    fn registry_epochs() -> Vec<u16> {
        MIGRATIONS.iter().map(|migration| migration.epoch).collect()
    }

    #[tokio::test]
    async fn migration_preserves_existing_entries_and_is_idempotent() {
        let db = db().await;
        db.query(
            "DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
             DEFINE FIELD written_at ON diary_entries TYPE int;
             DEFINE FIELD body ON diary_entries TYPE string;
             CREATE ONLY diary_entries:legacy
                 SET written_at = 1, body = 'preserved';",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        apply(&db).await.unwrap();
        apply(&db).await.unwrap();
        assert_eq!(applied_epochs(&db).await.unwrap(), registry_epochs());

        let mut response = db
            .query("SELECT VALUE body FROM diary_entries:legacy")
            .await
            .unwrap()
            .check()
            .unwrap();
        let bodies: Vec<String> = response.take(0).unwrap();
        assert_eq!(bodies, ["preserved"]);
    }

    #[tokio::test]
    async fn older_binary_refuses_a_newer_ledger() {
        let db = db().await;
        db.query(LEDGER_SCHEMA).await.unwrap().check().unwrap();
        for epoch in 1..=CURRENT_SCHEMA_EPOCH + 1 {
            record_applied(&db, epoch).await.unwrap();
        }
        let error = apply(&db).await.unwrap_err();
        assert!(error.contains("newer than this binary"), "{error}");
    }

    #[tokio::test]
    async fn direct_permissions_accept_current_and_claimless_but_filter_future_sessions() {
        let db = db().await;
        apply(&db).await.unwrap();
        define_test_access(&db).await;

        let current_id = "2026-08-08T10-00-00-04-00";
        let future_id = "2026-08-08T10-00-01-04-00";
        let claimless_id = "2026-08-08T10-00-02-04-00";
        assert!(
            create_as(&db, Some(CURRENT_SCHEMA_EPOCH), current_id)
                .await
                .is_some()
        );
        assert!(
            create_as(&db, Some(CURRENT_SCHEMA_EPOCH + 1), future_id)
                .await
                .is_none()
        );
        assert!(create_as(&db, None, claimless_id).await.is_some());

        assert_eq!(
            ids_as(&db, Some(CURRENT_SCHEMA_EPOCH)).await,
            [current_id, claimless_id]
        );
        assert!(ids_as(&db, Some(CURRENT_SCHEMA_EPOCH + 1)).await.is_empty());
        assert_eq!(ids_as(&db, None).await, [current_id, claimless_id]);
    }

    #[tokio::test]
    async fn permission_filtered_create_is_empty_and_never_acknowledged() {
        let db = db().await;
        apply(&db).await.unwrap();
        define_test_access(&db).await;
        let future = direct_session(&db, Some(CURRENT_SCHEMA_EPOCH + 1)).await;
        let id = "2026-08-08T10-01-00-04-00";
        let entry = diary_core::entry::DiaryEntry::from_parts(id.to_string(), 1, "not persisted");

        let error = diary_core::store::insert_entry(&future, &entry)
            .await
            .unwrap_err();
        assert!(error.contains("create returned no matching id"), "{error}");
        let mut response = db
            .query("SELECT VALUE record::id(id) FROM diary_entries")
            .await
            .unwrap()
            .check()
            .unwrap();
        let ids: Vec<String> = response.take(0).unwrap();
        assert!(
            ids.is_empty(),
            "a filtered CREATE was acknowledged: {ids:?}"
        );
    }

    #[tokio::test]
    async fn cached_handle_repairs_an_old_global_schema_overwrite() {
        let db = db().await;
        apply(&db).await.unwrap();
        define_test_access(&db).await;

        db.query(OLD_GLOBAL_DIARY_SCHEMA)
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(!schema_matches(
            current_migration().unwrap(),
            &schema_snapshot(&db).await.unwrap()
        ));
        // The predecessor permission reads the absent old claim as a
        // claimless bridge, so even a future schema epoch can write.
        assert!(
            create_as(
                &db,
                Some(CURRENT_SCHEMA_EPOCH + 1),
                "2026-08-08T10-02-00-04-00"
            )
            .await
            .is_some()
        );

        require_current(&db).await.unwrap();
        assert!(schema_matches(
            current_migration().unwrap(),
            &schema_snapshot(&db).await.unwrap()
        ));
        assert!(
            create_as(&db, Some(CURRENT_SCHEMA_EPOCH), "2026-08-08T10-02-01-04-00")
                .await
                .is_some()
        );
        assert!(
            create_as(
                &db,
                Some(CURRENT_SCHEMA_EPOCH + 1),
                "2026-08-08T10-02-02-04-00"
            )
            .await
            .is_none()
        );
    }

    #[tokio::test]
    async fn reconciliation_does_not_downgrade_a_newer_ledger() {
        let db = db().await;
        apply(&db).await.unwrap();
        record_applied(&db, CURRENT_SCHEMA_EPOCH + 1).await.unwrap();
        db.query("DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL PERMISSIONS NONE")
            .await
            .unwrap()
            .check()
            .unwrap();
        let before = schema_snapshot(&db).await.unwrap();

        reconcile_current(&db).await.unwrap();

        assert_eq!(schema_snapshot(&db).await.unwrap(), before);
    }

    #[test]
    fn current_migration_owns_the_server_entry_limit() {
        let sql = current_migration().unwrap().sql;
        assert!(
            sql.contains(&format!(
                "string::len($value) <= {}",
                diary_core::entry::MAX_ENTRY_CHARS
            )),
            "current migration no longer mirrors diary-core's entry limit"
        );
    }
}
