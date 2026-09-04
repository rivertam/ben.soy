use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::guidance::{Derived, GuidanceContext, GuideConfig, derive};
use crate::queue::{OutboxState, QueuedWorkout};
use crate::text::{
    effort_to_hundredths, hundredths_text, js_trim, normalize_title, pounds_to_milli, reps_value,
    safe_local_id, truncate_utf16, valid_set_type, valid_text, weight_text,
};
use crate::{
    MAX_DURATION_SECONDS, MAX_EFFORT_HUNDREDTHS, MAX_EXERCISES, MAX_REPS, MAX_SETS,
    MAX_WEIGHT_MILLI, MIN_EFFORT_HUNDREDTHS,
};

const DRAFT_VERSION: u16 = 1;
const MAX_ACTIVE_DRAFT_AGE_SECONDS: i64 = 4 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub version: u16,
    pub started_at_utc: String,
    pub title: String,
    pub notes: String,
    pub exercises: Vec<DraftExercise>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DraftExercise {
    pub id: String,
    pub name: String,
    pub sets: Vec<DraftSet>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DraftSet {
    pub id: String,
    pub weight: String,
    pub reps: String,
    /// Canonical RPE text. The page presents its inverse as reps in reserve.
    pub effort: String,
    pub set_type: String,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalizedWorkout {
    pub started_at_utc: String,
    pub ended_at_utc: String,
    pub title: String,
    pub notes: Option<String>,
    pub exercises: Vec<FinalizedExercise>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalizedExercise {
    pub name: String,
    pub sets: Vec<FinalizedSet>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalizedSet {
    pub weight_milli: Option<i64>,
    pub reps: u64,
    pub effort_hundredths: Option<u64>,
    pub set_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapInput {
    pub stored_draft: Option<Value>,
    pub guide: GuideConfig,
    pub now_utc: String,
    #[serde(default)]
    pub context: GuidanceContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BootstrapOutput {
    pub draft: Draft,
    pub guide: GuideConfig,
    pub derived: Derived,
    pub restored_start_reset: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    SetTitle {
        value: String,
    },
    SetNotes {
        value: String,
    },
    AddExercise {
        name: String,
        exercise_id: String,
        set_id: String,
    },
    RemoveExercise {
        exercise_id: String,
    },
    AddSet {
        exercise_id: String,
        set_id: String,
    },
    RemoveSet {
        exercise_id: String,
        set_id: String,
        replacement_set_id: String,
    },
    SetField {
        exercise_id: String,
        set_id: String,
        field: DraftField,
        value: String,
    },
    SetType {
        exercise_id: String,
        set_id: String,
        set_type: String,
    },
    ToggleSet {
        exercise_id: String,
        set_id: String,
    },
    UseLoad {
        exercise_id: String,
        set_id: String,
        weight_milli: Option<i64>,
        set_type: Option<String>,
    },
    AdjustWeight {
        exercise_id: String,
        set_id: String,
        delta_pounds: i64,
    },
    SetRir {
        exercise_id: String,
        set_id: String,
        effort_hundredths: Option<u64>,
    },
    Discard {
        now_utc: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftField {
    Weight,
    Reps,
    Effort,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionInput {
    pub draft: Draft,
    pub guide: GuideConfig,
    pub action: Action,
    #[serde(default)]
    pub context: GuidanceContext,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffect {
    None,
    Render,
    FocusExercise { exercise_id: String },
    FocusSet { exercise_id: String, set_id: String },
    Reset,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionError {
    pub message: String,
    pub exercise_id: Option<String>,
    pub set_id: Option<String>,
    pub field: Option<String>,
    pub review_field: Option<String>,
}

impl ActionError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exercise_id: None,
            set_id: None,
            field: None,
            review_field: None,
        }
    }

    fn set(message: impl Into<String>, exercise_id: &str, set_id: &str, field: &str) -> Self {
        Self {
            message: message.into(),
            exercise_id: Some(exercise_id.to_string()),
            set_id: Some(set_id.to_string()),
            field: Some(field.to_string()),
            review_field: None,
        }
    }

    fn review(message: impl Into<String>, field: &str) -> Self {
        Self {
            message: message.into(),
            exercise_id: None,
            set_id: None,
            field: None,
            review_field: Some(field.to_string()),
        }
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ActionError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitionOutput {
    pub draft: Draft,
    pub derived: Derived,
    pub effect: ActionEffect,
    pub error: Option<ActionError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizeInput {
    pub draft: Draft,
    pub guide: GuideConfig,
    pub ended_at_utc: String,
    pub queue_id: String,
    pub enqueued_at_ms: u64,
    #[serde(default)]
    pub context: GuidanceContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinalizeOutput {
    pub draft: Draft,
    pub derived: Derived,
    pub queued: Option<QueuedWorkout>,
    pub error: Option<ActionError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreOutput {
    pub draft: Draft,
    pub error: Option<ActionError>,
    pub restored_start_reset: bool,
}

pub fn new_draft(now_utc: &str) -> Result<Draft, ActionError> {
    eastern_time::utc_timestamp(now_utc)
        .map_err(|_| ActionError::message("The device clock did not provide a valid UTC time."))?;
    Ok(Draft {
        version: DRAFT_VERSION,
        started_at_utc: now_utc.to_string(),
        title: "Workout".to_string(),
        notes: String::new(),
        exercises: Vec::new(),
    })
}

pub fn bootstrap_draft(input: BootstrapInput) -> Result<BootstrapOutput, ActionError> {
    input.guide.validate()?;
    let now = eastern_time::utc_timestamp(&input.now_utc)
        .map_err(|_| ActionError::message("The device clock did not provide a valid UTC time."))?;
    let restored = input.stored_draft.and_then(sanitize_draft);
    let was_restored = restored.is_some();
    let mut draft = restored.unwrap_or(new_draft(&input.now_utc)?);
    let start = eastern_time::utc_timestamp(&draft.started_at_utc).ok();
    let age = start.map(|start| now.as_second() - start.as_second());
    let reset = was_restored
        && (draft_is_empty(&draft)
            || age.is_none_or(|age| !(0..MAX_ACTIVE_DRAFT_AGE_SECONDS).contains(&age)));
    if reset {
        draft.started_at_utc = input.now_utc;
    }
    let derived = derive(&draft, &input.guide, &input.context);
    Ok(BootstrapOutput {
        draft,
        guide: input.guide,
        derived,
        restored_start_reset: reset,
    })
}

fn sanitize_draft(value: Value) -> Option<Draft> {
    let object = value.as_object()?;
    if object.get("version")?.as_u64()? != u64::from(DRAFT_VERSION) {
        return None;
    }
    let started_at_utc = object.get("started_at_utc")?.as_str()?.to_string();
    eastern_time::utc_timestamp(&started_at_utc).ok()?;
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(|value| truncate_utf16(value, 241))
        .unwrap_or_else(|| "Workout".to_string());
    let notes = object
        .get("notes")
        .and_then(Value::as_str)
        .map(|value| truncate_utf16(value, 10_001))
        .unwrap_or_default();
    let mut exercises = Vec::new();
    let mut seen_names = HashSet::new();
    let mut exercise_ids = HashSet::new();
    let mut set_ids = HashSet::new();
    let mut total_sets = 0;
    for (exercise_index, raw) in object
        .get("exercises")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_EXERCISES)
        .enumerate()
    {
        if total_sets >= MAX_SETS {
            break;
        }
        let Some(raw) = raw.as_object() else {
            continue;
        };
        let Some(name) = raw.get("name").and_then(Value::as_str) else {
            continue;
        };
        // A queued/restored draft can outlive an archive rename. Keep a
        // bounded historical name visible and let the native server perform
        // authoritative alias projection at publication time. The guide is
        // still the only source from which a new exercise can be selected.
        if !valid_text(name, 1, 240)
            || js_trim(name).is_empty()
            || !seen_names.insert(name.to_string())
        {
            continue;
        }
        let raw_id = raw.get("id").and_then(Value::as_str).unwrap_or("");
        let id = restored_id(
            raw_id,
            format!("restored-exercise-{exercise_index:04}"),
            &mut exercise_ids,
        );
        let mut sets = Vec::new();
        for (set_index, raw_set) in raw
            .get("sets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_SETS.saturating_sub(total_sets))
            .enumerate()
        {
            let Some(raw_set) = raw_set.as_object() else {
                continue;
            };
            let raw_set_id = raw_set.get("id").and_then(Value::as_str).unwrap_or("");
            let set_id = restored_id(
                raw_set_id,
                format!("restored-set-{exercise_index:04}-{set_index:04}"),
                &mut set_ids,
            );
            let weight = raw_set
                .get("weight")
                .and_then(Value::as_str)
                .map(|value| truncate_utf16(value, 24))
                .unwrap_or_default();
            let reps = raw_set
                .get("reps")
                .and_then(Value::as_str)
                .map(|value| truncate_utf16(value, 24))
                .unwrap_or_default();
            let effort = raw_set
                .get("effort")
                .and_then(Value::as_str)
                .map(|value| truncate_utf16(value, 24))
                .unwrap_or_default();
            let set_type = raw_set
                .get("set_type")
                .or_else(|| raw_set.get("setType"))
                .and_then(Value::as_str)
                .filter(|value| valid_set_type(value))
                .unwrap_or("NORMAL_SET")
                .to_string();
            let valid = valid_draft_set(&weight, &reps, &effort).is_ok();
            sets.push(DraftSet {
                id: set_id,
                weight,
                reps,
                effort,
                set_type,
                done: raw_set
                    .get("done")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && valid,
            });
            total_sets += 1;
        }
        if sets.is_empty() && total_sets < MAX_SETS {
            sets.push(DraftSet::empty(format!(
                "restored-set-{exercise_index:04}-empty"
            )));
            total_sets += 1;
        }
        exercises.push(DraftExercise {
            id,
            name: name.to_string(),
            sets,
        });
    }
    Some(Draft {
        version: DRAFT_VERSION,
        started_at_utc,
        title,
        notes,
        exercises,
    })
}

fn restored_id(candidate: &str, fallback: String, seen: &mut HashSet<String>) -> String {
    if safe_local_id(candidate) && seen.insert(candidate.to_string()) {
        return candidate.to_string();
    }
    if seen.insert(fallback.clone()) {
        return fallback;
    }
    for suffix in 1..=MAX_SETS {
        let value = format!("{fallback}-{suffix}");
        if seen.insert(value.clone()) {
            return value;
        }
    }
    unreachable!("a bounded restored draft always has an unused local id")
}

impl DraftSet {
    fn empty(id: String) -> Self {
        Self {
            id,
            weight: String::new(),
            reps: String::new(),
            effort: String::new(),
            set_type: "NORMAL_SET".to_string(),
            done: false,
        }
    }

    fn copied(id: String, previous: &Self) -> Self {
        Self {
            id,
            weight: previous.weight.clone(),
            reps: String::new(),
            effort: previous.effort.clone(),
            set_type: previous.set_type.clone(),
            done: false,
        }
    }
}

pub fn transition(input: TransitionInput) -> TransitionOutput {
    let mut draft = input.draft;
    let original = draft.clone();
    let result = apply_action(&mut draft, &input.guide, input.action);
    let (effect, error) = match result {
        Ok(effect) => (effect, None),
        Err(error) => {
            draft = original;
            (ActionEffect::None, Some(error))
        }
    };
    let derived = derive(&draft, &input.guide, &input.context);
    TransitionOutput {
        draft,
        derived,
        effect,
        error,
    }
}

fn apply_action(
    draft: &mut Draft,
    guide: &GuideConfig,
    action: Action,
) -> Result<ActionEffect, ActionError> {
    match action {
        Action::SetTitle { value } => {
            draft.title = truncate_utf16(&value, 241);
            Ok(ActionEffect::None)
        }
        Action::SetNotes { value } => {
            draft.notes = truncate_utf16(&value, 10_001);
            Ok(ActionEffect::None)
        }
        Action::AddExercise {
            name,
            exercise_id,
            set_id,
        } => {
            if let Some(existing) = draft
                .exercises
                .iter()
                .find(|exercise| exercise.name == name)
            {
                return Ok(ActionEffect::FocusExercise {
                    exercise_id: existing.id.clone(),
                });
            }
            if !guide.contains(&name) {
                return Err(ActionError::message(
                    "That exercise is no longer available.",
                ));
            }
            require_new_id(&exercise_id)?;
            require_new_id(&set_id)?;
            require_unused_exercise_id(draft, &exercise_id)?;
            require_unused_set_id(draft, &set_id)?;
            if draft.exercises.len() >= MAX_EXERCISES || total_rows(draft) >= MAX_SETS {
                return Err(ActionError::message(
                    "A workout can contain at most 50 sets.",
                ));
            }
            draft.exercises.push(DraftExercise {
                id: exercise_id.clone(),
                name,
                sets: vec![DraftSet::empty(set_id.clone())],
            });
            Ok(ActionEffect::FocusSet {
                exercise_id,
                set_id,
            })
        }
        Action::RemoveExercise { exercise_id } => {
            draft
                .exercises
                .retain(|exercise| exercise.id != exercise_id);
            Ok(ActionEffect::Render)
        }
        Action::AddSet {
            exercise_id,
            set_id,
        } => {
            require_new_id(&set_id)?;
            require_unused_set_id(draft, &set_id)?;
            if total_rows(draft) >= MAX_SETS {
                return Err(ActionError::message(
                    "A workout can contain at most 50 sets.",
                ));
            }
            let exercise = find_exercise_mut(draft, &exercise_id)?;
            let previous = exercise.sets.last().cloned();
            exercise.sets.push(previous.as_ref().map_or_else(
                || DraftSet::empty(set_id.clone()),
                |previous| DraftSet::copied(set_id.clone(), previous),
            ));
            Ok(ActionEffect::FocusSet {
                exercise_id,
                set_id,
            })
        }
        Action::RemoveSet {
            exercise_id,
            set_id,
            replacement_set_id,
        } => {
            require_new_id(&replacement_set_id)?;
            require_unused_set_id(draft, &replacement_set_id)?;
            let exercise = find_exercise_mut(draft, &exercise_id)?;
            let Some(index) = exercise.sets.iter().position(|set| set.id == set_id) else {
                return Err(ActionError::message("That set is no longer available."));
            };
            let next_id = if exercise.sets.len() == 1 {
                exercise.sets[0] = DraftSet::empty(replacement_set_id.clone());
                replacement_set_id
            } else {
                exercise.sets.remove(index);
                exercise.sets[index.min(exercise.sets.len() - 1)].id.clone()
            };
            Ok(ActionEffect::FocusSet {
                exercise_id,
                set_id: next_id,
            })
        }
        Action::SetField {
            exercise_id,
            set_id,
            field,
            value,
        } => {
            let set = find_set_mut(draft, &exercise_id, &set_id)?;
            let value = truncate_utf16(&value, 24);
            match field {
                DraftField::Weight => set.weight = value,
                DraftField::Reps => set.reps = value,
                DraftField::Effort => set.effort = value,
            }
            if set.done && valid_draft_set(&set.weight, &set.reps, &set.effort).is_err() {
                set.done = false;
            }
            Ok(ActionEffect::None)
        }
        Action::SetType {
            exercise_id,
            set_id,
            set_type,
        } => {
            if !valid_set_type(&set_type) {
                return Err(ActionError::message("That set type is not supported."));
            }
            find_set_mut(draft, &exercise_id, &set_id)?.set_type = set_type;
            Ok(ActionEffect::Render)
        }
        Action::ToggleSet {
            exercise_id,
            set_id,
        } => {
            let set = find_set_mut(draft, &exercise_id, &set_id)?;
            if set.done {
                set.done = false;
            } else if let Err(field) = valid_draft_set(&set.weight, &set.reps, &set.effort) {
                return Err(set_error(&exercise_id, &set_id, field));
            } else {
                set.done = true;
            }
            Ok(ActionEffect::Render)
        }
        Action::UseLoad {
            exercise_id,
            set_id,
            weight_milli,
            set_type,
        } => {
            if weight_milli
                .is_some_and(|value| !(-MAX_WEIGHT_MILLI..=MAX_WEIGHT_MILLI).contains(&value))
            {
                return Err(ActionError::message(
                    "That load is outside the supported range.",
                ));
            }
            if set_type
                .as_deref()
                .is_some_and(|value| !valid_set_type(value))
            {
                return Err(ActionError::message("That set type is not supported."));
            }
            let set = find_set_mut(draft, &exercise_id, &set_id)?;
            set.weight = weight_milli.map(weight_text).unwrap_or_default();
            if let Some(set_type) = set_type {
                set.set_type = set_type;
            }
            Ok(ActionEffect::Render)
        }
        Action::AdjustWeight {
            exercise_id,
            set_id,
            delta_pounds,
        } => {
            if ![-10, -5, 5, 10].contains(&delta_pounds) {
                return Err(ActionError::message(
                    "That load adjustment is not supported.",
                ));
            }
            let set = find_set_mut(draft, &exercise_id, &set_id)?;
            let current = if js_trim(&set.weight).is_empty() {
                0
            } else {
                pounds_to_milli(&set.weight).ok_or_else(|| {
                    ActionError::set(
                        "Fix the weight before adjusting it.",
                        &exercise_id,
                        &set_id,
                        "weight",
                    )
                })?
            };
            let next = current
                .checked_add(delta_pounds * 1_000)
                .filter(|value| (-MAX_WEIGHT_MILLI..=MAX_WEIGHT_MILLI).contains(value))
                .ok_or_else(|| {
                    ActionError::set(
                        "Weight must stay between -1,000,000 and 1,000,000 lb.",
                        &exercise_id,
                        &set_id,
                        "weight",
                    )
                })?;
            set.weight = weight_text(next);
            Ok(ActionEffect::Render)
        }
        Action::SetRir {
            exercise_id,
            set_id,
            effort_hundredths,
        } => {
            if effort_hundredths.is_some_and(|value| {
                !(MIN_EFFORT_HUNDREDTHS..=MAX_EFFORT_HUNDREDTHS).contains(&value) || value % 50 != 0
            }) {
                return Err(ActionError::message("That RIR choice is not supported."));
            }
            find_set_mut(draft, &exercise_id, &set_id)?.effort =
                effort_hundredths.map(hundredths_text).unwrap_or_default();
            Ok(ActionEffect::Render)
        }
        Action::Discard { now_utc } => {
            *draft = new_draft(&now_utc)?;
            Ok(ActionEffect::Reset)
        }
    }
}

fn require_new_id(id: &str) -> Result<(), ActionError> {
    safe_local_id(id)
        .then_some(())
        .ok_or_else(|| ActionError::message("The browser could not create a local row identity."))
}

fn require_unused_exercise_id(draft: &Draft, id: &str) -> Result<(), ActionError> {
    (!draft.exercises.iter().any(|exercise| exercise.id == id))
        .then_some(())
        .ok_or_else(|| ActionError::message("That exercise row identity is already in use."))
}

fn require_unused_set_id(draft: &Draft, id: &str) -> Result<(), ActionError> {
    (!draft
        .exercises
        .iter()
        .flat_map(|exercise| &exercise.sets)
        .any(|set| set.id == id))
    .then_some(())
    .ok_or_else(|| ActionError::message("That set row identity is already in use."))
}

fn total_rows(draft: &Draft) -> usize {
    draft
        .exercises
        .iter()
        .map(|exercise| exercise.sets.len())
        .sum()
}

fn find_exercise_mut<'a>(
    draft: &'a mut Draft,
    exercise_id: &str,
) -> Result<&'a mut DraftExercise, ActionError> {
    draft
        .exercises
        .iter_mut()
        .find(|exercise| exercise.id == exercise_id)
        .ok_or_else(|| ActionError::message("That exercise is no longer available."))
}

fn find_set_mut<'a>(
    draft: &'a mut Draft,
    exercise_id: &str,
    set_id: &str,
) -> Result<&'a mut DraftSet, ActionError> {
    find_exercise_mut(draft, exercise_id)?
        .sets
        .iter_mut()
        .find(|set| set.id == set_id)
        .ok_or_else(|| ActionError::message("That set is no longer available."))
}

fn valid_draft_set(weight: &str, reps: &str, effort: &str) -> Result<(), &'static str> {
    if reps_value(reps).is_none() {
        return Err("reps");
    }
    if !js_trim(weight).is_empty() && pounds_to_milli(weight).is_none() {
        return Err("weight");
    }
    if !js_trim(effort).is_empty() && effort_to_hundredths(effort).is_none() {
        return Err("effort");
    }
    Ok(())
}

fn set_error(exercise_id: &str, set_id: &str, field: &str) -> ActionError {
    let message = match field {
        "reps" => "Enter whole-number reps before completing a set.",
        "weight" => "Weight accepts pounds with up to three decimals.",
        _ => "RIR must be between 0 and 4.",
    };
    ActionError::set(message, exercise_id, set_id, field)
}

pub fn finalize(input: FinalizeInput) -> FinalizeOutput {
    let result = build_finalized(&input.draft, &input.ended_at_utc).and_then(|workout| {
        require_new_id(&input.queue_id)?;
        let projection = eastern_time::eastern_instant(&workout.started_at_utc, 0)
            .map_err(|_| ActionError::message("The workout start could not be projected."))?;
        let predicted_location =
            format!("/fitness/lift/{}", eastern_time::public_path(&projection));
        let draft = new_draft(&input.ended_at_utc)?;
        let queued = QueuedWorkout {
            queue_id: input.queue_id,
            enqueued_at_ms: input.enqueued_at_ms,
            state: OutboxState::Pending,
            workout,
            predicted_location: Some(predicted_location),
            receipt: None,
            failure: None,
            rebase_on_restore: false,
        };
        Ok((draft, queued))
    });

    match result {
        Ok((draft, queued)) => FinalizeOutput {
            derived: derive(&draft, &input.guide, &input.context),
            draft,
            queued: Some(queued),
            error: None,
        },
        Err(error) => FinalizeOutput {
            derived: derive(&input.draft, &input.guide, &input.context),
            draft: input.draft,
            queued: None,
            error: Some(error),
        },
    }
}

fn build_finalized(draft: &Draft, ended_at_utc: &str) -> Result<FinalizedWorkout, ActionError> {
    let title = normalize_title(&draft.title);
    if !valid_text(&title, 1, 240) {
        return Err(ActionError::review(
            "Workout title must be 1–240 characters.",
            "title",
        ));
    }
    let notes = js_trim(&draft.notes);
    if !notes.is_empty() && !valid_text(notes, 1, 10_000) {
        return Err(ActionError::review(
            "Workout notes must be at most 10,000 characters.",
            "notes",
        ));
    }
    let mut exercises = Vec::new();
    let mut total = 0;
    for exercise in &draft.exercises {
        let mut sets = Vec::new();
        for set in &exercise.sets {
            if !set.done {
                continue;
            }
            if let Err(field) = valid_draft_set(&set.weight, &set.reps, &set.effort) {
                return Err(set_error(&exercise.id, &set.id, field));
            }
            sets.push(FinalizedSet {
                weight_milli: if js_trim(&set.weight).is_empty() {
                    None
                } else {
                    pounds_to_milli(&set.weight)
                },
                reps: reps_value(&set.reps).expect("validated reps"),
                effort_hundredths: if js_trim(&set.effort).is_empty() {
                    None
                } else {
                    effort_to_hundredths(&set.effort)
                },
                set_type: set.set_type.clone(),
            });
            total += 1;
        }
        if !sets.is_empty() {
            exercises.push(FinalizedExercise {
                name: exercise.name.clone(),
                sets,
            });
        }
    }
    if total == 0 {
        return Err(ActionError::message(
            "Record at least one set before finishing.",
        ));
    }
    let workout = FinalizedWorkout {
        started_at_utc: draft.started_at_utc.clone(),
        ended_at_utc: ended_at_utc.to_string(),
        title,
        notes: (!notes.is_empty()).then(|| notes.to_string()),
        exercises,
    };
    validate_finalized(&workout).map_err(ActionError::message)?;
    Ok(workout)
}

pub fn validate_finalized(workout: &FinalizedWorkout) -> Result<(), String> {
    let started = eastern_time::utc_timestamp(&workout.started_at_utc)
        .map_err(|_| "started_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time".to_string())?;
    let ended = eastern_time::utc_timestamp(&workout.ended_at_utc)
        .map_err(|_| "ended_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time".to_string())?;
    let duration = ended
        .as_second()
        .checked_sub(started.as_second())
        .ok_or_else(|| "workout duration is outside the supported range".to_string())?;
    if !(0..=MAX_DURATION_SECONDS).contains(&duration) {
        return Err("ended_at_utc must be at or after the start and within seven days".to_string());
    }
    if !valid_text(&workout.title, 1, 240) || js_trim(&workout.title).is_empty() {
        return Err("title must be 1-240 non-whitespace characters".to_string());
    }
    if workout
        .notes
        .as_deref()
        .is_some_and(|notes| !valid_text(notes, 1, 10_000))
    {
        return Err("notes must be null or 1-10000 characters".to_string());
    }
    if !(1..=MAX_EXERCISES).contains(&workout.exercises.len()) {
        return Err(format!(
            "a workout must contain 1-{MAX_EXERCISES} exercise blocks"
        ));
    }
    let mut sets = 0;
    for (exercise_index, exercise) in workout.exercises.iter().enumerate() {
        if exercise.sets.is_empty()
            || !valid_text(&exercise.name, 1, 240)
            || js_trim(&exercise.name).is_empty()
        {
            return Err(format!(
                "exercise {} has an invalid name or no sets",
                exercise_index + 1
            ));
        }
        for set in &exercise.sets {
            sets += 1;
            if sets > MAX_SETS {
                return Err(format!("a workout may contain at most {MAX_SETS} sets"));
            }
            if set
                .weight_milli
                .is_some_and(|weight| !(-MAX_WEIGHT_MILLI..=MAX_WEIGHT_MILLI).contains(&weight))
            {
                return Err(format!("set {sets} has weight outside the supported range"));
            }
            if set.reps > MAX_REPS {
                return Err(format!("set {sets} has reps outside the supported range"));
            }
            if set.effort_hundredths.is_some_and(|effort| {
                !(MIN_EFFORT_HUNDREDTHS..=MAX_EFFORT_HUNDREDTHS).contains(&effort)
            }) {
                return Err(format!("set {sets} has effort outside the supported range"));
            }
            if !valid_set_type(&set.set_type) {
                return Err(format!("set {sets} has an unsupported set_type"));
            }
        }
    }
    Ok(())
}

pub fn draft_is_empty(draft: &Draft) -> bool {
    draft.title == "Workout" && js_trim(&draft.notes).is_empty() && draft.exercises.is_empty()
}

pub fn restore_failed(draft: &Draft, queued: &QueuedWorkout, now_utc: &str) -> RestoreOutput {
    if queued.state != OutboxState::Failed {
        return RestoreOutput {
            draft: draft.clone(),
            error: Some(ActionError::message(
                "Only a rejected workout can be restored.",
            )),
            restored_start_reset: false,
        };
    }
    if !draft_is_empty(draft) {
        return RestoreOutput {
            draft: draft.clone(),
            error: Some(ActionError::message(
                "Finish or discard the current draft before restoring this workout.",
            )),
            restored_start_reset: false,
        };
    }
    let now = match eastern_time::utc_timestamp(now_utc) {
        Ok(now) => now,
        Err(_) => {
            return RestoreOutput {
                draft: draft.clone(),
                error: Some(ActionError::message(
                    "The device clock did not provide a valid UTC time.",
                )),
                restored_start_reset: false,
            };
        }
    };
    let original_start = eastern_time::utc_timestamp(&queued.workout.started_at_utc).ok();
    let age = original_start.map(|start| now.as_second() - start.as_second());
    let restored_start_reset = queued.rebase_on_restore
        || age.is_none_or(|age| !(0..MAX_ACTIVE_DRAFT_AGE_SECONDS).contains(&age));
    let started_at_utc = if restored_start_reset {
        now_utc.to_string()
    } else {
        queued.workout.started_at_utc.clone()
    };
    let suffix: String = queued
        .queue_id
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .take(12)
        .map(char::from)
        .collect();
    let exercises = queued
        .workout
        .exercises
        .iter()
        .enumerate()
        .map(|(exercise_index, exercise)| DraftExercise {
            id: format!("restore-exercise-{exercise_index:02}-{suffix}"),
            name: exercise.name.clone(),
            sets: exercise
                .sets
                .iter()
                .enumerate()
                .map(|(set_index, set)| DraftSet {
                    id: format!("restore-set-{exercise_index:02}-{set_index:02}-{suffix}"),
                    weight: set.weight_milli.map(weight_text).unwrap_or_default(),
                    reps: set.reps.to_string(),
                    effort: set
                        .effort_hundredths
                        .map(hundredths_text)
                        .unwrap_or_default(),
                    set_type: set.set_type.clone(),
                    done: true,
                })
                .collect(),
        })
        .collect();
    RestoreOutput {
        draft: Draft {
            version: DRAFT_VERSION,
            started_at_utc,
            title: queued.workout.title.clone(),
            notes: queued.workout.notes.clone().unwrap_or_default(),
            exercises,
        },
        error: None,
        restored_start_reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guidance::{ExerciseGuide, GuideConfig};

    fn guide() -> GuideConfig {
        GuideConfig {
            version: 1,
            today: "2026-09-03".into(),
            weekly_pace_tenths: 20,
            muscle_needs: Default::default(),
            exercises: vec![ExerciseGuide::fixture("Squat")],
        }
    }

    fn draft() -> Draft {
        Draft {
            version: 1,
            started_at_utc: "2026-09-03 14:00:00".into(),
            title: " Workout ".into(),
            notes: String::new(),
            exercises: vec![DraftExercise {
                id: "exercise-0001".into(),
                name: "Squat".into(),
                sets: vec![DraftSet {
                    id: "set-00000001".into(),
                    weight: "225.5".into(),
                    reps: "5".into(),
                    effort: "9.5".into(),
                    set_type: "NORMAL_SET".into(),
                    done: true,
                }],
            }],
        }
    }

    #[test]
    fn malformed_restoration_is_bounded_and_rebased() {
        let raw = serde_json::json!({
            "version": 1,
            "started_at_utc": "2026-09-03 08:00:00",
            "title": "Old",
            "notes": "kept",
            "exercises": [{
                "id": "bad!",
                "name": "Squat",
                "sets": [{"id": null, "weight": "wat", "reps": "5", "effort": "", "setType": "NOPE", "done": true}]
            }, {"name": "Unknown", "sets": []}]
        });
        let output = bootstrap_draft(BootstrapInput {
            stored_draft: Some(raw),
            guide: guide(),
            now_utc: "2026-09-03 14:00:00".into(),
            context: GuidanceContext::default(),
        })
        .unwrap();
        assert!(output.restored_start_reset);
        assert_eq!(output.draft.started_at_utc, "2026-09-03 14:00:00");
        assert_eq!(output.draft.exercises.len(), 2);
        assert_eq!(output.draft.exercises[0].sets[0].set_type, "NORMAL_SET");
        assert!(!output.draft.exercises[0].sets[0].done);
    }

    #[test]
    fn restoration_repairs_duplicate_row_ids_and_bounds_total_sets() {
        let sets: Vec<Value> = (0..60)
            .map(|_| {
                serde_json::json!({
                    "id": "duplicate-set",
                    "weight": "10",
                    "reps": "1",
                    "effort": "9",
                    "set_type": "NORMAL_SET",
                    "done": true
                })
            })
            .collect();
        let raw = serde_json::json!({
            "version": 1,
            "started_at_utc": "2026-09-03 13:59:00",
            "title": "Workout",
            "notes": "",
            "exercises": [{"id": "duplicate-row", "name": "Squat", "sets": sets}]
        });
        let output = bootstrap_draft(BootstrapInput {
            stored_draft: Some(raw),
            guide: guide(),
            now_utc: "2026-09-03 14:00:00".into(),
            context: GuidanceContext::default(),
        })
        .unwrap();
        let sets = &output.draft.exercises[0].sets;
        assert_eq!(sets.len(), MAX_SETS);
        assert_eq!(
            sets.iter().map(|set| &set.id).collect::<HashSet<_>>().len(),
            MAX_SETS
        );
    }

    #[test]
    fn actions_keep_completion_and_parsing_in_rust() {
        let mut value = draft();
        let toggled = transition(TransitionInput {
            draft: value.clone(),
            guide: guide(),
            action: Action::ToggleSet {
                exercise_id: "exercise-0001".into(),
                set_id: "set-00000001".into(),
            },
            context: GuidanceContext::default(),
        });
        assert!(!toggled.draft.exercises[0].sets[0].done);
        value.exercises[0].sets[0].done = false;
        value.exercises[0].sets[0].reps.clear();
        let rejected = transition(TransitionInput {
            draft: value,
            guide: guide(),
            action: Action::ToggleSet {
                exercise_id: "exercise-0001".into(),
                set_id: "set-00000001".into(),
            },
            context: GuidanceContext::default(),
        });
        assert_eq!(rejected.error.unwrap().field.as_deref(), Some("reps"));
    }

    #[test]
    fn rir_actions_and_set_types_derive_the_canonical_view() {
        let mut value = draft();
        value.exercises[0].sets[0].done = false;
        let changed = transition(TransitionInput {
            draft: value,
            guide: guide(),
            action: Action::SetRir {
                exercise_id: "exercise-0001".into(),
                set_id: "set-00000001".into(),
                effort_hundredths: Some(950),
            },
            context: GuidanceContext::default(),
        });
        assert_eq!(changed.draft.exercises[0].sets[0].effort, "9.5");
        assert_eq!(changed.derived.set_views[0].rir_display, "0.5");

        let changed = transition(TransitionInput {
            draft: changed.draft,
            guide: guide(),
            action: Action::SetType {
                exercise_id: "exercise-0001".into(),
                set_id: "set-00000001".into(),
                set_type: "DROP_SET".into(),
            },
            context: GuidanceContext::default(),
        });
        assert_eq!(changed.derived.set_views[0].set_type_label, "DROP");
        assert_eq!(changed.derived.set_views[0].set_kind, "drop");
    }

    #[test]
    fn finalization_filters_uncompleted_sets_and_freezes_exact_values() {
        let mut value = draft();
        value.exercises[0]
            .sets
            .push(DraftSet::empty("set-00000002".into()));
        let output = finalize(FinalizeInput {
            draft: value,
            guide: guide(),
            ended_at_utc: "2026-09-03 15:00:00".into(),
            queue_id: "queue-00000001".into(),
            enqueued_at_ms: 123,
            context: GuidanceContext::default(),
        });
        let queued = output.queued.unwrap();
        assert_eq!(queued.workout.exercises[0].sets.len(), 1);
        assert_eq!(
            queued.workout.exercises[0].sets[0].weight_milli,
            Some(225_500)
        );
        assert_eq!(
            queued.workout.exercises[0].sets[0].effort_hundredths,
            Some(950)
        );
        assert!(draft_is_empty(&output.draft));
    }

    #[test]
    fn finalized_validation_is_the_server_parity_boundary() {
        let workout = build_finalized(&draft(), "2026-09-03 15:00:00").unwrap();
        assert!(validate_finalized(&workout).is_ok());
        let mut bad = workout;
        bad.exercises[0].sets[0].effort_hundredths = Some(599);
        assert!(validate_finalized(&bad).is_err());
    }

    #[test]
    fn restore_requires_an_empty_draft_and_preserves_exact_payload() {
        let output = finalize(FinalizeInput {
            draft: draft(),
            guide: guide(),
            ended_at_utc: "2026-09-03 15:00:00".into(),
            queue_id: "queue-00000001".into(),
            enqueued_at_ms: 123,
            context: GuidanceContext::default(),
        });
        let mut queued = output.queued.unwrap();
        queued.state = OutboxState::Failed;
        assert!(
            restore_failed(&draft(), &queued, "2026-09-03 15:00:01")
                .error
                .is_some()
        );
        let restored = restore_failed(&output.draft, &queued, "2026-09-03 15:00:01");
        assert!(restored.error.is_none());
        assert!(!restored.restored_start_reset);
        assert_eq!(restored.draft.exercises[0].sets[0].weight, "225.5");
        assert!(restored.draft.exercises[0].sets[0].done);
    }

    #[test]
    fn collision_restore_rebases_the_mutable_draft() {
        let output = finalize(FinalizeInput {
            draft: draft(),
            guide: guide(),
            ended_at_utc: "2026-09-03 15:00:00".into(),
            queue_id: "queue-00000001".into(),
            enqueued_at_ms: 123,
            context: GuidanceContext::default(),
        });
        let mut queued = output.queued.unwrap();
        queued.state = OutboxState::Failed;
        queued.rebase_on_restore = true;
        let restored = restore_failed(&output.draft, &queued, "2026-09-03 15:05:00");
        assert!(restored.error.is_none());
        assert!(restored.restored_start_reset);
        assert_eq!(restored.draft.started_at_utc, "2026-09-03 15:05:00");
    }
}
