use serde::{Deserialize, Serialize};

use crate::draft::{ActionError, FinalizedWorkout};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutboxState {
    Pending,
    Failed,
    Saved,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QueuedWorkout {
    pub queue_id: String,
    pub enqueued_at_ms: u64,
    pub state: OutboxState,
    /// Immutable after finalization. Queue transitions may change only the
    /// delivery metadata around this value.
    pub workout: FinalizedWorkout,
    pub predicted_location: Option<String>,
    pub receipt: Option<PublishReceipt>,
    pub failure: Option<String>,
    /// A timestamp collision cannot be repaired by editing sets or copy. The
    /// restored mutable draft receives a fresh start before another attempt.
    #[serde(default)]
    pub rebase_on_restore: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublishReceipt {
    pub location: String,
    pub duplicate: bool,
    pub share_text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Publication {
    pub path: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseDisposition {
    Saved { receipt: PublishReceipt },
    AuthBlocked,
    Failed { reason: String, collision: bool },
    Retry { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedResponse {
    pub queued: QueuedWorkout,
    pub continue_flushing: bool,
    pub auth_blocked: bool,
}

/// Return the device queue in its canonical presentation/delivery order.
pub fn ordered_outbox(outbox: &[QueuedWorkout]) -> Vec<QueuedWorkout> {
    let mut ordered = outbox.to_vec();
    ordered.sort_by(|left, right| {
        left.enqueued_at_ms
            .cmp(&right.enqueued_at_ms)
            .then_with(|| left.queue_id.cmp(&right.queue_id))
    });
    ordered
}

/// Select pending work oldest-first. Storage adapters never choose delivery
/// order or reinterpret queue states themselves.
pub fn pending_outbox(outbox: &[QueuedWorkout]) -> Vec<QueuedWorkout> {
    ordered_outbox(outbox)
        .into_iter()
        .filter(|queued| queued.state == OutboxState::Pending)
        .collect()
}

pub fn publication(queued: &QueuedWorkout) -> Result<Publication, ActionError> {
    if queued.state != OutboxState::Pending {
        return Err(ActionError::message(
            "Only a pending workout can be published.",
        ));
    }
    let body = serde_json::to_string(&queued.workout)
        .map_err(|_| ActionError::message("The queued workout could not be encoded."))?;
    Ok(Publication {
        path: crate::PUBLISH_PATH.to_string(),
        body,
    })
}

pub fn classify_response(status: u16, body: &str) -> ResponseDisposition {
    match status {
        200 => serde_json::from_str::<PublishReceipt>(body)
            .ok()
            .filter(valid_receipt)
            .map_or_else(
                || ResponseDisposition::Retry {
                    reason: "The server returned a malformed success response.".to_string(),
                },
                |receipt| ResponseDisposition::Saved { receipt },
            ),
        401 | 404 => ResponseDisposition::AuthBlocked,
        409 | 413 | 415 | 422 => ResponseDisposition::Failed {
            reason: response_error(body).unwrap_or_else(|| format!("Workout rejected ({status}).")),
            collision: status == 409,
        },
        _ => ResponseDisposition::Retry {
            reason: response_error(body).unwrap_or_else(|| format!("Publish deferred ({status}).")),
        },
    }
}

fn valid_receipt(receipt: &PublishReceipt) -> bool {
    let Some(segment) = receipt.location.strip_prefix("/fitness/lift/") else {
        return false;
    };
    !segment.contains(['/', '?', '#'])
        && eastern_time::parse_public_path(segment).is_some()
        && !receipt.share_text.is_empty()
}

fn response_error(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: String,
    }
    let parsed: ErrorBody = serde_json::from_str(body).ok()?;
    let message = parsed.error.trim();
    (!message.is_empty()).then(|| message.chars().take(500).collect())
}

pub fn apply_response(queued: &QueuedWorkout, disposition: ResponseDisposition) -> AppliedResponse {
    if queued.state != OutboxState::Pending {
        return AppliedResponse {
            queued: queued.clone(),
            continue_flushing: true,
            auth_blocked: false,
        };
    }
    let mut next = queued.clone();
    let (continue_flushing, auth_blocked) = match disposition {
        ResponseDisposition::Saved { receipt } => {
            next.state = OutboxState::Saved;
            next.predicted_location = None;
            next.receipt = Some(receipt);
            next.failure = None;
            next.rebase_on_restore = false;
            (true, false)
        }
        ResponseDisposition::AuthBlocked => (false, true),
        ResponseDisposition::Failed { reason, collision } => {
            next.state = OutboxState::Failed;
            // A rejected candidate is never presented as hosted.
            next.predicted_location = None;
            next.receipt = None;
            next.failure = Some(reason);
            next.rebase_on_restore = collision;
            (true, false)
        }
        ResponseDisposition::Retry { .. } => (false, false),
    };
    AppliedResponse {
        queued: next,
        continue_flushing,
        auth_blocked,
    }
}

pub fn dismiss_receipt(queued: &QueuedWorkout) -> Result<(), ActionError> {
    (queued.state == OutboxState::Saved)
        .then_some(())
        .ok_or_else(|| ActionError::message("Only a saved Workout Receipt can be dismissed."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{FinalizedExercise, FinalizedSet};

    fn queued() -> QueuedWorkout {
        QueuedWorkout {
            queue_id: "queue-00000001".into(),
            enqueued_at_ms: 10,
            state: OutboxState::Pending,
            workout: FinalizedWorkout {
                started_at_utc: "2026-09-03 14:00:00".into(),
                ended_at_utc: "2026-09-03 15:00:00".into(),
                title: "Workout".into(),
                notes: None,
                exercises: vec![FinalizedExercise {
                    name: "Squat".into(),
                    sets: vec![FinalizedSet {
                        weight_milli: Some(225_000),
                        reps: 5,
                        effort_hundredths: Some(900),
                        failure: false,
                        set_type: "NORMAL_SET".into(),
                    }],
                }],
            },
            predicted_location: Some("/fitness/lift/2026-09-03T10-00-00-04-00".into()),
            receipt: None,
            failure: None,
            rebase_on_restore: false,
        }
    }

    fn success() -> &'static str {
        r#"{"location":"/fitness/lift/2026-09-03T10-00-00-04-00","duplicate":false,"share_text":"Workout\nhttps://ben.soy/fitness/lift/2026-09-03T10-00-00-04-00"}"#
    }

    #[test]
    fn response_matrix_matches_queue_policy() {
        assert!(matches!(
            classify_response(200, success()),
            ResponseDisposition::Saved { .. }
        ));
        assert!(matches!(
            classify_response(200, "{}"),
            ResponseDisposition::Retry { .. }
        ));
        for status in [401, 404] {
            assert_eq!(
                classify_response(status, ""),
                ResponseDisposition::AuthBlocked
            );
        }
        for status in [409, 413, 415, 422] {
            assert!(matches!(
                classify_response(status, r#"{"error":"nope"}"#),
                ResponseDisposition::Failed { .. }
            ));
        }
        for status in [400, 403, 500, 503] {
            assert!(matches!(
                classify_response(status, ""),
                ResponseDisposition::Retry { .. }
            ));
        }
    }

    #[test]
    fn success_and_permanent_failure_transition_in_place() {
        let row = queued();
        let saved = apply_response(&row, classify_response(200, success()));
        assert_eq!(saved.queued.state, OutboxState::Saved);
        assert!(saved.queued.receipt.is_some());
        assert!(saved.continue_flushing);

        let failed = apply_response(
            &row,
            classify_response(409, r#"{"error":"timestamp collision"}"#),
        );
        assert_eq!(failed.queued.state, OutboxState::Failed);
        assert!(failed.queued.predicted_location.is_none());
        assert!(failed.queued.rebase_on_restore);
        assert!(failed.continue_flushing);
    }

    #[test]
    fn retries_and_auth_keep_the_exact_pending_value() {
        let row = queued();
        let bytes = publication(&row).unwrap().body;
        let retry = apply_response(
            &row,
            ResponseDisposition::Retry {
                reason: "offline".into(),
            },
        );
        assert_eq!(retry.queued, row);
        assert_eq!(publication(&retry.queued).unwrap().body, bytes);
        assert!(!retry.continue_flushing);
        let auth = apply_response(&row, ResponseDisposition::AuthBlocked);
        assert_eq!(auth.queued, row);
        assert!(auth.auth_blocked);
    }

    #[test]
    fn multiple_pending_workouts_are_selected_oldest_first() {
        let oldest = queued();
        let mut tied_later = oldest.clone();
        tied_later.queue_id = "queue-00000002".into();
        let mut newest = oldest.clone();
        newest.queue_id = "queue-00000003".into();
        newest.enqueued_at_ms = 20;
        let mut receipt = oldest.clone();
        receipt.queue_id = "queue-saved-0001".into();
        receipt.enqueued_at_ms = 1;
        receipt.state = OutboxState::Saved;

        assert_eq!(
            pending_outbox(&[newest, receipt, tied_later, oldest])
                .into_iter()
                .map(|row| row.queue_id)
                .collect::<Vec<_>>(),
            ["queue-00000001", "queue-00000002", "queue-00000003"]
        );
    }

    #[test]
    fn malformed_or_cross_origin_success_never_saves() {
        for body in [
            r#"{"location":"https://evil.example/fitness/lift/2026-09-03T10-00-00-04-00","duplicate":false,"share_text":"x"}"#,
            r#"{"location":"/fitness/lift/not-a-path","duplicate":false,"share_text":"x"}"#,
            r#"{"location":"/fitness/lift/2026-09-03T10-00-00-04-00","duplicate":false,"share_text":""}"#,
        ] {
            assert!(matches!(
                classify_response(200, body),
                ResponseDisposition::Retry { .. }
            ));
        }
    }

    #[test]
    fn duplicate_success_is_a_receipt_and_only_receipts_dismiss() {
        let duplicate = success().replace("false", "true");
        let saved = apply_response(&queued(), classify_response(200, &duplicate)).queued;
        assert_eq!(
            saved.receipt.as_ref().map(|receipt| receipt.duplicate),
            Some(true)
        );
        assert!(dismiss_receipt(&saved).is_ok());
        assert!(dismiss_receipt(&queued()).is_err());
    }
}
