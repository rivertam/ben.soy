//! Import payload validation — the Rust `parseImportPayload`.
//!
//! Every message and check order is a verbatim port of `fitness.ts`
//! (`parseImportPayload`/`parseWorkout`/`parseExercise`/`parseSet`), with
//! one deliberate contract change: set objects no longer carry a
//! `records` array. Records are derived from history, never imported —
//! `fitness_sync` moves in lockstep (import contract v2).

use serde_json::Value;

use super::eastern;
use super::validate;

pub const BODY_LIMIT_BYTES: usize = 1_000_000;
const MAX_IMPORT_SETS: usize = 50;
const MAX_IMPORT_WORKOUTS: usize = 50;
const MAX_IMPORT_EXERCISES: usize = 75;
const MAX_TOTAL_TAGS: usize = 300;
const FITNESS_ID_PREFIX: &str = "fitness:";

#[derive(Clone, Debug, PartialEq)]
pub struct Payload {
    pub workouts: Vec<IncomingWorkout>,
    pub exercises: Vec<IncomingExercise>,
    pub sets: Vec<IncomingSet>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncomingWorkout {
    pub id: String,
    pub title: String,
    pub raw_title: String,
    pub started_at_utc: String,
    /// Derived server-side from `started_at_utc`, exactly like the Worker.
    pub started_at_local: String,
    pub eastern_offset_minutes: i64,
    pub duration_seconds: i64,
    pub duration_suspicious: bool,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncomingExercise {
    pub name: String,
    pub tags: Vec<IncomingTag>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IncomingTag {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncomingSet {
    pub id: String,
    pub workout_id: String,
    pub ordinal: i64,
    pub exercise_name: String,
    pub raw_exercise_name: String,
    pub exercise_note: Option<String>,
    pub superset_id: Option<i64>,
    pub weight_milli: Option<i64>,
    pub weight_unit: String,
    pub reps: Option<i64>,
    pub effort_hundredths: Option<i64>,
    pub distance_milli: Option<i64>,
    pub set_time_seconds: Option<i64>,
    pub set_type: String,
    /// Derived: no reps, distance, or time recorded.
    pub incomplete: bool,
}

/// The stored-vs-incoming tag comparison key: `kind\0value` entries,
/// sorted, joined with `\u{1}` — byte-for-byte the Worker's
/// `tagSignature`, so replace-on-change decisions can never drift.
pub fn tag_signature(tags: &[IncomingTag]) -> String {
    let mut entries: Vec<String> = tags
        .iter()
        .map(|tag| format!("{}\u{0}{}", tag.kind, tag.value))
        .collect();
    entries.sort();
    entries.join("\u{1}")
}

pub fn parse_import_payload(value: &Value) -> Result<Payload, String> {
    let Some(body) = value.as_object() else {
        return Err("body must be an object".to_string());
    };
    if !validate::has_only_keys(body, &["workouts", "exercises", "sets"]) {
        return Err("body may contain only workouts, exercises, and sets".to_string());
    }
    let Some(raw_workouts) = body.get("workouts").and_then(Value::as_array) else {
        return Err("workouts must be an array".to_string());
    };
    let Some(raw_exercises) = body.get("exercises").and_then(Value::as_array) else {
        return Err("exercises must be an array".to_string());
    };
    let Some(raw_sets) = body.get("sets").and_then(Value::as_array) else {
        return Err("sets must be an array".to_string());
    };
    if raw_workouts.is_empty() || raw_workouts.len() > MAX_IMPORT_WORKOUTS {
        return Err(format!(
            "workouts must contain 1-{MAX_IMPORT_WORKOUTS} entries"
        ));
    }
    if raw_exercises.is_empty() || raw_exercises.len() > MAX_IMPORT_EXERCISES {
        return Err(format!(
            "exercises must contain 1-{MAX_IMPORT_EXERCISES} entries"
        ));
    }
    if raw_sets.is_empty() || raw_sets.len() > MAX_IMPORT_SETS {
        return Err(format!("sets must contain 1-{MAX_IMPORT_SETS} entries"));
    }

    let mut workouts = Vec::with_capacity(raw_workouts.len());
    for (index, raw) in raw_workouts.iter().enumerate() {
        workouts.push(parse_workout(raw).map_err(|err| format!("workouts[{index}]: {err}"))?);
    }
    let mut exercises = Vec::with_capacity(raw_exercises.len());
    for (index, raw) in raw_exercises.iter().enumerate() {
        exercises.push(parse_exercise(raw).map_err(|err| format!("exercises[{index}]: {err}"))?);
    }
    let mut sets = Vec::with_capacity(raw_sets.len());
    for (index, raw) in raw_sets.iter().enumerate() {
        sets.push(parse_set(raw).map_err(|err| format!("sets[{index}]: {err}"))?);
    }

    if let Some(id) = first_duplicate(workouts.iter().map(|workout| workout.id.as_str())) {
        return Err(format!("duplicate workout id: {id}"));
    }
    if let Some(name) = first_duplicate(exercises.iter().map(|exercise| exercise.name.as_str())) {
        return Err(format!("duplicate exercise name: {name}"));
    }
    if let Some(id) = first_duplicate(sets.iter().map(|set| set.id.as_str())) {
        return Err(format!("duplicate set id: {id}"));
    }

    let workout_ids: std::collections::HashSet<&str> =
        workouts.iter().map(|workout| workout.id.as_str()).collect();
    let exercise_names: std::collections::HashSet<&str> = exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let mut ordinals = std::collections::HashSet::new();
    for set in &sets {
        if !workout_ids.contains(set.workout_id.as_str()) {
            return Err(format!(
                "set {} references a workout absent from this chunk",
                set.id
            ));
        }
        if !exercise_names.contains(set.exercise_name.as_str()) {
            return Err(format!(
                "set {} references an exercise absent from this chunk",
                set.id
            ));
        }
        if !ordinals.insert((set.workout_id.as_str(), set.ordinal)) {
            return Err(format!(
                "duplicate ordinal {} in workout {}",
                set.ordinal, set.workout_id
            ));
        }
    }

    let total_tags: usize = exercises.iter().map(|exercise| exercise.tags.len()).sum();
    if total_tags > MAX_TOTAL_TAGS {
        return Err(format!("chunk may contain at most {MAX_TOTAL_TAGS} tags"));
    }
    Ok(Payload {
        workouts,
        exercises,
        sets,
    })
}

fn parse_workout(value: &Value) -> Result<IncomingWorkout, String> {
    let Some(workout) = value.as_object() else {
        return Err("must be an object".to_string());
    };
    if !validate::has_only_keys(
        workout,
        &[
            "id",
            "title",
            "raw_title",
            "started_at_utc",
            "duration_seconds",
            "duration_suspicious",
            "notes",
            "description",
            "source",
        ],
    ) {
        return Err("contains unknown or missing fields".to_string());
    }
    let id = match workout.get("id").and_then(Value::as_str) {
        Some(id) if validate::valid_id(id) => id,
        _ => return Err("bad id".to_string()),
    };
    let title = match validate::text_value(workout.get("title"), 1, 240) {
        Some(title) if !validate::js_trim(title).is_empty() => title,
        _ => return Err("title must be 1-240 non-whitespace characters".to_string()),
    };
    let raw_title = validate::text_value(workout.get("raw_title"), 1, 240)
        .ok_or("raw_title must be 1-240 characters")?;
    let started_at_utc = match workout.get("started_at_utc").and_then(Value::as_str) {
        Some(utc) if validate::valid_local_datetime(utc) => utc,
        _ => return Err("started_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time".to_string()),
    };
    if id
        != format!(
            "{FITNESS_ID_PREFIX}{}",
            started_at_utc.replacen(' ', "T", 1)
        )
    {
        return Err("id must be the UTC-derived fitness timestamp".to_string());
    }
    let duration_seconds = validate::integer_value(workout.get("duration_seconds"), 0, 604_800)
        .ok_or("bad duration_seconds")?;
    let duration_suspicious = validate::bool_value(workout.get("duration_suspicious"))
        .ok_or("bad duration_suspicious")?;
    if duration_suspicious != (duration_seconds == 0 || duration_seconds >= 14_400) {
        return Err("duration_suspicious does not match duration_seconds".to_string());
    }
    let notes = validate::nullable_text_value(workout.get("notes"), 10_000)
        .ok_or("notes must be null or 1-10000 characters")?;
    let description = validate::nullable_text_value(workout.get("description"), 10_000)
        .ok_or("description must be null or 1-10000 characters")?;
    if workout.get("source").and_then(Value::as_str) != Some("workout-data-csv") {
        return Err("source must be workout-data-csv".to_string());
    }
    let eastern = eastern::eastern_instant(started_at_utc, 0)
        .map_err(|_| "started_at_utc must be a real YYYY-MM-DD HH:MM:SS UTC time".to_string())?;
    Ok(IncomingWorkout {
        id: id.to_string(),
        title: title.to_string(),
        raw_title: raw_title.to_string(),
        started_at_utc: started_at_utc.to_string(),
        started_at_local: eastern.local,
        eastern_offset_minutes: i64::from(eastern.offset_minutes),
        duration_seconds,
        duration_suspicious,
        notes,
        description,
        source: "workout-data-csv".to_string(),
    })
}

fn parse_exercise(value: &Value) -> Result<IncomingExercise, String> {
    let Some(exercise) = value.as_object() else {
        return Err("must be an object".to_string());
    };
    if !validate::has_only_keys(exercise, &["name", "tags"]) {
        return Err("contains unknown or missing fields".to_string());
    }
    let name = match validate::text_value(exercise.get("name"), 1, 240) {
        Some(name) if !validate::js_trim(name).is_empty() => name,
        _ => return Err("name must be 1-240 non-whitespace characters".to_string()),
    };
    let raw_tags = match exercise.get("tags").and_then(Value::as_array) {
        Some(tags) if tags.len() <= 60 => tags,
        _ => return Err("tags must be an array of at most 60 entries".to_string()),
    };
    let mut tags = Vec::with_capacity(raw_tags.len());
    let mut seen = std::collections::HashSet::new();
    for (index, raw) in raw_tags.iter().enumerate() {
        let entry = raw
            .as_object()
            .filter(|tag| validate::has_only_keys(tag, &["kind", "value"]));
        let Some(tag) = entry else {
            return Err(format!("tags[{index}] must contain only kind and value"));
        };
        let kind = match tag.get("kind").and_then(Value::as_str) {
            Some(kind) if validate::valid_tag_kind(kind) => kind,
            _ => return Err(format!("bad tags[{index}].kind")),
        };
        let value = match tag.get("value").and_then(Value::as_str) {
            Some(value) if validate::valid_tag_value(value) => value,
            _ => return Err(format!("bad tags[{index}].value")),
        };
        if !seen.insert((kind.to_string(), value.to_string())) {
            return Err(format!("duplicate tag: {kind}/{value}"));
        }
        tags.push(IncomingTag {
            kind: kind.to_string(),
            value: value.to_string(),
        });
    }
    Ok(IncomingExercise {
        name: name.to_string(),
        tags,
    })
}

fn parse_set(value: &Value) -> Result<IncomingSet, String> {
    let Some(set) = value.as_object() else {
        return Err("must be an object".to_string());
    };
    if !validate::has_only_keys(
        set,
        &[
            "id",
            "workout_id",
            "ordinal",
            "exercise_name",
            "raw_exercise_name",
            "exercise_note",
            "superset_id",
            "weight_milli",
            "weight_unit",
            "reps",
            "effort_hundredths",
            "distance_milli",
            "set_time_seconds",
            "set_type",
        ],
    ) {
        return Err("contains unknown or missing fields".to_string());
    }
    let id = match set.get("id").and_then(Value::as_str) {
        Some(id) if validate::valid_id(id) => id,
        _ => return Err("bad id".to_string()),
    };
    let workout_id = match set.get("workout_id").and_then(Value::as_str) {
        Some(id) if validate::valid_id(id) => id,
        _ => return Err("bad workout_id".to_string()),
    };
    let ordinal = validate::integer_value(set.get("ordinal"), 1, 10_000).ok_or("bad ordinal")?;
    let exercise_name =
        validate::text_value(set.get("exercise_name"), 1, 240).ok_or("bad exercise_name")?;
    let raw_exercise_name = validate::text_value(set.get("raw_exercise_name"), 1, 240)
        .ok_or("bad raw_exercise_name")?;
    let exercise_note = validate::nullable_text_value(set.get("exercise_note"), 2_000)
        .ok_or("bad exercise_note")?;
    let superset_id = validate::nullable_integer_value(set.get("superset_id"), 0, 1_000_000)
        .ok_or("bad superset_id")?;
    let weight_milli =
        validate::nullable_integer_value(set.get("weight_milli"), -1_000_000_000, 1_000_000_000)
            .ok_or("bad weight_milli")?;
    if set.get("weight_unit").and_then(Value::as_str) != Some("lbs") {
        return Err("bad weight_unit".to_string());
    }
    let reps = validate::nullable_integer_value(set.get("reps"), 0, 1_000_000).ok_or("bad reps")?;
    let effort_hundredths =
        validate::nullable_integer_value(set.get("effort_hundredths"), 0, 100_000)
            .ok_or("bad effort_hundredths")?;
    let distance_milli =
        validate::nullable_integer_value(set.get("distance_milli"), 0, 1_000_000_000)
            .ok_or("bad distance_milli")?;
    let set_time_seconds =
        validate::nullable_integer_value(set.get("set_time_seconds"), 0, 604_800)
            .ok_or("bad set_time_seconds")?;
    let set_type = match set.get("set_type").and_then(Value::as_str) {
        Some(set_type) if validate::valid_set_type(set_type) => set_type,
        _ => return Err("bad set_type".to_string()),
    };
    Ok(IncomingSet {
        id: id.to_string(),
        workout_id: workout_id.to_string(),
        ordinal,
        exercise_name: exercise_name.to_string(),
        raw_exercise_name: raw_exercise_name.to_string(),
        exercise_note,
        superset_id,
        weight_milli,
        weight_unit: "lbs".to_string(),
        reps,
        effort_hundredths,
        distance_milli,
        set_time_seconds,
        set_type: set_type.to_string(),
        incomplete: reps.is_none() && distance_milli.is_none() && set_time_seconds.is_none(),
    })
}

fn first_duplicate<'a>(mut values: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = std::collections::HashSet::new();
    values.find(|value| !seen.insert(*value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workout() -> Value {
        serde_json::json!({
            "id": "fitness:2026-07-21T14:39:04",
            "title": "Leg day",
            "raw_title": "Leg day",
            "started_at_utc": "2026-07-21 14:39:04",
            "duration_seconds": 3600,
            "duration_suspicious": false,
            "notes": null,
            "description": null,
            "source": "workout-data-csv"
        })
    }

    fn exercise() -> Value {
        serde_json::json!({
            "name": "Squat (Barbell)",
            "tags": [
                {"kind": "movement", "value": "squat-type"},
                {"kind": "muscle", "value": "quads"}
            ]
        })
    }

    fn set() -> Value {
        serde_json::json!({
            "id": "fitness:2026-07-21T14:39:04:0001",
            "workout_id": "fitness:2026-07-21T14:39:04",
            "ordinal": 1,
            "exercise_name": "Squat (Barbell)",
            "raw_exercise_name": "Squat (Barbell)",
            "exercise_note": null,
            "superset_id": null,
            "weight_milli": 225000,
            "weight_unit": "lbs",
            "reps": 5,
            "effort_hundredths": null,
            "distance_milli": null,
            "set_time_seconds": null,
            "set_type": "NORMAL_SET"
        })
    }

    fn payload() -> Value {
        serde_json::json!({
            "workouts": [workout()],
            "exercises": [exercise()],
            "sets": [set()]
        })
    }

    #[test]
    fn accepts_a_valid_chunk_and_derives_server_side_fields() {
        let parsed = parse_import_payload(&payload()).unwrap();
        let w = &parsed.workouts[0];
        assert_eq!(w.started_at_local, "2026-07-21 10:39:04");
        assert_eq!(w.eastern_offset_minutes, -240);
        assert!(!parsed.sets[0].incomplete, "reps recorded");
        assert_eq!(parsed.exercises[0].tags.len(), 2);
    }

    #[test]
    fn accepts_bounded_negative_assistance_but_not_negative_reps() {
        let mut assisted = payload();
        assisted["sets"][0]["weight_milli"] = Value::from(-225_000);
        let parsed = parse_import_payload(&assisted).unwrap();
        assert_eq!(parsed.sets[0].weight_milli, Some(-225_000));

        assisted["sets"][0]["weight_milli"] = Value::from(-1_000_000_001_i64);
        assert_eq!(
            parse_import_payload(&assisted).unwrap_err(),
            "sets[0]: bad weight_milli"
        );

        let mut negative_reps = payload();
        negative_reps["sets"][0]["reps"] = Value::from(-1);
        assert_eq!(
            parse_import_payload(&negative_reps).unwrap_err(),
            "sets[0]: bad reps"
        );
    }

    #[test]
    fn records_key_is_no_longer_accepted() {
        // Import contract v2: a payload still sending records must fail
        // the exact-key-set check with the Worker's message.
        let mut body = payload();
        body["sets"][0]["records"] = serde_json::json!([]);
        assert_eq!(
            parse_import_payload(&body).unwrap_err(),
            "sets[0]: contains unknown or missing fields",
        );
    }

    #[test]
    fn top_level_messages_match_the_worker() {
        assert_eq!(
            parse_import_payload(&serde_json::json!([])).unwrap_err(),
            "body must be an object",
        );
        assert_eq!(
            parse_import_payload(&serde_json::json!({"workouts": []})).unwrap_err(),
            "body may contain only workouts, exercises, and sets",
        );
        let mut body = payload();
        body["workouts"] = serde_json::json!([]);
        assert_eq!(
            parse_import_payload(&body).unwrap_err(),
            "workouts must contain 1-50 entries",
        );
        let mut body = payload();
        body["sets"] = Value::from_iter(std::iter::repeat_n(set(), 51));
        assert_eq!(
            parse_import_payload(&body).unwrap_err(),
            "sets must contain 1-50 entries",
        );
    }

    #[test]
    fn workout_validation_order_and_messages() {
        let mut bad_id = payload();
        bad_id["workouts"][0]["id"] = Value::from("fitness:2026-07-21T14:39:05");
        assert_eq!(
            parse_import_payload(&bad_id).unwrap_err(),
            "workouts[0]: id must be the UTC-derived fitness timestamp",
        );

        let mut mismatch = payload();
        mismatch["workouts"][0]["duration_suspicious"] = Value::from(true);
        assert_eq!(
            parse_import_payload(&mismatch).unwrap_err(),
            "workouts[0]: duration_suspicious does not match duration_seconds",
        );

        let mut zero = payload();
        zero["workouts"][0]["duration_seconds"] = Value::from(0);
        assert_eq!(
            parse_import_payload(&zero).unwrap_err(),
            "workouts[0]: duration_suspicious does not match duration_seconds",
        );
        zero["workouts"][0]["duration_suspicious"] = Value::from(true);
        assert!(parse_import_payload(&zero).is_ok(), "0 is suspicious");

        let mut missing_notes = payload();
        missing_notes["workouts"][0]
            .as_object_mut()
            .unwrap()
            .remove("notes");
        assert_eq!(
            parse_import_payload(&missing_notes).unwrap_err(),
            "workouts[0]: contains unknown or missing fields",
        );
    }

    #[test]
    fn set_referential_checks() {
        let mut orphan = payload();
        orphan["sets"][0]["workout_id"] = Value::from("fitness:2026-01-01T00:00:00");
        assert_eq!(
            parse_import_payload(&orphan).unwrap_err(),
            "set fitness:2026-07-21T14:39:04:0001 references a workout absent from this chunk",
        );

        let mut dup_ordinal = payload();
        let mut second = set();
        second["id"] = Value::from("fitness:2026-07-21T14:39:04:0002");
        dup_ordinal["sets"] = Value::from_iter([set(), second]);
        assert_eq!(
            parse_import_payload(&dup_ordinal).unwrap_err(),
            "duplicate ordinal 1 in workout fitness:2026-07-21T14:39:04",
        );
    }

    #[test]
    fn incomplete_derivation_matches_the_worker() {
        let mut body = payload();
        body["sets"][0]["reps"] = Value::Null;
        let parsed = parse_import_payload(&body).unwrap();
        assert!(parsed.sets[0].incomplete, "no reps, distance, or time");

        let mut timed = payload();
        timed["sets"][0]["reps"] = Value::Null;
        timed["sets"][0]["set_time_seconds"] = Value::from(60);
        assert!(!parse_import_payload(&timed).unwrap().sets[0].incomplete);
    }

    #[test]
    fn tag_signature_is_sorted_and_delimited_like_the_worker() {
        let tags = vec![
            IncomingTag {
                kind: "muscle".into(),
                value: "quads".into(),
            },
            IncomingTag {
                kind: "movement".into(),
                value: "squat-type".into(),
            },
        ];
        assert_eq!(
            tag_signature(&tags),
            "movement\u{0}squat-type\u{1}muscle\u{0}quads",
        );
        assert_eq!(tag_signature(&[]), "");
    }

    #[test]
    fn duplicate_tags_and_bad_kinds_are_rejected() {
        let mut dup = payload();
        dup["exercises"][0]["tags"] = serde_json::json!([
            {"kind": "muscle", "value": "quads"},
            {"kind": "muscle", "value": "quads"}
        ]);
        assert_eq!(
            parse_import_payload(&dup).unwrap_err(),
            "exercises[0]: duplicate tag: muscle/quads",
        );

        let mut bad = payload();
        bad["exercises"][0]["tags"] = serde_json::json!([{"kind": "vibe", "value": "quads"}]);
        assert_eq!(
            parse_import_payload(&bad).unwrap_err(),
            "exercises[0]: bad tags[0].kind",
        );
    }
}
