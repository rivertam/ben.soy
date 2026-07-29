//! Install a production `podrick_*` snapshot into an empty local database.
//!
//! Used by local reset (`PODRICK_SEED_URL` + `PODRICK_SYNC_TOKEN`) so Discord
//! history and announcement watermarks are not rebuilt from scratch.

use std::time::Duration;

use benjisponge::data::{Db, podrick_models::PodrickSeed};

use crate::db;

const SEED_URL_VAR: &str = "PODRICK_SEED_URL";
/// Older name from the pants-only seed; still accepted.
const LEGACY_SEED_URL_VAR: &str = "PODRICK_PANTS_SEED_URL";
const SYNC_TOKEN_VAR: &str = "PODRICK_SYNC_TOKEN";
const SEED_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum SeedError {
    Message(String),
    Database(String),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeedError::Message(error) => write!(f, "podrick seed: {error}"),
            SeedError::Database(error) => write!(f, "podrick seed database: {error}"),
        }
    }
}

impl std::error::Error for SeedError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeedReport {
    pub announcements: usize,
    pub pants_messages: usize,
    pub pants_actions: usize,
    pub meta: usize,
}

impl SeedReport {
    pub fn is_quiet(&self) -> bool {
        self.announcements == 0
            && self.pants_messages == 0
            && self.pants_actions == 0
            && self.meta == 0
    }
}

/// When local Podrick state is empty and a seed URL is configured, pull
/// production's full `podrick_*` snapshot. Returns `None` when seeding is
/// skipped (dry-run, unset URL, or local state already present).
pub async fn maybe_install_from_api(
    db: &Db,
    dry_run: bool,
    configured_pants_channel: Option<&str>,
) -> Result<Option<SeedReport>, SeedError> {
    if dry_run {
        return Ok(None);
    }
    let url = std::env::var(SEED_URL_VAR)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var(LEGACY_SEED_URL_VAR)
                .ok()
                .filter(|value| !value.is_empty())
        });
    let Some(url) = url else {
        return Ok(None);
    };
    if !db::podrick_state_empty(db)
        .await
        .map_err(|error| SeedError::Database(error.to_string()))?
    {
        return Ok(None);
    }
    let token = std::env::var(SYNC_TOKEN_VAR)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SeedError::Message(format!("seed URL is set but {SYNC_TOKEN_VAR} is missing"))
        })?;

    let seed = fetch_seed(&url, &token).await?;
    if let Some(stored) = seed.meta.get(db::PANTS_SOURCE_CHANNEL)
        && let Some(configured) = configured_pants_channel
        && stored != configured
    {
        return Err(SeedError::Message(format!(
            "seed pants_source_channel is {stored}, but local \
             PODRICK_PANTS_CHANNEL_ID is {configured}"
        )));
    }

    install_seed(db, seed).await
}

async fn install_seed(db: &Db, seed: PodrickSeed) -> Result<Option<SeedReport>, SeedError> {
    let report = SeedReport {
        announcements: seed.announcements.len(),
        pants_messages: seed.pants_messages.len(),
        pants_actions: seed.pants_actions.len(),
        meta: seed.meta.len(),
    };

    for announcement in &seed.announcements {
        db::store_announcement(db, announcement)
            .await
            .map_err(|error| SeedError::Database(error.to_string()))?;
    }
    for message in seed.pants_messages {
        db::store_pants_message(db, &message.into_row())
            .await
            .map_err(|error| SeedError::Database(error.to_string()))?;
    }
    for action in &seed.pants_actions {
        db::store_pants_action(db, action)
            .await
            .map_err(|error| SeedError::Database(error.to_string()))?;
    }

    // Write non-cursor meta first, then pants_cursor last so a crash mid-install
    // leaves an empty-looking DB (no cursor) that retries rather than a
    // half-seed that skips Discord incorrectly.
    let mut cursor = None;
    for (key, value) in &seed.meta {
        if key == db::PANTS_CURSOR {
            cursor = Some(value.clone());
            continue;
        }
        db::set_meta(db, key, value)
            .await
            .map_err(|error| SeedError::Database(error.to_string()))?;
    }
    if let Some(cursor) = cursor {
        db::set_meta(db, db::PANTS_CURSOR, &cursor)
            .await
            .map_err(|error| SeedError::Database(error.to_string()))?;
    }

    Ok(Some(report))
}

async fn fetch_seed(url: &str, token: &str) -> Result<PodrickSeed, SeedError> {
    let client = reqwest::Client::builder()
        .timeout(SEED_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| SeedError::Message(error.to_string()))?;
    let response = client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|error| SeedError::Message(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(SeedError::Message(format!(
            "GET {url} returned {status}: {}",
            truncate(&body)
        )));
    }
    response
        .json::<PodrickSeed>()
        .await
        .map_err(|error| SeedError::Message(format!("invalid seed JSON: {error}")))
}

fn truncate(body: &str) -> String {
    const LIMIT: usize = 300;
    let mut truncated = body.chars().take(LIMIT).collect::<String>();
    if body.chars().count() > LIMIT {
        truncated.push('…');
    }
    truncated
}
