//! What `/podrick` reads.
//!
//! The site never writes a `podrick_*` table — those belong to the worker
//! (`db.rs`). Keeping the read side in its own module means the site binary
//! does not compile the worker's write queries, and neither copy carries the
//! other's unused code.

use std::collections::BTreeMap;

use benjisponge::data::{
    Data, Db,
    podrick_models::{
        PantsSeedMessage, PodrickAnnouncement, PodrickMeta, PodrickPantsAction,
        PodrickPantsMessage, PodrickSeed,
    },
};

/// What the page shows. A database that is unreachable or has never been
/// written yields an explicit unavailable/unseeded state rather than an error
/// page.
#[derive(Clone, Debug, Default)]
pub struct PantsStatus {
    pub database_available: bool,
    pub history_seeded: bool,
    pub messages: Vec<PodrickPantsMessage>,
}

pub async fn load(data: &Data) -> PantsStatus {
    let Ok(db) = data.db().await else {
        return PantsStatus::default();
    };
    pants_status(&db).await.unwrap_or_default()
}

/// Full production `podrick_*` snapshot for local reset.
pub async fn export_podrick_seed(
    data: &Data,
) -> Result<PodrickSeed, Box<dyn std::error::Error + Send + Sync>> {
    let db = data.db().await?;
    Ok(podrick_seed_export(&db).await?)
}

/// The seed cursor and the source messages in one round trip. A failure of
/// either statement renders exactly like an unreachable database rather than
/// like an empty history.
async fn pants_status(db: &Db) -> surrealdb::Result<PantsStatus> {
    let mut response = db
        .query(
            "SELECT VALUE v FROM type::record('podrick_meta', 'pants_cursor');
             SELECT record::id(id) AS id, message_id, channel_id, author_id,
                    posted_at
             FROM podrick_pants_messages
             ORDER BY posted_at ASC;",
        )
        .await?
        .check()?;
    let cursor: Vec<String> = response.take(0)?;
    let messages: Vec<PodrickPantsMessage> = response.take(1)?;
    Ok(PantsStatus {
        database_available: true,
        history_seeded: !cursor.is_empty(),
        messages,
    })
}

async fn podrick_seed_export(db: &Db) -> surrealdb::Result<PodrickSeed> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, workout_id, workout_path, channel_id,
                    message_id, claimed_at, posted_at, attempts
             FROM podrick_announcements
             ORDER BY claimed_at ASC;
             SELECT record::id(id) AS id, message_id, channel_id, author_id,
                    posted_at
             FROM podrick_pants_messages
             ORDER BY posted_at ASC;
             SELECT record::id(id) AS id, action_kind, reason,
                    target_channel_id, source_message_id, content, claimed_at,
                    completed_at, output_message_id, attempts
             FROM podrick_pants_actions
             ORDER BY claimed_at ASC;
             SELECT k, v FROM podrick_meta;",
        )
        .await?
        .check()?;
    let announcements: Vec<PodrickAnnouncement> = response.take(0)?;
    let messages: Vec<PodrickPantsMessage> = response.take(1)?;
    let pants_actions: Vec<PodrickPantsAction> = response.take(2)?;
    let meta_rows: Vec<PodrickMeta> = response.take(3)?;
    let mut meta = BTreeMap::new();
    for row in meta_rows {
        meta.insert(row.k, row.v);
    }
    Ok(PodrickSeed {
        announcements,
        pants_messages: messages.iter().map(PantsSeedMessage::from).collect(),
        pants_actions,
        meta,
    })
}
