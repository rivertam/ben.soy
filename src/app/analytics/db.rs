//! Analytics database writes.

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
        .await
        .context("analytics visitor resolution failed")?
        .check()
        .context("analytics visitor resolution failed")?;
    let visitor: Option<String> = response
        .take(0)
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
    let insert = async {
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

const SESSION_IDLE_SECONDS: i64 = 30 * 60;

async fn server_session(db: &Db, visitor_hash: &str, occurred_at: i64) -> anyhow::Result<String> {
    let candidate = hash_identifier("analytics-session", &Uuid::new_v4().to_string());
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
        .await
        .context("analytics session upsert failed")?
        .check()
        .context("analytics session upsert failed")?;
    let session: Option<String> = response
        .take(0)
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
    .await
    .context("analytics engagement update failed")?
    .check()
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
}
