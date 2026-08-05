//! The device-local diary store: mirror and outbox in ONE table.
//!
//! On the phone this runs against `indxdb://` (SurrealDB's IndexedDB engine)
//! inside the service worker and the page; under `cargo test` it runs
//! against `mem://`. Nothing here knows which: every function takes the same
//! `Surreal<Any>` handle the site's server code uses, which is the point —
//! the logic is written once and exercised natively before it ships to wasm.
//!
//! The local `diary_entries` table holds the server fields (id, written_at,
//! body) plus local-only sync state: a queued entry is just a row with
//! `state = 'pending'`, a delivered one flips to `'synced'` in place, and a
//! permanently rejected one to `'failed'`. Ids are predicted locally with
//! the SAME probe-and-dedupe loop the server runs ([`crate::store`]), so an
//! entry's permalink is right from the moment it is enqueued and the page
//! reconciles bubbles by nothing more than "same id". Rows are never
//! deleted on delivery — that delete-before-report gap is what forced the
//! old page to keep a provisional-bubble machine.
//!
//! Deliberately no `Send` bounds on [`flush`]'s transport: on wasm the
//! injected future wraps a browser `fetch` and is `!Send`; natively the test
//! doubles are ordinary futures. Single-threaded wasm never needs `Send`,
//! and adding it would make the shared signature uncompilable there.
//!
//! The query shapes follow docs/surrealdb-notes.md: explicit projections
//! (because `SELECT *` omits `option` fields holding `NONE`), string keys
//! returned via `record::id(id)`, and one `=` per delete.

use std::future::Future;

use serde::{Deserialize, Serialize};
use surrealdb::{engine::any, types::SurrealValue};

use crate::contract::{COLLISION_PROBES, SendOutcome, WireEntry, normalize_lines};
use crate::store::entry_key;

pub use crate::Db;

/// Local namespace/database names. Nothing else ever lives in this store.
const NAMESPACE: &str = "diary";
const DATABASE: &str = "diary";

pub const STATE_SYNCED: &str = "synced";
pub const STATE_PENDING: &str = "pending";
pub const STATE_FAILED: &str = "failed";

/// The local schema, reconciled by [`open`] the way `src/data.rs` reconciles
/// the committed server schema. Two deliberate absences: no length ASSERT on
/// `body` (the queue must hold whatever text is already on the device — the
/// server is the judge, and its rejection marks the entry failed instead of
/// stranding it), and no id-shape ASSERT (a row whose `written_at` cannot
/// project to a real key is still never dropped — it ports under a synthetic
/// key as failed).
const SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL PERMISSIONS NONE;\n\
    DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int;\n\
    DEFINE FIELD OVERWRITE body ON diary_entries TYPE string;\n\
    DEFINE FIELD OVERWRITE state ON diary_entries TYPE string \
        ASSERT $value IN ['synced', 'pending', 'failed'];\n\
    DEFINE FIELD OVERWRITE reason ON diary_entries TYPE option<string>;\n\
    DEFINE FIELD OVERWRITE enqueued_at ON diary_entries TYPE int;\n";

/// The pre-single-store queue table (v1). Still DEFINEd on every open so the
/// standing drain below can always SELECT it: during a deploy's version skew
/// an old page or worker happily re-creates and writes `diary_outbox` after
/// the new code emptied it, so the drain is a permanent cheap step (exactly
/// like the pre-wasm IndexedDB migration before it), never a one-shot.
const V1_SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_outbox SCHEMAFULL PERMISSIONS NONE;\n\
    DEFINE FIELD OVERWRITE written_at ON diary_outbox TYPE int;\n\
    DEFINE FIELD OVERWRITE body ON diary_outbox TYPE string;\n\
    DEFINE FIELD OVERWRITE state ON diary_outbox TYPE string \
        ASSERT $value IN ['pending', 'failed'];\n\
    DEFINE FIELD OVERWRITE reason ON diary_outbox TYPE option<string>;\n\
    DEFINE FIELD OVERWRITE enqueued_at ON diary_outbox TYPE int;\n";

#[derive(Debug)]
pub enum OutboxError {
    /// Empty after normalization — nothing to queue. This is the ONLY
    /// validation applied to new text; anything non-empty is queued and the
    /// server judges it on replay, so a rejection marks the entry failed
    /// with its text preserved instead of bouncing it into the lossy
    /// form-POST fallback.
    InvalidBody,
    Db(String),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutboxError::InvalidBody => write!(f, "not an entry we would store"),
            OutboxError::Db(error) => write!(f, "local store failed: {error}"),
        }
    }
}

impl std::error::Error for OutboxError {}

/// One local row. `id` is the entry's predicted permalink key (or a
/// synthetic key for unprojectable failed rows); `enqueued_at`
/// (caller-supplied milliseconds) orders the flush.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct LocalEntry {
    pub id: String,
    pub written_at: i64,
    pub body: String,
    pub state: String,
    pub reason: Option<String>,
    pub enqueued_at: i64,
}

/// A queue entry arriving from the pre-wasm IndexedDB store (the worker
/// reads them out once and passes them to [`import`]). Unknown fields — the
/// old store's `qid` — are ignored, and bodies are preserved byte-for-byte:
/// the server is the only judge of old text.
#[derive(Debug, Deserialize)]
pub struct LegacyEntry {
    pub written_at: i64,
    pub body: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub enqueued_at: Option<i64>,
}

/// What one flush did, in the shape the page's BroadcastChannel message has
/// always carried (`blocked` serializes to `null` / `"auth"` / `"net"`).
/// `saved_entries` maps each delivered entry's local id (`qid`, its
/// predicted key) to the identity the server actually assigned — identical
/// in the common case, different only when the server's cross-device
/// collision probe bumped the second.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlushReport {
    pub saved: u32,
    pub pending: u32,
    pub failed: u32,
    pub blocked: Option<Blocked>,
    pub saved_entries: Vec<SavedEntry>,
    /// How many mirror rows the pull that follows a flush changed — `None`
    /// when the pull was skipped or classified Auth/Retry (a no-op). Filled
    /// by `sync::run`, not by [`flush`] itself; stale pages ignore it.
    pub pulled: Option<u32>,
}

/// One entry a flush landed: the local id it was queued under and the
/// permanent identity the server holds it under now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SavedEntry {
    pub qid: String,
    pub id: String,
    pub written_at: i64,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    Auth,
    Net,
}

/// Connect to an endpoint (`indxdb://diary` on the device, `mem://` in
/// tests), select the fixed namespace, reconcile both schemas, and drain any
/// v1 queue rows into the single store. Local engines need no signin.
/// Concurrent opens (the page and the worker both call this) are safe: the
/// drain's placement loop is check-CREATE-recheck, so a lost race reads as
/// the twin it is.
pub async fn open(endpoint: &str) -> Result<Db, OutboxError> {
    let db = any::connect(endpoint).await.map_err(db_error)?;
    db.use_ns(NAMESPACE)
        .use_db(DATABASE)
        .await
        .map_err(db_error)?;
    db.query(SCHEMA)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    db.query(V1_SCHEMA)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    drain_v1(&db).await?;
    Ok(db)
}

/// Queue one entry composed at `written_at` (epoch seconds). Line endings
/// are normalized here so what sits locally is byte-identical to what the
/// server would store — but the length bound deliberately is NOT checked:
/// over-length text queues fine, replays, gets the server's 422, and stays
/// on the page as a failed entry for manual copy. `enqueued_at_ms` comes
/// from the caller because the caller owns the clock (the page passes
/// `Date.now()`).
///
/// The id is resolved HERE, with the same probe-and-dedupe rule the server
/// applies: same second + same body at any probed key is the same entry
/// (returned as-is — a double-tap converges instead of minting a twin);
/// only a different body probes forward. The permalink is therefore right
/// from birth, and the server's own probe remains the cross-device backstop.
pub async fn enqueue(
    db: &Db,
    written_at: i64,
    raw_body: &str,
    enqueued_at_ms: i64,
) -> Result<LocalEntry, OutboxError> {
    let body = normalize_lines(raw_body);
    if body.is_empty() {
        return Err(OutboxError::InvalidBody);
    }
    place(db, written_at, &body, STATE_PENDING, None, enqueued_at_ms).await
}

/// Every not-yet-synced row, oldest enqueue first — flush order and the
/// page's bubble order. Synced rows are deliberately absent: the server
/// (or the worker's offline render) already shows them.
pub async fn queued(db: &Db) -> Result<Vec<LocalEntry>, OutboxError> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, written_at, body, state, reason, enqueued_at \
             FROM diary_entries WHERE state != 'synced' \
             ORDER BY enqueued_at ASC, id ASC",
        )
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

/// Every local row in every state, oldest write first — the pull's diff
/// input ([`crate::sync::apply_pull`] decides creates/updates/deletes
/// against this).
pub(crate) async fn all_local(db: &Db) -> Result<Vec<LocalEntry>, OutboxError> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, written_at, body, state, reason, enqueued_at \
             FROM diary_entries ORDER BY written_at ASC, id ASC",
        )
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

/// Drop a queued or failed row — the page's discard button. Synced rows are
/// out of reach on purpose: locally discarding delivered history would just
/// resurrect on the next pull, so it cannot be expressed at all. Idempotent.
pub async fn discard(db: &Db, id: &str) -> Result<(), OutboxError> {
    db.query("DELETE type::record('diary_entries', $id) WHERE state != 'synced'")
        .bind(("id", id.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    Ok(())
}

/// Import the pre-wasm IndexedDB queue. Idempotent by `(written_at, body)`
/// — the placement loop's dedupe — so the caller deleting old records only
/// after this returns makes a crash between the two re-run safely.
/// Returns how many entries were newly written.
pub async fn import(db: &Db, legacy: &[LegacyEntry]) -> Result<u32, OutboxError> {
    let mut imported = 0;
    for entry in legacy {
        if entry.body.is_empty() {
            continue; // the old page never queued empty text; nothing to keep
        }
        let state = match entry.state.as_deref() {
            Some(STATE_FAILED) => STATE_FAILED,
            _ => STATE_PENDING,
        };
        let placed = place_preserving(
            db,
            entry.written_at,
            &entry.body,
            state,
            entry.reason.as_deref(),
            entry.enqueued_at.unwrap_or(0),
        )
        .await?;
        if placed {
            imported += 1;
        }
    }
    Ok(imported)
}

/// Replay every pending entry, oldest first, through `send`. Stop on the
/// first Auth/Retry outcome so composition order survives; permanent
/// rejections mark the entry failed and move on. A delivered entry flips to
/// `synced` IN PLACE — never deleted — so no snapshot taken mid-flush can
/// watch an entry blink out of existence, and the offline render always has
/// the full transcript. The server's same-second + same-body dedupe is the
/// real idempotency guarantee: a retried send whose response was lost
/// counts as saved there.
pub async fn flush<F, Fut>(db: &Db, mut send: F) -> Result<FlushReport, OutboxError>
where
    F: FnMut(WireEntry) -> Fut,
    Fut: Future<Output = SendOutcome>,
{
    let rows = queued(db).await?;
    let mut saved_entries = Vec::new();
    let mut blocked = None;
    for entry in rows.iter().filter(|entry| entry.state == STATE_PENDING) {
        let wire = WireEntry {
            written_at: entry.written_at,
            body: entry.body.clone(),
        };
        match send(wire).await {
            SendOutcome::Saved(server) => {
                mark_synced(db, entry, &server.id, server.written_at).await?;
                saved_entries.push(SavedEntry {
                    qid: entry.id.clone(),
                    id: server.id,
                    written_at: server.written_at,
                    body: entry.body.clone(),
                });
            }
            SendOutcome::Auth => {
                blocked = Some(Blocked::Auth);
                break;
            }
            SendOutcome::Retry => {
                blocked = Some(Blocked::Net);
                break;
            }
            SendOutcome::Rejected(status) => {
                mark_failed(db, &entry.id, &crate::contract::rejection_reason(status)).await?;
            }
        }
    }
    let after = queued(db).await?;
    Ok(FlushReport {
        saved: saved_entries.len() as u32,
        pending: count_state(&after, STATE_PENDING),
        failed: count_state(&after, STATE_FAILED),
        blocked,
        saved_entries,
        pulled: None,
    })
}

fn count_state(entries: &[LocalEntry], state: &str) -> u32 {
    entries.iter().filter(|entry| entry.state == state).count() as u32
}

/// Flip a delivered row to synced. Same server identity (the overwhelmingly
/// common case): one in-place UPDATE, guarded on `state = 'pending'` so an
/// entry discarded mid-send stays discarded (the text is safe server-side).
/// A server bump re-keys the row to the id the server assigned, in one
/// transaction; if THAT key is already a different local row (a pending
/// twin-second — double collision), the old row is simply released and the
/// next-in-lock pull places the server's copy, per the plan's re-key rule.
async fn mark_synced(
    db: &Db,
    entry: &LocalEntry,
    server_id: &str,
    server_written_at: i64,
) -> Result<(), OutboxError> {
    if server_id == entry.id {
        db.query(
            "UPDATE type::record('diary_entries', $id) \
             SET state = 'synced', reason = NONE WHERE state = 'pending'",
        )
        .bind(("id", entry.id.clone()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
        return Ok(());
    }
    let moved = db
        .query(
            "BEGIN TRANSACTION;
             DELETE type::record('diary_entries', $old);
             CREATE ONLY type::record('diary_entries', $new)
                 SET written_at = $written_at, body = $body,
                     state = 'synced', enqueued_at = $enqueued_at;
             COMMIT TRANSACTION;",
        )
        .bind(("old", entry.id.clone()))
        .bind(("new", server_id.to_string()))
        .bind(("written_at", server_written_at))
        .bind(("body", entry.body.clone()))
        .bind(("enqueued_at", entry.enqueued_at))
        .await
        .map_err(db_error)?
        .check();
    if moved.is_err() {
        // The bumped key is occupied by a different local row. Release the
        // delivered pending row alone; the pull that follows in the same
        // lock hold materializes the server's copy.
        db.query("DELETE type::record('diary_entries', $old)")
            .bind(("old", entry.id.clone()))
            .await
            .map_err(db_error)?
            .check()
            .map_err(db_error)?;
    }
    Ok(())
}

/// Only pending entries fail; an entry discarded mid-flush stays discarded.
async fn mark_failed(db: &Db, id: &str, reason: &str) -> Result<(), OutboxError> {
    db.query(
        "UPDATE type::record('diary_entries', $id) \
         SET state = 'failed', reason = $reason WHERE state = 'pending'",
    )
    .bind(("id", id.to_string()))
    .bind(("reason", reason.to_string()))
    .await
    .map_err(db_error)?
    .check()
    .map_err(db_error)?;
    Ok(())
}

/// The local placement loop — the same probe-and-dedupe shape as
/// `store::save_entry`, extended with the local state columns. Same second +
/// same body at ANY probed key returns that existing row (dedupe, any
/// state); a different body probes forward; check-CREATE-recheck absorbs a
/// concurrent placer. Rows whose epoch cannot project get a deterministic
/// synthetic key and land as failed — never dropped.
async fn place(
    db: &Db,
    written_at: i64,
    body: &str,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<LocalEntry, OutboxError> {
    match place_inner(db, written_at, body, state, reason, enqueued_at).await? {
        Placement::Placed(entry) | Placement::Deduped(entry) => Ok(entry),
        Placement::Exhausted => Err(OutboxError::Db(format!("no free second near {written_at}"))),
    }
}

/// [`place`], but exhaustion and unprojectable epochs fall back to a
/// synthetic failed row instead of erroring — the migration paths use this
/// because migrated text must never be dropped or error out of the drain.
/// Returns whether a row was newly written (a dedupe hit is not).
async fn place_preserving(
    db: &Db,
    written_at: i64,
    body: &str,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<bool, OutboxError> {
    if entry_key(written_at).is_some() {
        match place_inner(db, written_at, body, state, reason, enqueued_at).await? {
            Placement::Placed(_) => return Ok(true),
            Placement::Deduped(_) => return Ok(false),
            Placement::Exhausted => {}
        }
    }
    synthetic_failed(db, written_at, body, reason, enqueued_at).await
}

enum Placement {
    /// Newly created under this key.
    Placed(LocalEntry),
    /// An existing row already held this second + body — the same entry.
    Deduped(LocalEntry),
    Exhausted,
}

async fn place_inner(
    db: &Db,
    written_at: i64,
    body: &str,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<Placement, OutboxError> {
    for offset in 0..COLLISION_PROBES {
        let epoch = written_at + offset;
        let Some(id) = entry_key(epoch) else {
            return Err(OutboxError::Db(format!("unprojectable epoch {epoch}")));
        };
        match local_by_id(db, &id).await? {
            Some(existing) if existing.body == body => {
                return Ok(Placement::Deduped(existing));
            }
            Some(_) => continue,
            None => {}
        }
        match create_row(db, &id, epoch, body, state, reason, enqueued_at).await {
            Ok(()) => {
                return Ok(Placement::Placed(LocalEntry {
                    id,
                    written_at: epoch,
                    body: body.to_string(),
                    state: state.to_string(),
                    reason: reason.map(str::to_string),
                    enqueued_at,
                }));
            }
            Err(error) => match local_by_id(db, &id).await? {
                Some(existing) if existing.body == body => {
                    return Ok(Placement::Deduped(existing));
                }
                Some(_) => continue,
                None => return Err(error),
            },
        }
    }
    Ok(Placement::Exhausted)
}

/// A deterministic non-projected key for text whose timestamp cannot become
/// a real permalink. Deterministic so re-running a crashed migration
/// converges on the same row.
async fn synthetic_failed(
    db: &Db,
    written_at: i64,
    body: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<bool, OutboxError> {
    let id = format!("failed-{written_at}-{enqueued_at}");
    match local_by_id(db, &id).await? {
        Some(_) => Ok(false),
        None => {
            let reason = reason.unwrap_or("unprojectable timestamp");
            create_row(
                db,
                &id,
                written_at,
                body,
                STATE_FAILED,
                Some(reason),
                enqueued_at,
            )
            .await?;
            Ok(true)
        }
    }
}

async fn local_by_id(db: &Db, id: &str) -> Result<Option<LocalEntry>, OutboxError> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, written_at, body, state, reason, enqueued_at \
             FROM type::record('diary_entries', $id)",
        )
        .bind(("id", id.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<LocalEntry> = response.take(0).map_err(db_error)?;
    Ok(rows.into_iter().next())
}

/// Two statement shapes rather than binding an Option: whether a bound
/// `None` lands as SurrealQL `NONE` or `null` is exactly the kind of
/// result-shape trap docs/surrealdb-notes.md exists for, and `option<string>`
/// only admits one of them.
async fn create_row(
    db: &Db,
    id: &str,
    written_at: i64,
    body: &str,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<(), OutboxError> {
    let statement = match reason {
        Some(_) => {
            "CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $body, \
             state = $state, reason = $reason, enqueued_at = $enqueued_at"
        }
        None => {
            "CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $body, \
             state = $state, enqueued_at = $enqueued_at"
        }
    };
    let mut query = db
        .query(statement)
        .bind(("id", id.to_string()))
        .bind(("written_at", written_at))
        .bind(("body", body.to_string()))
        .bind(("state", state.to_string()))
        .bind(("enqueued_at", enqueued_at));
    if let Some(reason) = reason {
        query = query.bind(("reason", reason.to_string()));
    }
    query.await.map_err(db_error)?.check().map_err(db_error)?;
    Ok(())
}

/// The standing v1 drain: port every `diary_outbox` row into the single
/// store (same placement rules as [`import`]), then delete the ported rows
/// one by one. Delete-after-write plus the placement dedupe makes a crash
/// anywhere re-runnable.
async fn drain_v1(db: &Db) -> Result<(), OutboxError> {
    #[derive(Deserialize, SurrealValue)]
    struct V1Row {
        qid: String,
        written_at: i64,
        body: String,
        state: String,
        reason: Option<String>,
        enqueued_at: i64,
    }

    let mut response = db
        .query(
            "SELECT record::id(id) AS qid, written_at, body, state, reason, enqueued_at \
             FROM diary_outbox ORDER BY enqueued_at ASC, qid ASC",
        )
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<V1Row> = response.take(0).map_err(db_error)?;
    for row in rows {
        if !row.body.is_empty() {
            let state = if row.state == STATE_FAILED {
                STATE_FAILED
            } else {
                STATE_PENDING
            };
            place_preserving(
                db,
                row.written_at,
                &row.body,
                state,
                row.reason.as_deref(),
                row.enqueued_at,
            )
            .await?;
        }
        db.query("DELETE type::record('diary_outbox', $qid)")
            .bind(("qid", row.qid))
            .await
            .map_err(db_error)?
            .check()
            .map_err(db_error)?;
    }
    Ok(())
}

fn db_error(error: surrealdb::Error) -> OutboxError {
    OutboxError::Db(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::future::ready;

    use crate::contract::SavedRef;

    use super::*;

    /// Every test opens `mem://` — a fresh, empty store per call, reached
    /// through the identical `Surreal<Any>` + `open()` path the device uses
    /// for `indxdb://diary`. That sameness is what these tests certify.
    async fn store() -> Db {
        open("mem://").await.expect("mem engine opens")
    }

    /// A server acceptance, as `classify_response` would build it.
    fn saved(id: &str, written_at: i64) -> SendOutcome {
        SendOutcome::Saved(SavedRef {
            id: id.to_string(),
            written_at,
        })
    }

    fn key(epoch: i64) -> String {
        entry_key(epoch).expect("test epoch projects")
    }

    /// Directly read one row in ANY state (queued() hides synced ones).
    async fn row(db: &Db, id: &str) -> Option<LocalEntry> {
        local_by_id(db, id).await.expect("read succeeds")
    }

    #[tokio::test]
    async fn open_is_idempotent_and_drain_reruns_quietly() {
        let db = store().await;
        db.query(SCHEMA).await.unwrap().check().unwrap();
        db.query(V1_SCHEMA).await.unwrap().check().unwrap();
        drain_v1(&db).await.unwrap();
        assert!(queued(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn enqueue_normalizes_predicts_the_permalink_and_round_trips() {
        let db = store().await;
        let entry = enqueue(&db, 1_753_640_000, "Dear diary,\r\nIt me.\r\n", 7)
            .await
            .unwrap();
        assert_eq!(entry.body, "Dear diary,\nIt me.");
        assert_eq!(entry.state, STATE_PENDING);
        // The id IS the permalink the server would assign this second.
        assert_eq!(entry.id, key(1_753_640_000));
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        // The explicit projection must surface the absent `reason` as None
        // instead of dropping the field (the `SELECT *` NONE-omission trap).
        assert_eq!(listed[0].reason, None);
        assert_eq!(listed[0].id, entry.id);
        assert_eq!(listed[0].written_at, 1_753_640_000);
        assert_eq!(listed[0].enqueued_at, 7);
    }

    #[tokio::test]
    async fn enqueue_refuses_only_empty_text() {
        let db = store().await;
        for bad in ["", "  \r\n\t "] {
            assert!(matches!(
                enqueue(&db, 1_753_640_000, bad, 1).await,
                Err(OutboxError::InvalidBody)
            ));
        }
        assert!(queued(&db).await.unwrap().is_empty());
    }

    /// A double-tap that slips the emptied-textarea guard converges on ONE
    /// row — the probe's per-key dedupe, not a second entry the server
    /// would then store twice.
    #[tokio::test]
    async fn enqueue_dedupes_a_same_second_twin() {
        let db = store().await;
        let first = enqueue(&db, 1_753_640_000, "tap", 10).await.unwrap();
        let twin = enqueue(&db, 1_753_640_000, "tap", 25).await.unwrap();
        assert_eq!(twin.id, first.id);
        assert_eq!(twin.enqueued_at, 10, "the original row is the entry");
        assert_eq!(queued(&db).await.unwrap().len(), 1);
    }

    /// Two different thoughts in one second get consecutive seconds — and
    /// therefore distinct, correct permalinks — at enqueue time.
    #[tokio::test]
    async fn enqueue_probes_a_different_body_forward() {
        let db = store().await;
        let first = enqueue(&db, 1_753_640_000, "one", 10).await.unwrap();
        let second = enqueue(&db, 1_753_640_000, "two", 20).await.unwrap();
        assert_eq!(first.written_at, 1_753_640_000);
        assert_eq!(second.written_at, 1_753_640_001);
        assert_eq!(second.id, key(1_753_640_001));
    }

    /// Over-length text must QUEUE (the server's 422 marks it failed with
    /// its text preserved) — refusing it here would bounce the entry into
    /// the lossy form-POST fallback, and offline that means silent loss.
    #[tokio::test]
    async fn enqueue_accepts_over_length_text_for_the_server_to_judge() {
        use crate::contract::MAX_ENTRY_CHARS;
        let db = store().await;
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        enqueue(&db, 1_753_640_000, &oversized, 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Rejected(422)))
            .await
            .unwrap();
        assert_eq!(report.failed, 1);
        let left = queued(&db).await.unwrap();
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].body, oversized, "failed text must survive intact");
    }

    #[tokio::test]
    async fn discard_removes_queued_rows_and_spares_synced_history() {
        let db = store().await;
        let queued_entry = enqueue(&db, 1_753_640_000, "keep me", 1).await.unwrap();
        discard(&db, &queued_entry.id).await.unwrap();
        discard(&db, &queued_entry.id).await.unwrap();
        assert!(queued(&db).await.unwrap().is_empty());

        let delivered = enqueue(&db, 1_753_640_100, "delivered", 2).await.unwrap();
        flush(&db, |wire: WireEntry| {
            ready(saved(&key(wire.written_at), wire.written_at))
        })
        .await
        .unwrap();
        discard(&db, &delivered.id).await.unwrap();
        assert!(
            row(&db, &delivered.id).await.is_some(),
            "synced history is out of discard's reach"
        );
    }

    /// The heart of the single store: a delivered entry flips to synced IN
    /// PLACE. No snapshot taken at any moment during the flush can watch it
    /// vanish — the gap the old delete-on-save queue forced the page to
    /// paper over with provisional bubbles.
    #[tokio::test]
    async fn flush_sends_oldest_first_and_flips_state_in_place() {
        let db = store().await;
        // Enqueued out of order on purpose; enqueued_at decides.
        let second = enqueue(&db, 1_753_640_200, "second", 20).await.unwrap();
        let first = enqueue(&db, 1_753_640_100, "first", 10).await.unwrap();
        let third = enqueue(&db, 1_753_640_300, "third", 30).await.unwrap();
        let sent = RefCell::new(Vec::new());
        let report = flush(&db, |wire: WireEntry| {
            sent.borrow_mut().push(wire.body.clone());
            ready(saved(&key(wire.written_at), wire.written_at))
        })
        .await
        .unwrap();
        assert_eq!(*sent.borrow(), ["first", "second", "third"]);
        assert_eq!(report.saved, 3);
        assert_eq!(
            (report.pending, report.failed, report.blocked),
            (0, 0, None)
        );
        assert_eq!(report.saved_entries.len(), 3);
        assert_eq!(report.saved_entries[0].qid, first.id);
        assert_eq!(
            report.saved_entries[0].id, first.id,
            "no bump: same identity"
        );
        // Queued view is empty — but every row still exists, synced.
        assert!(queued(&db).await.unwrap().is_empty());
        for entry in [&first, &second, &third] {
            let kept = row(&db, &entry.id).await.expect("row survives delivery");
            assert_eq!(kept.state, STATE_SYNCED);
            assert_eq!(kept.body, entry.body);
        }
    }

    #[tokio::test]
    async fn flush_stops_on_retryable_trouble() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "first", 10).await.unwrap();
        enqueue(&db, 1_753_640_200, "second", 20).await.unwrap();
        enqueue(&db, 1_753_640_300, "third", 30).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            saved(&key(1_753_640_100), 1_753_640_100),
            SendOutcome::Retry,
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().expect("third never sent");
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 2, 0));
        assert_eq!(report.blocked, Some(Blocked::Net));
        // The stopped entries survive, order intact, for the next flush.
        let left: Vec<String> = queued(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.body)
            .collect();
        assert_eq!(left, ["second", "third"]);
    }

    #[tokio::test]
    async fn flush_stops_quietly_when_signed_out() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "private", 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Auth)).await.unwrap();
        assert_eq!(report.blocked, Some(Blocked::Auth));
        assert_eq!(report.pending, 1);
    }

    #[tokio::test]
    async fn flush_marks_rejections_failed_and_moves_on() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "rejected", 10).await.unwrap();
        enqueue(&db, 1_753_640_200, "accepted", 20).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            SendOutcome::Rejected(422),
            saved(&key(1_753_640_200), 1_753_640_200),
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().unwrap();
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 0, 1));
        assert_eq!(report.blocked, None);
        assert_eq!(report.saved_entries[0].body, "accepted");
        let left = queued(&db).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].reason.as_deref(), Some("rejected (HTTP 422)"));
        // A failed entry is kept for manual copy, never re-sent.
        let report = flush(&db, |_| ready(saved("never", 0))).await.unwrap();
        assert_eq!(report.saved, 0);
        assert_eq!(queued(&db).await.unwrap().len(), 1);
    }

    /// The server's cross-device probe bumped the entry a second forward:
    /// the local row moves to the server's identity, and the report maps
    /// the predicted id to the real one for the page's single fix-up.
    #[tokio::test]
    async fn flush_rekeys_a_server_bumped_entry() {
        let db = store().await;
        let entry = enqueue(&db, 1_753_640_000, "bumped", 10).await.unwrap();
        let server_id = key(1_753_640_001);
        let report = flush(&db, |_| ready(saved(&server_id, 1_753_640_001)))
            .await
            .unwrap();
        assert_eq!(report.saved_entries[0].qid, entry.id);
        assert_eq!(report.saved_entries[0].id, server_id);
        assert!(row(&db, &entry.id).await.is_none(), "old key released");
        let moved = row(&db, &server_id).await.expect("row lives at server id");
        assert_eq!(moved.state, STATE_SYNCED);
        assert_eq!(moved.written_at, 1_753_640_001);
        assert_eq!(moved.body, "bumped");
    }

    /// Double collision: the server bumps an entry onto a second that a
    /// DIFFERENT local pending row already occupies. The delivered row is
    /// released (its text is safe server-side; the next-in-lock pull will
    /// place it) and the pending neighbor is never overwritten — queued
    /// diary text is never silently clobbered.
    #[tokio::test]
    async fn a_bump_never_clobbers_a_pending_neighbor() {
        let db = store().await;
        let bumped = enqueue(&db, 1_753_640_000, "mine", 10).await.unwrap();
        let neighbor = enqueue(&db, 1_753_640_001, "neighbor", 20).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            // The server saved "mine" at T+1 (some other device's entry
            // occupied T there)…
            saved(&key(1_753_640_001), 1_753_640_001),
            // …and the flush stops before the neighbor is attempted.
            SendOutcome::Retry,
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().unwrap();
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!(report.saved, 1);
        assert!(
            row(&db, &bumped.id).await.is_none(),
            "delivered row released"
        );
        let kept = row(&db, &neighbor.id).await.expect("neighbor survives");
        assert_eq!(kept.state, STATE_PENDING);
        assert_eq!(kept.body, "neighbor");
    }

    #[tokio::test]
    async fn the_v1_drain_ports_and_empties_the_old_queue() {
        let db = store().await;
        for (written_at, body, state, reason, enqueued_at) in [
            (1_753_640_000_i64, "old pending", "pending", None, 1_i64),
            (
                1_753_640_060,
                "old failure",
                "failed",
                Some("rejected (HTTP 422)"),
                2,
            ),
        ] {
            let statement = match reason {
                Some(_) => {
                    "CREATE diary_outbox SET written_at = $written_at, body = $body, \
                     state = $state, reason = $reason, enqueued_at = $enqueued_at"
                }
                None => {
                    "CREATE diary_outbox SET written_at = $written_at, body = $body, \
                     state = $state, enqueued_at = $enqueued_at"
                }
            };
            let mut query = db
                .query(statement)
                .bind(("written_at", written_at))
                .bind(("body", body.to_string()))
                .bind(("state", state.to_string()))
                .bind(("enqueued_at", enqueued_at));
            if let Some(reason) = reason {
                query = query.bind(("reason", reason.to_string()));
            }
            query.await.unwrap().check().unwrap();
        }
        drain_v1(&db).await.unwrap();
        // Re-running the drain (a crash replay) changes nothing.
        drain_v1(&db).await.unwrap();
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].body, "old pending");
        assert_eq!(listed[0].state, STATE_PENDING);
        assert_eq!(
            listed[0].id,
            key(1_753_640_000),
            "ported rows get real permalink keys"
        );
        assert_eq!(listed[1].state, STATE_FAILED);
        assert_eq!(listed[1].reason.as_deref(), Some("rejected (HTTP 422)"));
        #[derive(Deserialize, SurrealValue)]
        struct CountRow {
            count: i64,
        }
        let mut response = db
            .query("SELECT count() FROM diary_outbox GROUP ALL")
            .await
            .unwrap()
            .check()
            .unwrap();
        let counts: Vec<CountRow> = response.take(0).unwrap();
        assert!(
            counts.is_empty() || counts[0].count == 0,
            "v1 queue emptied"
        );
    }

    /// A row whose timestamp cannot project still ports — under a synthetic
    /// key, failed, with its text intact. Never dropped, never an error.
    #[tokio::test]
    async fn unprojectable_rows_port_as_synthetic_failures() {
        let db = store().await;
        db.query(
            "CREATE diary_outbox SET written_at = $written_at, body = $body, \
             state = 'pending', enqueued_at = 9",
        )
        .bind(("written_at", i64::MIN))
        .bind(("body", "clock went sideways".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
        drain_v1(&db).await.unwrap();
        drain_v1(&db).await.unwrap();
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].state, STATE_FAILED);
        assert_eq!(listed[0].reason.as_deref(), Some("unprojectable timestamp"));
        assert_eq!(listed[0].body, "clock went sideways");
        assert!(listed[0].id.starts_with("failed-"));
    }

    #[tokio::test]
    async fn import_preserves_and_dedupes() {
        let db = store().await;
        let legacy = vec![
            LegacyEntry {
                written_at: 1_753_640_000,
                body: "old pending".to_string(),
                state: Some(STATE_PENDING.to_string()),
                reason: None,
                enqueued_at: Some(1),
            },
            LegacyEntry {
                written_at: 1_753_640_060,
                body: "old failure".to_string(),
                state: Some(STATE_FAILED.to_string()),
                reason: Some("rejected (HTTP 422)".to_string()),
                enqueued_at: Some(2),
            },
        ];
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        // A crash between import and the caller's delete replays the whole
        // batch; the placement dedupe absorbs it.
        assert_eq!(import(&db, &legacy).await.unwrap(), 0);
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].body, "old pending");
        assert_eq!(listed[0].state, STATE_PENDING);
        assert_eq!(listed[1].state, STATE_FAILED);
        assert_eq!(listed[1].reason.as_deref(), Some("rejected (HTTP 422)"));
    }

    #[tokio::test]
    async fn import_tolerates_the_old_shape() {
        // Real legacy records carry qid and enqueued_at; a hand-cleared
        // store might hold less. Unknown fields are ignored, absent
        // optionals default.
        let raw = r#"[
            {"qid": 3, "written_at": 1753640000, "body": "with qid", "state": "pending",
             "reason": null, "enqueued_at": 1753640000000},
            {"written_at": 1753640060, "body": "bare"}
        ]"#;
        let legacy: Vec<LegacyEntry> = serde_json::from_str(raw).unwrap();
        let db = store().await;
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed[0].body, "bare"); // enqueued_at defaults to 0
        assert_eq!(listed[1].body, "with qid");
        assert_eq!(listed[1].state, STATE_PENDING);
    }

    /// The report is the BroadcastChannel message the page has always read;
    /// `qid` is the predicted record id now (a permalink-shaped string).
    #[test]
    fn reports_serialize_in_the_broadcast_shape() {
        let report = FlushReport {
            saved: 1,
            pending: 1,
            failed: 0,
            blocked: Some(Blocked::Net),
            saved_entries: vec![SavedEntry {
                qid: "2026-07-27T14-30-45-04-00".to_string(),
                id: "2026-07-27T14-30-46-04-00".to_string(),
                written_at: 1_753_640_000,
                body: "Dear diary,".to_string(),
            }],
            pulled: Some(2),
        };
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"saved":1,"pending":1,"failed":0,"blocked":"net","saved_entries":[{"qid":"2026-07-27T14-30-45-04-00","id":"2026-07-27T14-30-46-04-00","written_at":1753640000,"body":"Dear diary,"}],"pulled":2}"#
        );
        let quiet = FlushReport {
            saved: 0,
            pending: 0,
            failed: 0,
            blocked: None,
            saved_entries: Vec::new(),
            pulled: None,
        };
        assert_eq!(
            serde_json::to_string(&quiet).unwrap(),
            r#"{"saved":0,"pending":0,"failed":0,"blocked":null,"saved_entries":[],"pulled":null}"#
        );
    }
}
