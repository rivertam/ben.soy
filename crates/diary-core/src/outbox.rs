//! The device-local write queue, stored in SurrealDB like everything else.
//!
//! On the phone this runs against `indxdb://` (SurrealDB's IndexedDB engine)
//! inside the service worker and the page; under `cargo test` it runs
//! against `mem://`. Nothing here knows which: every function takes the same
//! `Surreal<Any>` handle the site's server code uses, which is the point —
//! the queue logic is written once and exercised natively before it ships to
//! wasm.
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
use surrealdb::{
    Surreal,
    engine::any::{self, Any},
    types::SurrealValue,
};

use crate::contract::{SendOutcome, WireEntry, normalize_lines, rejection_reason};

pub type Db = Surreal<Any>;

/// Local namespace/database names. Nothing else ever lives in this store.
const NAMESPACE: &str = "diary";
const DATABASE: &str = "diary";

pub const STATE_PENDING: &str = "pending";
pub const STATE_FAILED: &str = "failed";

/// The local schema, reconciled by [`open`] the way `src/data.rs` reconciles
/// the committed server schema. No length ASSERT on `body` on purpose: the
/// queue must accept whatever text is already on the device — the server is
/// the judge of what it will store, and its rejection marks the entry failed
/// instead of stranding it locally.
const SCHEMA: &str = "\
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
    /// validation the queue applies to new text; anything non-empty is
    /// queued and the server judges it on replay, so a rejection marks the
    /// entry failed with its text preserved instead of bouncing it into
    /// the lossy form-POST fallback.
    InvalidBody,
    Db(String),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutboxError::InvalidBody => write!(f, "not an entry we would store"),
            OutboxError::Db(error) => write!(f, "outbox store failed: {error}"),
        }
    }
}

impl std::error::Error for OutboxError {}

/// One queued entry. `qid` is the record's key as a string; `enqueued_at`
/// (caller-supplied milliseconds) orders the flush and the page's queue
/// rendering the way the old store's auto-increment key did.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct QueuedEntry {
    pub qid: String,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FlushReport {
    pub saved: u32,
    pub pending: u32,
    pub failed: u32,
    pub blocked: Option<Blocked>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    Auth,
    Net,
}

/// Connect to an endpoint (`indxdb://diary` on the device, `mem://` in
/// tests), select the fixed namespace, and reconcile the schema. Local
/// engines need no signin.
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
    Ok(db)
}

/// Queue one entry composed at `written_at` (epoch seconds). Line endings
/// are normalized here so what sits in the queue is byte-identical to what
/// the server would store — but the length bound deliberately is NOT
/// checked: over-length text queues fine, replays, gets the server's 422,
/// and stays on the page as a failed entry for manual copy.
/// `enqueued_at_ms` comes from the caller because the caller owns the
/// clock (the worker passes `Date.now()`).
pub async fn enqueue(
    db: &Db,
    written_at: i64,
    raw_body: &str,
    enqueued_at_ms: i64,
) -> Result<QueuedEntry, OutboxError> {
    let body = normalize_lines(raw_body);
    if body.is_empty() {
        return Err(OutboxError::InvalidBody);
    }
    let qid = create(db, written_at, &body, STATE_PENDING, None, enqueued_at_ms).await?;
    Ok(QueuedEntry {
        qid,
        written_at,
        body,
        state: STATE_PENDING.to_string(),
        reason: None,
        enqueued_at: enqueued_at_ms,
    })
}

/// Every queued entry, oldest enqueue first — flush order and render order.
pub async fn entries(db: &Db) -> Result<Vec<QueuedEntry>, OutboxError> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS qid, written_at, body, state, reason, enqueued_at \
             FROM diary_outbox ORDER BY enqueued_at ASC, qid ASC",
        )
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

/// Idempotent delete — the page's discard button and a saved flush both
/// land here.
pub async fn remove(db: &Db, qid: &str) -> Result<(), OutboxError> {
    db.query("DELETE type::record('diary_outbox', $qid)")
        .bind(("qid", qid.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    Ok(())
}

/// Import the pre-wasm queue. Idempotent by `(written_at, body)`: the caller
/// deletes the old records only after this returns, so a crash between the
/// two re-runs safely, and a replayed twin is skipped, never duplicated.
/// Returns how many entries were newly written.
pub async fn import(db: &Db, legacy: &[LegacyEntry]) -> Result<u32, OutboxError> {
    let mut imported = 0;
    for entry in legacy {
        if entry.body.is_empty() {
            continue; // the old page never queued empty text; nothing to keep
        }
        if find_twin(db, entry.written_at, &entry.body)
            .await?
            .is_some()
        {
            continue;
        }
        let state = match entry.state.as_deref() {
            Some(STATE_FAILED) => STATE_FAILED,
            _ => STATE_PENDING,
        };
        create(
            db,
            entry.written_at,
            &entry.body,
            state,
            entry.reason.as_deref(),
            entry.enqueued_at.unwrap_or(0),
        )
        .await?;
        imported += 1;
    }
    Ok(imported)
}

/// Replay every pending entry, oldest first, through `send`. Stop on the
/// first Auth/Retry outcome so composition order survives; permanent
/// rejections mark the entry failed and move on. The server's same-second +
/// same-body dedupe is the real idempotency guarantee — a retried send whose
/// response was lost counts as saved there, so deleting on `Saved` here can
/// never lose text.
pub async fn flush<F, Fut>(db: &Db, mut send: F) -> Result<FlushReport, OutboxError>
where
    F: FnMut(WireEntry) -> Fut,
    Fut: Future<Output = SendOutcome>,
{
    let queued = entries(db).await?;
    let mut saved = 0u32;
    let mut blocked = None;
    for entry in queued.iter().filter(|entry| entry.state == STATE_PENDING) {
        let wire = WireEntry {
            written_at: entry.written_at,
            body: entry.body.clone(),
        };
        match send(wire).await {
            SendOutcome::Saved => {
                remove(db, &entry.qid).await?;
                saved += 1;
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
                mark_failed(db, &entry.qid, &rejection_reason(status)).await?;
            }
        }
    }
    let after = entries(db).await?;
    Ok(FlushReport {
        saved,
        pending: count_state(&after, STATE_PENDING),
        failed: count_state(&after, STATE_FAILED),
        blocked,
    })
}

fn count_state(entries: &[QueuedEntry], state: &str) -> u32 {
    entries.iter().filter(|entry| entry.state == state).count() as u32
}

/// Only pending entries fail; an entry discarded mid-flush stays discarded.
async fn mark_failed(db: &Db, qid: &str, reason: &str) -> Result<(), OutboxError> {
    db.query(
        "UPDATE type::record('diary_outbox', $qid) \
         SET state = 'failed', reason = $reason WHERE state = 'pending'",
    )
    .bind(("qid", qid.to_string()))
    .bind(("reason", reason.to_string()))
    .await
    .map_err(db_error)?
    .check()
    .map_err(db_error)?;
    Ok(())
}

/// Only the row's existence matters; the derives keep the field "read".
#[derive(Deserialize, SurrealValue)]
struct QidRow {
    qid: String,
}

async fn find_twin(db: &Db, written_at: i64, body: &str) -> Result<Option<()>, OutboxError> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS qid FROM diary_outbox \
             WHERE written_at = $written_at AND body = $body LIMIT 1",
        )
        .bind(("written_at", written_at))
        .bind(("body", body.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<QidRow> = response.take(0).map_err(db_error)?;
    Ok(rows.first().map(|_| ()))
}

/// Two statement shapes rather than binding an Option: whether a bound
/// `None` lands as SurrealQL `NONE` or `null` is exactly the kind of
/// result-shape trap docs/surrealdb-notes.md exists for, and `option<string>`
/// only admits one of them.
async fn create(
    db: &Db,
    written_at: i64,
    body: &str,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<String, OutboxError> {
    let statement = match reason {
        Some(_) => {
            "CREATE diary_outbox SET written_at = $written_at, body = $body, \
             state = $state, reason = $reason, enqueued_at = $enqueued_at \
             RETURN VALUE record::id(id)"
        }
        None => {
            "CREATE diary_outbox SET written_at = $written_at, body = $body, \
             state = $state, enqueued_at = $enqueued_at \
             RETURN VALUE record::id(id)"
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
    let mut response = query.await.map_err(db_error)?.check().map_err(db_error)?;
    let ids: Vec<String> = response.take(0).map_err(db_error)?;
    ids.into_iter()
        .next()
        .ok_or_else(|| OutboxError::Db("create returned no id".to_string()))
}

fn db_error(error: surrealdb::Error) -> OutboxError {
    OutboxError::Db(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::future::ready;

    use super::*;

    /// Every test opens `mem://` — a fresh, empty store per call, reached
    /// through the identical `Surreal<Any>` + `open()` path the worker uses
    /// for `indxdb://diary`. That sameness is what these tests certify.
    async fn store() -> Db {
        open("mem://").await.expect("mem engine opens")
    }

    #[tokio::test]
    async fn schema_reconciles_twice() {
        let db = store();
        let db = db.await;
        db.query(SCHEMA).await.unwrap().check().unwrap();
        assert!(entries(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn enqueue_normalizes_and_round_trips() {
        let db = store().await;
        let queued = enqueue(&db, 1_753_640_000, "Dear diary,\r\nIt me.\r\n", 7)
            .await
            .unwrap();
        assert_eq!(queued.body, "Dear diary,\nIt me.");
        assert_eq!(queued.state, STATE_PENDING);
        let listed = entries(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        // The explicit projection must surface the absent `reason` as None
        // instead of dropping the field (the `SELECT *` NONE-omission trap).
        assert_eq!(listed[0].reason, None);
        assert_eq!(listed[0].qid, queued.qid);
        assert_eq!(listed[0].written_at, 1_753_640_000);
        assert_eq!(listed[0].enqueued_at, 7);
    }

    #[tokio::test]
    async fn enqueue_refuses_only_empty_text() {
        let db = store().await;
        for bad in ["", "  \r\n\t "] {
            assert!(matches!(
                enqueue(&db, 1, bad, 1).await,
                Err(OutboxError::InvalidBody)
            ));
        }
        assert!(entries(&db).await.unwrap().is_empty());
    }

    /// Over-length text must QUEUE (the server's 422 marks it failed with
    /// its text preserved) — refusing it here would bounce the entry into
    /// the lossy form-POST fallback, and offline that means silent loss.
    #[tokio::test]
    async fn enqueue_accepts_over_length_text_for_the_server_to_judge() {
        use crate::contract::MAX_ENTRY_CHARS;
        let db = store().await;
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        enqueue(&db, 100, &oversized, 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Rejected(422)))
            .await
            .unwrap();
        assert_eq!(report.failed, 1);
        let left = entries(&db).await.unwrap();
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].body, oversized, "failed text must survive intact");
    }

    #[tokio::test]
    async fn discard_removes_and_stays_idempotent() {
        let db = store().await;
        let queued = enqueue(&db, 1, "keep me", 1).await.unwrap();
        remove(&db, &queued.qid).await.unwrap();
        remove(&db, &queued.qid).await.unwrap();
        assert!(entries(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn flush_sends_oldest_first_and_empties_the_queue() {
        let db = store().await;
        // Enqueued out of order on purpose; enqueued_at decides.
        enqueue(&db, 200, "second", 20).await.unwrap();
        enqueue(&db, 100, "first", 10).await.unwrap();
        enqueue(&db, 300, "third", 30).await.unwrap();
        let sent = RefCell::new(Vec::new());
        let report = flush(&db, |wire: WireEntry| {
            sent.borrow_mut().push(wire.body.clone());
            ready(SendOutcome::Saved)
        })
        .await
        .unwrap();
        assert_eq!(*sent.borrow(), ["first", "second", "third"]);
        assert_eq!(
            report,
            FlushReport {
                saved: 3,
                pending: 0,
                failed: 0,
                blocked: None
            }
        );
        assert!(entries(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn flush_stops_on_retryable_trouble() {
        let db = store().await;
        enqueue(&db, 100, "first", 10).await.unwrap();
        enqueue(&db, 200, "second", 20).await.unwrap();
        enqueue(&db, 300, "third", 30).await.unwrap();
        let script = RefCell::new(VecDeque::from([SendOutcome::Saved, SendOutcome::Retry]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().expect("third never sent");
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!(
            report,
            FlushReport {
                saved: 1,
                pending: 2,
                failed: 0,
                blocked: Some(Blocked::Net)
            }
        );
        // The stopped entries survive, order intact, for the next flush.
        let left: Vec<String> = entries(&db)
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
        enqueue(&db, 100, "private", 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Auth)).await.unwrap();
        assert_eq!(report.blocked, Some(Blocked::Auth));
        assert_eq!(report.pending, 1);
        assert_eq!(entries(&db).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn flush_marks_rejections_failed_and_moves_on() {
        let db = store().await;
        enqueue(&db, 100, "rejected", 10).await.unwrap();
        enqueue(&db, 200, "accepted", 20).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            SendOutcome::Rejected(422),
            SendOutcome::Saved,
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().unwrap();
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!(
            report,
            FlushReport {
                saved: 1,
                pending: 0,
                failed: 1,
                blocked: None
            }
        );
        let left = entries(&db).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].reason.as_deref(), Some("rejected (HTTP 422)"));
        // A failed entry is kept for manual copy, never re-sent.
        let report = flush(&db, |_| ready(SendOutcome::Saved)).await.unwrap();
        assert_eq!(report.saved, 0);
        assert_eq!(entries(&db).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn import_preserves_and_dedupes() {
        let db = store().await;
        let legacy = vec![
            LegacyEntry {
                written_at: 100,
                body: "old pending".to_string(),
                state: Some(STATE_PENDING.to_string()),
                reason: None,
                enqueued_at: Some(1),
            },
            LegacyEntry {
                written_at: 200,
                body: "old failure".to_string(),
                state: Some(STATE_FAILED.to_string()),
                reason: Some("rejected (HTTP 422)".to_string()),
                enqueued_at: Some(2),
            },
        ];
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        // A crash between import and the caller's delete replays the whole
        // batch; the (written_at, body) twin check absorbs it.
        assert_eq!(import(&db, &legacy).await.unwrap(), 0);
        let listed = entries(&db).await.unwrap();
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
            {"qid": 3, "written_at": 100, "body": "with qid", "state": "pending",
             "reason": null, "enqueued_at": 1753640000000},
            {"written_at": 200, "body": "bare"}
        ]"#;
        let legacy: Vec<LegacyEntry> = serde_json::from_str(raw).unwrap();
        let db = store().await;
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        let listed = entries(&db).await.unwrap();
        assert_eq!(listed[0].body, "bare"); // enqueued_at defaults to 0
        assert_eq!(listed[1].body, "with qid");
        assert_eq!(listed[1].state, STATE_PENDING);
    }

    /// The report is the BroadcastChannel message the page has always read.
    #[test]
    fn reports_serialize_in_the_broadcast_shape() {
        let report = FlushReport {
            saved: 2,
            pending: 1,
            failed: 0,
            blocked: Some(Blocked::Net),
        };
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"saved":2,"pending":1,"failed":0,"blocked":"net"}"#
        );
        let quiet = FlushReport {
            saved: 0,
            pending: 0,
            failed: 0,
            blocked: None,
        };
        assert_eq!(
            serde_json::to_string(&quiet).unwrap(),
            r#"{"saved":0,"pending":0,"failed":0,"blocked":null}"#
        );
    }
}
