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
//! worker holds the `diary-store` Web Lock across it): the pull's snapshot
//! is then always newer than the last push, so a stale dump can never
//! delete a freshly delivered row. The device store owns the local
//! reconciliation details; this module only sequences its [`Remote`].

use crate::Db;
use crate::contract::{PullOutcome, SendOutcome};
use crate::entry::{ComposedEntry, SavedRef};
use crate::outbox::{self, Blocked, FlushReport, OutboxError};

/// The transport to one remote diary. Implementations classify their own
/// failures ([`PullOutcome`] / [`SendOutcome`]); nothing here retries — the
/// caller's next kick is the retry.
pub trait Remote {
    fn push(&self, entry: ComposedEntry) -> impl Future<Output = SendOutcome>;
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
        report.pulled = Some(outbox::reconcile(db, &entries).await?);
    }
    Ok(report)
}

/// The direct transport: the remote IS a `Surreal<Any>` handle — connected
/// and `authenticate()`d by the caller (natively in tests over `mem://`;
/// on wasm over the WebSocket engine with a minted record-access token).
/// Push runs the SAME probe-and-dedupe `store::save_entry` the server's
/// replay endpoint runs — one algorithm, two engines — and pull is the same
/// snapshot read the endpoint serves. This is the isomorphism the crate
/// exists for.
pub struct DirectRemote {
    db: Db,
    validation_now: i64,
}

impl DirectRemote {
    /// One direct remote is short-lived for one sync pass. Its validation
    /// instant comes from the authenticated server connection, never the
    /// composing device's potentially skewed clock.
    pub fn new(db: Db, validation_now: i64) -> Self {
        Self { db, validation_now }
    }
}

impl Remote for DirectRemote {
    async fn push(&self, entry: ComposedEntry) -> SendOutcome {
        match crate::store::save_entry(&self.db, entry, self.validation_now).await {
            Ok(saved) => SendOutcome::Saved(SavedRef::from(&saved.entry)),
            Err(crate::store::SaveError::Rejected(rejection)) => {
                SendOutcome::Rejected(rejection.status_code())
            }
            // The server replay endpoint answers probe exhaustion with 409;
            // classify identically so the entry lands failed, not retried
            // forever.
            Err(crate::store::SaveError::Exhausted) => SendOutcome::Rejected(409),
            Err(crate::store::SaveError::Store(error)) => {
                if is_auth_error(&error) {
                    SendOutcome::Auth
                } else {
                    SendOutcome::Retry
                }
            }
        }
    }

    async fn pull(&self) -> PullOutcome {
        match crate::store::all_entries(&self.db).await {
            Ok(entries) => PullOutcome::Data(entries),
            Err(error) if is_auth_error(&error) => PullOutcome::Auth,
            Err(_) => PullOutcome::Retry,
        }
    }
}

/// The SDK flattens auth failures to strings (like the conflict retry in
/// docs/surrealdb-notes.md, matching them is upstream's own pattern). Keep
/// the net wide enough for expired sessions and missing permissions, narrow
/// enough that transport noise still classifies Retry.
fn is_auth_error(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    lowered.contains("authent") // "authentication", "not authenticated"
        || lowered.contains("session has expired")
        || lowered.contains("not enough permissions")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::contract::{SnapshotWire, classify_pull};
    use crate::entry::{DiaryEntry, SavedRef};
    use crate::outbox::{LocalEntry, STATE_SYNCED};
    use crate::store;

    use super::*;

    async fn device() -> Db {
        outbox::open("mem://").await.expect("mem device opens")
    }

    async fn enqueue(
        db: &Db,
        written_at: i64,
        body: &str,
        enqueued_at: i64,
    ) -> Result<LocalEntry, OutboxError> {
        outbox::enqueue(db, ComposedEntry::new(written_at, body), enqueued_at).await
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
            db.query(store::TEST_SCHEMA)
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
        async fn push(&self, entry: ComposedEntry) -> SendOutcome {
            *self.pushes.borrow_mut() += 1;
            match store::save_entry(&self.db, entry.clone(), entry.written_at).await {
                Ok(saved) => SendOutcome::Saved(SavedRef::from(&saved.entry)),
                Err(_) => SendOutcome::Retry,
            }
        }

        async fn pull(&self) -> PullOutcome {
            *self.pulls.borrow_mut() += 1;
            match store::all_entries(&self.db).await {
                Ok(entries) => PullOutcome::Data(entries),
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
        async fn push(&self, _entry: ComposedEntry) -> SendOutcome {
            self.push_outcome.clone()
        }
        async fn pull(&self) -> PullOutcome {
            *self.pulls.borrow_mut() += 1;
            self.pull_outcome.clone()
        }
    }

    fn snap(id: &str, written_at: i64, body: &str) -> DiaryEntry {
        DiaryEntry::from_parts(id, written_at, body)
    }

    #[test]
    fn pulls_classify_like_pushes() {
        let populated = serde_json::to_string(&SnapshotWire::new(vec![snap("x", 1, "b")])).unwrap();
        assert_eq!(
            classify_pull(200, &populated),
            PullOutcome::Data(vec![snap("x", 1, "b")])
        );
        let empty = serde_json::to_string(&SnapshotWire::new(Vec::new())).unwrap();
        assert_eq!(classify_pull(200, &empty), PullOutcome::Data(Vec::new()));
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
        assert_eq!(
            classify_pull(200, r#"{"schema_epoch":99,"entries":[]}"#),
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
        let pending = enqueue(&db, 1_753_640_000, "pending text", 1)
            .await
            .unwrap();
        let report = outbox::flush(&db, |_| std::future::ready(SendOutcome::Rejected(422)))
            .await
            .unwrap();
        assert_eq!(report.failed, 1); // the pending row became our failed row
        let failed_id = pending.id.clone();
        let kept = enqueue(&db, 1_753_640_100, "kept synced", 2).await.unwrap();
        let dropped = enqueue(&db, 1_753_640_200, "dropped synced", 3)
            .await
            .unwrap();
        outbox::flush(&db, |wire: ComposedEntry| {
            let id = store::entry_key(wire.written_at).unwrap();
            std::future::ready(SendOutcome::Saved(SavedRef {
                id,
                written_at: wire.written_at,
            }))
        })
        .await
        .unwrap();
        let still_pending = enqueue(&db, 1_753_640_300, "still pending", 4)
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
        let changed = outbox::reconcile(&db, &incoming).await.unwrap();
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
        let changed = outbox::reconcile(&db, &incoming).await.unwrap();
        assert_eq!(changed, 0);
    }

    #[tokio::test]
    async fn run_skips_pull_when_auth_blocked_and_reports_it_otherwise() {
        let db = device().await;
        enqueue(&db, 1_753_640_000, "blocked", 1).await.unwrap();
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

    /// The direct transport, store to store: the remote IS another
    /// `Surreal<Any>` — the exact wasm shape minus `authenticate()`, which
    /// only the engine cares about. Push runs the real probe, pull the real
    /// snapshot; deletes propagate.
    #[tokio::test]
    async fn direct_remote_syncs_store_to_store() {
        let server = TestServer::start().await;
        let remote = DirectRemote::new(server.db.clone(), 1_753_640_000);
        let device_db = device().await;
        enqueue(&device_db, 1_753_640_000, "straight to the database", 1)
            .await
            .unwrap();
        enqueue(&device_db, 1_753_640_060, "second entry", 2)
            .await
            .unwrap();
        let report = run(&device_db, &remote).await.unwrap();
        assert_eq!(report.saved, 2);
        assert_eq!(report.blocked, None);
        let server_rows = store::all_entries(&server.db).await.unwrap();
        assert_eq!(server_rows.len(), 2);
        // A server-side delete propagates through the same handle. (Only
        // one of two: an EMPTY snapshot deliberately never wipes a
        // populated mirror — the guard has its own test.)
        store::remove_entry(&server.db, &server_rows[0].id)
            .await
            .unwrap();
        let report = run(&device_db, &remote).await.unwrap();
        assert_eq!(report.pulled, Some(1));
        let rows = outbox::all_local(&device_db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "second entry");
    }

    #[tokio::test]
    async fn direct_remote_rejects_invalid_entries_like_http() {
        use crate::entry::{MAX_ENTRY_CHARS, MAX_PAST_SECONDS};

        let server = TestServer::start().await;
        let now = 1_753_640_000;
        let remote = DirectRemote::new(server.db.clone(), now);
        assert_eq!(
            remote
                .push(ComposedEntry::new(now, "x".repeat(MAX_ENTRY_CHARS + 1)))
                .await,
            SendOutcome::Rejected(422)
        );
        assert_eq!(
            remote
                .push(ComposedEntry::new(now - MAX_PAST_SECONDS, "too old"))
                .await,
            SendOutcome::Rejected(422)
        );
        assert_eq!(
            remote
                .push(
                    ComposedEntry::new(now, "bad parent")
                        .with_reply_to(Some("not-a-diary-id".to_string())),
                )
                .await,
            SendOutcome::Rejected(422)
        );
        assert!(store::all_entries(&server.db).await.unwrap().is_empty());

        let saved = remote
            .push(ComposedEntry::new(now, "  normalized\r\nbody  "))
            .await;
        assert!(matches!(saved, SendOutcome::Saved(_)));
        assert_eq!(
            store::all_entries(&server.db).await.unwrap()[0].body,
            "normalized\nbody"
        );
    }

    /// The wipe guard: a silently-deauthed session's pull (SurrealDB
    /// filters denied reads to empty instead of erroring) must never empty
    /// a populated mirror; a genuinely emptied server converges again from
    /// the next non-empty snapshot.
    #[tokio::test]
    async fn an_empty_snapshot_never_wipes_a_populated_mirror() {
        let db = device().await;
        enqueue(&db, 1_753_640_000, "history", 1).await.unwrap();
        outbox::flush(&db, |wire: ComposedEntry| {
            let id = store::entry_key(wire.written_at).unwrap();
            std::future::ready(SendOutcome::Saved(SavedRef {
                id,
                written_at: wire.written_at,
            }))
        })
        .await
        .unwrap();
        assert_eq!(outbox::reconcile(&db, &[]).await.unwrap(), 0);
        assert_eq!(
            outbox::all_local(&db).await.unwrap().len(),
            1,
            "mirror intact"
        );
        // A later non-empty snapshot's deletes still apply — convergence
        // is deferred, not lost.
        let replacement = snap("2026-01-01T07-00-00-05-00", 1_767_250_800, "only survivor");
        assert_eq!(outbox::reconcile(&db, &[replacement]).await.unwrap(), 2);
        let rows = outbox::all_local(&db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "only survivor");
    }

    #[test]
    fn auth_errors_classify_as_auth_everything_else_retries() {
        assert!(is_auth_error("There was a problem with authentication"));
        assert!(is_auth_error("The session has expired"));
        assert!(is_auth_error(
            "Not enough permissions to perform this action"
        ));
        assert!(!is_auth_error("Connection refused (os error 111)"));
        assert!(!is_auth_error(
            "There was a problem with the key-value store: Transaction conflict"
        ));
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

        let entry_a = enqueue(&device_a, second, "from device A", 1)
            .await
            .unwrap();
        let entry_b = enqueue(&device_b, second, "from device B", 1)
            .await
            .unwrap();
        assert_eq!(entry_a.id, entry_b.id, "both predicted the same key");

        let report_a = run(&device_a, &server).await.unwrap();
        assert_eq!(report_a.saved, 1);
        assert_eq!(
            report_a.saved_refs[0].saved.id, entry_a.id,
            "A kept its key"
        );

        let report_b = run(&device_b, &server).await.unwrap();
        assert_eq!(report_b.saved, 1);
        let bumped_id = report_b.saved_refs[0].saved.id.clone();
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
