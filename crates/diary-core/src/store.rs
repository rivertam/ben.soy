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

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Db;
use crate::contract::COLLISION_PROBES;
use crate::eastern;

/// Transcript page size — the server page and the worker's offline SSR agree
/// through this constant.
pub const PAGE_SIZE: usize = 20;
/// Far past any real diary and far under the store's signed-64-bit `START`
/// limit. Wilder page numbers behave like unparseable ones instead of
/// surfacing a SurrealDB parse error as a fake outage.
pub const MAX_PAGE: usize = 1_000_000;

/// One stored entry. `id` is the Eastern public-path record key;
/// `written_at` is UTC epoch seconds, the instant the key projects.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct DiaryEntry {
    pub id: String,
    pub written_at: i64,
    pub body: String,
}

/// The saved-or-deduped outcome [`save_entry`] reports.
pub struct SavedWrite {
    pub id: String,
    pub written_at: i64,
    pub deduped: bool,
}

pub enum SaveError {
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
            "SELECT *, record::id(id) AS id FROM diary_entries \
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

pub async fn entry_by_id(db: &Db, id: &str) -> Result<Option<DiaryEntry>, String> {
    let mut response = db
        .query("SELECT *, record::id(id) AS id FROM type::record('diary_entries', $id)")
        .bind(("id", id.to_string()))
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let entries: Vec<DiaryEntry> = response.take(0).map_err(|error| error.to_string())?;
    Ok(entries.into_iter().next())
}

/// CREATE, not UPSERT: two entries in the same second would collide on the
/// key, and overwriting a diary entry is worse than asking to resubmit.
pub async fn insert_entry(db: &Db, id: &str, written_at: i64, body: &str) -> Result<(), String> {
    db.query(
        "CREATE ONLY type::record('diary_entries', $id)
             SET written_at = $written_at,
                 body = $body",
    )
    .bind(("id", id.to_string()))
    .bind(("written_at", written_at))
    .bind(("body", body.to_string()))
    .await
    .map_err(|error| error.to_string())?
    .check()
    .map_err(|error| error.to_string())?;
    Ok(())
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
/// collision holding the SAME body is a replay of a write whose response was
/// lost, so it counts as saved. A different body probes forward one second
/// at a time, re-running the dedupe check at EVERY probed id — the killer
/// replay is "bumped to T+1, response lost, retried from T", which must land
/// on the T+1 dedupe, not insert again at T+2. Check-first, then CREATE,
/// then re-check when CREATE fails: a lost race is indistinguishable from a
/// replayed twin, and neither is an error (never sniff the store's error
/// strings). Never overwrites.
///
/// Invariant: the stored `written_at` is always the epoch its id projects
/// from, so `ORDER BY written_at DESC, id DESC` and the permalink agree
/// about when an entry happened.
pub async fn save_entry(db: &Db, written_at: i64, body: &str) -> Result<SavedWrite, SaveError> {
    for offset in 0..COLLISION_PROBES {
        let epoch = written_at + offset;
        let Some(id) = entry_key(epoch) else {
            return Err(SaveError::Store(format!("unprojectable epoch {epoch}")));
        };
        match entry_by_id(db, &id).await.map_err(SaveError::Store)? {
            Some(existing) if existing.body == body => {
                return Ok(SavedWrite {
                    id,
                    written_at: existing.written_at,
                    deduped: true,
                });
            }
            Some(_) => continue,
            None => {}
        }
        match insert_entry(db, &id, epoch, body).await {
            Ok(()) => {
                return Ok(SavedWrite {
                    id,
                    written_at: epoch,
                    deduped: false,
                });
            }
            Err(error) => match entry_by_id(db, &id).await.map_err(SaveError::Store)? {
                Some(existing) if existing.body == body => {
                    return Ok(SavedWrite {
                        id,
                        written_at: existing.written_at,
                        deduped: true,
                    });
                }
                Some(_) => continue,
                None => return Err(SaveError::Store(error)),
            },
        }
    }
    Err(SaveError::Exhausted)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        db.query(
            "DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
             DEFINE FIELD id ON diary_entries TYPE string;
             DEFINE FIELD written_at ON diary_entries TYPE int;
             DEFINE FIELD body ON diary_entries TYPE string;",
        )
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
            Err(SaveError::Exhausted) => panic!("save exhausted its probes"),
            Err(SaveError::Store(error)) => panic!("store failed: {error}"),
        }
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
    async fn save_dedupes_a_replayed_twin() {
        let db = store().await;
        let first = saved(save_entry(&db, 100, "Dear diary,").await);
        assert!(!first.deduped);
        let replay = saved(save_entry(&db, 100, "Dear diary,").await);
        assert!(replay.deduped);
        assert_eq!(replay.id, first.id);
        assert_eq!(replay.written_at, first.written_at);
        let (_, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn save_probes_forward_on_a_different_body() {
        let db = store().await;
        let first = saved(save_entry(&db, 100, "one").await);
        let second = saved(save_entry(&db, 100, "two").await);
        assert_eq!(second.written_at, 101);
        assert_ne!(second.id, first.id);
        // The stored written_at is always the epoch the id projects from.
        assert_eq!(entry_key(101).as_deref(), Some(second.id.as_str()));
    }

    #[tokio::test]
    async fn the_killer_replay_lands_on_the_bumped_dedupe() {
        let db = store().await;
        saved(save_entry(&db, 100, "occupant").await);
        // Bumped to T+1, response lost…
        let bumped = saved(save_entry(&db, 100, "mine").await);
        assert_eq!(bumped.written_at, 101);
        // …retried from T: must dedupe at T+1, never insert at T+2.
        let retried = saved(save_entry(&db, 100, "mine").await);
        assert!(retried.deduped);
        assert_eq!(retried.id, bumped.id);
        let (_, total) = entry_page(&db, 1).await.unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn save_exhausts_after_bounded_probes() {
        let db = store().await;
        for offset in 0..COLLISION_PROBES {
            let saved = save_entry(&db, 100 + offset, &format!("filler {offset}"))
                .await
                .ok()
                .unwrap();
            assert_eq!(saved.written_at, 100 + offset);
        }
        assert!(matches!(
            save_entry(&db, 100, "no room").await,
            Err(SaveError::Exhausted)
        ));
    }

    #[tokio::test]
    async fn pages_read_newest_first_with_totals() {
        let db = store().await;
        for (epoch, body) in [(100, "oldest"), (200, "middle"), (300, "newest")] {
            saved(save_entry(&db, epoch, body).await);
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
