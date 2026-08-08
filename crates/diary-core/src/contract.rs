//! The sync boundary: the current canonical diary values and what each
//! response means to the flusher. Domain acceptance lives in [`crate::entry`].
//!
//! Persisted stores are migrated to one current shape. The epoch is therefore
//! an exact deployment fence, not a catalogue of wire adapters: stale clients
//! pause and keep their outbox until their worker updates and migrations run.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::entry::{ComposedEntry, DiaryEntry, SavedRef};

/// The queue-replay endpoint. The worker flushes here and nowhere else.
pub const API_PATH: &str = "/api/diary/entries";

/// The one schema epoch understood by this build. Server rows and device rows
/// are migrated to this shape before the build may sync them.
pub const CURRENT_SCHEMA_EPOCH: u16 = 1;

/// Required on every HTTP sync request. The server answers a mismatch with a
/// retryable status before it parses or returns canonical entry content.
pub const SCHEMA_EPOCH_HEADER: &str = "x-diary-schema-epoch";

/// Version-independent name of the record access method. The signed epoch
/// claim and migration-owned table permission provide the exact fence.
pub const DIRECT_ACCESS: &str = "diary_sync";

/// Strict current push envelope. Keeping the epoch in the body as well as the
/// request header makes a newer worker fail closed against an older server
/// that does not yet know to inspect the header.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PushWire {
    pub schema_epoch: u16,
    pub entry: ComposedEntry,
}

impl PushWire {
    pub fn new(entry: ComposedEntry) -> Self {
        Self {
            schema_epoch: CURRENT_SCHEMA_EPOCH,
            entry,
        }
    }
}

/// One serialized argument across the JavaScript→wasm enqueue Seam. Adding
/// entry behavior changes [`EntryContent`](crate::entry::EntryContent), not
/// the exported wasm function's positional signature.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComposeCommand {
    pub schema_epoch: u16,
    pub entry: ComposedEntry,
    pub enqueued_at_ms: i64,
}

impl ComposeCommand {
    pub fn new(entry: ComposedEntry, enqueued_at_ms: i64) -> Self {
        Self {
            schema_epoch: CURRENT_SCHEMA_EPOCH,
            entry,
            enqueued_at_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    Malformed,
    SchemaEpochMismatch(u16),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "malformed diary wire value"),
            Self::SchemaEpochMismatch(epoch) => {
                write!(f, "diary schema epoch {epoch} is not current")
            }
        }
    }
}

impl std::error::Error for WireError {}

fn require_current(epoch: u16) -> Result<(), WireError> {
    if epoch == CURRENT_SCHEMA_EPOCH {
        Ok(())
    } else {
        Err(WireError::SchemaEpochMismatch(epoch))
    }
}

/// Deserialize the current canonical value without silently consuming an
/// unknown business field. Serde cannot combine `deny_unknown_fields` with
/// the nested flattened lifecycle wrappers, so compare the source JSON with
/// the parsed value serialized back out.
fn decode_exact<T>(body: &[u8]) -> Result<T, WireError>
where
    T: DeserializeOwned + Serialize,
{
    let source: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| WireError::Malformed)?;
    let parsed: T = serde_json::from_slice(body).map_err(|_| WireError::Malformed)?;
    let canonical = serde_json::to_value(&parsed).map_err(|_| WireError::Malformed)?;
    if source == canonical {
        Ok(parsed)
    } else {
        Err(WireError::Malformed)
    }
}

pub fn decode_push(body: &[u8]) -> Result<ComposedEntry, WireError> {
    let wire: PushWire = decode_exact(body)?;
    require_current(wire.schema_epoch)?;
    Ok(wire.entry)
}

pub fn decode_compose_command(body: &str) -> Result<ComposeCommand, WireError> {
    let command: ComposeCommand = decode_exact(body.as_bytes())?;
    require_current(command.schema_epoch)?;
    Ok(command)
}

/// What one replay attempt means for the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The server holds the entry — a fresh insert or a deduped replay —
    /// under the identity it reported back.
    Saved(SavedRef),
    /// 401/404: signed out or the wrong account. Retrying cannot fix it
    /// (and burns Background Sync's bounded attempt budget), so the flush
    /// stops quietly and the page shows the sign-in banner.
    Auth,
    /// Network failure, epoch skew, 403, 5xx, or a 200 that is not our JSON —
    /// never the entry's fault; stop and retry later.
    Retry,
    /// 409/413/415/422: the entry itself can never succeed. It is marked
    /// failed and kept on the page for manual copy.
    Rejected(u16),
}

pub fn classify_response(status: u16, body: &str) -> SendOutcome {
    #[derive(Deserialize)]
    struct SavedResponse {
        status: String,
        id: String,
        written_at: i64,
    }

    match status {
        200..=299 => match serde_json::from_str::<SavedResponse>(body) {
            Ok(response) if response.status == "saved" => SendOutcome::Saved(SavedRef {
                id: response.id,
                written_at: response.written_at,
            }),
            _ => SendOutcome::Retry,
        },
        401 | 404 => SendOutcome::Auth,
        // A generated request cannot be malformed. An older server may answer
        // the current envelope with 400, so deployment skew remains retryable.
        400 => SendOutcome::Retry,
        409 | 413 | 415 | 422 => SendOutcome::Rejected(status),
        _ => SendOutcome::Retry,
    }
}

pub fn rejection_reason(status: u16) -> String {
    format!("rejected (HTTP {status})")
}

/// The snapshot endpoint the worker pulls the mirror from.
pub const SNAPSHOT_PATH: &str = "/api/diary/snapshot";

/// The token mint for direct client→SurrealDB sync (flag-gated; 404 when the
/// flag is off).
pub const TOKEN_PATH: &str = "/api/diary/token";

/// The cookie-gated grant that arms one direct database sync pass.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectTokenGrant {
    pub token: String,
    pub ns: String,
    pub db: String,
    pub schema_epoch: u16,
}

impl DirectTokenGrant {
    pub fn new(token: impl Into<String>, ns: impl Into<String>, db: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            ns: ns.into(),
            db: db.into(),
            schema_epoch: CURRENT_SCHEMA_EPOCH,
        }
    }

    pub fn supports_current(&self) -> bool {
        self.schema_epoch == CURRENT_SCHEMA_EPOCH
    }
}

/// Strict current snapshot envelope.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotWire {
    pub schema_epoch: u16,
    pub entries: Vec<DiaryEntry>,
}

impl SnapshotWire {
    pub fn new(entries: Vec<DiaryEntry>) -> Self {
        Self {
            schema_epoch: CURRENT_SCHEMA_EPOCH,
            entries,
        }
    }
}

pub fn decode_snapshot(body: &[u8]) -> Result<Vec<DiaryEntry>, WireError> {
    let wire: SnapshotWire = decode_exact(body)?;
    require_current(wire.schema_epoch)?;
    Ok(wire.entries)
}

/// What one pull attempt means for the mirror.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullOutcome {
    Data(Vec<DiaryEntry>),
    Auth,
    /// Any transport, epoch, or parse failure. The mirror is untouched.
    Retry,
}

pub fn classify_pull(status: u16, body: &str) -> PullOutcome {
    match status {
        200..=299 => match decode_snapshot(body.as_bytes()) {
            Ok(entries) => PullOutcome::Data(entries),
            Err(_) => PullOutcome::Retry,
        },
        401 | 404 => PullOutcome::Auth,
        _ => PullOutcome::Retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_push_round_trips_exactly() {
        let entry = ComposedEntry::new(1_753_640_000, "Dear diary,\n\nIt me.");
        let json = serde_json::to_vec(&PushWire::new(entry.clone())).unwrap();
        assert_eq!(decode_push(&json).unwrap(), entry);

        let mut unknown_entry = serde_json::to_value(PushWire::new(ComposedEntry::new(1, "x")))
            .expect("wire serializes");
        unknown_entry["entry"]["extra"] = serde_json::json!(true);
        let mut unknown_envelope = serde_json::to_value(PushWire::new(ComposedEntry::new(1, "x")))
            .expect("wire serializes");
        unknown_envelope["extra"] = serde_json::json!(true);
        for bad in [
            br#"{"written_at":1753640000,"body":"old"}"#.to_vec(),
            br#"{"schema_epoch":0,"entry":{"written_at":1753640000,"body":"old"}}"#.to_vec(),
            serde_json::to_vec(&unknown_entry).unwrap(),
            serde_json::to_vec(&unknown_envelope).unwrap(),
            b"not json".to_vec(),
        ] {
            assert!(
                decode_push(&bad).is_err(),
                "accepted {}",
                String::from_utf8_lossy(&bad)
            );
        }
        assert_eq!(
            decode_push(br#"{"schema_epoch":99,"entry":{"written_at":1,"body":"x"}}"#),
            Err(WireError::SchemaEpochMismatch(99))
        );
    }

    #[test]
    fn compose_command_requires_the_current_epoch_and_exact_shape() {
        let command = ComposeCommand::new(ComposedEntry::new(12, "queued"), 34_000);
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(decode_compose_command(&json).unwrap(), command);
        assert_eq!(
            decode_compose_command(
                r#"{"schema_epoch":99,"entry":{"written_at":12,"body":"queued"},"enqueued_at_ms":34000}"#
            ),
            Err(WireError::SchemaEpochMismatch(99))
        );
        let mut unknown = serde_json::to_value(&command).expect("command serializes");
        unknown["extra"] = serde_json::json!(true);
        assert_eq!(
            decode_compose_command(&serde_json::to_string(&unknown).unwrap()),
            Err(WireError::Malformed)
        );
    }

    #[test]
    fn snapshots_require_the_current_epoch_and_exact_shape() {
        let entry = DiaryEntry::from_parts("one", 1, "first");
        let current = serde_json::to_vec(&SnapshotWire::new(vec![entry.clone()])).unwrap();
        assert_eq!(decode_snapshot(&current).unwrap(), vec![entry]);
        assert_eq!(
            decode_snapshot(br#"{"entries":[]}"#),
            Err(WireError::Malformed)
        );
        assert_eq!(
            decode_snapshot(br#"{"schema_epoch":99,"entries":[]}"#),
            Err(WireError::SchemaEpochMismatch(99))
        );
        let mut unknown = serde_json::to_value(SnapshotWire::new(vec![DiaryEntry::from_parts(
            "one", 1, "first",
        )]))
        .expect("snapshot serializes");
        unknown["entries"][0]["extra"] = serde_json::json!(true);
        assert_eq!(
            decode_snapshot(&serde_json::to_vec(&unknown).unwrap()),
            Err(WireError::Malformed)
        );
    }

    #[test]
    fn responses_preserve_outbox_on_epoch_skew() {
        assert_eq!(
            classify_response(
                200,
                r#"{"status":"saved","id":"x","written_at":1,"deduped":false}"#
            ),
            SendOutcome::Saved(SavedRef {
                id: "x".to_string(),
                written_at: 1,
            })
        );
        assert_eq!(
            classify_response(200, "<html>hotel wifi</html>"),
            SendOutcome::Retry
        );
        assert_eq!(classify_response(401, ""), SendOutcome::Auth);
        assert_eq!(classify_response(404, ""), SendOutcome::Auth);
        assert_eq!(classify_response(400, ""), SendOutcome::Retry);
        assert_eq!(classify_response(503, ""), SendOutcome::Retry);
        for permanent in [409, 413, 415, 422] {
            assert_eq!(
                classify_response(permanent, ""),
                SendOutcome::Rejected(permanent)
            );
        }
        assert_eq!(rejection_reason(422), "rejected (HTTP 422)");
    }

    #[test]
    fn direct_grants_carry_the_exact_schema_epoch() {
        let grant = DirectTokenGrant::new("token", "namespace", "database");
        assert!(grant.supports_current());
        let json = serde_json::to_string(&grant).unwrap();
        assert!(json.contains(&format!(r#""schema_epoch":{CURRENT_SCHEMA_EPOCH}"#)));
        let future = json.replace(
            &format!(r#""schema_epoch":{CURRENT_SCHEMA_EPOCH}"#),
            r#""schema_epoch":99"#,
        );
        let future: DirectTokenGrant = serde_json::from_str(&future).unwrap();
        assert!(!future.supports_current());
    }
}
