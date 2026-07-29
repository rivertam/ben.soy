//! What `/podrick` reads.
//!
//! The site never writes a `podrick_*` table — those belong to the worker
//! (`db.rs`). Keeping the read side in its own module means the site binary
//! does not compile the worker's write queries, and neither copy carries the
//! other's unused code.

use benjisponge::data::{
    Data, Db,
    podrick_models::{PodrickAnnouncement, PodrickPantsMessage},
};

/// What the page shows. A database that is unreachable or has never been
/// written yields an explicit unavailable/unseeded Pants state rather than an
/// error page.
#[derive(Clone, Debug, Default)]
pub struct PodrickStatus {
    pub posted: usize,
    pub latest: Option<PodrickAnnouncement>,
    pub pants: PantsStatus,
}

#[derive(Clone, Debug, Default)]
pub struct PantsStatus {
    pub database_available: bool,
    pub history_seeded: bool,
    pub messages: Vec<PodrickPantsMessage>,
}

pub async fn load(data: &Data) -> PodrickStatus {
    let Ok(db) = data.db().await else {
        return PodrickStatus::default();
    };
    let posted = posted_count(&db).await.unwrap_or(0);
    let latest = latest_posted(&db).await.unwrap_or_default();
    let pants = match (pants_seeded(&db).await, pants_messages(&db).await) {
        (Ok(history_seeded), Ok(messages)) => PantsStatus {
            database_available: true,
            history_seeded,
            messages,
        },
        _ => PantsStatus::default(),
    };
    PodrickStatus {
        posted: posted.max(0) as usize,
        latest,
        pants,
    }
}

async fn posted_count(db: &Db) -> surrealdb::Result<i64> {
    let mut response = db
        .query(
            "SELECT VALUE count()
             FROM podrick_announcements
             WHERE message_id IS NOT NONE
             GROUP ALL;",
        )
        .await?
        .check()?;
    let counts: Vec<i64> = response.take(0)?;
    Ok(counts.into_iter().next().unwrap_or(0))
}

async fn latest_posted(db: &Db) -> surrealdb::Result<Option<PodrickAnnouncement>> {
    let mut response = db
        .query(
            // Explicit projection, not `SELECT *`: option fields holding NONE
            // are omitted entirely rather than returned as null, which breaks
            // deserialization into the model's `Option` fields.
            "SELECT record::id(id) AS id, workout_id, workout_path, channel_id,
                    message_id, claimed_at, posted_at, attempts
             FROM podrick_announcements
             WHERE message_id IS NOT NONE
             ORDER BY posted_at DESC
             LIMIT 1;",
        )
        .await?
        .check()?;
    let mut rows: Vec<PodrickAnnouncement> = response.take(0)?;
    Ok(rows.pop())
}

async fn pants_seeded(db: &Db) -> surrealdb::Result<bool> {
    let mut response = db
        .query(
            "SELECT VALUE v
             FROM type::record('podrick_meta', 'pants_cursor');",
        )
        .await?
        .check()?;
    let values: Vec<String> = response.take(0)?;
    Ok(!values.is_empty())
}

async fn pants_messages(db: &Db) -> surrealdb::Result<Vec<PodrickPantsMessage>> {
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
