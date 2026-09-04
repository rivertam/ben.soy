//! Pure domain core for the local-first Fitness workout logger.
//!
//! Browser APIs stay in the service-worker/page adapters. This crate owns the
//! mutable Workout Draft, validation and freezing, guidance scoring, Queued
//! Workout transitions, predicted links, and publish response contract.

mod draft;
mod guidance;
mod queue;
mod text;

pub use draft::{
    Action, ActionEffect, ActionError, BootstrapInput, BootstrapOutput, Draft, DraftExercise,
    DraftSet, FinalizeInput, FinalizeOutput, FinalizedExercise, FinalizedSet, FinalizedWorkout,
    RestoreOutput, TransitionInput, TransitionOutput, bootstrap_draft, draft_is_empty, finalize,
    restore_failed, transition, validate_finalized,
};
pub use eastern_time as eastern;
pub use guidance::{
    Coverage, Derived, ExerciseGuide, GuidanceContext, GuideConfig, GuideMark, LoadPreset,
    SearchHit, SetView, Suggestion, derive, set_volume_points,
};
pub use queue::{
    AppliedResponse, OutboxState, Publication, PublishReceipt, QueuedWorkout, ResponseDisposition,
    apply_response, classify_response, dismiss_receipt, ordered_outbox, pending_outbox,
    publication,
};
pub use text::{hundredths_text, js_trim, pounds_to_milli, utf16_len, valid_set_type, weight_text};

pub const PROTOCOL_VERSION: u16 = 1;
pub const PUBLISH_PATH: &str = "/fitness/entry/publish";

pub const MAX_EXERCISES: usize = 75;
pub const MAX_SETS: usize = 50;
pub const MAX_DURATION_SECONDS: i64 = 7 * 24 * 60 * 60;
pub const MAX_WEIGHT_MILLI: i64 = 1_000_000_000;
pub const MAX_REPS: u64 = 1_000_000;
pub const MIN_EFFORT_HUNDREDTHS: u64 = 600;
pub const MAX_EFFORT_HUNDREDTHS: u64 = 1_000;
