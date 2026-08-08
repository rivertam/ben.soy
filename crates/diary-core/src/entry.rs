//! Canonical diary-entry values, from composition through persistence.
//!
//! Business fields live in [`EntryContent`]. [`ComposedEntry`] pairs them
//! with a proposed placement second, and [`DiaryEntry`] adds the Entry Key
//! selected by placement. Collision handling may re-anchor the proposed
//! second. The two wrappers flatten for JSON and SurrealDB, preserving the
//! existing row/wire shape while giving Rust one value to pass between
//! adapters.

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use surrealdb::types::{SurrealValue, Value};

/// Mirrors the schema's `string::len($value) <= 65536` ASSERT on
/// `diary_entries.body` (a test in `diary.rs` holds the two together).
pub const MAX_ENTRY_CHARS: usize = 65_536;

/// Proposed placement seconds may trail "now" by up to a year (a long-
/// offline queue) and lead it by five minutes (clock skew). Out of window is
/// a rejection, never a clamp: clamping would mint a fresh key per replay,
/// so retrying a write whose response was lost would double-post.
pub const MAX_PAST_SECONDS: i64 = 365 * 24 * 60 * 60;
pub const MAX_FUTURE_SECONDS: i64 = 300;

/// Why a composed entry cannot be accepted by a remote store. Queue
/// preparation is intentionally looser: it preserves over-length and
/// clock-skewed text locally so the remote can reject it in place rather
/// than the page falling back to a lossy form submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryRejection {
    TimestampOutOfRange,
    InvalidBody,
    BodyTooLong,
}

impl EntryRejection {
    pub const fn status_code(self) -> u16 {
        422
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::TimestampOutOfRange => "timestamp out of range",
            Self::InvalidBody | Self::BodyTooLong => "that didn't validate",
        }
    }
}

/// Fields that make two diary entries the same thought for replay dedupe.
///
/// New business relationships belong here. Equality is deliberately only
/// content equality: ids, collision-bumped seconds, and local sync state do
/// not decide whether a replay is the same entry.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, SurrealValue)]
pub struct EntryContent {
    pub body: String,
}

impl EntryContent {
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }
}

/// An entry prepared for placement. `written_at` proposes the first second
/// to try; collision placement assigns its Entry Key and may bump the
/// stored second.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, SurrealValue)]
pub struct ComposedEntry {
    pub written_at: i64,
    #[serde(flatten)]
    #[surreal(flatten)]
    pub content: EntryContent,
}

impl ComposedEntry {
    pub fn new(written_at: i64, body: impl Into<String>) -> Self {
        Self::from_content(written_at, EntryContent::new(body))
    }

    pub fn from_content(written_at: i64, content: EntryContent) -> Self {
        Self {
            written_at,
            content,
        }
    }

    /// Keep the thought but assign the actual second selected by collision
    /// placement.
    pub fn placed_at(&self, written_at: i64) -> Self {
        Self::from_content(written_at, self.content.clone())
    }
}

impl Deref for ComposedEntry {
    type Target = EntryContent;

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

impl DerefMut for ComposedEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.content
    }
}

/// One persisted diary entry. `id` is the Eastern public-path record key;
/// a device may predict it before the server confirms it. The composed
/// fields flatten beside it in both JSON and SurrealDB rows.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, SurrealValue)]
pub struct DiaryEntry {
    pub id: String,
    #[serde(flatten)]
    #[surreal(flatten)]
    pub composed: ComposedEntry,
}

impl DiaryEntry {
    pub fn new(id: impl Into<String>, composed: ComposedEntry) -> Self {
        Self {
            id: id.into(),
            composed,
        }
    }

    pub fn from_parts(id: impl Into<String>, written_at: i64, body: impl Into<String>) -> Self {
        Self::new(id, ComposedEntry::new(written_at, body))
    }

    /// Replay identity ignores the requested/stored second and compares all
    /// business content in one place.
    pub fn has_content(&self, content: &EntryContent) -> bool {
        self.content == *content
    }
}

impl Deref for DiaryEntry {
    type Target = ComposedEntry;

    fn deref(&self) -> &Self::Target {
        &self.composed
    }
}

impl DerefMut for DiaryEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.composed
    }
}

/// The identity the server assigned a saved entry: the permalink id and the
/// second it actually landed on. Not always the proposed `written_at` — the
/// collision probes may have bumped it — so the page must render these
/// fields rather than assuming the prediction won.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SavedRef {
    pub id: String,
    pub written_at: i64,
}

impl SavedRef {
    pub fn new(id: impl Into<String>, written_at: i64) -> Self {
        Self {
            id: id.into(),
            written_at,
        }
    }
}

impl From<&DiaryEntry> for SavedRef {
    fn from(entry: &DiaryEntry) -> Self {
        Self::new(entry.id.clone(), entry.written_at)
    }
}

/// The explicit projection every flat diary-entry row read shares. Optional
/// business fields added later are added here once, then inherited by both
/// server and device-store reads.
pub const PROJECTION: &str = "record::id(id) AS id, written_at, body";

pub fn written_at_in_window(written_at: i64, now: i64) -> bool {
    written_at > now - MAX_PAST_SECONDS && written_at <= now + MAX_FUTURE_SECONDS
}

/// Normalize browser textarea line endings and surrounding whitespace.
/// Interior blank lines survive.
pub fn normalize_lines(raw: &str) -> String {
    let body = raw.replace("\r\n", "\n").replace('\r', "\n");
    body.trim().to_string()
}

/// Intrinsic preparation shared by every device-store enqueue path. It
/// normalizes content and rejects only an empty entry; remote-only policy
/// (age and size) must not prevent text from entering the durable queue.
pub fn prepare_for_queue(mut entry: ComposedEntry) -> Result<ComposedEntry, EntryRejection> {
    entry.body = normalize_lines(&entry.body);
    if entry.body.is_empty() {
        Err(EntryRejection::InvalidBody)
    } else {
        Ok(entry)
    }
}

/// The single remote acceptance boundary. Both HTTP and direct-to-database
/// adapters reach this through `store::save_entry`, and the plain form uses
/// it before insertion, so acceptance cannot drift between transports.
pub fn accept_for_save(entry: ComposedEntry, now: i64) -> Result<ComposedEntry, EntryRejection> {
    if !written_at_in_window(entry.written_at, now) {
        return Err(EntryRejection::TimestampOutOfRange);
    }
    let entry = prepare_for_queue(entry)?;
    if entry.body.chars().count() > MAX_ENTRY_CHARS {
        return Err(EntryRejection::BodyTooLong);
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_values_keep_the_flat_external_shape() {
        let composed = ComposedEntry::new(1_753_640_000, "Dear diary,");
        assert_eq!(
            serde_json::to_string(&composed).unwrap(),
            r#"{"written_at":1753640000,"body":"Dear diary,"}"#
        );
        let entry = DiaryEntry::new("entry-id", composed);
        assert_eq!(
            serde_json::to_string(&entry).unwrap(),
            r#"{"id":"entry-id","written_at":1753640000,"body":"Dear diary,"}"#
        );
        assert_eq!(entry.body, "Dear diary,");
    }

    #[test]
    fn flattened_values_parse_and_require_the_canonical_fields() {
        let entry: DiaryEntry = serde_json::from_str(
            r#"{"id":"entry-id","written_at":1753640000,"body":"Dear diary,"}"#,
        )
        .unwrap();
        assert_eq!(entry.id, "entry-id");
        assert_eq!(entry.written_at, 1_753_640_000);
        assert!(serde_json::from_str::<ComposedEntry>(r#"{"body":"x"}"#).is_err());
        assert!(
            serde_json::from_str::<DiaryEntry>(r#"{"written_at":1753640000,"body":"x"}"#).is_err()
        );
    }

    #[test]
    fn content_equality_ignores_placement_identity() {
        let requested = ComposedEntry::new(100, "same thought");
        let placed = DiaryEntry::new("bumped", requested.placed_at(101));
        assert!(placed.has_content(&requested.content));
        assert_ne!(placed.composed, requested);
    }

    #[test]
    fn bodies_normalize_line_ends_and_bounds() {
        assert_eq!(normalize_lines("hi\r\nthere\r\n"), "hi\nthere");
        assert_eq!(normalize_lines("  \r\n\t "), "");
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        assert_eq!(normalize_lines(&oversized), oversized);

        let exactly_max = "é".repeat(MAX_ENTRY_CHARS);
        assert!(accept_for_save(ComposedEntry::new(100, &exactly_max), 100).is_ok());
        assert_eq!(
            accept_for_save(
                ComposedEntry::new(100, "a".repeat(MAX_ENTRY_CHARS + 1)),
                100
            ),
            Err(EntryRejection::BodyTooLong)
        );
    }

    #[test]
    fn queue_preparation_preserves_text_for_remote_rejection() {
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        let queued =
            prepare_for_queue(ComposedEntry::new(100, format!("\r\n{oversized}\r\n"))).unwrap();
        assert_eq!(queued.body, oversized);
        assert_eq!(
            accept_for_save(queued, 100),
            Err(EntryRejection::BodyTooLong)
        );
        assert_eq!(
            prepare_for_queue(ComposedEntry::new(100, " \r\n\t ")),
            Err(EntryRejection::InvalidBody)
        );
    }

    #[test]
    fn remote_acceptance_normalizes_and_enforces_the_timestamp_window() {
        let now = 1_800_000_000;
        let accepted = accept_for_save(ComposedEntry::new(now, "  one\r\ntwo  "), now).unwrap();
        assert_eq!(accepted.body, "one\ntwo");
        assert_eq!(
            accept_for_save(ComposedEntry::new(now - MAX_PAST_SECONDS, "old"), now),
            Err(EntryRejection::TimestampOutOfRange)
        );
        assert_eq!(
            accept_for_save(
                ComposedEntry::new(now + MAX_FUTURE_SECONDS + 1, "future"),
                now
            ),
            Err(EntryRejection::TimestampOutOfRange)
        );
    }

    #[test]
    fn proposed_seconds_stay_inside_the_window() {
        let now = 1_800_000_000;
        assert!(written_at_in_window(now, now));
        assert!(written_at_in_window(now - MAX_PAST_SECONDS + 1, now));
        assert!(!written_at_in_window(now - MAX_PAST_SECONDS, now));
        assert!(written_at_in_window(now + MAX_FUTURE_SECONDS, now));
        assert!(!written_at_in_window(now + MAX_FUTURE_SECONDS + 1, now));
        assert!(!written_at_in_window(0, now));
    }
}
