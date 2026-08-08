//! The diary entry store: model, key projection, and queries, written against
//! the crate's one [`Db`] handle so the server's real database, the native
//! tests' `mem://`, and the device's `indxdb://` all run identical code.
//! Moved from `src/app/diary.rs`, where it was written against the server
//! handle alone; the server now calls these with `data.db().await?`.
//!
//! The query shapes follow docs/surrealdb-notes.md: explicit projections
//! (`SELECT *` omits `option` fields holding `NONE` — none here yet, but the
//! local store grows them), string keys via `record::id(id)`, keys returned
//! from creates via `RETURN VALUE record::id(id)`, one `=` per delete.

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use surrealdb::types::{SurrealValue, Value};

use crate::Db;
use crate::contract::CURRENT_WIRE_VERSION;
use crate::eastern;
use crate::entry::{ComposedEntry, EntryRejection, PROJECTION, accept_for_save};
use crate::placement::{self, Occupant, Placement};

pub use crate::entry::DiaryEntry;

/// Transcript page size — the server page and the worker's offline SSR agree
/// through this constant.
pub const PAGE_SIZE: usize = 20;
/// Far past any real diary and far under the store's signed-64-bit `START`
/// limit. Wilder page numbers behave like unparseable ones instead of
/// surfacing a SurrealDB parse error as a fake outage.
pub const MAX_PAGE: usize = 1_000_000;

/// The generation-agnostic permission installed by every server release.
/// Keeping the expression stable is what makes schema bootstrap monotonic:
/// an older instance may reapply it, but each row still fences itself from a
/// token whose entry model is too old to interpret it. The claimless branch
/// is only for an already-minted predecessor token during the first rollout:
/// it may create legacy rows, but it cannot forge a versioned row.
pub const DIRECT_SYNC_ROW_PERMISSIONS: &str = concat!(
    "PERMISSIONS\n",
    "    FOR select WHERE $access = 'diary_sync'\n",
    "        AND (entry_version = NONE\n",
    "            OR $token.diary_wire_version >= entry_version)\n",
    "    FOR create WHERE $access = 'diary_sync'\n",
    "        AND (($token.diary_wire_version = NONE\n",
    "                AND entry_version = NONE)\n",
    "            OR ($token.diary_wire_version IS NOT NONE\n",
    "                AND entry_version IS NOT NONE\n",
    "                AND $token.diary_wire_version = entry_version))",
);

/// Test stores share one server-row fixture so a new business field has one
/// test-schema seam rather than separate store and sync copies.
#[cfg(test)]
pub(crate) const TEST_SCHEMA: &str = "\
    DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
    DEFINE FIELD id ON diary_entries TYPE string;
    DEFINE FIELD written_at ON diary_entries TYPE int;
    DEFINE FIELD body ON diary_entries TYPE string;
    DEFINE FIELD entry_version ON diary_entries TYPE option<int>;";

/// Store metadata around the canonical value, not another business-field
/// representation. A future `EntryContent` field remains flattened into this
/// wrapper automatically.
#[derive(Clone, Debug, Serialize, SurrealValue)]
struct PersistedEntry {
    #[serde(flatten)]
    #[surreal(flatten)]
    composed: ComposedEntry,
    entry_version: i64,
}

impl PersistedEntry {
    fn current(composed: ComposedEntry) -> Self {
        Self {
            composed,
            entry_version: i64::from(CURRENT_WIRE_VERSION),
        }
    }
}

/// The saved-or-deduped outcome [`save_entry`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedWrite {
    pub entry: DiaryEntry,
    pub deduped: bool,
}

impl SavedWrite {
    fn new(entry: DiaryEntry, deduped: bool) -> Self {
        Self { entry, deduped }
    }
}

impl Deref for SavedWrite {
    type Target = DiaryEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

pub enum SaveError {
    /// The remote's shared acceptance policy rejected this composed value.
    Rejected(EntryRejection),
    /// Every probed second held a different entry — give up rather than
    /// stamp the entry minutes away from its composition time.
    Exhausted,
    Store(String),
}

/// The record key a UTC epoch second projects to: the Eastern public path
/// that becomes the permalink. `None` only for epochs outside jiff's
/// representable range.
pub fn entry_key(epoch: i64) -> Option<String> {
    let utc = jiff::Timestamp::from_second(epoch)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();
    let instant = eastern::eastern_instant(&utc, 0).ok()?;
    Some(eastern::public_path(&instant))
}

/// One page of entries, newest first, plus the total count — one round trip.
pub async fn entry_page(db: &Db, page_number: usize) -> Result<(Vec<DiaryEntry>, usize), String> {
    entry_page_for_version(db, page_number, CURRENT_WIRE_VERSION).await
}

async fn entry_page_for_version(
    db: &Db,
    page_number: usize,
    max_entry_version: u16,
) -> Result<(Vec<DiaryEntry>, usize), String> {
    #[derive(Deserialize, SurrealValue)]
    struct CountRow {
        count: i64,
    }

    let start = page_number.saturating_sub(1).saturating_mul(PAGE_SIZE);
    // LIMIT/START are server-computed integers, formatted rather than bound
    // to keep the statement inside plainly supported syntax. The id
    // tie-break is deterministic: same-table keys compare as strings, and
    // the DST-fold pair (…-04-00 before …-05-00) even sorts chronologically.
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION} FROM diary_entries \
                 WHERE entry_version = NONE OR entry_version <= $max_entry_version \
                 ORDER BY written_at DESC, id DESC LIMIT {PAGE_SIZE} START {start};
             SELECT count() FROM diary_entries \
                 WHERE entry_version = NONE OR entry_version <= $max_entry_version \
                 GROUP ALL;"
        ))
        .bind(("max_entry_version", i64::from(max_entry_version)))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let entries: Vec<DiaryEntry> = response.take(0).map_err(|error| error.to_string())?;
    let counts: Vec<CountRow> = response.take(1).map_err(|error| error.to_string())?;
    let total = counts
        .into_iter()
        .next()
        .map(|row| row.count.max(0) as usize)
        .unwrap_or(0);
    Ok((entries, total))
}

/// Every entry, oldest first — the pull snapshot's source. (On a local
/// store the extra state columns are simply not projected.)
pub async fn all_entries(db: &Db) -> Result<Vec<DiaryEntry>, String> {
    all_entries_for_version(db, CURRENT_WIRE_VERSION).await
}

async fn all_entries_for_version(
    db: &Db,
    max_entry_version: u16,
) -> Result<Vec<DiaryEntry>, String> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION} FROM diary_entries \
             WHERE entry_version = NONE OR entry_version <= $max_entry_version \
             ORDER BY written_at ASC, id ASC"
        ))
        .bind(("max_entry_version", i64::from(max_entry_version)))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    response.take(0).map_err(|error| error.to_string())
}

pub async fn entry_by_id(db: &Db, id: &str) -> Result<Option<DiaryEntry>, String> {
    entry_by_id_for_version(db, id, CURRENT_WIRE_VERSION).await
}

async fn entry_by_id_for_version(
    db: &Db,
    id: &str,
    max_entry_version: u16,
) -> Result<Option<DiaryEntry>, String> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION} FROM type::record('diary_entries', $id) \
             WHERE entry_version = NONE OR entry_version <= $max_entry_version"
        ))
        .bind(("id", id.to_string()))
        .bind(("max_entry_version", i64::from(max_entry_version)))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let entries: Vec<DiaryEntry> = response.take(0).map_err(|error| error.to_string())?;
    Ok(entries.into_iter().next())
}

/// CREATE, not UPSERT: two entries in the same second would collide on the
/// key, and overwriting a diary entry is worse than asking to resubmit. The
/// canonical composed value is the whole row content; future optional fields
/// do not create another SET/bind branch here.
pub async fn insert_entry(db: &Db, entry: &DiaryEntry) -> Result<(), String> {
    let persisted = PersistedEntry::current(entry.composed.clone());
    let mut response = db
        .query(
            "CREATE ONLY type::record('diary_entries', $id) CONTENT $entry \
             RETURN VALUE record::id(id)",
        )
        .bind(("id", entry.id.clone()))
        .bind(("entry", persisted))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let created: Option<String> = response.take(0).map_err(|error| error.to_string())?;
    if created.as_deref() == Some(entry.id.as_str()) {
        Ok(())
    } else {
        // Record permissions filter denied CREATEs to an empty successful
        // result. Treat that as a failed write: direct sync must never
        // acknowledge content the database did not actually persist.
        Err("diary entry create returned no matching id".to_string())
    }
}

/// Idempotent: deleting an already-deleted entry succeeds quietly.
pub async fn remove_entry(db: &Db, id: &str) -> Result<(), String> {
    db.query("DELETE type::record('diary_entries', $id)")
        .bind(("id", id.to_string()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Replay-safe insert. The id derives from the entry's composition epoch; a
/// collision holding the SAME Entry Content is a replay of a write whose
/// response was lost, so it counts as saved. Different content probes
/// forward one second at a time, re-running the dedupe check at EVERY probed id — the killer
/// replay is "bumped to T+1, response lost, retried from T", which must land
/// on the T+1 dedupe, not insert again at T+2. Check-first, then CREATE,
/// then re-check when CREATE fails: a lost race is indistinguishable from a
/// replayed twin, and neither is an error (never sniff the store's error
/// strings). Never overwrites.
///
/// Invariant: the stored `written_at` is always the epoch its id projects
/// from, so `ORDER BY written_at DESC, id DESC` and the permalink agree
/// about when an entry happened.
pub async fn save_entry(
    db: &Db,
    entry: ComposedEntry,
    validation_now: i64,
) -> Result<SavedWrite, SaveError> {
    let entry = accept_for_save(entry, validation_now).map_err(SaveError::Rejected)?;
    let placed = placement::place(
        &entry,
        |epoch| {
            entry_key(epoch).ok_or_else(|| SaveError::Store(format!("unprojectable epoch {epoch}")))
        },
        |id| async move {
            entry_by_id(db, &id)
                .await
                .map(|entry| entry.map(Occupant::Known))
                .map_err(SaveError::Store)
        },
        |candidate| async move { insert_entry(db, &candidate).await.map_err(SaveError::Store) },
    )
    .await?;
    match placed {
        Placement::Placed(entry) => Ok(SavedWrite::new(entry, false)),
        Placement::Deduped(entry) => Ok(SavedWrite::new(entry, true)),
        Placement::Exhausted => Err(SaveError::Exhausted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::COLLISION_PROBES;

    const TEST_ACCESS_KEY: &str =
        "diary-rollout-test-key-that-is-deliberately-long-enough-for-hs512";

    #[derive(Clone, Copy)]
    enum TestTokenGeneration {
        Omitted,
        Null,
        Version(u16),
    }

    /// Same `mem://` isomorphism proof the outbox tests run: these functions
    /// execute against the identical handle type the server and the device
    /// use, so passing here certifies the shared behavior, not a test double.
    /// The table shape mirrors the committed server schema (a fresh store has
    /// no tables, and 3.2 errors on queries against undefined ones); the
    /// ASSERTs stay in `src/schema.surql` — prod's job, not this contract's.
    async fn store() -> Db {
        let db = surrealdb::engine::any::connect("mem://")
            .await
            .expect("mem engine opens");
        db.use_ns("diary").use_db("diary").await.expect("ns");
        db.query(TEST_SCHEMA)
            .await
            .expect("schema applies")
            .check()
            .expect("schema statements succeed");
        db
    }

    /// Unwrap a save, surfacing the store's actual error text on failure.
    fn saved(result: Result<SavedWrite, SaveError>) -> SavedWrite {
        match result {
            Ok(write) => write,
            Err(SaveError::Rejected(rejection)) => panic!("save rejected: {rejection:?}"),
            Err(SaveError::Exhausted) => panic!("save exhausted its probes"),
            Err(SaveError::Store(error)) => panic!("store failed: {error}"),
        }
    }

    async fn save_at(db: &Db, written_at: i64, body: &str) -> Result<SavedWrite, SaveError> {
        save_entry(db, ComposedEntry::new(written_at, body), written_at).await
    }

    async fn apply_generation_bootstrap(db: &Db, include_future_field: bool) {
        let future_field = if include_future_field {
            "DEFINE FIELD OVERWRITE future_field ON diary_entries TYPE option<string>;"
        } else {
            ""
        };
        db.query(format!(
            "DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL \
                 {DIRECT_SYNC_ROW_PERMISSIONS};
             DEFINE FIELD OVERWRITE id ON diary_entries TYPE string;
             DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int;
             DEFINE FIELD OVERWRITE body ON diary_entries TYPE string;
             DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int>
                 ASSERT $value IS NONE OR ($value >= 1 AND $value <= 65535);
             {future_field}"
        ))
        .await
        .expect("generation bootstrap applies")
        .check()
        .expect("generation bootstrap statements succeed");
    }

    fn direct_test_token(generation: TestTokenGeneration) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock is after the epoch")
            .as_secs();
        let mut claims = serde_json::json!({
            "ns": "diary",
            "db": "diary",
            "ac": "diary_sync",
            "id": "diary_device:admin",
            "iat": now,
            "exp": now + 300,
        });
        match generation {
            TestTokenGeneration::Omitted => {}
            TestTokenGeneration::Null => {
                claims["diary_wire_version"] = serde_json::Value::Null;
            }
            TestTokenGeneration::Version(version) => {
                claims["diary_wire_version"] = serde_json::json!(version);
            }
        }
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS512),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_ACCESS_KEY.as_bytes()),
        )
        .expect("test token encodes")
    }

    async fn authenticate_as(db: &Db, generation: TestTokenGeneration) {
        db.authenticate(direct_test_token(generation))
            .await
            .expect("test access token authenticates");
    }

    async fn permission_create(
        db: &Db,
        epoch: i64,
        body: &str,
        entry_version: Option<u16>,
    ) -> Option<String> {
        let id = entry_key(epoch).unwrap();
        let query = match entry_version {
            Some(entry_version) => db
                .query(
                    "CREATE ONLY type::record('diary_entries', $id)
                     SET written_at = $written_at,
                         body = $body,
                         entry_version = $entry_version
                     RETURN VALUE record::id(id)",
                )
                .bind(("entry_version", i64::from(entry_version))),
            None => db.query(
                "CREATE ONLY type::record('diary_entries', $id)
                     SET written_at = $written_at,
                         body = $body
                     RETURN VALUE record::id(id)",
            ),
        };
        let mut response = query
            .bind(("id", id))
            .bind(("written_at", epoch))
            .bind(("body", body.to_string()))
            .await
            .expect("permission test query runs")
            .check()
            .expect("permission filtering is a successful empty result");
        response.take(0).expect("create result decodes")
    }

    async fn permission_visible_ids(db: &Db) -> Vec<String> {
        let mut response = db
            .query(
                "SELECT VALUE record::id(id) FROM diary_entries
                     ORDER BY written_at ASC, id ASC",
            )
            .await
            .expect("permission test select runs")
            .check()
            .expect("permission test select succeeds");
        response.take(0).expect("visible ids decode")
    }

    #[tokio::test]
    async fn save_rejects_before_touching_the_store() {
        use crate::entry::{EntryRejection, MAX_ENTRY_CHARS};

        let db = store().await;
        let result = save_entry(
            &db,
            ComposedEntry::new(100, "x".repeat(MAX_ENTRY_CHARS + 1)),
            100,
        )
        .await;
        assert!(matches!(
            result,
            Err(SaveError::Rejected(EntryRejection::BodyTooLong))
        ));
        assert!(all_entries(&db).await.unwrap().is_empty());
    }

    #[test]
    fn entry_keys_project_from_client_epochs() {
        let epoch = "2026-07-27T18:30:45Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        assert_eq!(
            entry_key(epoch).as_deref(),
            Some("2026-07-27T14-30-45-04-00")
        );
        // Same wall clock on both sides of the November fold: the embedded
        // offset digits keep the keys distinct and parseable.
        let edt = "2026-11-01T05:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        let est = "2026-11-01T06:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        assert_eq!(entry_key(edt).as_deref(), Some("2026-11-01T01-30-00-04-00"));
        assert_eq!(entry_key(est).as_deref(), Some("2026-11-01T01-30-00-05-00"));
        assert!(entry_key(i64::MIN).is_none());
    }

    #[test]
    fn collision_probes_stay_valid_across_the_fold() {
        // Probing +1s across the fall-back instant keeps producing distinct,
        // parseable keys. (They are NOT string-monotonic across the fold —
        // fine: reads order by written_at first; ids only tie-break.)
        let base = "2026-11-01T05:59:58Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        let mut seen = std::collections::BTreeSet::new();
        for offset in 0..COLLISION_PROBES {
            let id = entry_key(base + offset).expect("probe projects");
            assert!(eastern::parse_public_path(&id).is_some(), "bad probe {id}");
            assert!(seen.insert(id), "duplicate probe key");
        }
    }

    #[tokio::test]
    async fn typed_entry_content_round_trips_through_content_write() {
        #[derive(Deserialize, SurrealValue)]
        struct GenerationRow {
            entry_version: i64,
        }

        let db = store().await;
        let entry = DiaryEntry::from_parts(entry_key(100).unwrap(), 100, "typed row");
        insert_entry(&db, &entry).await.unwrap();
        assert_eq!(entry_by_id(&db, &entry.id).await.unwrap(), Some(entry));
        let mut response = db
            .query("SELECT entry_version FROM type::record('diary_entries', $id)")
            .bind(("id", entry_key(100).unwrap()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let rows: Vec<GenerationRow> = response.take(0).unwrap();
        assert_eq!(rows[0].entry_version, i64::from(CURRENT_WIRE_VERSION));
    }

    #[tokio::test]
    async fn row_generation_survives_v1_v2_v1_bootstrap_order() {
        let db = store().await;
        apply_generation_bootstrap(&db, false).await;
        db.query(
            "CREATE ONLY diary_entries:v1 \
                 SET written_at = 1, body = 'old', entry_version = 1;",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        apply_generation_bootstrap(&db, true).await;
        db.query(
            "CREATE ONLY diary_entries:v2 \
                 SET written_at = 2, body = 'new', entry_version = 2, \
                     future_field = 'preserved';",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        // A late older instance overwrites the table definition and its known
        // fields. The common permission does not downgrade, the newer field
        // remains defined, and application reads still hide the opaque row.
        apply_generation_bootstrap(&db, false).await;

        let old = all_entries_for_version(&db, 1).await.unwrap();
        assert_eq!(
            old.iter()
                .map(|entry| entry.body.as_str())
                .collect::<Vec<_>>(),
            ["old"]
        );
        assert!(
            entry_by_id_for_version(&db, "v2", 1)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(entry_page_for_version(&db, 1, 1).await.unwrap().1, 1);

        let current = all_entries_for_version(&db, 2).await.unwrap();
        assert_eq!(
            current
                .iter()
                .map(|entry| entry.body.as_str())
                .collect::<Vec<_>>(),
            ["old", "new"]
        );
        assert_eq!(entry_page_for_version(&db, 1, 2).await.unwrap().1, 2);
        let mut response = db
            .query("SELECT VALUE future_field FROM diary_entries:v2")
            .await
            .unwrap()
            .check()
            .unwrap();
        let future_fields: Vec<String> = response.take(0).unwrap();
        assert_eq!(future_fields, ["preserved"]);
    }

    #[tokio::test]
    async fn direct_row_permissions_bridge_only_the_claimless_predecessor_shape() {
        let db = store().await;
        apply_generation_bootstrap(&db, false).await;
        db.query(
            "DEFINE ACCESS OVERWRITE diary_sync ON DATABASE TYPE RECORD
                 WITH JWT ALGORITHM HS512 KEY $access_key
                 DURATION FOR SESSION 15m;",
        )
        .bind(("access_key", TEST_ACCESS_KEY.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();

        // The predecessor worker omitted the claim and emitted no row
        // generation. Its in-flight token must still persist that exact old
        // shape after the generation-aware schema lands.
        authenticate_as(&db, TestTokenGeneration::Omitted).await;
        let legacy_id = permission_create(&db, 100, "legacy", None)
            .await
            .expect("claimless predecessor create persists");
        assert_eq!(permission_visible_ids(&db).await, vec![legacy_id.clone()]);
        assert_eq!(
            permission_create(&db, 101, "claimless cannot forge v1", Some(1)).await,
            None
        );

        // Current tokens cannot down-label a row as legacy, and a v1 token
        // cannot write a v2 row.
        authenticate_as(&db, TestTokenGeneration::Version(1)).await;
        assert_eq!(
            permission_create(&db, 102, "v1 cannot omit", None).await,
            None
        );
        let v1_id = permission_create(&db, 103, "v1", Some(1))
            .await
            .expect("v1 creates exactly v1");
        assert_eq!(
            permission_create(&db, 104, "v1 cannot forge v2", Some(2)).await,
            None
        );
        assert_eq!(
            permission_visible_ids(&db).await,
            [legacy_id.clone(), v1_id.clone()]
        );

        // A newer token may read older rows, but equality on CREATE prevents
        // it from forging historical-generation content.
        authenticate_as(&db, TestTokenGeneration::Version(2)).await;
        let v2_id = permission_create(&db, 105, "v2", Some(2))
            .await
            .expect("v2 creates exactly v2");
        assert_eq!(
            permission_create(&db, 106, "v2 cannot forge v1", Some(1)).await,
            None
        );
        assert_eq!(
            permission_visible_ids(&db).await,
            [legacy_id.clone(), v1_id.clone(), v2_id]
        );

        authenticate_as(&db, TestTokenGeneration::Omitted).await;
        assert_eq!(permission_visible_ids(&db).await, vec![legacy_id.clone()]);
        authenticate_as(&db, TestTokenGeneration::Version(1)).await;
        assert_eq!(permission_visible_ids(&db).await, [legacy_id, v1_id]);

        // Permission-denied CREATEs return an empty success in SurrealDB.
        // The current insert adapter must turn that into an error rather than
        // acknowledging text that was not stored.
        authenticate_as(&db, TestTokenGeneration::Version(2)).await;
        let mismatched = DiaryEntry::from_parts(entry_key(107).unwrap(), 107, "not acknowledged");
        assert!(
            insert_entry(&db, &mismatched)
                .await
                .unwrap_err()
                .contains("no matching id")
        );

        // Only an omitted predecessor claim gets the legacy CREATE bridge;
        // explicit JSON null is a distinct value and remains denied.
        authenticate_as(&db, TestTokenGeneration::Null).await;
        assert_eq!(
            permission_create(&db, 108, "null is not omitted", None).await,
            None
        );
    }

    #[tokio::test]
    async fn save_dedupes_a_replayed_twin() {
        let db = store().await;
        let first = saved(save_at(&db, 100, "Dear diary,").await);
        assert!(!first.deduped);
        let replay = saved(save_at(&db, 100, "Dear diary,").await);
        assert!(replay.deduped);
        assert_eq!(replay.id, first.id);
        assert_eq!(replay.written_at, first.written_at);
        let (_, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn save_probes_forward_on_a_different_body() {
        let db = store().await;
        let first = saved(save_at(&db, 100, "one").await);
        let second = saved(save_at(&db, 100, "two").await);
        assert_eq!(second.written_at, 101);
        assert_ne!(second.id, first.id);
        // The stored written_at is always the epoch the id projects from.
        assert_eq!(entry_key(101).as_deref(), Some(second.id.as_str()));
    }

    #[tokio::test]
    async fn the_killer_replay_lands_on_the_bumped_dedupe() {
        let db = store().await;
        saved(save_at(&db, 100, "occupant").await);
        // Bumped to T+1, response lost…
        let bumped = saved(save_at(&db, 100, "mine").await);
        assert_eq!(bumped.written_at, 101);
        // …retried from T: must dedupe at T+1, never insert at T+2.
        let retried = saved(save_at(&db, 100, "mine").await);
        assert!(retried.deduped);
        assert_eq!(retried.id, bumped.id);
        let (_, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn save_exhausts_after_bounded_probes() {
        let db = store().await;
        for offset in 0..COLLISION_PROBES {
            let saved = save_at(&db, 100 + offset, &format!("filler {offset}"))
                .await
                .ok()
                .unwrap();
            assert_eq!(saved.written_at, 100 + offset);
        }
        assert!(matches!(
            save_at(&db, 100, "no room").await,
            Err(SaveError::Exhausted)
        ));
    }

    #[tokio::test]
    async fn pages_read_newest_first_with_totals() {
        let db = store().await;
        for (epoch, body) in [(100, "oldest"), (200, "middle"), (300, "newest")] {
            saved(save_at(&db, epoch, body).await);
        }
        let (entries, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 3);
        let bodies: Vec<&str> = entries.iter().map(|entry| entry.body.as_str()).collect();
        assert_eq!(bodies, ["newest", "middle", "oldest"]);
        let fetched = entry_by_id(&db, &entries[0].id).await.unwrap().unwrap();
        assert_eq!(fetched.body, "newest");
        remove_entry(&db, &entries[0].id).await.unwrap();
        remove_entry(&db, &entries[0].id).await.unwrap();
        let (_, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 2);
    }
}
