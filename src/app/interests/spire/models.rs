//! Spire database models.
//!
//! The record identifier is projected to its raw string key when these
//! models are loaded, so the site's domain and API continue to use the run
//! file stem rather than exposing SurrealDB record ids.

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// A Slay the Spire run, minus the original `.run` payload.
///
/// `raw` deliberately lives in [`SpireRunRaw`]: dragging ~100 KB of JSON per
/// run into every list read would swamp the container. Splitting the table
/// makes `raw` write-only by construction.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct SpireRun {
    pub id: String,
    pub game: String,
    pub date: String,
    pub start_time: i64,
    pub character: String,
    pub win: bool,
    pub abandoned: bool,
    pub ascension: i64,
    pub acts: i64,
    pub floors: i64,
    pub killed_by: Option<String>,
    pub kill_kind: Option<String>,
    pub run_time: i64,
    pub seed: String,
    pub game_mode: String,
    pub build_id: String,
    pub added_at: i64,
}

/// The whole original `.run` file, kept so future redesigns never need a
/// re-scrape. Written by import, read by nothing.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct SpireRunRaw {
    pub id: String,
    pub game: String,
    pub raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct SpireMeta {
    pub k: String,
    pub v: i64,
}
