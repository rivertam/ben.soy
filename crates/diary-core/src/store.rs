//! The diary entry store: model, key projection, and queries, written against
//! the crate's one [`Db`] handle so the server's real database, the native
//! tests' `mem://`, and the device's `indxdb://` all run identical code.
//! Moved from `src/app/diary.rs`, where it was written against the server
//! handle alone; the server now calls these with `data.db().await?`.
//!
//! The query shapes follow docs/surrealdb-notes.md: explicit projections
//! (`SELECT *` omits `option` fields holding `NONE` — `reply_to` is one),
//! string keys via `record::id(id)`, keys returned
//! from creates via `RETURN VALUE record::id(id)`, one `=` per delete.

use std::ops::Deref;

use serde::Deserialize;
use surrealdb::types::SurrealValue;

use crate::Db;
use crate::eastern;
use crate::entry::{ComposedEntry, EntryRejection, PROJECTION, accept_for_save};
use crate::placement::{self, Placement};

pub use crate::entry::DiaryEntry;

/// Transcript page size — the server page and the worker's offline SSR agree
/// through this constant.
pub const PAGE_SIZE: usize = 20;
/// Far past any real diary and far under the store's signed-64-bit `START`
/// limit. Wilder page numbers behave like unparseable ones instead of
/// surfacing a SurrealDB parse error as a fake outage.
pub const MAX_PAGE: usize = 1_000_000;

/// Test stores share one server-row fixture so a new business field has one
/// test-schema seam rather than separate store and sync copies.
#[cfg(test)]
pub(crate) const TEST_SCHEMA: &str = "\
    DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
    DEFINE FIELD id ON diary_entries TYPE string;
    DEFINE FIELD written_at ON diary_entries TYPE int;
    DEFINE FIELD body ON diary_entries TYPE string;
    DEFINE FIELD reply_to ON diary_entries TYPE option<string>;";

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
                 ORDER BY written_at DESC, id DESC LIMIT {PAGE_SIZE} START {start};
             SELECT count() FROM diary_entries GROUP ALL;"
        ))
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
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION} FROM diary_entries \
             ORDER BY written_at ASC, id ASC"
        ))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    response.take(0).map_err(|error| error.to_string())
}

pub async fn entry_by_id(db: &Db, id: &str) -> Result<Option<DiaryEntry>, String> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION} FROM type::record('diary_entries', $id)"
        ))
        .bind(("id", id.to_string()))
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
    let mut response = db
        .query(
            "CREATE ONLY type::record('diary_entries', $id) CONTENT $entry \
             RETURN VALUE record::id(id)",
        )
        .bind(("id", entry.id.clone()))
        .bind(("entry", entry.composed.clone()))
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
        |id| async move { entry_by_id(db, &id).await.map_err(SaveError::Store) },
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

    /// Same `mem://` isomorphism proof the outbox tests run: these functions
    /// execute against the identical handle type the server and the device
    /// use, so passing here certifies the shared behavior, not a test double.
    /// The table shape mirrors the committed server schema (a fresh store has
    /// no tables, and 3.2 errors on queries against undefined ones); the
    /// ASSERTs stay in the server migration — prod's job, not this contract's.
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

    async fn save_reply_at(
        db: &Db,
        written_at: i64,
        body: &str,
        reply_to: &str,
    ) -> Result<SavedWrite, SaveError> {
        save_entry(
            db,
            ComposedEntry::new(written_at, body).with_reply_to(Some(reply_to.to_string())),
            written_at,
        )
        .await
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

    #[tokio::test]
    async fn save_rejects_a_malformed_reply_target_before_touching_the_store() {
        let db = store().await;
        let result = save_entry(
            &db,
            ComposedEntry::new(100, "reply")
                .with_reply_to(Some("/diary/not-a-permalink".to_string())),
            100,
        )
        .await;
        assert!(matches!(
            result,
            Err(SaveError::Rejected(EntryRejection::InvalidReplyTarget))
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
        let db = store().await;
        let parent = "2026-07-27T14-30-45-04-00";
        let entry = DiaryEntry::new(
            entry_key(100).unwrap(),
            ComposedEntry::new(100, "typed row").with_reply_to(Some(parent.to_string())),
        );
        insert_entry(&db, &entry).await.unwrap();
        assert_eq!(entry_by_id(&db, &entry.id).await.unwrap(), Some(entry));
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
    async fn same_body_with_a_different_parent_is_not_a_replay() {
        let db = store().await;
        let parent = "2026-07-27T14-30-45-04-00";
        let top_level = saved(save_at(&db, 100, "same words").await);
        let reply = saved(save_reply_at(&db, 100, "same words", parent).await);
        assert_eq!(reply.written_at, 101);
        assert_ne!(reply.id, top_level.id);
        assert_eq!(reply.reply_to.as_deref(), Some(parent));

        let replay = saved(save_reply_at(&db, 100, "same words", parent).await);
        assert!(replay.deduped);
        assert_eq!(replay.id, reply.id);
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
