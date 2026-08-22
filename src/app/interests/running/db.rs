//! Direct persistence for the small running log.
//!
//! Unlike lifting, runs have no derived records or filter engine, so they do
//! not join `FitnessStore`'s full snapshot. Reads explicitly project optional
//! fields because SurrealDB omits `NONE` values from `SELECT *`.

use std::time::Duration;

use benjisponge::data::{Db, running_models::RunningActivity};

const PROJECTION: &str = "record::id(id) AS id,
    source,
    source_activity_id,
    source_url,
    title,
    activity_type,
    started_at_utc,
    started_at_local,
    eastern_offset_minutes,
    duration_milliseconds,
    moving_duration_milliseconds,
    distance_millimeters,
    ascent_millimeters,
    imported_at";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateOutcome {
    Added,
    Duplicate,
    Conflict,
}

pub async fn list(db: &Db) -> surrealdb::Result<Vec<RunningActivity>> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}
             FROM running_activities
             ORDER BY started_at_utc DESC, id DESC"
        ))
        .await?
        .check()?;
    response.take(0)
}

pub async fn by_id(db: &Db, id: &str) -> surrealdb::Result<Option<RunningActivity>> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}
             FROM type::record('running_activities', $id)"
        ))
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let rows: Vec<RunningActivity> = response.take(0)?;
    Ok(rows.into_iter().next())
}

pub async fn by_source_activity_id(
    db: &Db,
    source_activity_id: &str,
) -> surrealdb::Result<Option<RunningActivity>> {
    by_source_identity(db, "garmin-connect", source_activity_id).await
}

pub async fn by_source_identity(
    db: &Db,
    source: &str,
    source_activity_id: &str,
) -> surrealdb::Result<Option<RunningActivity>> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}
             FROM running_activities
             WHERE source = $source
                 AND source_activity_id = $source_activity_id"
        ))
        .bind(("source", source.to_string()))
        .bind(("source_activity_id", source_activity_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<RunningActivity> = response.take(0)?;
    Ok(rows.into_iter().next())
}

/// Create-only and idempotent. A later Garmin edit never silently rewrites a
/// published run: correcting history needs an explicit replacement workflow,
/// just like lifting.
pub async fn create(db: &Db, incoming: &RunningActivity) -> surrealdb::Result<CreateOutcome> {
    // SurrealDB 3.2.3 can report a transient transaction conflict while two
    // writers touch a shared unique index. Re-read after every failure to
    // classify a winner, then retry only while no row has become visible.
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        if let Some(outcome) = existing_outcome(db, incoming).await? {
            return Ok(outcome);
        }
        let attempted = db
            .query(
                "CREATE ONLY type::record('running_activities', $id)
                     CONTENT $activity
                     RETURN VALUE record::id(id)",
            )
            .bind(("id", incoming.id.clone()))
            .bind(("activity", incoming.clone()))
            .await
            .and_then(|response| response.check());
        match attempted {
            Ok(mut response) => {
                let created: Vec<String> = response.take(0)?;
                if created.len() == 1 && created[0] == incoming.id {
                    return Ok(CreateOutcome::Added);
                }
                return Ok(existing_outcome(db, incoming)
                    .await?
                    .unwrap_or(CreateOutcome::Conflict));
            }
            Err(error) => {
                // The write may have committed even when Surreal returned an
                // error. Classify when the re-read works; if that read is
                // transiently unavailable too, preserve the original write
                // error and consume the remaining bounded attempts.
                if let Ok(Some(outcome)) = existing_outcome(db, incoming).await {
                    return Ok(outcome);
                }
                if attempt == MAX_ATTEMPTS {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
            }
        }
    }
    unreachable!("the bounded running-import loop always returns")
}

async fn existing_outcome(
    db: &Db,
    incoming: &RunningActivity,
) -> surrealdb::Result<Option<CreateOutcome>> {
    if let Some(existing) = by_id(db, &incoming.id).await? {
        return Ok(Some(classify(&existing, incoming)));
    }
    Ok(
        by_source_identity(db, &incoming.source, &incoming.source_activity_id)
            .await?
            .map(|existing| classify(&existing, incoming)),
    )
}

fn classify(existing: &RunningActivity, incoming: &RunningActivity) -> CreateOutcome {
    let same_identity = existing.id == incoming.id
        && existing.source == incoming.source
        && existing.source_activity_id == incoming.source_activity_id;
    let same_summary = existing.title == incoming.title
        && existing.activity_type == incoming.activity_type
        && existing.duration_milliseconds == incoming.duration_milliseconds
        && existing.moving_duration_milliseconds == incoming.moving_duration_milliseconds
        && existing.distance_millimeters == incoming.distance_millimeters
        && existing.ascent_millimeters == incoming.ascent_millimeters;
    // A manual form's hidden token owns its stable identity. The first write
    // owns server-stamped timing metadata, so replaying that same form later
    // must not conflict merely because the clock advanced. Garmin summaries,
    // by contrast, carry their own source timestamp and still compare it.
    let same = same_identity
        && same_summary
        && (incoming.source == "manual"
            || (existing.started_at_utc == incoming.started_at_utc
                && existing.started_at_local == incoming.started_at_local
                && existing.eastern_offset_minutes == incoming.eastern_offset_minutes));
    if same {
        CreateOutcome::Duplicate
    } else {
        CreateOutcome::Conflict
    }
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;

    async fn memory_db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db.query(include_str!("../../../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db
    }

    fn activity() -> RunningActivity {
        RunningActivity {
            id: super::super::garmin::storage_id("123"),
            source: "garmin-connect".to_string(),
            source_activity_id: "123".to_string(),
            source_url: Some("https://connect.garmin.com/app/activity/123".to_string()),
            title: "Morning Running".to_string(),
            activity_type: "running".to_string(),
            started_at_utc: "2026-08-21 11:00:00".to_string(),
            started_at_local: "2026-08-21 07:00:00".to_string(),
            eastern_offset_minutes: -240,
            duration_milliseconds: 1_800_000,
            moving_duration_milliseconds: None,
            distance_millimeters: 5_000_000,
            ascent_millimeters: None,
            imported_at: 1,
        }
    }

    #[tokio::test]
    async fn create_is_idempotent_but_never_rewrites() {
        let db = memory_db().await;
        let run = activity();
        assert_eq!(create(&db, &run).await.unwrap(), CreateOutcome::Added);

        let mut replay = run.clone();
        replay.imported_at = 2;
        assert_eq!(
            create(&db, &replay).await.unwrap(),
            CreateOutcome::Duplicate
        );

        replay.title = "Renamed Running".to_string();
        assert_eq!(create(&db, &replay).await.unwrap(), CreateOutcome::Conflict);
        assert_eq!(list(&db).await.unwrap(), vec![run]);
    }

    #[tokio::test]
    async fn optional_fields_deserialize_when_none() {
        let db = memory_db().await;
        let mut run = activity();
        run.source_url = None;
        create(&db, &run).await.unwrap();
        let loaded = by_source_activity_id(&db, "123").await.unwrap().unwrap();
        assert_eq!(loaded.source_url, None);
        assert_eq!(loaded.moving_duration_milliseconds, None);
        assert_eq!(loaded.ascent_millimeters, None);
    }

    #[tokio::test]
    async fn concurrent_replays_converge_on_one_activity() {
        let db = memory_db().await;
        let run = activity();
        let (first, second) = tokio::join!(create(&db, &run), create(&db, &run));
        let outcomes = [first.unwrap(), second.unwrap()];
        assert!(outcomes.contains(&CreateOutcome::Added));
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome, CreateOutcome::Added | CreateOutcome::Duplicate))
        );
        assert_eq!(list(&db).await.unwrap(), vec![run]);
    }

    #[tokio::test]
    async fn delayed_manual_replay_keeps_the_first_timestamp_but_changed_metrics_conflict() {
        let db = memory_db().await;
        let mut run = activity();
        run.id = "b".repeat(64);
        run.source = "manual".to_string();
        run.source_activity_id = "c".repeat(64);
        run.title = "Run".to_string();
        assert_eq!(create(&db, &run).await.unwrap(), CreateOutcome::Added);

        let mut delayed = run.clone();
        delayed.started_at_utc = "2026-08-22 11:00:00".to_string();
        delayed.started_at_local = "2026-08-22 07:00:00".to_string();
        delayed.imported_at += 86_400;
        assert_eq!(
            create(&db, &delayed).await.unwrap(),
            CreateOutcome::Duplicate
        );

        delayed.distance_millimeters += 1;
        assert_eq!(
            create(&db, &delayed).await.unwrap(),
            CreateOutcome::Conflict
        );
        assert_eq!(list(&db).await.unwrap(), vec![run]);
    }
}
