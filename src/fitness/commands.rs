//! Complete-record validation. Adapters assemble merges before calling this
//! module; derived fields are rebuilt from their canonical inputs.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{eastern, import, validate};

pub const INTERRUPTION_EMOJIS: &[&str] = &[
    "🤒", "🤧", "🤢", "🤕", "😷", "😴", "😭", "✈️", "🏖️", "🚗", "🏥", "💊",
];

/// Validate a complete stored record and return its canonical content, without
/// the record key. The caller owns table/field allowlisting and persistence.
pub fn record(table: &str, id: &str, mut value: Value) -> Result<Value, String> {
    let row = value.as_object_mut().ok_or("record must be an object")?;
    match table {
        "workouts" => {
            let imported_at = integer(row, "imported_at", 0, 253_402_300_799)?;
            row.remove("imported_at");
            row.remove("started_at_local");
            row.remove("eastern_offset_minutes");
            let duration = integer(row, "duration_seconds", 0, 604_800)?;
            row.insert(
                "duration_suspicious".into(),
                json!(duration == 0 || duration >= 14_400),
            );
            row.insert("id".into(), json!(id));
            value =
                serde_json::to_value(import::parse_workout(&value)?).map_err(|e| e.to_string())?;
            value["imported_at"] = json!(imported_at);
            value.as_object_mut().unwrap().remove("id");
        }
        "sets" => {
            let kind = text(row, "set_type", 64)?;
            if !validate::valid_set_type(kind) {
                return Err("bad set_type".into());
            }
            row.remove("incomplete");
            row.insert("id".into(), json!(id));
            value = serde_json::to_value(import::parse_set(&value)?).map_err(|e| e.to_string())?;
            value.as_object_mut().unwrap().remove("id");
        }
        "exercises" => {
            text(row, "name", 240)?;
        }
        "exercise_tags" => {
            import::parse_exercise(&json!({
                "name": row.get("exercise_name"),
                "tags": [{"kind": row.get("kind"), "value": row.get("value")}],
            }))?;
        }
        "exercise_aliases" => {
            let alias = text(row, "alias_name", 240)?;
            if alias == text(row, "canonical_name", 240)? {
                return Err("an alias cannot target itself".into());
            }
            integer(row, "updated_at", 0, 253_402_300_799)?;
        }
        "exercise_muscles" => {
            let name = text(row, "exercise_name", 200)?;
            let muscle = text(row, "muscle", 64)?;
            if id != hash(&format!("{name}\n{muscle}")) {
                return Err("exercise muscle id must match exercise_name and muscle".into());
            }
            integer(row, "ratio_hundredths", 1, 100)?;
            if !matches!(text(row, "source", 16)?, "seed" | "derived" | "admin") {
                return Err("bad muscle weight source".into());
            }
            integer(row, "updated_at", 0, 253_402_300_799)?;
        }
        "fitness_interruptions" => {
            if !hex_id(id, 32) {
                return Err("bad interruption id".into());
            }
            let from = date(text(row, "from_date", 10)?)?;
            if let Some(to) =
                validate::nullable_text_value(row.get("to_date"), 10).ok_or("bad to_date")?
            {
                let to = date(&to)?;
                let days = to.since(from).map_err(|e| e.to_string())?.get_days();
                if !(0..=365).contains(&days) {
                    return Err("bad interruption date range".into());
                }
            }
            text(row, "note", 200)?;
            if !INTERRUPTION_EMOJIS.contains(&text(row, "emoji", 16)?) {
                return Err("bad interruption emoji".into());
            }
            integer(row, "updated_at", 0, 253_402_300_799)?;
        }
        "running_activities" => {
            let source = text(row, "source", 32)?;
            let activity = text(row, "source_activity_id", 64)?;
            match source {
                "manual" if hex_id(activity, 64) && row.get("source_url") == Some(&Value::Null) => {
                }
                "garmin-connect"
                    if activity.len() <= 20
                        && activity.bytes().all(|b| b.is_ascii_digit())
                        && row.get("source_url")
                            == Some(&json!(format!(
                                "https://connect.garmin.com/app/activity/{activity}"
                            ))) => {}
                _ => return Err("bad run source identity".into()),
            }
            if id != hash(&format!("{source}\n{activity}")) {
                return Err("bad run id".into());
            }
            text(row, "title", 200)?;
            text(row, "activity_type", 80)?;
            let utc = text(row, "started_at_utc", 19)?;
            if !validate::valid_local_datetime(utc) {
                return Err("bad started_at_utc".into());
            }
            let projection = eastern::eastern_instant(utc, 0).map_err(|e| e.to_string())?;
            row.insert("started_at_local".into(), json!(projection.local));
            row.insert(
                "eastern_offset_minutes".into(),
                json!(projection.offset_minutes),
            );
            let duration = integer(row, "duration_milliseconds", 1, 604_800_000)?;
            validate::nullable_integer_value(row.get("moving_duration_milliseconds"), 1, duration)
                .ok_or("bad moving_duration_milliseconds")?;
            integer(row, "distance_millimeters", 1, 1_000_000_000)?;
            validate::nullable_integer_value(row.get("ascent_millimeters"), 0, 100_000_000)
                .ok_or("bad ascent_millimeters")?;
            integer(row, "imported_at", 0, 253_402_300_799)?;
        }
        _ => return Err("table is service-managed or unknown".into()),
    }
    Ok(value)
}

/// Browser adapters have already derived these fields, but still cross the
/// same validation boundary as an MCP record before they can be published.
pub fn payload(payload: &import::Payload) -> Result<(), String> {
    for workout in &payload.workouts {
        let mut row = serde_json::to_value(workout).map_err(|e| e.to_string())?;
        row.as_object_mut().unwrap().remove("id");
        row["imported_at"] = json!(0);
        record("workouts", &workout.id, row)?;
    }
    for set in &payload.sets {
        let mut row = serde_json::to_value(set).map_err(|e| e.to_string())?;
        row.as_object_mut().unwrap().remove("id");
        record("sets", &set.id, row)?;
    }
    for exercise in &payload.exercises {
        import::parse_exercise(&serde_json::to_value(exercise).map_err(|e| e.to_string())?)?;
    }
    Ok(())
}

fn text<'a>(row: &'a Map<String, Value>, key: &str, max: usize) -> Result<&'a str, String> {
    validate::text_value(row.get(key), 1, max)
        .filter(|s| !validate::js_trim(s).is_empty())
        .ok_or_else(|| format!("bad {key}"))
}

fn integer(row: &Map<String, Value>, key: &str, min: i64, max: i64) -> Result<i64, String> {
    validate::integer_value(row.get(key), min, max).ok_or_else(|| format!("bad {key}"))
}

fn date(value: &str) -> Result<jiff::civil::Date, String> {
    if !validate::valid_date(value) {
        return Err("bad date".into());
    }
    value
        .parse::<jiff::civil::Date>()
        .map_err(|e| e.to_string())
}

fn hex_id(id: &str, length: usize) -> bool {
    id.len() == length
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
