//! Podrick's worker-side queries.
//!
//! Two responsibilities, both about *not* announcing the wrong thing: the
//! watermark keeps imported history out of the channel, and the create-only
//! claim keeps any single workout from being announced twice.
//!
//! Compiled into the `podrick` binary only — the site's read-side queries live
//! in `status.rs`, so neither copy carries the other's unused code.
//!
//! The `workouts` reads here are the only place Podrick touches a fitness
//! table, and they are read-only. Everything a message actually *says* comes
//! from the public API (`announce.rs`), so the Eastern projection, permanent
//! paths, and derived records keep their single implementation in
//! `lifting/archive`.

use benjisponge::data::{
    Db,
    podrick_models::{PodrickAnnouncement, PodrickMeta, PodrickPantsAction, PodrickPantsMessage},
};
use serde::Deserialize;
use surrealdb::{Notification, method::QueryStream, types::SurrealValue};

/// The cursor key holding the newest workout that predates Podrick.
pub const ANNOUNCE_WATERMARK: &str = "announce_watermark";
/// Newest Discord snowflake covered by a completed Pants Off history seed or
/// live pass. Actions are produced only for messages newer than this cursor.
pub const PANTS_CURSOR: &str = "pants_cursor";
/// Newest message present when the one-time backwards history walk began.
pub const PANTS_BACKFILL_HEAD: &str = "pants_backfill_head";
/// Exclusive `before` snowflake for the next backwards history page.
pub const PANTS_BACKFILL_BEFORE: &str = "pants_backfill_before";
/// Immutable source channel bound on the first Pants Off run. Moving the
/// cursor to another channel would otherwise skip its history and mix facts.
pub const PANTS_SOURCE_CHANNEL: &str = "pants_source_channel";

/// The workout source Podrick announces. CSV history is deliberately excluded:
/// it never joins the `/log` timeline or `/feed.xml` either, and a resync
/// would otherwise replay years of workouts into the channel.
pub const ANNOUNCED_SOURCE: &str = "manual";

/// A workout row, reduced to what deciding-and-linking needs.
#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub struct AnnounceCandidate {
    pub id: String,
    pub title: String,
    pub started_at_utc: String,
    pub started_at_local: String,
    pub eastern_offset_minutes: i64,
}

/// A manual workout mutation delivered by SurrealDB's live query.
///
/// The notification is only a wake-up hint. The announcer still discovers and
/// claims work through [`workouts_after`], so a dropped or duplicate live
/// notification cannot lose or duplicate a Discord post.
#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub struct WorkoutChange {
    pub id: String,
}

pub type WorkoutChanges = QueryStream<Notification<WorkoutChange>>;

/// Watch future mutations to manual workouts.
///
/// Live queries have no initial snapshot and are not durable across a socket
/// reset. The run loop therefore establishes this stream before reconciling
/// [`workouts_after`] and repeats that ordering whenever the stream ends.
pub async fn watch_manual_workouts(db: &Db) -> surrealdb::Result<WorkoutChanges> {
    let mut response = db
        .query(
            "LIVE SELECT record::id(id) AS id
             FROM workouts
             WHERE source = $source;",
        )
        .bind(("source", ANNOUNCED_SOURCE.to_string()))
        .await?
        .check()?;
    response.stream::<Notification<WorkoutChange>>(0)
}

/// The newest stored workout's `started_at_utc`, or `None` for an empty
/// archive. Read once, to seed the watermark.
pub async fn newest_workout_start(db: &Db) -> surrealdb::Result<Option<String>> {
    let mut response = db
        .query(
            "SELECT VALUE started_at_utc
             FROM workouts
             ORDER BY started_at_utc DESC
             LIMIT 1;",
        )
        .await?
        .check()?;
    let starts: Vec<String> = response.take(0)?;
    Ok(starts.into_iter().next())
}

/// Read a cursor value.
pub async fn meta(db: &Db, key: &str) -> surrealdb::Result<Option<String>> {
    let mut response = db
        .query("SELECT VALUE v FROM type::record('podrick_meta', $key);")
        .bind(("key", key.to_string()))
        .await?
        .check()?;
    let values: Vec<String> = response.take(0)?;
    Ok(values.into_iter().next())
}

/// Seed a cursor only if unset, and return the value now in force.
///
/// `CREATE ONLY` rather than `UPSERT`: the watermark must be written exactly
/// once, at first run, and never moved afterwards. If two workers race here the
/// loser reads the winner's value instead of overwriting it — either way the
/// archive stays unannounced.
pub async fn init_meta(db: &Db, key: &str, value: &str) -> surrealdb::Result<String> {
    if let Some(existing) = meta(db, key).await? {
        return Ok(existing);
    }
    let row = PodrickMeta {
        k: key.to_string(),
        v: value.to_string(),
    };
    let created = db
        .query("CREATE ONLY type::record('podrick_meta', $key) CONTENT $row RETURN VALUE v;")
        .bind(("key", key.to_string()))
        .bind(("row", row))
        .await
        .and_then(|mut response| response.take::<Option<String>>(0));
    match created {
        Ok(Some(value)) => Ok(value),
        // A key collision means another worker seeded it first; adopt theirs.
        _ => Ok(meta(db, key).await?.unwrap_or_else(|| value.to_string())),
    }
}

/// Set a moving cursor value. Unlike [`init_meta`], Pants Off's history
/// position and live high-water mark intentionally advance.
pub async fn set_meta(db: &Db, key: &str, value: &str) -> surrealdb::Result<()> {
    db.query(
        "UPSERT ONLY type::record('podrick_meta', $key)
         SET k = $key, v = $value
         RETURN NONE;",
    )
    .bind(("key", key.to_string()))
    .bind(("value", value.to_string()))
    .await?
    .check()?;
    Ok(())
}

/// Manual workouts strictly newer than `watermark` that have never been
/// claimed, oldest first.
///
/// Strictly newer, so the workout that seeded the watermark is never
/// announced. Oldest first, so a burst is announced in the order it happened.
///
/// The unclaimed filter is load-bearing, not an optimization. The watermark is
/// a fixed floor rather than a moving cursor, so without it this window would
/// stay pinned to the oldest `limit` workouts past the floor and a new lift
/// would stop appearing as soon as `limit` of them had accumulated. Excluding
/// claimed rows is what lets the window advance. `podrick_announcements` holds
/// one row per announced lift — a few hundred over years — so the subquery
/// stays cheap.
pub async fn workouts_after(
    db: &Db,
    watermark: &str,
    limit: usize,
) -> surrealdb::Result<Vec<AnnounceCandidate>> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, title, started_at_utc,
                    started_at_local, eastern_offset_minutes
             FROM workouts
             WHERE source = $source
               AND started_at_utc > $watermark
               AND record::id(id) NOT IN (
                   SELECT VALUE workout_id FROM podrick_announcements
               )
             ORDER BY started_at_utc ASC
             LIMIT $limit;",
        )
        .bind(("source", ANNOUNCED_SOURCE.to_string()))
        .bind(("watermark", watermark.to_string()))
        .bind(("limit", limit as i64))
        .await?
        .check()?;
    response.take(0)
}

/// One workout by id, for retrying a claim whose post never confirmed.
pub async fn workout(db: &Db, workout_id: &str) -> surrealdb::Result<Option<AnnounceCandidate>> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, title, started_at_utc,
                    started_at_local, eastern_offset_minutes
             FROM workouts
             WHERE record::id(id) = $workout_id;",
        )
        .bind(("workout_id", workout_id.to_string()))
        .await?
        .check()?;
    let mut rows: Vec<AnnounceCandidate> = response.take(0)?;
    Ok(rows.pop())
}

/// Claims Discord never confirmed, oldest first. These are the crash-recovery
/// cases: the process died between claiming and posting.
pub async fn unposted_claims(db: &Db, limit: usize) -> surrealdb::Result<Vec<PodrickAnnouncement>> {
    // Every field is projected explicitly. `SELECT *` omits `option` fields
    // that hold NONE rather than returning null, so an unposted claim — which
    // by definition has no `message_id` — would arrive missing the very field
    // that identifies it and fail to deserialize.
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, workout_id, workout_path, channel_id,
                    message_id, claimed_at, posted_at, attempts
             FROM podrick_announcements
             WHERE message_id IS NONE
             ORDER BY claimed_at ASC
             LIMIT $limit;",
        )
        .bind(("limit", limit as i64))
        .await?
        .check()?;
    response.take(0)
}

/// The outcome of trying to take responsibility for a workout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// This process owns the announcement and must post it.
    Won,
    /// Someone else already owns it, posted or not. Do nothing.
    Taken,
}

/// Take exactly-once ownership of a workout's announcement.
///
/// The record key is the workout id, so `CREATE ONLY` *is* the mutual
/// exclusion: the database rejects the second writer rather than two processes
/// both deciding they should post.
pub async fn claim(
    db: &Db,
    workout_id: &str,
    workout_path: &str,
    channel_id: &str,
    claimed_at: i64,
) -> surrealdb::Result<Claim> {
    let row = PodrickAnnouncement {
        id: workout_id.to_string(),
        workout_id: workout_id.to_string(),
        workout_path: workout_path.to_string(),
        channel_id: channel_id.to_string(),
        message_id: None,
        claimed_at,
        posted_at: None,
        attempts: 0,
    };
    // `RETURN VALUE record::id(id)` rather than the created record: CREATE
    // returns `id` as a record id, which does not deserialize into the
    // model's `String`. Reading the row back would make a *successful* write
    // look like a lost race, and the announcement would never be posted.
    let created = db
        .query(
            "CREATE ONLY type::record('podrick_announcements', $workout_id)
             CONTENT $row
             RETURN VALUE record::id(id);",
        )
        .bind(("workout_id", workout_id.to_string()))
        .bind(("row", row))
        .await
        .and_then(|mut response| response.take::<Option<String>>(0));
    match created {
        Ok(Some(_)) => Ok(Claim::Won),
        // Either the key existed or the write raced and lost; both mean this
        // process must not post. `Taken` is conservative by design — a missed
        // announcement is recoverable on the next tick, a duplicate is not.
        _ => Ok(Claim::Taken),
    }
}

/// Record that Discord accepted the message. Idempotent by record key.
pub async fn mark_posted(
    db: &Db,
    workout_id: &str,
    message_id: &str,
    posted_at: i64,
) -> surrealdb::Result<()> {
    db.query(
        "UPDATE type::record('podrick_announcements', $workout_id)
         SET message_id = $message_id, posted_at = $posted_at
         RETURN NONE;",
    )
    .bind(("workout_id", workout_id.to_string()))
    .bind(("message_id", message_id.to_string()))
    .bind(("posted_at", posted_at))
    .await?
    .check()?;
    Ok(())
}

/// Count a failed post so a permanently poisoned message is visible in the
/// data, not only in logs.
pub async fn record_attempt(db: &Db, workout_id: &str) -> surrealdb::Result<()> {
    db.query(
        "UPDATE type::record('podrick_announcements', $workout_id)
         SET attempts = attempts + 1
         RETURN NONE;",
    )
    .bind(("workout_id", workout_id.to_string()))
    .await?
    .check()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pants Off source facts and side-effect outbox.

/// Store one recognized source message by its Discord snowflake.
///
/// An UPSERT is intentional: a history page can be replayed after a crash and
/// two pollers can overlap without turning an already-synced source fact into
/// an error. Discord message creation time and author are immutable for the
/// rules this job uses; content is neither fetched nor stored.
pub async fn store_pants_message(db: &Db, message: &PodrickPantsMessage) -> surrealdb::Result<()> {
    db.query(
        "UPSERT ONLY type::record('podrick_pants_messages', $message_id)
         SET message_id = $message_id,
             channel_id = $channel_id,
             author_id = $author_id,
             posted_at = $posted_at
         RETURN NONE;",
    )
    .bind(("message_id", message.message_id.clone()))
    .bind(("channel_id", message.channel_id.clone()))
    .bind(("author_id", message.author_id.clone()))
    .bind(("posted_at", message.posted_at))
    .await?
    .check()?;
    Ok(())
}

/// Install a production announcement claim during local reset seeding.
pub async fn store_announcement(
    db: &Db,
    announcement: &PodrickAnnouncement,
) -> surrealdb::Result<()> {
    db.query(
        "UPSERT ONLY type::record('podrick_announcements', $workout_id)
         CONTENT $row
         RETURN NONE;",
    )
    .bind(("workout_id", announcement.workout_id.clone()))
    .bind(("row", announcement.clone()))
    .await?
    .check()?;
    Ok(())
}

/// Install a production Pants action row during local reset seeding.
pub async fn store_pants_action(db: &Db, action: &PodrickPantsAction) -> surrealdb::Result<()> {
    db.query(
        "UPSERT ONLY type::record('podrick_pants_actions', $action_id)
         CONTENT $row
         RETURN NONE;",
    )
    .bind(("action_id", action.id.clone()))
    .bind(("row", action.clone()))
    .await?
    .check()?;
    Ok(())
}

/// True when no Podrick cursors/claims exist — the local-reset starting point.
pub async fn podrick_state_empty(db: &Db) -> surrealdb::Result<bool> {
    let mut response = db
        .query(
            "SELECT count() AS count FROM podrick_meta GROUP ALL;
             SELECT count() AS count FROM podrick_announcements GROUP ALL;
             SELECT count() AS count FROM podrick_pants_messages GROUP ALL;
             SELECT count() AS count FROM podrick_pants_actions GROUP ALL;",
        )
        .await?
        .check()?;
    let meta: Vec<CountRow> = response.take(0)?;
    let announcements: Vec<CountRow> = response.take(1)?;
    let messages: Vec<CountRow> = response.take(2)?;
    let actions: Vec<CountRow> = response.take(3)?;
    Ok(count_of(&meta) + count_of(&announcements) + count_of(&messages) + count_of(&actions) == 0)
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
struct CountRow {
    count: i64,
}

fn count_of(rows: &[CountRow]) -> i64 {
    rows.first().map_or(0, |row| row.count)
}

/// Every stored source fact, oldest first.
///
/// Reconciliation intentionally reads the full set. The immutable action floor
/// suppresses the silent history seed, while retaining all later facts means
/// an asynkwerm that became final during a long outage can still be claimed
/// after restart.
pub async fn pants_messages(db: &Db) -> surrealdb::Result<Vec<PodrickPantsMessage>> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, message_id, channel_id, author_id,
                    posted_at
             FROM podrick_pants_messages
             ORDER BY posted_at ASC;",
        )
        .await?
        .check()?;
    response.take(0)
}

/// Claim one Discord side effect before attempting it.
pub async fn claim_pants_action(db: &Db, action: &PodrickPantsAction) -> surrealdb::Result<Claim> {
    let created = db
        .query(
            "CREATE ONLY type::record('podrick_pants_actions', $action_id)
             CONTENT $row
             RETURN VALUE record::id(id);",
        )
        .bind(("action_id", action.id.clone()))
        .bind(("row", action.clone()))
        .await
        .and_then(|mut response| response.take::<Option<String>>(0));
    match created {
        Ok(Some(_)) => Ok(Claim::Won),
        Ok(None) => Ok(Claim::Taken),
        Err(error) => {
            // A key collision is the ordinary "another worker owns it"
            // result. A schema/query failure must not masquerade as that:
            // advancing the source cursor without a durable action would lose
            // the side effect forever.
            if pants_action_exists(db, &action.id).await? {
                Ok(Claim::Taken)
            } else {
                Err(error)
            }
        }
    }
}

async fn pants_action_exists(db: &Db, action_id: &str) -> surrealdb::Result<bool> {
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM type::record('podrick_pants_actions', $action_id);",
        )
        .bind(("action_id", action_id.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = response.take(0)?;
    Ok(!ids.is_empty())
}

/// Unconfirmed Pants Off side effects, oldest first.
pub async fn uncompleted_pants_actions(
    db: &Db,
    limit: usize,
) -> surrealdb::Result<Vec<PodrickPantsAction>> {
    // Both optional fields are projected explicitly; Surreal omits NONE
    // option fields from SELECT * rather than returning null.
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, action_kind, reason,
                    target_channel_id, source_message_id, content, claimed_at,
                    completed_at, output_message_id, attempts
             FROM podrick_pants_actions
             WHERE completed_at IS NONE
             ORDER BY claimed_at ASC
             LIMIT $limit;",
        )
        .bind(("limit", limit as i64))
        .await?
        .check()?;
    response.take(0)
}

/// Confirm a post or idempotent reaction. `output_message_id` is present for
/// infarction posts and absent for reactions.
pub async fn mark_pants_action_completed(
    db: &Db,
    action_id: &str,
    completed_at: i64,
    output_message_id: Option<&str>,
) -> surrealdb::Result<()> {
    db.query(
        "UPDATE type::record('podrick_pants_actions', $action_id)
         SET completed_at = $completed_at,
             output_message_id = $output_message_id
         RETURN NONE;",
    )
    .bind(("action_id", action_id.to_string()))
    .bind(("completed_at", completed_at))
    .bind((
        "output_message_id",
        output_message_id.map(ToString::to_string),
    ))
    .await?
    .check()?;
    Ok(())
}

/// Count a failed Pants Off side effect for operator visibility.
pub async fn record_pants_action_attempt(db: &Db, action_id: &str) -> surrealdb::Result<()> {
    db.query(
        "UPDATE type::record('podrick_pants_actions', $action_id)
         SET attempts = attempts + 1
         RETURN NONE;",
    )
    .bind(("action_id", action_id.to_string()))
    .await?
    .check()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::time::Duration;
    use surrealdb::types::Action;

    #[tokio::test]
    async fn workout_watch_emits_only_manual_workouts() {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db.query("DEFINE TABLE workouts SCHEMALESS;")
            .await
            .unwrap()
            .check()
            .unwrap();

        let mut changes = watch_manual_workouts(&db).await.unwrap();
        db.query(
            "CREATE workouts:history SET source = 'csv';
             CREATE workouts:fresh SET source = 'manual';",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        let change = tokio::time::timeout(Duration::from_secs(2), changes.next())
            .await
            .expect("manual workout notification")
            .expect("live query remains open")
            .expect("valid live notification");
        assert_eq!(change.action, Action::Create);
        assert_eq!(change.data.id, "fresh");
    }
}
