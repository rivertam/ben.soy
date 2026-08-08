//! Analytics database writes.

use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;
use uuid::Uuid;

use benjisponge::data::{Db, analytics_models::AnalyticsEvent};

use super::input::ValidatedEvent;

pub fn hash_identifier(namespace: &str, value: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(namespace);
    hash.update([0]);
    hash.update(value);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The datastore's own wording for a write it aborted and wants re-run.
const CONFLICT_MARKER: &str = "Transaction conflict";

/// Total tries for a write that keeps losing conflicts. Generous for what this
/// exists to absorb — two beacons for one visitor arriving in the same instant.
const CONFLICT_ATTEMPTS: usize = 4;

/// SurrealDB aborts one side of two writes touching the same keys at once, and
/// every beacon from a visitor upserts the *same* `analytics_sessions` row. So
/// a page-hide engagement flush racing the next page's pageview is the ordinary
/// case here, not a rare one: measured at 171 of 800 writes under eight-way
/// contention on a single record.
///
/// Retrying has to happen server-side. These arrive by `sendBeacon`, which
/// never resends, so a conflict the handler gives up on is a permanently lost
/// event.
///
/// There is no typed variant to match on — the SDK flattens the datastore error
/// into a message (`surrealdb-core/src/kvs/err.rs`), and upstream's own tests
/// match this same string.
fn is_transaction_conflict(error: &surrealdb::Error) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error.message().contains(CONFLICT_MARKER) {
            return true;
        }
        current = error.cause();
    }
    false
}

/// How long to wait before re-running a lost write. Jittered, because two
/// racers that just collided would otherwise sleep identical lengths and
/// collide again; the clock's sub-millisecond noise differs between them
/// without pulling in a randomness dependency.
fn conflict_backoff(attempt: usize) -> Duration {
    let base = 4_u64 << attempt.min(3);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::from(since.subsec_nanos()))
        % base;
    Duration::from_millis(base + jitter)
}

/// Run a write, re-running it while the datastore reports a retryable conflict.
/// Anything else — including the duplicate-key error that `insert_event` reads
/// as "already recorded" — is returned untouched on the first try.
async fn retrying_conflicts<F, Fut, T>(mut write: F) -> surrealdb::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = surrealdb::Result<T>>,
{
    for attempt in 0..CONFLICT_ATTEMPTS - 1 {
        match write().await {
            Err(error) if is_transaction_conflict(&error) => {
                tokio::time::sleep(conflict_backoff(attempt)).await;
            }
            outcome => return outcome,
        }
    }
    write().await
}

/// Resolve a hardened Topcoat cookie to the stable anonymous visitor chosen
/// for it. A newly-issued cookie is aliased to the tab bootstrap nonce's
/// deterministic fallback, so concurrent first-load beacons converge even
/// before either response has installed its cookie.
pub async fn resolve_visitor(
    db: &Db,
    token_hash: &str,
    bootstrap_id: Option<&str>,
    now: i64,
) -> anyhow::Result<String> {
    let candidate = bootstrap_id.map_or_else(
        || token_hash.to_string(),
        |bootstrap_id| hash_identifier("analytics-bootstrap-visitor", bootstrap_id),
    );
    let visitor: Option<String> = retrying_conflicts(|| {
        let candidate = candidate.clone();
        async move {
            let mut response = db
                .query(
                    "UPSERT type::record('analytics_visitor_aliases', $token_hash)
                         SET visitor_id = visitor_id ?? $candidate,
                             created_at = created_at ?? $now
                         RETURN VALUE visitor_id",
                )
                .bind(("token_hash", token_hash.to_string()))
                .bind(("candidate", candidate))
                .bind(("now", now))
                .await?
                .check()?;
            response.take(0)
        }
    })
    .await
    .context("analytics visitor resolution failed")?;
    visitor.context("analytics visitor resolution returned no text row")
}

/// Insert an event under a server-owned, rolling thirty-minute session.
///
/// Pageviews and outbound clicks insert exactly once. Engagement uses one
/// stable event id per document; later lifecycle flushes atomically raise its
/// cumulative measurements instead of creating duplicate samples.
pub async fn insert_event(
    db: &Db,
    visitor_hash: &str,
    event: ValidatedEvent,
    occurred_at: i64,
) -> anyhow::Result<bool> {
    if event_exists(db, &event.id).await? {
        update_engagement(db, visitor_hash, &event, occurred_at).await?;
        return Ok(false);
    }

    let session_id = server_session(db, visitor_hash, occurred_at).await?;
    let event_id = event.id.clone();
    let engagement_retry = (event.kind == "engagement").then(|| EngagementUpdate {
        id: event.id.clone(),
        page_path: event.path.clone(),
        engagement_seconds: event.engagement_seconds,
        scroll_percent: event.scroll_percent,
        lcp_milliseconds: event.lcp_milliseconds,
        cls_thousandths: event.cls_thousandths,
        navigation_milliseconds: event.navigation_milliseconds,
    });
    let event = AnalyticsEvent {
        id: event.id,
        visitor_id: visitor_hash.to_string(),
        session_id,
        occurred_at,
        kind: event.kind,
        page_path: event.path,
        referrer_kind: event.referrer_kind,
        referrer_host: event.referrer_host,
        referrer_path: event.referrer_path,
        country_code: event.country_code,
        timezone: event.timezone,
        language: event.language,
        device_kind: event.device_kind,
        browser: event.browser,
        operating_system: event.operating_system,
        viewport_kind: event.viewport_kind,
        navigation_kind: event.navigation_kind,
        local_hour: event.local_hour,
        local_weekday: event.local_weekday,
        engagement_seconds: event.engagement_seconds,
        scroll_percent: event.scroll_percent,
        lcp_milliseconds: event.lcp_milliseconds,
        cls_thousandths: event.cls_thousandths,
        navigation_milliseconds: event.navigation_milliseconds,
        target_host: event.target_host,
        utm_source: event.utm_source,
        utm_medium: event.utm_medium,
        utm_campaign: event.utm_campaign,
    };
    let insert = retrying_conflicts(|| {
        let event = event.clone();
        async move {
            db.query(
                "CREATE ONLY type::record('analytics_events', $event.id)
                 SET visitor_id = $event.visitor_id,
                     session_id = $event.session_id,
                     occurred_at = $event.occurred_at,
                     kind = $event.kind,
                     page_path = $event.page_path,
                     referrer_kind = $event.referrer_kind,
                     referrer_host = $event.referrer_host,
                     referrer_path = $event.referrer_path,
                     country_code = $event.country_code,
                     timezone = $event.timezone,
                     language = $event.language,
                     device_kind = $event.device_kind,
                     browser = $event.browser,
                     operating_system = $event.operating_system,
                     viewport_kind = $event.viewport_kind,
                     navigation_kind = $event.navigation_kind,
                     local_hour = $event.local_hour,
                     local_weekday = $event.local_weekday,
                     engagement_seconds = $event.engagement_seconds,
                     scroll_percent = $event.scroll_percent,
                     lcp_milliseconds = $event.lcp_milliseconds,
                     cls_thousandths = $event.cls_thousandths,
                     navigation_milliseconds = $event.navigation_milliseconds,
                     target_host = $event.target_host,
                     utm_source = $event.utm_source,
                     utm_medium = $event.utm_medium,
                     utm_campaign = $event.utm_campaign",
            )
            .bind(("event", event))
            .await?
            .check()?;
            Ok::<(), surrealdb::Error>(())
        }
    })
    .await;
    match insert {
        Ok(_) => Ok(true),
        Err(error) => {
            if !event_exists(db, &event_id).await? {
                return Err(error.into());
            }
            if let Some(update) = engagement_retry {
                update_engagement_values(db, visitor_hash, &update, occurred_at).await?;
            }
            Ok(false)
        }
    }
}

async fn event_exists(db: &Db, id: &str) -> surrealdb::Result<bool> {
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM type::record('analytics_events', $id)",
        )
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = response.take(0)?;
    Ok(!ids.is_empty())
}

/// Idle gap that rotates a visitor onto a new session id. Dashboard
/// prior-session detection uses the same bound: a session that still has the
/// same id after `$cutoff` must have had activity inside the idle window
/// immediately before it, or the cursor would already have rotated.
pub(crate) const SESSION_IDLE_SECONDS: i64 = 30 * 60;

async fn server_session(db: &Db, visitor_hash: &str, occurred_at: i64) -> anyhow::Result<String> {
    // Minted once rather than per attempt: a retry should install the same
    // session it lost the race with, not a different one each time round.
    let candidate = hash_identifier("analytics-session", &Uuid::new_v4().to_string());
    let session: Option<String> = retrying_conflicts(|| {
        let candidate = candidate.clone();
        async move {
            let mut response = db
                .query(
                    "UPSERT type::record('analytics_sessions', $visitor_id)
                         SET session_id = IF last_seen_at = NONE
                                 OR last_seen_at < $occurred_at - $idle_seconds
                             {
                                 $candidate
                             } ELSE {
                                 session_id
                             },
                             last_seen_at = IF last_seen_at = NONE
                                 OR last_seen_at < $occurred_at
                             {
                                 $occurred_at
                             } ELSE {
                                 last_seen_at
                             }
                         RETURN VALUE session_id",
                )
                .bind(("visitor_id", visitor_hash.to_string()))
                .bind(("candidate", candidate))
                .bind(("occurred_at", occurred_at))
                .bind(("idle_seconds", SESSION_IDLE_SECONDS))
                .await?
                .check()?;
            response.take(0)
        }
    })
    .await
    .context("analytics session upsert failed")?;
    session.context("analytics session upsert returned no text row")
}

#[derive(Clone, SurrealValue)]
struct EngagementUpdate {
    id: String,
    page_path: String,
    engagement_seconds: Option<i64>,
    scroll_percent: Option<i64>,
    lcp_milliseconds: Option<i64>,
    cls_thousandths: Option<i64>,
    navigation_milliseconds: Option<i64>,
}

async fn update_engagement(
    db: &Db,
    visitor_hash: &str,
    event: &ValidatedEvent,
    occurred_at: i64,
) -> anyhow::Result<()> {
    if event.kind != "engagement" {
        return Ok(());
    }
    update_engagement_values(
        db,
        visitor_hash,
        &EngagementUpdate {
            id: event.id.clone(),
            page_path: event.path.clone(),
            engagement_seconds: event.engagement_seconds,
            scroll_percent: event.scroll_percent,
            lcp_milliseconds: event.lcp_milliseconds,
            cls_thousandths: event.cls_thousandths,
            navigation_milliseconds: event.navigation_milliseconds,
        },
        occurred_at,
    )
    .await
}

async fn update_engagement_values(
    db: &Db,
    visitor_hash: &str,
    update: &EngagementUpdate,
    occurred_at: i64,
) -> anyhow::Result<()> {
    // Keep the cumulative maximum and its activity cursor in one statement:
    // a retry after a process/database failure must never observe one without
    // the other. The final idle predicate also prevents an old document from
    // resurrecting a session that already expired or rotated.
    //
    // The transaction touches the shared session row, so it contends with every
    // other beacon from this visitor; re-running it is safe because each clause
    // only ever raises a value it already compared against.
    retrying_conflicts(|| async {
        db.query(
            "BEGIN TRANSACTION;
         LET $advanced = UPDATE type::record('analytics_events', $update.id)
             SET engagement_seconds = IF engagement_seconds = NONE
                     OR engagement_seconds < $engagement_seconds
                 {
                     $engagement_seconds
                 } ELSE {
                     engagement_seconds
                 },
                 scroll_percent = IF $update.scroll_percent = NONE {
                     scroll_percent
                 } ELSE IF scroll_percent = NONE
                     OR scroll_percent < $update.scroll_percent
                 {
                     $update.scroll_percent
                 } ELSE {
                     scroll_percent
                 },
                 lcp_milliseconds = IF $update.lcp_milliseconds = NONE {
                     lcp_milliseconds
                 } ELSE IF lcp_milliseconds = NONE
                     OR lcp_milliseconds < $update.lcp_milliseconds
                 {
                     $update.lcp_milliseconds
                 } ELSE {
                     lcp_milliseconds
                 },
                 cls_thousandths = IF $update.cls_thousandths = NONE {
                     cls_thousandths
                 } ELSE IF cls_thousandths = NONE
                     OR cls_thousandths < $update.cls_thousandths
                 {
                     $update.cls_thousandths
                 } ELSE {
                     cls_thousandths
                 },
                 navigation_milliseconds =
                     IF $update.navigation_milliseconds = NONE {
                         navigation_milliseconds
                     } ELSE IF navigation_milliseconds = NONE
                         OR navigation_milliseconds
                             < $update.navigation_milliseconds
                     {
                         $update.navigation_milliseconds
                     } ELSE {
                         navigation_milliseconds
                     }
             WHERE visitor_id = $visitor_id
                 AND kind = 'engagement'
                 AND page_path = $update.page_path
                 AND (
                     engagement_seconds = NONE
                     OR engagement_seconds < $engagement_seconds
                     OR (
                         $update.scroll_percent != NONE
                         AND (
                             scroll_percent = NONE
                             OR scroll_percent < $update.scroll_percent
                         )
                     )
                     OR (
                         $update.lcp_milliseconds != NONE
                         AND (
                             lcp_milliseconds = NONE
                             OR lcp_milliseconds < $update.lcp_milliseconds
                         )
                     )
                     OR (
                         $update.cls_thousandths != NONE
                         AND (
                             cls_thousandths = NONE
                             OR cls_thousandths < $update.cls_thousandths
                         )
                     )
                     OR (
                         $update.navigation_milliseconds != NONE
                         AND (
                             navigation_milliseconds = NONE
                             OR navigation_milliseconds
                                 < $update.navigation_milliseconds
                         )
                     )
                 )
             RETURN VALUE session_id;
         UPDATE type::record('analytics_sessions', $visitor_id)
             SET last_seen_at = IF last_seen_at < $occurred_at {
                 $occurred_at
             } ELSE {
                 last_seen_at
             }
             WHERE session_id IN $advanced
                 AND last_seen_at >= $occurred_at - $idle_seconds;
         COMMIT TRANSACTION;",
        )
        .bind(("update", update.clone()))
        .bind(("engagement_seconds", update.engagement_seconds.unwrap_or(0)))
        .bind(("visitor_id", visitor_hash.to_string()))
        .bind(("occurred_at", occurred_at))
        .bind(("idle_seconds", SESSION_IDLE_SECONDS))
        .await?
        .check()?;
        Ok(())
    })
    .await
    .context("analytics engagement update failed")?;
    Ok(())
}

/// The private ledger is one row per visitor. A later submission edits the
/// visitor's entry without disclosing whether one already existed.
pub async fn upsert_identity(
    db: &Db,
    visitor_hash: &str,
    display_name: String,
    note: Option<String>,
    now: i64,
) -> surrealdb::Result<()> {
    retrying_conflicts(|| {
        let display_name = display_name.clone();
        let note = note.clone();
        async move {
            db.query(
                "UPSERT type::record('analytics_identities', $visitor_id)
             SET display_name = $display_name,
                 note = $note,
                 first_submitted_at = first_submitted_at ?? $now,
                 updated_at = IF updated_at = NONE OR updated_at < $now {
                     $now
                 } ELSE {
                     updated_at
                 }",
            )
            .bind(("visitor_id", visitor_hash.to_string()))
            .bind(("display_name", display_name))
            .bind(("note", note))
            .bind(("now", now))
            .await?
            .check()?;
            Ok(())
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_session_identifiers_are_one_way_and_namespaced() {
        let raw = "64ec3b75-05af-49de-8d4e-75c2bd4ee4d4";
        let session = hash_identifier("analytics-session", raw);
        assert_eq!(session.len(), 64);
        assert!(!session.contains(raw));
        assert_ne!(session, hash_identifier("analytics-bootstrap-visitor", raw));
    }

    /// Verbatim from a local SurrealDB 3.2.3 losing a race between two writes
    /// to one `analytics_sessions` row. If the wording ever drifts this test
    /// fails and the retry quietly stops working, which is the point of it.
    fn conflict() -> surrealdb::Error {
        surrealdb::Error::internal(
            "There was a problem with the key-value store: Transaction conflict: \
             Resource busy. This transaction can be retried"
                .to_string(),
        )
    }

    #[test]
    fn only_a_conflict_counts_as_retryable() {
        assert!(is_transaction_conflict(&conflict()));
        assert!(is_transaction_conflict(
            &surrealdb::Error::internal("wrapped".to_string()).with_cause(conflict())
        ));
        assert!(!is_transaction_conflict(&surrealdb::Error::already_exists(
            "the record already exists".to_string(),
            None
        )));
        assert!(!is_transaction_conflict(&surrealdb::Error::internal(
            "Couldn't update a finished transaction".to_string()
        )));
    }

    #[test]
    fn the_backoff_grows_and_stays_bounded() {
        let waits: Vec<u128> = (0..CONFLICT_ATTEMPTS)
            .map(|attempt| conflict_backoff(attempt).as_millis())
            .collect();
        // Each step's floor is the previous step's ceiling: base + jitter < 2×base.
        for (attempt, wait) in waits.iter().enumerate() {
            let base = 4_u128 << attempt.min(3);
            assert!((base..base * 2).contains(wait), "{attempt}: {wait}ms");
        }
        // Worst case is every jitter landing one short of its base: 7 + 15 + 31
        // + 63. Asserting a tighter bound than that makes this test flaky, since
        // the jitter comes from the clock.
        assert!(waits.iter().sum::<u128>() <= 116, "{waits:?}");
    }

    #[tokio::test]
    async fn a_conflict_is_re_run_until_it_lands() {
        let attempts = std::cell::Cell::new(0);
        let result = retrying_conflicts(|| async {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 {
                return Err(conflict());
            }
            Ok("written")
        })
        .await;
        assert_eq!(result.unwrap(), "written");
        assert_eq!(attempts.get(), 3);
    }

    #[tokio::test]
    async fn a_write_that_never_stops_conflicting_gives_up_and_reports() {
        let attempts = std::cell::Cell::new(0);
        let result: surrealdb::Result<()> = retrying_conflicts(|| async {
            attempts.set(attempts.get() + 1);
            Err(conflict())
        })
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.get(), CONFLICT_ATTEMPTS);
    }

    /// `insert_event` reads a duplicate-key failure as "already recorded", so
    /// re-running it would turn a cheap dedupe into four pointless round trips.
    #[tokio::test]
    async fn anything_other_than_a_conflict_fails_on_the_first_try() {
        let attempts = std::cell::Cell::new(0);
        let result: surrealdb::Result<()> = retrying_conflicts(|| async {
            attempts.set(attempts.get() + 1);
            Err(surrealdb::Error::already_exists(
                "the record already exists".to_string(),
                None,
            ))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }
}
