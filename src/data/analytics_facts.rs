//! Exact analytics visitor/day facts and their resumable background backfill.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use surrealdb::types::SurrealValue;
use uuid::Uuid;

use super::{
    Db,
    analytics_models::{
        AnalyticsDimensionFact, AnalyticsEngagementFact, AnalyticsEvent, AnalyticsEventFact,
        AnalyticsSessionFact, AnalyticsVisitorDay,
    },
};

const DAY_SECONDS: i64 = 86_400;
/// Source rows scanned (or dirty keys drained) per leased round. Kept small so
/// a single round cannot monopolize SurrealDB while the legacy dashboard and
/// live event writes still share the same datastore.
const BATCH: i64 = 32;
const LEASE_SECONDS: i64 = 30;
const CONFLICT_MARKER: &str = "Transaction conflict";
const CONFLICT_ATTEMPTS: usize = 4;
const BACKFILL_IDLE: Duration = Duration::from_secs(5);
const BACKFILL_BACKOFF_CAP: Duration = Duration::from_secs(60);

#[derive(Deserialize, SurrealValue)]
struct EventKey {
    id: String,
    visitor_id: String,
    occurred_at: i64,
}

#[derive(Deserialize, SurrealValue)]
struct FactRow {
    utc_day: i64,
    payload: String,
}

#[derive(Deserialize, SurrealValue)]
struct BackfillState {
    cursor_at: i64,
    cursor_id: String,
    phase: String,
    lease_owner: Option<String>,
    lease_until: i64,
    parity_mask: i64,
}

pub fn start_backfill(db: Db) {
    // Off by default: the leased scan rebuilds enough visitor-days to reset
    // SurrealDB connections and starve the legacy dashboard. Re-enable with
    // ANALYTICS_FACTS_BACKFILL=1 once raw reads are healthy again.
    match std::env::var("ANALYTICS_FACTS_BACKFILL") {
        Ok(value) if matches!(value.as_str(), "1" | "true" | "TRUE" | "yes") => {}
        _ => {
            eprintln!(
                "analytics facts: backfill worker idle (set ANALYTICS_FACTS_BACKFILL=1 to enable)"
            );
            return;
        }
    }
    tokio::spawn(async move {
        let owner = Uuid::new_v4().to_string();
        let mut failures: u32 = 0;
        loop {
            match backfill_round(&db, &owner).await {
                Ok(()) => failures = 0,
                Err(error) => {
                    failures = failures.saturating_add(1);
                    eprintln!("analytics facts: backfill round failed: {error}");
                }
            }
            tokio::time::sleep(backfill_pause(failures)).await;
        }
    });
}

/// Refresh the visitor/day fact for a just-written event, when safe.
///
/// During scan/reconcile the leased backfill owns rebuilds. Live writes still
/// mark the absolute key dirty through `DEFINE EVENT`, so skipping the request
/// path here avoids racing the backfill and starving the legacy dashboard.
/// Once phase is `ready`, this keeps facts fresh ahead of the reconciler.
pub async fn rebuild_for_event(db: &Db, event_id: &str) -> anyhow::Result<()> {
    let Some(state) = state(db).await? else {
        return Ok(());
    };
    if state.phase != "ready" {
        return Ok(());
    }
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, visitor_id, occurred_at
         FROM type::record('analytics_events', $id)",
        )
        .bind(("id", event_id.to_owned()))
        .await?
        .check()?;
    let row: Option<EventKey> = response.take(0)?;
    if let Some(row) = row {
        rebuild_visitor_day(db, &row.visitor_id, row.occurred_at.div_euclid(DAY_SECONDS)).await?;
    }
    Ok(())
}

pub async fn rebuild_visitor_day(db: &Db, visitor_id: &str, utc_day: i64) -> anyhow::Result<()> {
    let floor = utc_day.saturating_mul(DAY_SECONDS);
    let ceiling = floor.saturating_add(DAY_SECONDS);
    let mut attempts = 0;
    loop {
        match rebuild_visitor_day_once(db, visitor_id, utc_day, floor, ceiling).await {
            Ok(()) => return Ok(()),
            Err(error) if is_transaction_conflict(&error) && attempts + 1 < CONFLICT_ATTEMPTS => {
                tokio::time::sleep(conflict_backoff(attempts)).await;
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn rebuild_visitor_day_once(
    db: &Db,
    visitor_id: &str,
    utc_day: i64,
    floor: i64,
    ceiling: i64,
) -> anyhow::Result<()> {
    for _ in 0..4 {
        // Read the dirty revision before the raw snapshot. The database
        // function compares it again immediately before replacement: a write
        // racing either side leaves a newer dirty revision for this retry (or
        // the leased reconciler), so an older snapshot can never win last.
        let mut response = db
            .query(
                "SELECT VALUE revision FROM analytics_fact_dirty
             WHERE utc_day = $day AND visitor_id = $visitor",
            )
            .bind(("day", utc_day))
            .bind(("visitor", visitor_id.to_owned()))
            .await?
            .check()?;
        let revisions: Vec<i64> = response.take(0)?;
        let revision = revisions.first().copied().unwrap_or(0);

        let mut response = db
            .query(
                "SELECT *, record::id(id) AS id FROM analytics_events
             WHERE visitor_id = $visitor_id
               AND occurred_at >= $floor AND occurred_at < $ceiling
             ORDER BY occurred_at ASC, id ASC",
            )
            .bind(("visitor_id", visitor_id.to_owned()))
            .bind(("floor", floor))
            .bind(("ceiling", ceiling))
            .await?
            .check()?;
        let events: Vec<AnalyticsEvent> = response.take(0)?;
        if events.is_empty() {
            // Deletes deliberately do not subtract retained facts.
            return Ok(());
        }
        let fact = compact(utc_day, visitor_id, events)?;
        let payload = serde_json::to_string(&fact)?;
        let mut response = db
            .query("fn::analytics::rebuild_visitor_day($day, $visitor, $payload, $now, $revision)")
            .bind(("day", utc_day))
            .bind(("visitor", visitor_id.to_owned()))
            .bind(("payload", payload))
            .bind(("now", now()))
            .bind(("revision", revision))
            .await?
            .check()?;
        let applied: Option<bool> = response.take(0)?;
        if applied == Some(true) {
            return Ok(());
        }
    }
    anyhow::bail!("analytics visitor-day kept changing during rebuild")
}

fn is_transaction_conflict(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains(CONFLICT_MARKER))
}

fn conflict_backoff(attempt: usize) -> Duration {
    let base = 4_u64 << attempt.min(3);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::from(since.subsec_nanos()))
        % base;
    Duration::from_millis(base + jitter)
}

fn backfill_pause(failures: u32) -> Duration {
    if failures == 0 {
        return BACKFILL_IDLE;
    }
    let shift = failures.saturating_sub(1).min(4);
    let millis = BACKFILL_IDLE
        .as_millis()
        .saturating_mul(1u128 << shift)
        .min(BACKFILL_BACKOFF_CAP.as_millis());
    Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
}

#[doc(hidden)]
pub fn compact(
    utc_day: i64,
    visitor_id: &str,
    events: Vec<AnalyticsEvent>,
) -> anyhow::Result<AnalyticsVisitorDay> {
    let mut grouped: BTreeMap<String, AnalyticsEventFact> = BTreeMap::new();
    let mut sessions: BTreeMap<String, AnalyticsSessionFact> = BTreeMap::new();
    let mut dimensions: BTreeMap<(String, String, Option<String>), u64> = BTreeMap::new();
    let mut engagement: BTreeMap<String, AnalyticsEngagementFact> = BTreeMap::new();
    for event in events {
        match event.kind.as_str() {
            "pageview" => {
                sessions
                    .entry(event.session_id.clone())
                    .and_modify(|session| {
                        session.pageviews = session.pageviews.saturating_add(1);
                        session.last_pageview_at = session.last_pageview_at.max(event.occurred_at);
                        if (event.occurred_at, event.id.as_str())
                            < (
                                session.first_pageview.occurred_at,
                                session.first_pageview.id.as_str(),
                            )
                        {
                            session.first_pageview = event.clone();
                        }
                    })
                    .or_insert_with(|| AnalyticsSessionFact {
                        session_id: event.session_id.clone(),
                        pageviews: 1,
                        first_pageview: event.clone(),
                        last_pageview_at: event.occurred_at,
                    });
                add_dimension(&mut dimensions, "page", &event.page_path, None);
                if let Some(country) = event.country_code.as_deref() {
                    add_dimension(&mut dimensions, "country", country, None);
                }
                for (dimension, key) in [
                    ("device", event.device_kind.as_str()),
                    ("browser", event.browser.as_str()),
                    ("os", event.operating_system.as_str()),
                ] {
                    add_dimension(&mut dimensions, dimension, key, None);
                }
                if let (Some(weekday), Some(hour)) = (event.local_weekday, event.local_hour) {
                    add_dimension(
                        &mut dimensions,
                        "local_hour",
                        &format!("{weekday}:{hour}"),
                        None,
                    );
                }
                if event.referrer_kind == "internal"
                    && let Some(from) = event.referrer_path.as_deref()
                    && from != event.page_path
                {
                    add_dimension(
                        &mut dimensions,
                        "journey",
                        from,
                        Some(event.page_path.clone()),
                    );
                }
            }
            "outbound" => {
                if let Some(host) = event.target_host.as_deref() {
                    add_dimension(&mut dimensions, "outbound", host, None);
                }
            }
            "engagement" => add_engagement(&mut engagement, &event),
            _ => {}
        }
        let mut identity = event.clone();
        identity.id.clear();
        identity.occurred_at = 0;
        let key = serde_json::to_string(&identity)?;
        grouped
            .entry(key)
            .and_modify(|fact| {
                fact.count = fact.count.saturating_add(1);
                fact.last_occurred_at = fact.last_occurred_at.max(event.occurred_at);
                if (event.occurred_at, event.id.as_str())
                    < (fact.event.occurred_at, fact.event.id.as_str())
                {
                    fact.event.id = event.id.clone();
                    fact.event.occurred_at = event.occurred_at;
                }
            })
            .or_insert(AnalyticsEventFact {
                last_occurred_at: event.occurred_at,
                event,
                count: 1,
            });
    }
    Ok(AnalyticsVisitorDay {
        utc_day,
        visitor_id: visitor_id.to_owned(),
        sessions: sessions.into_values().collect(),
        dimensions: dimensions
            .into_iter()
            .map(
                |((dimension, key, secondary), count)| AnalyticsDimensionFact {
                    dimension,
                    key,
                    secondary,
                    count,
                },
            )
            .collect(),
        engagement: engagement.into_values().collect(),
        events: grouped.into_values().collect(),
    })
}

fn add_dimension(
    dimensions: &mut BTreeMap<(String, String, Option<String>), u64>,
    dimension: &str,
    key: &str,
    secondary: Option<String>,
) {
    let count = dimensions
        .entry((dimension.to_owned(), key.to_owned(), secondary))
        .or_default();
    *count = count.saturating_add(1);
}

fn add_engagement(
    engagement: &mut BTreeMap<String, AnalyticsEngagementFact>,
    event: &AnalyticsEvent,
) {
    let fact = engagement
        .entry(event.page_path.clone())
        .or_insert_with(|| AnalyticsEngagementFact {
            page_path: event.page_path.clone(),
            ..AnalyticsEngagementFact::default()
        });
    fact.events = fact.events.saturating_add(1);
    push_metric(
        event.engagement_seconds,
        &mut fact.engagement_seconds_sum,
        &mut fact.engagement_seconds_samples,
    );
    if let Some(scroll) = event.scroll_percent {
        fact.scroll_percent_sum += i128::from(scroll);
        fact.scroll_percent_samples = fact.scroll_percent_samples.saturating_add(1);
        fact.finish_count = fact.finish_count.saturating_add(u64::from(scroll >= 90));
    }
    push_metric(
        event.lcp_milliseconds,
        &mut fact.lcp_milliseconds_sum,
        &mut fact.lcp_milliseconds_samples,
    );
    push_metric(
        event.cls_thousandths,
        &mut fact.cls_thousandths_sum,
        &mut fact.cls_thousandths_samples,
    );
    push_metric(
        event.navigation_milliseconds,
        &mut fact.navigation_milliseconds_sum,
        &mut fact.navigation_milliseconds_samples,
    );
}

fn push_metric(value: Option<i64>, sum: &mut i128, samples: &mut u64) {
    if let Some(value) = value {
        *sum += i128::from(value);
        *samples = samples.saturating_add(1);
    }
}

pub async fn load(db: &Db, cutoff: i64) -> anyhow::Result<Option<Vec<AnalyticsVisitorDay>>> {
    let Some(state) = state(db).await? else {
        return Ok(None);
    };
    if state.phase != "ready" || state.parity_mask != 15 {
        return Ok(None);
    }
    let first_day = cutoff.div_euclid(DAY_SECONDS).saturating_sub(1);
    let started = Instant::now();
    let facts = timed_load(db, first_day).await?;
    eprintln!(
        "analytics facts: loaded {} visitor-days in {} ms",
        facts.len(),
        started.elapsed().as_millis()
    );
    Ok(Some(facts))
}

pub async fn ready_for_parity(db: &Db) -> anyhow::Result<bool> {
    Ok(state(db).await?.is_some_and(|state| state.phase == "ready"))
}

pub async fn record_parity(db: &Db, bit: i64) -> anyhow::Result<i64> {
    let mut response = db
        .query(
            "UPDATE type::record('analytics_fact_backfill', 'state')
         SET parity_mask = IF $bit = 1 AND parity_mask IN [0,2,4,6,8,10,12,14] {
                 parity_mask + 1
             } ELSE IF $bit = 2 AND parity_mask IN [0,1,4,5,8,9,12,13] {
                 parity_mask + 2
             } ELSE IF $bit = 4 AND parity_mask IN [0,1,2,3,8,9,10,11] {
                 parity_mask + 4
             } ELSE IF $bit = 8 AND parity_mask < 8 {
                 parity_mask + 8
             } ELSE {
                 parity_mask
             },
             updated_at = $now
         RETURN VALUE parity_mask",
        )
        .bind(("bit", bit))
        .bind(("now", now()))
        .await?
        .check()?;
    let mask: Option<i64> = response.take(0)?;
    Ok(mask.unwrap_or(0))
}

pub async fn load_for_parity(db: &Db, cutoff: i64) -> anyhow::Result<Vec<AnalyticsVisitorDay>> {
    let first_day = cutoff.div_euclid(DAY_SECONDS).saturating_sub(1);
    timed_load(db, first_day).await
}

async fn timed_load(db: &Db, first_day: i64) -> anyhow::Result<Vec<AnalyticsVisitorDay>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut response = db
            .query(
                "SELECT utc_day, payload FROM analytics_visitor_days
             WHERE utc_day >= $first_day ORDER BY utc_day ASC",
            )
            .bind(("first_day", first_day))
            .await?
            .check()?;
        let rows: Vec<FactRow> = response.take(0)?;
        decode_rows(rows)
    })
    .await
    .map_err(|_| anyhow::anyhow!("analytics fact load exceeded three seconds"))?
}

fn decode_rows(rows: Vec<FactRow>) -> anyhow::Result<Vec<AnalyticsVisitorDay>> {
    rows.into_iter()
        .map(|row| {
            let fact: AnalyticsVisitorDay = serde_json::from_str(&row.payload)?;
            anyhow::ensure!(
                fact.utc_day == row.utc_day,
                "analytics fact envelope day mismatch"
            );
            Ok(fact)
        })
        .collect()
}

async fn state(db: &Db) -> anyhow::Result<Option<BackfillState>> {
    let mut response = db
        .query(
            "SELECT cursor_at, cursor_id, phase, lease_owner, lease_until, parity_mask
         FROM type::record('analytics_fact_backfill', 'state')",
        )
        .await?
        .check()?;
    Ok(response.take(0)?)
}

async fn backfill_round(db: &Db, owner: &str) -> anyhow::Result<()> {
    let timestamp = now();
    db.query(
        "UPSERT type::record('analytics_fact_backfill', 'state')
         SET cursor_at = cursor_at ?? -1, cursor_id = cursor_id ?? '',
             phase = phase ?? 'scan', lease_owner = lease_owner ?? NONE,
             lease_until = lease_until ?? 0, parity_mask = parity_mask ?? 0,
             updated_at = updated_at ?? $now",
    )
    .bind(("now", timestamp))
    .await?
    .check()?;
    let mut response = db
        .query(
            "UPDATE type::record('analytics_fact_backfill', 'state')
         SET lease_owner = $owner, lease_until = $until, updated_at = $now
         WHERE lease_until < $now OR lease_owner = $owner
         RETURN cursor_at, cursor_id, phase, lease_owner, lease_until, parity_mask",
        )
        .bind(("owner", owner.to_owned()))
        .bind(("until", timestamp + LEASE_SECONDS))
        .bind(("now", timestamp))
        .await?
        .check()?;
    let Some(state): Option<BackfillState> = response.take(0)? else {
        return Ok(());
    };
    if state.lease_owner.as_deref() != Some(owner) {
        return Ok(());
    }

    if state.phase == "scan" {
        let mut response = db.query(
            "SELECT record::id(id) AS id, visitor_id, occurred_at FROM analytics_events
             WHERE occurred_at > $cursor_at OR (occurred_at = $cursor_at AND record::id(id) > $cursor_id)
             ORDER BY occurred_at ASC, id ASC LIMIT $limit"
        ).bind(("cursor_at", state.cursor_at)).bind(("cursor_id", state.cursor_id))
            .bind(("limit", BATCH)).await?.check()?;
        let rows: Vec<EventKey> = response.take(0)?;
        if rows.is_empty() {
            db.query("UPDATE type::record('analytics_fact_backfill', 'state') SET phase = 'reconcile', updated_at = $now")
                .bind(("now", now())).await?.check()?;
            return Ok(());
        }
        let mut keys = BTreeMap::new();
        for row in &rows {
            keys.insert(
                (
                    row.visitor_id.clone(),
                    row.occurred_at.div_euclid(DAY_SECONDS),
                ),
                (),
            );
        }
        for ((visitor, day), ()) in keys {
            rebuild_visitor_day(db, &visitor, day).await?;
        }
        let last = rows.last().expect("non-empty batch");
        db.query("UPDATE type::record('analytics_fact_backfill', 'state') SET cursor_at = $at, cursor_id = $id, updated_at = $now WHERE lease_owner = $owner")
            .bind(("at", last.occurred_at)).bind(("id", last.id.clone())).bind(("now", now()))
            .bind(("owner", owner.to_owned())).await?.check()?;
        eprintln!("analytics facts: backfilled {} source rows", rows.len());
    } else if state.phase == "reconcile" {
        let mut response = db
            .query("SELECT utc_day, visitor_id FROM analytics_fact_dirty LIMIT $limit")
            .bind(("limit", BATCH))
            .await?
            .check()?;
        #[derive(Deserialize, SurrealValue)]
        struct Dirty {
            utc_day: i64,
            visitor_id: String,
        }
        let dirty: Vec<Dirty> = response.take(0)?;
        for row in &dirty {
            rebuild_visitor_day(db, &row.visitor_id, row.utc_day).await?;
        }
        if dirty.is_empty() {
            db.query("UPDATE type::record('analytics_fact_backfill', 'state') SET phase = 'ready', parity_mask = 0, updated_at = $now WHERE lease_owner = $owner")
                .bind(("now", now())).bind(("owner", owner.to_owned())).await?.check()?;
            eprintln!("analytics facts: backfill reconciled; awaiting four-window parity");
        }
    } else {
        // Keep maintenance writes converged after readiness.
        let mut response = db
            .query("SELECT utc_day, visitor_id FROM analytics_fact_dirty LIMIT $limit")
            .bind(("limit", BATCH))
            .await?
            .check()?;
        #[derive(Deserialize, SurrealValue)]
        struct Dirty {
            utc_day: i64,
            visitor_id: String,
        }
        let dirty: Vec<Dirty> = response.take(0)?;
        for row in dirty {
            rebuild_visitor_day(db, &row.visitor_id, row.utc_day).await?;
        }
    }
    Ok(())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;

    #[test]
    fn compaction_keeps_counts_and_deterministic_edges() {
        let mut first = fixture("b", 10);
        let mut second = first.clone();
        second.id = "a".into();
        second.occurred_at = 10;
        let mut third = first.clone();
        third.id = "c".into();
        third.occurred_at = 20;
        let fact = compact(0, "v", vec![first.clone(), third, second]).unwrap();
        assert_eq!(fact.events.len(), 1);
        assert_eq!(fact.events[0].count, 3);
        assert_eq!(fact.events[0].event.id, "a");
        assert_eq!(fact.events[0].last_occurred_at, 20);
        first.id.clear();
    }

    #[test]
    fn engagement_facts_count_zero_samples_and_optional_denominators() {
        let mut zero = fixture("zero", 10);
        zero.kind = "engagement".into();
        zero.engagement_seconds = Some(0);
        zero.scroll_percent = Some(0);
        zero.lcp_milliseconds = Some(0);
        let mut missing = zero.clone();
        missing.id = "missing".into();
        missing.occurred_at = 11;
        missing.engagement_seconds = None;
        missing.scroll_percent = None;
        missing.lcp_milliseconds = None;
        let fact = compact(0, "v", vec![zero, missing]).unwrap();
        let engagement = &fact.engagement[0];
        assert_eq!(engagement.events, 2);
        assert_eq!(engagement.engagement_seconds_samples, 1);
        assert_eq!(engagement.engagement_seconds_sum, 0);
        assert_eq!(engagement.scroll_percent_samples, 1);
        assert_eq!(engagement.finish_count, 0);
        assert_eq!(engagement.lcp_milliseconds_samples, 1);
        assert_eq!(engagement.navigation_milliseconds_samples, 0);
    }

    #[test]
    fn additive_site_epoch_accepts_a_newer_contiguous_ledger() {
        assert!(crate::data::schema_migrations::validate_ledger(&[1, 2, 3]).is_ok());
        assert!(crate::data::schema_migrations::validate_ledger(&[1, 3]).is_err());
    }

    fn fixture(id: &str, occurred_at: i64) -> AnalyticsEvent {
        AnalyticsEvent {
            id: id.into(),
            visitor_id: "v".into(),
            session_id: "s".into(),
            occurred_at,
            kind: "pageview".into(),
            page_path: "/".into(),
            referrer_kind: "direct".into(),
            referrer_host: None,
            referrer_path: None,
            country_code: None,
            timezone: None,
            language: None,
            device_kind: "unknown".into(),
            browser: "Other".into(),
            operating_system: "Other".into(),
            viewport_kind: "unknown".into(),
            navigation_kind: None,
            local_hour: None,
            local_weekday: None,
            engagement_seconds: None,
            scroll_percent: None,
            lcp_milliseconds: None,
            cls_thousandths: None,
            navigation_milliseconds: None,
            target_host: None,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
        }
    }

    #[tokio::test]
    async fn event_rebuild_backfill_resume_and_parity_are_operational() {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("facts").use_db("facts").await.unwrap();
        db.query(crate::data::SCHEMA)
            .await
            .unwrap()
            .check()
            .unwrap();
        crate::data::schema_migrations::apply(&db).await.unwrap();

        // Epoch 3 removes the live DEFINE EVENT while backfill is paused.
        // Reinstall it here so the dirty-revision CAS path stays covered.
        db.query(
            "DEFINE EVENT analytics_events_rebuild_visitor_day ON TABLE analytics_events
             WHEN $event IN ['CREATE', 'UPDATE'] THEN {
                 fn::analytics::rebuild_visitor_day(
                     <int>($after.occurred_at / 86400),
                     $after.visitor_id,
                     NONE,
                     time::unix(),
                     NONE
                 );
             };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        let visitor = "a".repeat(64);
        let session = "b".repeat(64);
        let id = "12345678-1234-4123-8123-123456789abc";
        let insert = "CREATE ONLY type::record('analytics_events', $id) SET
                visitor_id=$visitor, session_id=$session_id, occurred_at=$occurred_at,
                kind='pageview', page_path='/', referrer_kind='direct',
                referrer_host=NONE, referrer_path=NONE, country_code=NONE,
                timezone=NONE, language=NONE, device_kind='unknown',
                browser='Other', operating_system='Other', viewport_kind='unknown',
                navigation_kind=NONE, local_hour=NONE, local_weekday=NONE,
                engagement_seconds=NONE, scroll_percent=NONE,
                lcp_milliseconds=NONE, cls_thousandths=NONE,
                navigation_milliseconds=NONE, target_host=NONE,
                utm_source=NONE, utm_medium=NONE, utm_campaign=NONE";
        db.query(insert)
            .bind(("id", id.to_owned()))
            .bind(("visitor", visitor.clone()))
            .bind(("session_id", session.clone()))
            .bind(("occurred_at", 86_401_i64))
            .await
            .unwrap()
            .check()
            .unwrap();

        let mut response = db
            .query("SELECT VALUE record::id(id) FROM analytics_fact_dirty")
            .await
            .unwrap()
            .check()
            .unwrap();
        let dirty: Vec<String> = response.take(0).unwrap();
        assert_eq!(dirty.len(), 1);

        // Capture revision 1's absolute payload, advance the source to
        // revision 2, then prove the stale replace is rejected.
        let mut response = db
            .query("SELECT *, record::id(id) AS id FROM analytics_events")
            .await
            .unwrap()
            .check()
            .unwrap();
        let old_events: Vec<AnalyticsEvent> = response.take(0).unwrap();
        let old_payload =
            serde_json::to_string(&compact(1, &visitor, old_events).unwrap()).unwrap();
        let second_id = "22345678-1234-4123-8123-123456789abc";
        db.query(insert)
            .bind(("id", second_id.to_owned()))
            .bind(("visitor", visitor.clone()))
            .bind(("session_id", session))
            .bind(("occurred_at", 86_402_i64))
            .await
            .unwrap()
            .check()
            .unwrap();
        let mut response = db
            .query("fn::analytics::rebuild_visitor_day(1, $visitor, $payload, 86402, 1)")
            .bind(("visitor", visitor.clone()))
            .bind(("payload", old_payload))
            .await
            .unwrap()
            .check()
            .unwrap();
        let applied: Option<bool> = response.take(0).unwrap();
        assert_eq!(applied, Some(false));

        // Request-path rebuild stays quiet until backfill reaches ready; the
        // absolute rebuild still converges the dirty key for live traffic.
        rebuild_for_event(&db, id).await.unwrap();
        assert!(load_for_parity(&db, 86_400).await.unwrap().is_empty());
        rebuild_visitor_day(&db, &visitor, 1).await.unwrap();
        let facts = load_for_parity(&db, 86_400).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].events[0].count, 2);

        // Separate rounds persist and resume the scan cursor, then perform a
        // final dirty-key reconciliation before readiness.
        backfill_round(&db, "worker-a").await.unwrap();
        backfill_round(&db, "worker-a").await.unwrap();
        backfill_round(&db, "worker-a").await.unwrap();
        assert!(ready_for_parity(&db).await.unwrap());
        // Once ready, the request path rebuilds again without changing counts.
        rebuild_for_event(&db, id).await.unwrap();
        assert_eq!(record_parity(&db, 1).await.unwrap(), 1);
        assert_eq!(record_parity(&db, 2).await.unwrap(), 3);
        assert_eq!(record_parity(&db, 4).await.unwrap(), 7);
        assert_eq!(record_parity(&db, 8).await.unwrap(), 15);
        assert!(load(&db, 86_400).await.unwrap().is_some());
    }

    #[test]
    fn backfill_pause_backs_off_on_repeated_failures() {
        assert_eq!(backfill_pause(0), BACKFILL_IDLE);
        assert_eq!(backfill_pause(1), Duration::from_secs(5));
        assert_eq!(backfill_pause(2), Duration::from_secs(10));
        assert_eq!(backfill_pause(3), Duration::from_secs(20));
        assert_eq!(backfill_pause(5), BACKFILL_BACKOFF_CAP);
        assert_eq!(backfill_pause(9), BACKFILL_BACKOFF_CAP);
    }
}
