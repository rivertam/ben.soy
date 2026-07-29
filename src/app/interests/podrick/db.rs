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
    podrick_models::{PodrickAnnouncement, PodrickMeta},
};
use serde::Deserialize;
use surrealdb::types::SurrealValue;

/// The cursor key holding the newest workout that predates Podrick.
pub const ANNOUNCE_WATERMARK: &str = "announce_watermark";

/// The workout source Podrick announces. CSV history is deliberately excluded:
/// it never joins the homepage timeline or `/feed.xml` either, and a resync
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
