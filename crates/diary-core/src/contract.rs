//! The wire half of the queue: what an entry looks like on
//! `POST /api/diary/entries`, which composition seconds the server accepts,
//! and what each response means to the flusher. The server parses and
//! validates with these exact items; the worker serializes and classifies
//! with them. A change here changes both sides in the same commit — that is
//! the point.

use serde::{Deserialize, Serialize};

/// The queue-replay endpoint. The worker flushes here and nowhere else.
pub const API_PATH: &str = "/api/diary/entries";

/// Mirrors the schema's `string::len($value) <= 65536` ASSERT on
/// `diary_entries.body` (a test in `diary.rs` holds the two together).
pub const MAX_ENTRY_CHARS: usize = 65_536;

/// Composition timestamps may trail "now" by up to a year (a long-offline
/// queue) and lead it by five minutes (clock skew). Out of window is a 422,
/// never a clamp: clamping would mint a fresh key per replay, so retrying a
/// write whose response was lost would double-post.
pub const MAX_PAST_SECONDS: i64 = 365 * 24 * 60 * 60;
pub const MAX_FUTURE_SECONDS: i64 = 300;

/// Same-second key collisions probe forward at most this many seconds.
pub const COLLISION_PROBES: i64 = 5;

/// One queued entry on the wire: the entry text plus the second it was
/// composed (the entry keeps the time it was written, not the time it
/// synced). `deny_unknown_fields` keeps the contract exact — a shape drift
/// between worker and server should fail loudly, not half-parse.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireEntry {
    pub written_at: i64,
    pub body: String,
}

pub fn written_at_in_window(written_at: i64, now: i64) -> bool {
    written_at > now - MAX_PAST_SECONDS && written_at <= now + MAX_FUTURE_SECONDS
}

/// Line-ending and whitespace normalization alone: browser textareas submit
/// CRLF line ends; store LF. Trimmed at both ends, interior blank lines
/// survive. The queue applies exactly this much — deliberately NOT the
/// length bound, which only the server enforces (see [`normalize_body`]).
pub fn normalize_lines(raw: &str) -> String {
    let body = raw.replace("\r\n", "\n").replace('\r', "\n");
    body.trim().to_string()
}

/// The server's full validation: [`normalize_lines`] plus the char bound
/// mirroring the schema ASSERT. `None` is "not an entry the server would
/// store". The queue does not use the bound on purpose — an over-length
/// entry must be QUEUED, replayed, 422'd, and kept on the page as failed
/// text, never bounced into a lossy fallback (queued diary text is never
/// silently dropped).
pub fn normalize_body(raw: &str) -> Option<String> {
    let body = normalize_lines(raw);
    if body.is_empty() || body.chars().count() > MAX_ENTRY_CHARS {
        return None;
    }
    Some(body)
}

/// The identity the server assigned a saved entry: the permalink id and the
/// second it actually landed on. Not always the queued `written_at` — the
/// collision probes may have bumped it — so the page must render THESE, not
/// what it enqueued.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SavedRef {
    pub id: String,
    pub written_at: i64,
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
    /// Network failure, 403, 5xx, or a 200 that is not our JSON (a captive
    /// portal) — never the entry's fault; stop and retry later.
    Retry,
    /// 400/409/413/415/422: the entry itself can never succeed. It is
    /// marked failed and kept on the page for manual copy — queued diary
    /// text is never silently dropped.
    Rejected(u16),
}

/// Classify one response from the replay endpoint. `body` is only consulted
/// for 2xx, where anything but our full saved JSON — status, id, and
/// written_at, the shape `api_write_entry` always returns — means something
/// other than the site answered.
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
        400 | 409 | 413 | 415 | 422 => SendOutcome::Rejected(status),
        _ => SendOutcome::Retry,
    }
}

/// The reason a permanent rejection leaves on the queued entry — the exact
/// text the page has always rendered next to failed entries.
pub fn rejection_reason(status: u16) -> String {
    format!("rejected (HTTP {status})")
}

/// The snapshot endpoint the worker pulls the mirror from.
pub const SNAPSHOT_PATH: &str = "/api/diary/snapshot";

/// One entry in a pull snapshot — the server fields, nothing else. The
/// server serializes exactly this; the worker deserializes exactly this.
/// `deny_unknown_fields` for the same reason as [`WireEntry`]: a drift
/// between the two sides must fail loudly, never half-parse.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEntry {
    pub id: String,
    pub written_at: i64,
    pub body: String,
}

/// The snapshot wire shape: `{"entries": [...]}`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotWire {
    pub entries: Vec<SnapshotEntry>,
}

/// What one pull attempt means for the mirror.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullOutcome {
    /// A strictly parsed snapshot from OUR server. The only outcome that
    /// may touch the mirror.
    Data(Vec<SnapshotEntry>),
    /// 401/404: signed out or the wrong account. No mirror changes.
    Auth,
    /// Anything else — network failure, 5xx, or a 200 whose body is not our
    /// exact snapshot JSON (a captive portal). No mirror changes. This
    /// classification is load-bearing: a pull that read hotel-wifi HTML as
    /// "zero entries" would delete every synced row on the device.
    Retry,
}

/// Classify one response from the snapshot endpoint, mirroring
/// [`classify_response`]'s rules for the push side.
pub fn classify_pull(status: u16, body: &str) -> PullOutcome {
    match status {
        200..=299 => match serde_json::from_str::<SnapshotWire>(body) {
            Ok(wire) => PullOutcome::Data(wire.entries),
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
    fn bodies_normalize_line_ends_and_bounds() {
        // The queue's half: normalization without the bound.
        assert_eq!(normalize_lines("hi\r\nthere\r\n"), "hi\nthere");
        assert_eq!(normalize_lines("  \r\n\t "), "");
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        assert_eq!(normalize_lines(&oversized), oversized);
        assert_eq!(
            normalize_body("Dear diary,\r\n\r\nIt me.\r\n").as_deref(),
            Some("Dear diary,\n\nIt me.")
        );
        assert_eq!(
            normalize_body("solo\rreturn").as_deref(),
            Some("solo\nreturn")
        );
        assert_eq!(normalize_body(""), None);
        assert_eq!(normalize_body("  \r\n\t "), None);
        let exactly_max = "é".repeat(MAX_ENTRY_CHARS);
        assert_eq!(
            normalize_body(&exactly_max).as_deref(),
            Some(exactly_max.as_str())
        );
        assert_eq!(normalize_body(&"a".repeat(MAX_ENTRY_CHARS + 1)), None);
    }

    #[test]
    fn client_timestamps_stay_inside_the_window() {
        let now = 1_800_000_000;
        assert!(written_at_in_window(now, now));
        assert!(written_at_in_window(now - MAX_PAST_SECONDS + 1, now));
        assert!(!written_at_in_window(now - MAX_PAST_SECONDS, now));
        assert!(written_at_in_window(now + MAX_FUTURE_SECONDS, now));
        assert!(!written_at_in_window(now + MAX_FUTURE_SECONDS + 1, now));
        assert!(!written_at_in_window(0, now));
    }

    #[test]
    fn wire_entries_parse_strictly() {
        let entry: WireEntry =
            serde_json::from_slice(br#"{"written_at": 1753640000, "body": "Dear diary,"}"#)
                .unwrap();
        assert_eq!(entry.written_at, 1_753_640_000);
        assert_eq!(entry.body, "Dear diary,");
        for bad in [
            &br#"{"written_at": 1753640000.5, "body": "x"}"#[..],
            br#"{"written_at": "1753640000", "body": "x"}"#,
            br#"{"body": "x"}"#,
            br#"{"written_at": 1753640000}"#,
            br#"{"written_at": 1753640000, "body": "x", "extra": true}"#,
            b"not json",
        ] {
            assert!(
                serde_json::from_slice::<WireEntry>(bad).is_err(),
                "accepted {:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    /// What the worker sends is exactly what the server parses: the wire
    /// struct round-trips through its own serialization.
    #[test]
    fn wire_entries_round_trip() {
        let entry = WireEntry {
            written_at: 1_753_640_000,
            body: "Dear diary,\n\nIt me.".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: WireEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.written_at, entry.written_at);
        assert_eq!(back.body, entry.body);
    }

    #[test]
    fn responses_classify_like_the_old_worker() {
        // A save carries the server-assigned identity out of the response —
        // the page renders the delivered message from these fields.
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
        // A 200 that is not our JSON is a captive portal, not a save; our
        // server never sends "saved" without the identity fields, so a
        // partial shape classifies the same way (the retry re-asks and the
        // dedupe absorbs it).
        assert_eq!(
            classify_response(200, "<html>hotel wifi</html>"),
            SendOutcome::Retry
        );
        assert_eq!(classify_response(200, ""), SendOutcome::Retry);
        assert_eq!(
            classify_response(200, r#"{"status":"saved"}"#),
            SendOutcome::Retry
        );
        assert_eq!(classify_response(401, ""), SendOutcome::Auth);
        assert_eq!(classify_response(404, ""), SendOutcome::Auth);
        for permanent in [400, 409, 413, 415, 422] {
            assert_eq!(
                classify_response(permanent, ""),
                SendOutcome::Rejected(permanent),
                "status {permanent}"
            );
        }
        // 403 is a same-origin/config failure and 5xx an outage — never the
        // entry's fault, always retried later.
        for retryable in [403, 500, 502, 503] {
            assert_eq!(classify_response(retryable, ""), SendOutcome::Retry);
        }
        assert_eq!(rejection_reason(422), "rejected (HTTP 422)");
    }
}
