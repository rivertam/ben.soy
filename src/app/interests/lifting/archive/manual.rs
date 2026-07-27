//! Strict parser for Lyfta's plain-text workout share format.
//!
//! The browser write path feeds this module one pasted workout. It produces
//! the same typed archive payload as the CSV sync path, but marks the workout
//! as `manual` and never widens the token-authenticated JSON import contract.

use std::collections::BTreeMap;

use jiff::civil::DateTime;

use super::eastern;
use super::import::{IncomingExercise, IncomingSet, IncomingTag, IncomingWorkout, Payload};
use crate::app::interests::lifting::taxonomy;

pub const LYFTA_TEXT_LIMIT: usize = 64 * 1024;
const MAX_SETS: usize = 50;
const MAX_EXERCISES: usize = 75;
const SOURCE: &str = "manual";
const FOOTER: &str = "Check out the workout and join me on Lyfta.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedWorkout {
    pub payload: Payload,
    pub public_path: String,
}

pub fn parse_lyfta(input: &str) -> Result<ParsedWorkout, ParseError> {
    if input.len() > LYFTA_TEXT_LIMIT {
        return Err(ParseError::new("the pasted workout is too large"));
    }
    if input
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ParseError::new(
            "the pasted workout contains unsupported control characters",
        ));
    }

    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let footer_index = lines
        .iter()
        .position(|line| *line == FOOTER)
        .ok_or_else(|| ParseError::new("missing the Lyfta footer"))?;
    validate_footer(&lines[footer_index + 1..])?;
    let body = &lines[..footer_index];
    if body.len() < 5 {
        return Err(ParseError::new("the pasted workout is incomplete"));
    }

    let raw_title = bounded_text(body[0], "workout title")?;
    let title = collapse_whitespace(&raw_title);
    let started_at_local = parse_date_line(body[1])?;
    let started_at_utc = eastern::eastern_local_to_utc(&started_at_local)
        .map_err(|_| ParseError::new("the workout time is invalid or ambiguous in New York"))?;
    let projection = eastern::eastern_instant(&started_at_utc, 0)
        .map_err(|_| ParseError::new("the workout time could not be projected"))?;
    let summary = parse_summary(body[2])?;

    let workout_id = format!("fitness:{}", started_at_utc.replacen(' ', "T", 1));
    let mut sets = Vec::new();
    let mut exercises = BTreeMap::<String, IncomingExercise>::new();
    let mut exercise_blocks = 0usize;
    let mut index = 3usize;
    while index < body.len() {
        let heading = bounded_text(body[index], "exercise name")?;
        if heading.starts_with("Set ") {
            return Err(ParseError::new(format!(
                "expected an exercise name before {heading:?}"
            )));
        }
        let raw_exercise_name = strip_exercise_number(&heading, exercise_blocks + 1)?.to_string();
        let exercise_name = collapse_whitespace(&raw_exercise_name);
        index += 1;
        exercise_blocks += 1;
        if exercise_blocks > MAX_EXERCISES {
            return Err(ParseError::new(format!(
                "a workout may contain at most {MAX_EXERCISES} exercises"
            )));
        }

        let mut exercise_set_number = 1usize;
        let first_set_index = sets.len();
        while index < body.len() && body[index].starts_with("Set ") {
            if sets.len() == MAX_SETS {
                return Err(ParseError::new(format!(
                    "a workout may contain at most {MAX_SETS} sets"
                )));
            }
            let ordinal = sets.len() + 1;
            sets.push(parse_set_line(
                body[index],
                exercise_set_number,
                ordinal,
                &workout_id,
                &exercise_name,
                &raw_exercise_name,
            )?);
            exercise_set_number += 1;
            index += 1;
        }
        if sets.len() == first_set_index {
            return Err(ParseError::new(format!(
                "exercise {exercise_name:?} has no sets"
            )));
        }

        exercises
            .entry(exercise_name.clone())
            .or_insert_with(|| IncomingExercise {
                name: exercise_name.clone(),
                tags: taxonomy::exercise_tags(&exercise_name)
                    .into_iter()
                    .map(|tag| IncomingTag {
                        kind: tag.kind,
                        value: tag.value,
                    })
                    .collect(),
            });
    }

    if exercise_blocks != summary.exercises {
        return Err(ParseError::new(format!(
            "summary says {} exercises, but {} were parsed",
            summary.exercises, exercise_blocks
        )));
    }
    if sets.len() != summary.sets {
        return Err(ParseError::new(format!(
            "summary says {} sets, but {} were parsed",
            summary.sets,
            sets.len()
        )));
    }

    let workout = IncomingWorkout {
        id: workout_id,
        title,
        raw_title,
        started_at_utc,
        started_at_local: projection.local.clone(),
        eastern_offset_minutes: i64::from(projection.offset_minutes),
        duration_seconds: summary.duration_seconds,
        duration_suspicious: summary.duration_seconds == 0 || summary.duration_seconds >= 14_400,
        notes: None,
        description: None,
        source: SOURCE.to_string(),
    };
    Ok(ParsedWorkout {
        public_path: eastern::public_path(&projection),
        payload: Payload {
            workouts: vec![workout],
            exercises: exercises.into_values().collect(),
            sets,
        },
    })
}

fn validate_footer(lines: &[&str]) -> Result<(), ParseError> {
    match lines {
        [] => Ok(()),
        [url] if valid_lyfta_url(url) => Ok(()),
        [_] => Err(ParseError::new("the Lyfta footer URL is malformed")),
        _ => Err(ParseError::new("unexpected text after the Lyfta footer")),
    }
}

fn valid_lyfta_url(value: &str) -> bool {
    let Some(id) = value.strip_prefix("https://lyfta.app/wk/") else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Lyfta shares sometimes number the exercise headings ("3. Pec Fly").
/// That prefix is share-sheet furniture, not part of the exercise name;
/// stored as identity it severs the exercise from its history — and
/// therefore from the records derived against that history. A numbered
/// heading must carry its 1-based position in the workout; headings that
/// merely contain a period ("St. Bench Row") pass through untouched.
fn strip_exercise_number(heading: &str, position: usize) -> Result<&str, ParseError> {
    let Some((number, name)) = heading.split_once(". ") else {
        return Ok(heading);
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(heading);
    }
    if number.parse() != Ok(position) {
        return Err(ParseError::new(format!(
            "exercise heading {heading:?} should be numbered {position}"
        )));
    }
    let name = name.trim_start();
    if name.is_empty() {
        return Err(ParseError::new(format!(
            "exercise heading {heading:?} has no name after its number"
        )));
    }
    Ok(name)
}

fn bounded_text(value: &str, label: &str) -> Result<String, ParseError> {
    let count = value.chars().count();
    if !(1..=240).contains(&count) {
        return Err(ParseError::new(format!("{label} must be 1-240 characters")));
    }
    Ok(value.to_string())
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Summary {
    duration_seconds: i64,
    exercises: usize,
    sets: usize,
}

fn parse_summary(line: &str) -> Result<Summary, ParseError> {
    let pieces: Vec<&str> = line.split('|').map(str::trim).collect();
    if pieces.len() != 4 {
        return Err(ParseError::new(
            "the workout summary must contain duration, volume, exercises, and sets",
        ));
    }
    let duration_seconds = parse_duration(pieces[0])?;
    validate_pounds_volume(pieces[1])?;
    let exercises = parse_count(pieces[2], "exercise")?;
    let sets = parse_count(pieces[3], "set")?;
    if exercises == 0 || exercises > MAX_EXERCISES {
        return Err(ParseError::new(format!(
            "the exercise count must be 1-{MAX_EXERCISES}"
        )));
    }
    if sets == 0 || sets > MAX_SETS {
        return Err(ParseError::new(format!(
            "the set count must be 1-{MAX_SETS}"
        )));
    }
    Ok(Summary {
        duration_seconds,
        exercises,
        sets,
    })
}

fn parse_duration(value: &str) -> Result<i64, ParseError> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let bytes = compact.as_bytes();
    let mut index = 0usize;
    let mut hours = None;
    let mut minutes = None;
    while index < bytes.len() {
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if start == index {
            return Err(ParseError::new("the workout duration is malformed"));
        }
        let number: i64 = compact[start..index]
            .parse()
            .map_err(|_| ParseError::new("the workout duration is too large"))?;
        if compact[index..].starts_with('h') {
            if hours.replace(number).is_some() {
                return Err(ParseError::new("the workout duration repeats hours"));
            }
            index += 1;
        } else if compact[index..].starts_with("min") {
            if minutes.replace(number).is_some() {
                return Err(ParseError::new("the workout duration repeats minutes"));
            }
            index += 3;
        } else if compact[index..].starts_with('m') {
            if minutes.replace(number).is_some() {
                return Err(ParseError::new("the workout duration repeats minutes"));
            }
            index += 1;
        } else {
            return Err(ParseError::new("the workout duration is malformed"));
        }
    }
    let total_minutes = hours
        .unwrap_or(0)
        .checked_mul(60)
        .and_then(|value| value.checked_add(minutes.unwrap_or(0)))
        .ok_or_else(|| ParseError::new("the workout duration is too large"))?;
    let seconds = total_minutes
        .checked_mul(60)
        .ok_or_else(|| ParseError::new("the workout duration is too large"))?;
    if seconds > 604_800 {
        return Err(ParseError::new("the workout duration exceeds seven days"));
    }
    Ok(seconds)
}

fn validate_pounds_volume(value: &str) -> Result<(), ParseError> {
    let lowercase = value.to_ascii_lowercase();
    let Some(number) = lowercase.strip_suffix("lbs") else {
        return Err(ParseError::new(
            "only Lyfta workouts recorded in pounds are supported",
        ));
    };
    let number: String = number
        .chars()
        .filter(|character| !matches!(character, ' ' | ',' | '_'))
        .collect();
    if number.is_empty()
        || number.matches('.').count() > 1
        || !number
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    {
        return Err(ParseError::new("the workout volume is malformed"));
    }
    Ok(())
}

fn parse_count(value: &str, noun: &str) -> Result<usize, ParseError> {
    let mut pieces = value.split_whitespace();
    let count: usize = pieces
        .next()
        .ok_or_else(|| ParseError::new(format!("missing {noun} count")))?
        .parse()
        .map_err(|_| ParseError::new(format!("the {noun} count is malformed")))?;
    let label = pieces
        .next()
        .ok_or_else(|| ParseError::new(format!("missing {noun} label")))?;
    if pieces.next().is_some()
        || !(label.eq_ignore_ascii_case(noun) || label.eq_ignore_ascii_case(&format!("{noun}s")))
    {
        return Err(ParseError::new(format!("the {noun} count is malformed")));
    }
    Ok(count)
}

fn parse_date_line(line: &str) -> Result<String, ParseError> {
    let pieces: Vec<&str> = line.split_whitespace().collect();
    if pieces.len() != 7 || !pieces[4].eq_ignore_ascii_case("at") {
        return Err(ParseError::new("the workout date is malformed"));
    }
    let weekday = pieces[0].trim_end_matches(',');
    let day: u8 = pieces[1]
        .trim_end_matches('.')
        .parse()
        .map_err(|_| ParseError::new("the workout day is malformed"))?;
    let month = month_number(pieces[2].trim_end_matches('.'))
        .ok_or_else(|| ParseError::new("the workout month is malformed"))?;
    let year: i16 = pieces[3]
        .parse()
        .map_err(|_| ParseError::new("the workout year is malformed"))?;
    let (raw_hour, minute) = pieces[5]
        .split_once(':')
        .ok_or_else(|| ParseError::new("the workout time is malformed"))?;
    let raw_hour: u8 = raw_hour
        .parse()
        .map_err(|_| ParseError::new("the workout hour is malformed"))?;
    let minute: u8 = minute
        .parse()
        .map_err(|_| ParseError::new("the workout minute is malformed"))?;
    if !(1..=12).contains(&raw_hour) || minute > 59 {
        return Err(ParseError::new("the workout time is malformed"));
    }
    let hour = match pieces[6].to_ascii_uppercase().as_str() {
        "AM" => raw_hour % 12,
        "PM" => raw_hour % 12 + 12,
        _ => return Err(ParseError::new("the workout time must use AM or PM")),
    };
    let local = format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:00");
    let datetime = DateTime::strptime("%Y-%m-%d %H:%M:%S", &local)
        .map_err(|_| ParseError::new("the workout date is not a real calendar date"))?;
    let actual_weekday = datetime.strftime("%A").to_string();
    if !actual_weekday.eq_ignore_ascii_case(weekday) {
        return Err(ParseError::new(format!(
            "the date is {actual_weekday}, not {weekday}"
        )));
    }
    Ok(local)
}

fn month_number(month: &str) -> Option<u8> {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .iter()
    .position(|candidate| candidate.eq_ignore_ascii_case(month))
    .map(|index| index as u8 + 1)
}

fn parse_set_line(
    line: &str,
    expected_set_number: usize,
    ordinal: usize,
    workout_id: &str,
    exercise_name: &str,
    raw_exercise_name: &str,
) -> Result<IncomingSet, ParseError> {
    let (core, annotations) = split_annotations(line)?;
    let Some(rest) = core.strip_prefix("Set ") else {
        return Err(ParseError::new("a set line must start with \"Set \""));
    };
    let (number, prescription) = rest
        .split_once(": ")
        .ok_or_else(|| ParseError::new(format!("malformed set line {line:?}")))?;
    let number: usize = number
        .parse()
        .map_err(|_| ParseError::new(format!("malformed set number in {line:?}")))?;
    if number != expected_set_number {
        return Err(ParseError::new(format!(
            "{exercise_name:?} expected Set {expected_set_number}, found Set {number}"
        )));
    }

    let (prescription, effort) = match prescription.split_once(" @ ") {
        Some((prescription, effort)) => {
            if effort.contains(" @ ") {
                return Err(ParseError::new(format!(
                    "set {ordinal} contains more than one effort value"
                )));
            }
            (prescription, Some(parse_effort(effort)?))
        }
        None => (prescription, None),
    };
    let pieces: Vec<&str> = prescription.split_whitespace().collect();
    if pieces.len() != 4
        || pieces[1] != "x"
        || !matches!(pieces[3].to_ascii_lowercase().as_str(), "rep" | "reps")
    {
        return Err(ParseError::new(format!(
            "malformed weight/reps prescription in {line:?}"
        )));
    }
    let weight_milli = parse_weight(pieces[0])?;
    let reps: i64 = pieces[2]
        .parse()
        .map_err(|_| ParseError::new(format!("malformed reps in {line:?}")))?;
    if !(0..=1_000_000).contains(&reps) {
        return Err(ParseError::new(format!(
            "reps are out of range in {line:?}"
        )));
    }

    let mut set_type = "NORMAL_SET";
    let mut side = None;
    for annotation in annotations {
        let normalized = annotation.to_ascii_lowercase();
        let candidate = match normalized.as_str() {
            "warm up" | "warm-up" | "warmup" => Some("WARMUP_SET"),
            "failure" => Some("FAILURE_SET"),
            "drop set" | "dropset" => Some("DROP_SET"),
            "partial reps" => Some("PARTIAL_REPS_SET"),
            "negative reps" => Some("NEGATIVE_REPS_SET"),
            "left" => {
                side = Some("Left");
                None
            }
            "right" => {
                side = Some("Right");
                None
            }
            _ => {
                return Err(ParseError::new(format!(
                    "unsupported set annotation ({annotation})"
                )));
            }
        };
        if let Some(candidate) = candidate {
            if set_type != "NORMAL_SET" {
                return Err(ParseError::new(format!(
                    "set {ordinal} contains more than one set type"
                )));
            }
            set_type = candidate;
        }
    }

    Ok(IncomingSet {
        id: format!("{workout_id}:{ordinal:04}"),
        workout_id: workout_id.to_string(),
        ordinal: ordinal as i64,
        exercise_name: exercise_name.to_string(),
        raw_exercise_name: raw_exercise_name.to_string(),
        exercise_note: side.map(str::to_string),
        superset_id: None,
        weight_milli: Some(weight_milli),
        weight_unit: "lbs".to_string(),
        reps: Some(reps),
        effort_hundredths: effort,
        distance_milli: None,
        set_time_seconds: None,
        set_type: set_type.to_string(),
        incomplete: false,
    })
}

fn split_annotations(line: &str) -> Result<(&str, Vec<&str>), ParseError> {
    let Some(first) = line.find(" (") else {
        return Ok((line, Vec::new()));
    };
    let core = &line[..first];
    let mut suffix = &line[first..];
    let mut annotations = Vec::new();
    while !suffix.is_empty() {
        let Some(rest) = suffix.strip_prefix(" (") else {
            return Err(ParseError::new(format!(
                "malformed set annotations in {line:?}"
            )));
        };
        let Some(end) = rest.find(')') else {
            return Err(ParseError::new(format!(
                "unclosed set annotation in {line:?}"
            )));
        };
        let annotation = &rest[..end];
        if annotation.is_empty() {
            return Err(ParseError::new("an empty set annotation is not valid"));
        }
        annotations.push(annotation);
        suffix = &rest[end + 1..];
    }
    Ok((core, annotations))
}

fn parse_weight(value: &str) -> Result<i64, ParseError> {
    let lower = value.to_ascii_lowercase();
    let Some(number) = lower.strip_suffix("lbs") else {
        return Err(ParseError::new(
            "only set weights recorded in pounds are supported",
        ));
    };
    let scaled = parse_scaled_decimal(number, 3, "weight")?;
    if scaled > 1_000_000_000 {
        return Err(ParseError::new("set weight is out of range"));
    }
    Ok(scaled)
}

fn parse_effort(value: &str) -> Result<i64, ParseError> {
    let lower = value.to_ascii_lowercase();
    let (number, is_rir) = if let Some(number) = lower.strip_suffix("rir") {
        (number, true)
    } else if let Some(number) = lower.strip_suffix("rpe") {
        (number, false)
    } else {
        return Err(ParseError::new(format!(
            "effort {value:?} must end in rir or rpe"
        )));
    };
    let scaled = parse_scaled_decimal(number, 2, "effort")?;
    if scaled > 1_000 {
        return Err(ParseError::new("effort must be between 0 and 10"));
    }
    Ok(if is_rir { 1_000 - scaled } else { scaled })
}

fn parse_scaled_decimal(value: &str, places: u32, label: &str) -> Result<i64, ParseError> {
    if value.is_empty() || value.starts_with(['-', '+']) {
        return Err(ParseError::new(format!("{label} is malformed")));
    }
    let mut pieces = value.split('.');
    let whole = pieces.next().unwrap_or_default();
    let fraction = pieces.next().unwrap_or_default();
    if pieces.next().is_some()
        || (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ParseError::new(format!("{label} is malformed")));
    }
    let places = places as usize;
    if fraction.len() > places && !fraction[places..].bytes().all(|byte| byte == b'0') {
        return Err(ParseError::new(format!(
            "{label} has too many decimal places"
        )));
    }
    let factor = 10_i128.pow(places as u32);
    let whole: i128 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| ParseError::new(format!("{label} is too large")))?
    };
    let kept = &fraction[..fraction.len().min(places)];
    let mut fraction: i128 = if kept.is_empty() {
        0
    } else {
        kept.parse()
            .map_err(|_| ParseError::new(format!("{label} is too large")))?
    };
    for _ in kept.len()..places {
        fraction *= 10;
    }
    whole
        .checked_mul(factor)
        .and_then(|value| value.checked_add(fraction))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| ParseError::new(format!("{label} is too large")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Quickest Arms in the Wesf
Friday 24. July 2026 at 10:38 AM

12min | 7 430lbs | 3 Exercises | 12 Sets

Incline Bench Press
Set 1: 45lbs x 10 reps (Warm Up)
Set 2: 65lbs x 6 reps (Warm Up)
Set 3: 135lbs x 6 reps @ 1rir
Set 4: 95lbs x 8 reps @ 0rir
Set 5: 95lbs x 6 reps @ 0rir

Upright Row
Set 1: 45lbs x 10 reps (Warm Up)
Set 2: 65lbs x 5 reps (Warm Up)
Set 3: 95lbs x 13 reps @ 0rir
Set 4: 95lbs x 8 reps (Failure)
Set 5: 95lbs x 6 reps @ 1rir

MTS Biceps Curl
Set 1: 55lbs x 6 reps @ 0rir (Warm Up)
Set 2: 45lbs x 5 reps (Failure)

Check out the workout and join me on Lyfta.
https://lyfta.app/wk/5";

    #[test]
    fn parses_the_supplied_lyfta_workout() {
        let parsed = parse_lyfta(SAMPLE).unwrap();
        let workout = &parsed.payload.workouts[0];
        assert_eq!(workout.title, "Quickest Arms in the Wesf");
        assert_eq!(workout.started_at_utc, "2026-07-24 14:38:00");
        assert_eq!(workout.started_at_local, "2026-07-24 10:38:00");
        assert_eq!(workout.eastern_offset_minutes, -240);
        assert_eq!(workout.duration_seconds, 720);
        assert_eq!(workout.source, "manual");
        assert_eq!(parsed.public_path, "2026-07-24T10-38-00-04-00");
        assert_eq!(parsed.payload.exercises.len(), 3);
        assert_eq!(parsed.payload.sets.len(), 12);
        assert_eq!(
            parsed
                .payload
                .sets
                .iter()
                .map(|set| set.ordinal)
                .collect::<Vec<_>>(),
            (1..=12).collect::<Vec<_>>()
        );
        assert_eq!(parsed.payload.sets[0].weight_milli, Some(45_000));
        assert_eq!(parsed.payload.sets[0].set_type, "WARMUP_SET");
        assert_eq!(parsed.payload.sets[2].effort_hundredths, Some(900));
        assert_eq!(parsed.payload.sets[3].effort_hundredths, Some(1_000));
        assert_eq!(parsed.payload.sets[8].set_type, "FAILURE_SET");
        assert_eq!(
            parsed.payload.sets[11].id,
            "fitness:2026-07-24T14:38:00:0012"
        );
    }

    #[test]
    fn supports_hour_durations_and_side_annotations() {
        let input = SAMPLE.replace("12min |", "1h 20m |").replace(
            "Set 3: 135lbs x 6 reps @ 1rir",
            "Set 3: 135lbs x 6 reps @ 1rir (Left)",
        );
        let parsed = parse_lyfta(&input).unwrap();
        assert_eq!(parsed.payload.workouts[0].duration_seconds, 4_800);
        assert_eq!(
            parsed.payload.sets[2].exercise_note.as_deref(),
            Some("Left")
        );
    }

    #[test]
    fn rejects_wrong_weekday_counts_units_and_set_sequence() {
        assert!(
            parse_lyfta(&SAMPLE.replace("Friday 24.", "Thursday 24."))
                .unwrap_err()
                .to_string()
                .contains("not Thursday")
        );
        assert!(
            parse_lyfta(&SAMPLE.replace("3 Exercises", "4 Exercises"))
                .unwrap_err()
                .to_string()
                .contains("summary says 4 exercises")
        );
        assert!(
            parse_lyfta(&SAMPLE.replace("7 430lbs", "3 370kg"))
                .unwrap_err()
                .to_string()
                .contains("pounds")
        );
        assert!(
            parse_lyfta(&SAMPLE.replacen("Set 2: 65lbs", "Set 3: 65lbs", 1))
                .unwrap_err()
                .to_string()
                .contains("expected Set 2")
        );
    }

    #[test]
    fn rejects_ambiguous_eastern_time_and_unknown_annotations() {
        let folded = SAMPLE.replace(
            "Friday 24. July 2026 at 10:38 AM",
            "Sunday 1. November 2026 at 1:30 AM",
        );
        assert!(
            parse_lyfta(&folded)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(
            parse_lyfta(&SAMPLE.replace("(Failure)", "(Cheat Reps)"))
                .unwrap_err()
                .to_string()
                .contains("unsupported set annotation")
        );
    }

    #[test]
    fn numbered_exercise_headings_shed_their_prefixes() {
        // Lyfta's numbered share variant. The stored exercise identity must
        // match the un-numbered history, or every set of the day competes
        // against an empty podium and sweeps fake PRs.
        let numbered = SAMPLE
            .replace("\nIncline Bench Press\n", "\n1. Incline Bench Press\n")
            .replace("\nUpright Row\n", "\n2. Upright Row\n")
            .replace("\nMTS Biceps Curl\n", "\n3. MTS Biceps Curl\n");
        let parsed = parse_lyfta(&numbered).unwrap();
        let names: Vec<&str> = parsed
            .payload
            .exercises
            .iter()
            .map(|exercise| exercise.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Incline Bench Press", "MTS Biceps Curl", "Upright Row"]
        );
        assert_eq!(parsed.payload.sets[0].exercise_name, "Incline Bench Press");
        assert_eq!(
            parsed.payload.sets[0].raw_exercise_name,
            "Incline Bench Press"
        );
    }

    #[test]
    fn misnumbered_headings_are_rejected_and_dotted_names_pass_through() {
        let shuffled = SAMPLE.replace("\nUpright Row\n", "\n5. Upright Row\n");
        assert!(
            parse_lyfta(&shuffled)
                .unwrap_err()
                .to_string()
                .contains("should be numbered 2")
        );
        let dotted = SAMPLE.replace("\nUpright Row\n", "\nSt. Bench Row\n");
        let parsed = parse_lyfta(&dotted).unwrap();
        assert!(
            parsed
                .payload
                .exercises
                .iter()
                .any(|exercise| exercise.name == "St. Bench Row")
        );
    }

    #[test]
    fn the_summary_volume_is_not_recomputed_from_sets() {
        // Lyfta reports 7,430 lbs for the supplied rows, while naïve
        // weight×reps totals 6,875 lbs. The aggregate is informational.
        assert!(parse_lyfta(SAMPLE).is_ok());
    }
}
