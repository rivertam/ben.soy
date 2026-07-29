//! Podrick database models.
//!
//! Like the Spire and fitness models, the record identifier is projected to
//! its raw string key on load, so nothing outside the query layer ever sees a
//! SurrealDB record id.

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// Podrick's claim on one workout's announcement.
///
/// The row's existence means "this workout is mine to announce"; `message_id`
/// means "and I confirmed it landed". The two states are deliberately separate
/// so a process that dies between the claim and Discord's response leaves a
/// retryable row rather than either a lost announcement or a duplicate one.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickAnnouncement {
    pub id: String,
    pub workout_id: String,
    /// The workout's canonical public path segment (`/lifting/{path}`).
    pub workout_path: String,
    pub channel_id: String,
    pub message_id: Option<String>,
    pub claimed_at: i64,
    pub posted_at: Option<i64>,
    pub attempts: i64,
}

/// A string-valued cursor row (`podrick_meta:<k>`).
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickMeta {
    pub k: String,
    pub v: String,
}
