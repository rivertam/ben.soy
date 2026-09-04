//! Coarse JSON wasm-bindgen interface over `fitness-entry-core`.
//!
//! IndexedDB, fetch, service-worker events, and browser clocks remain in the
//! JavaScript adapter. Every domain transition crosses this boundary as one
//! input and one output, making persistence-before-reply straightforward.
#![cfg(target_arch = "wasm32")]

use fitness_entry_core::{
    Draft, FinalizeInput, GuidanceContext, GuideConfig, QueuedWorkout, ResponseDisposition,
    TransitionInput, apply_response, bootstrap_draft, classify_response, derive, dismiss_receipt,
    finalize, ordered_outbox, pending_outbox, publication, restore_failed, transition,
};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn fitness_protocol_version() -> u16 {
    fitness_entry_core::PROTOCOL_VERSION
}

#[wasm_bindgen]
pub fn fitness_bootstrap(input_json: String) -> Result<String, JsError> {
    encode(&bootstrap_draft(decode(&input_json)?).map_err(domain_error)?)
}

#[wasm_bindgen]
pub fn fitness_transition(input_json: String) -> Result<String, JsError> {
    let input: TransitionInput = decode(&input_json)?;
    encode(&transition(input))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeriveInput {
    draft: Draft,
    guide: GuideConfig,
    #[serde(default)]
    context: GuidanceContext,
}

#[wasm_bindgen]
pub fn fitness_derive(input_json: String) -> Result<String, JsError> {
    let input: DeriveInput = decode(&input_json)?;
    encode(&derive(&input.draft, &input.guide, &input.context))
}

#[wasm_bindgen]
pub fn fitness_finalize(input_json: String) -> Result<String, JsError> {
    let input: FinalizeInput = decode(&input_json)?;
    encode(&finalize(input))
}

#[wasm_bindgen]
pub fn fitness_publication(queued_json: String) -> Result<String, JsError> {
    let queued: QueuedWorkout = decode(&queued_json)?;
    encode(&publication(&queued).map_err(domain_error)?)
}

#[wasm_bindgen]
pub fn fitness_order_outbox(outbox_json: String) -> Result<String, JsError> {
    let outbox: Vec<QueuedWorkout> = decode(&outbox_json)?;
    encode(&ordered_outbox(&outbox))
}

#[wasm_bindgen]
pub fn fitness_pending_outbox(outbox_json: String) -> Result<String, JsError> {
    let outbox: Vec<QueuedWorkout> = decode(&outbox_json)?;
    encode(&pending_outbox(&outbox))
}

#[wasm_bindgen]
pub fn fitness_classify_response(status: u16, body: String) -> Result<String, JsError> {
    encode(&classify_response(status, &body))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyInput {
    queued: QueuedWorkout,
    disposition: ResponseDisposition,
}

#[wasm_bindgen]
pub fn fitness_apply_response(input_json: String) -> Result<String, JsError> {
    let input: ApplyInput = decode(&input_json)?;
    encode(&apply_response(&input.queued, input.disposition))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreInput {
    draft: Draft,
    queued: QueuedWorkout,
    now_utc: String,
}

#[wasm_bindgen]
pub fn fitness_restore(input_json: String) -> Result<String, JsError> {
    let input: RestoreInput = decode(&input_json)?;
    encode(&restore_failed(&input.draft, &input.queued, &input.now_utc))
}

#[wasm_bindgen]
pub fn fitness_dismiss(queued_json: String) -> Result<(), JsError> {
    let queued: QueuedWorkout = decode(&queued_json)?;
    dismiss_receipt(&queued).map_err(domain_error)
}

fn decode<T: DeserializeOwned>(json: &str) -> Result<T, JsError> {
    serde_json::from_str(json).map_err(|error| JsError::new(&error.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<String, JsError> {
    serde_json::to_string(value).map_err(|error| JsError::new(&error.to_string()))
}

fn domain_error(error: impl std::fmt::Display) -> JsError {
    JsError::new(&error.to_string())
}
