//! Sync: one flush-then-pull pass between the device store and a remote.
//!
//! [`Remote`] is the transport seam — two methods, deliberately NO `Send`
//! bounds (the wasm impl wraps browser `fetch`, whose futures are `!Send`;
//! see `outbox::flush`'s identical rule). The worker's HTTP impl lives in
//! `crates/diary-worker`; native tests implement it over a second
//! `Surreal<Any>` store, which is also exactly the shape a direct
//! client→SurrealDB transport takes later.
//!
//! [`run`] must execute as ONE critical section per device (the service
//! worker holds the `diary-flush` Web Lock across it): the pull's snapshot
//! is then always newer than the last push, so a stale dump can never
//! delete a freshly delivered row. [`apply_pull`] additionally guards every
//! statement on `state = 'synced'` — a pending or failed row is NEVER
//! touched by a pull, even at the same id; the pending row wins and
//! converges after its own flush.

use crate::Db;
use crate::contract::{PullOutcome, SendOutcome, SnapshotEntry, WireEntry};
use crate::outbox::{self, Blocked, FlushReport, LocalEntry, OutboxError, STATE_SYNCED};

/// The transport to one remote diary. Implementations classify their own
/// failures ([`PullOutcome`] / [`SendOutcome`]); nothing here retries — the
/// caller's next kick is the retry.
pub trait Remote {
    fn push(&self, entry: WireEntry) -> impl Future<Output = SendOutcome>;
    fn pull(&self) -> impl Future<Output = PullOutcome>;
}

/// Flush every pending entry, then pull the mirror up to date. Pull is
/// skipped when the flush stopped on auth (the pull would only 401 too);
/// a pull classified Auth/Retry is a silent no-op on the mirror — the
/// report's `pulled` stays `None`.
pub async fn run<R: Remote>(db: &Db, remote: &R) -> Result<FlushReport, OutboxError> {
    let mut report = outbox::flush(db, |entry| remote.push(entry)).await?;
    if report.blocked != Some(Blocked::Auth)
        && let PullOutcome::Data(entries) = remote.pull().await
    {
        report.pulled = Some(apply_pull(db, &entries).await?);
    }
    Ok(report)
}

/// Make the local mirror agree with a snapshot: create missing rows as
/// synced, update synced rows the server changed, delete synced rows the
/// server no longer has. The diff is computed in Rust and applied as one
/// small transaction (usually empty or a handful of statements; the
/// first-ever hydration is the one large batch). Deletes are one `=`-shaped
/// record delete each, per docs/surrealdb-notes.md.
///
/// Returns how many rows changed.
pub async fn apply_pull(db: &Db, incoming: &[SnapshotEntry]) -> Result<u32, OutboxError> {
    // Same-record write races (the page enqueueing while this runs) abort
    // one side with a retryable conflict; re-read and re-apply, per the
    // reference handling in the site's analytics module.
    let mut last_error = None;
    for _ in 0..3 {
        match apply_once(db, incoming).await {
            Err(OutboxError::Db(message)) if message.contains("Resource busy") => {
                last_error = Some(OutboxError::Db(message));
            }
            other => return other,
        }
    }
    Err(last_error.expect("loop only exits early or stores an error"))
}

async fn apply_once(db: &Db, incoming: &[SnapshotEntry]) -> Result<u32, OutboxError> {
    let local = outbox::all_local(db).await?;
    let plan = diff(&local, incoming);
    let changes = (plan.creates.len() + plan.updates.len() + plan.deletes.len()) as u32;
    if changes == 0 {
        return Ok(0);
    }

    let mut statements = String::from("BEGIN TRANSACTION;\n");
    let mut string_binds: Vec<(String, String)> = Vec::new();
    let mut int_binds: Vec<(String, i64)> = Vec::new();
    for (index, entry) in plan.creates.iter().enumerate() {
        statements.push_str(&format!(
            "CREATE ONLY type::record('diary_entries', $c{index}_id) \
             SET written_at = $c{index}_wa, body = $c{index}_body, \
             state = 'synced', enqueued_at = 0;\n"
        ));
        string_binds.push((format!("c{index}_id"), entry.id.clone()));
        int_binds.push((format!("c{index}_wa"), entry.written_at));
        string_binds.push((format!("c{index}_body"), entry.body.clone()));
    }
    for (index, entry) in plan.updates.iter().enumerate() {
        statements.push_str(&format!(
            "UPDATE type::record('diary_entries', $u{index}_id) \
             SET written_at = $u{index}_wa, body = $u{index}_body \
             WHERE state = 'synced';\n"
        ));
        string_binds.push((format!("u{index}_id"), entry.id.clone()));
        int_binds.push((format!("u{index}_wa"), entry.written_at));
        string_binds.push((format!("u{index}_body"), entry.body.clone()));
    }
    for (index, id) in plan.deletes.iter().enumerate() {
        statements.push_str(&format!(
            "DELETE type::record('diary_entries', $d{index}_id) WHERE state = 'synced';\n"
        ));
        string_binds.push((format!("d{index}_id"), id.clone()));
    }
    statements.push_str("COMMIT TRANSACTION;");

    let mut query = db.query(statements);
    for (name, value) in string_binds {
        query = query.bind((name, value));
    }
    for (name, value) in int_binds {
        query = query.bind((name, value));
    }
    query
        .await
        .map_err(|error| OutboxError::Db(error.to_string()))?
        .check()
        .map_err(|error| OutboxError::Db(error.to_string()))?;
    Ok(changes)
}

struct PullPlan<'a> {
    creates: Vec<&'a SnapshotEntry>,
    updates: Vec<&'a SnapshotEntry>,
    deletes: Vec<String>,
}

fn diff<'a>(local: &[LocalEntry], incoming: &'a [SnapshotEntry]) -> PullPlan<'a> {
    use std::collections::HashMap;
    let by_id: HashMap<&str, &LocalEntry> =
        local.iter().map(|row| (row.id.as_str(), row)).collect();
    let mut incoming_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut creates = Vec::new();
    let mut updates = Vec::new();
    for entry in incoming {
        incoming_ids.insert(entry.id.as_str());
        match by_id.get(entry.id.as_str()) {
            None => creates.push(entry),
            Some(row) if row.state == STATE_SYNCED => {
                if row.written_at != entry.written_at || row.body != entry.body {
                    updates.push(entry);
                }
            }
            // Pending/failed rows win; their own flush converges them.
            Some(_) => {}
        }
    }
    let deletes = local
        .iter()
        .filter(|row| row.state == STATE_SYNCED && !incoming_ids.contains(row.id.as_str()))
        .map(|row| row.id.clone())
        .collect();
    PullPlan {
        creates,
        updates,
        deletes,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::contract::{SavedRef, classify_pull};
    use crate::store;

    use super::*;

    async fn device() -> Db {
        outbox::open("mem://").await.expect("mem device opens")
    }

    /// A real second store standing in for the server — pushes run the
    /// actual `store::save_entry` probe, pulls dump `store::all_entries`.
    /// This is byte-for-byte the shape a direct client→SurrealDB transport
    /// has, which is the point of the trait.
    struct TestServer {
        db: Db,
        pulls: RefCell<u32>,
        pushes: RefCell<u32>,
    }

    impl TestServer {
        async fn start() -> TestServer {
            let db = surrealdb::engine::any::connect("mem://")
                .await
                .expect("mem server opens");
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
            TestServer {
                db,
                pulls: RefCell::new(0),
                pushes: RefCell::new(0),
            }
        }
    }

    impl Remote for TestServer {
        async fn push(&self, entry: WireEntry) -> SendOutcome {
            *self.pushes.borrow_mut() += 1;
            match store::save_entry(&self.db, entry.written_at, &entry.body).await {
                Ok(saved) => SendOutcome::Saved(SavedRef {
                    id: saved.id,
                    written_at: saved.written_at,
                }),
                Err(_) => SendOutcome::Retry,
            }
        }

        async fn pull(&self) -> PullOutcome {
            *self.pulls.borrow_mut() += 1;
            match store::all_entries(&self.db).await {
                Ok(entries) => PullOutcome::Data(
                    entries
                        .into_iter()
                        .map(|entry| SnapshotEntry {
                            id: entry.id,
                            written_at: entry.written_at,
                            body: entry.body,
                        })
                        .collect(),
                ),
                Err(_) => PullOutcome::Retry,
            }
        }
    }

    /// A scripted remote for outcome-specific walks.
    struct Scripted {
        push_outcome: SendOutcome,
        pull_outcome: PullOutcome,
        pulls: RefCell<u32>,
    }

    impl Remote for Scripted {
        async fn push(&self, _entry: WireEntry) -> SendOutcome {
            self.push_outcome.clone()
        }
        async fn pull(&self) -> PullOutcome {
            *self.pulls.borrow_mut() += 1;
            self.pull_outcome.clone()
        }
    }

    fn snap(id: &str, written_at: i64, body: &str) -> SnapshotEntry {
        SnapshotEntry {
            id: id.to_string(),
            written_at,
            body: body.to_string(),
        }
    }

    #[test]
    fn pulls_classify_like_pushes() {
        assert_eq!(
            classify_pull(200, r#"{"entries":[{"id":"x","written_at":1,"body":"b"}]}"#),
            PullOutcome::Data(vec![snap("x", 1, "b")])
        );
        assert_eq!(
            classify_pull(200, r#"{"entries":[]}"#),
            PullOutcome::Data(Vec::new())
        );
        // A 200 that is not our exact JSON is a captive portal, never an
        // empty diary.
        assert_eq!(
            classify_pull(200, "<html>hotel wifi</html>"),
            PullOutcome::Retry
        );
        assert_eq!(classify_pull(200, ""), PullOutcome::Retry);
        assert_eq!(
            classify_pull(200, r#"{"entries":[{"id":"x"}]}"#),
            PullOutcome::Retry
        );
        assert_eq!(
            classify_pull(
                200,
                r#"{"entries":[{"id":"x","written_at":1,"body":"b","extra":1}]}"#
            ),
            PullOutcome::Retry
        );
        assert_eq!(classify_pull(401, ""), PullOutcome::Auth);
        assert_eq!(classify_pull(404, ""), PullOutcome::Auth);
        for retryable in [403, 500, 502, 503] {
            assert_eq!(classify_pull(retryable, ""), PullOutcome::Retry);
        }
    }

    #[tokio::test]
    async fn apply_creates_updates_and_deletes_only_synced_rows() {
        let db = device().await;
        // Local state: one pending, one failed, two synced.
        let pending = outbox::enqueue(&db, 1_753_640_000, "pending text", 1)
            .await
            .unwrap();
        let report = outbox::flush(&db, |_| std::future::ready(SendOutcome::Rejected(422)))
            .await
            .unwrap();
        assert_eq!(report.failed, 1); // the pending row became our failed row
        let failed_id = pending.id.clone();
        let kept = outbox::enqueue(&db, 1_753_640_100, "kept synced", 2)
            .await
            .unwrap();
        let dropped = outbox::enqueue(&db, 1_753_640_200, "dropped synced", 3)
            .await
            .unwrap();
        outbox::flush(&db, |wire: WireEntry| {
            let id = store::entry_key(wire.written_at).unwrap();
            std::future::ready(SendOutcome::Saved(SavedRef {
                id,
                written_at: wire.written_at,
            }))
        })
        .await
        .unwrap();
        let still_pending = outbox::enqueue(&db, 1_753_640_300, "still pending", 4)
            .await
            .unwrap();

        // The server: has `kept` (body edited server-side), a brand-new row,
        // does NOT have `dropped`, and — adversarially — claims a row at the
        // failed id and at the pending id (must not be touched).
        let incoming = vec![
            snap(&kept.id, kept.written_at, "kept synced, edited"),
            snap(
                "2026-01-01T07-00-00-05-00",
                1_767_250_800,
                "from another device",
            ),
            snap(&failed_id, 1_753_640_000, "server version of failed"),
            snap(
                &still_pending.id,
                still_pending.written_at,
                "server twin of pending",
            ),
        ];
        let changed = apply_pull(&db, &incoming).await.unwrap();
        assert_eq!(changed, 3, "create + update + delete");

        let rows = outbox::all_local(&db).await.unwrap();
        let by_id: std::collections::HashMap<&str, &LocalEntry> =
            rows.iter().map(|row| (row.id.as_str(), row)).collect();
        assert_eq!(by_id[kept.id.as_str()].body, "kept synced, edited");
        assert_eq!(
            by_id["2026-01-01T07-00-00-05-00"].state, STATE_SYNCED,
            "new server row materialized synced"
        );
        assert!(
            !by_id.contains_key(dropped.id.as_str()),
            "server delete propagated"
        );
        assert_eq!(
            by_id[failed_id.as_str()].body,
            "pending text",
            "failed row untouched"
        );
        assert_eq!(by_id[failed_id.as_str()].state, "failed");
        assert_eq!(
            by_id[still_pending.id.as_str()].body,
            "still pending",
            "pending row untouched even at a matching id"
        );
        // Idempotent: the same snapshot again changes only what still
        // differs (the failed/pending guards keep refusing).
        let changed = apply_pull(&db, &incoming).await.unwrap();
        assert_eq!(changed, 0);
    }

    #[tokio::test]
    async fn run_skips_pull_when_auth_blocked_and_reports_it_otherwise() {
        let db = device().await;
        outbox::enqueue(&db, 1_753_640_000, "blocked", 1)
            .await
            .unwrap();
        let auth_remote = Scripted {
            push_outcome: SendOutcome::Auth,
            pull_outcome: PullOutcome::Data(Vec::new()),
            pulls: RefCell::new(0),
        };
        let report = run(&db, &auth_remote).await.unwrap();
        assert_eq!(report.blocked, Some(Blocked::Auth));
        assert_eq!(report.pulled, None);
        assert_eq!(*auth_remote.pulls.borrow(), 0, "no pull while signed out");

        // A retryable pull is a silent no-op: mirror untouched, pulled None.
        let net_remote = Scripted {
            push_outcome: SendOutcome::Retry,
            pull_outcome: PullOutcome::Retry,
            pulls: RefCell::new(0),
        };
        let report = run(&db, &net_remote).await.unwrap();
        assert_eq!(report.blocked, Some(Blocked::Net));
        assert_eq!(report.pulled, None);
        assert_eq!(*net_remote.pulls.borrow(), 1);
        assert_eq!(outbox::queued(&db).await.unwrap().len(), 1);
    }

    /// The plan's two-device walk, end to end against a REAL server store:
    /// both devices write different bodies at the same second while
    /// offline; the second flush gets bumped by the server's probe; both
    /// mirrors converge to the server exactly, no text lost.
    #[tokio::test]
    async fn two_devices_same_second_converge() {
        let server = TestServer::start().await;
        let device_a = device().await;
        let device_b = device().await;
        let second = 1_753_640_000;

        let entry_a = outbox::enqueue(&device_a, second, "from device A", 1)
            .await
            .unwrap();
        let entry_b = outbox::enqueue(&device_b, second, "from device B", 1)
            .await
            .unwrap();
        assert_eq!(entry_a.id, entry_b.id, "both predicted the same key");

        let report_a = run(&device_a, &server).await.unwrap();
        assert_eq!(report_a.saved, 1);
        assert_eq!(report_a.saved_entries[0].id, entry_a.id, "A kept its key");

        let report_b = run(&device_b, &server).await.unwrap();
        assert_eq!(report_b.saved, 1);
        let bumped_id = report_b.saved_entries[0].id.clone();
        assert_ne!(bumped_id, entry_b.id, "B got bumped a second forward");

        // B's mirror now holds BOTH entries under the server's identities.
        let report_a2 = run(&device_a, &server).await.unwrap();
        assert_eq!(report_a2.saved, 0);
        for device_db in [&device_a, &device_b] {
            let rows = outbox::all_local(device_db).await.unwrap();
            let mut ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
            ids.sort_unstable();
            let server_rows = store::all_entries(&server.db).await.unwrap();
            let mut server_ids: Vec<&str> = server_rows.iter().map(|row| row.id.as_str()).collect();
            server_ids.sort_unstable();
            assert_eq!(ids, server_ids, "mirror matches server");
            assert!(rows.iter().all(|row| row.state == STATE_SYNCED));
            let bodies: std::collections::BTreeSet<&str> =
                rows.iter().map(|row| row.body.as_str()).collect();
            assert!(bodies.contains("from device A"));
            assert!(bodies.contains("from device B"), "no text lost");
        }

        // A server-side delete propagates on the next pass.
        store::remove_entry(&server.db, &bumped_id).await.unwrap();
        let report = run(&device_a, &server).await.unwrap();
        assert_eq!(report.pulled, Some(1));
        let rows = outbox::all_local(&device_a).await.unwrap();
        assert!(rows.iter().all(|row| row.id != bumped_id));
    }
}
