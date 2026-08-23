//! fitness_sync — import a Strong workout CSV export into ben.soy.
//!
//! The importer keeps source spelling for provenance, produces normalized
//! display names for filtering, and assigns stable ids from the workout's
//! UTC start time plus each row's ordinal within the whole workout.
//! Cross-layer invariants and taxonomy workflow: `docs/fitness.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::json;

mod taxonomy;

#[cfg(test)]
use taxonomy::SQUAT_TYPE_EXERCISES;
use taxonomy::{ExerciseTag, exercise_tags};

const DEFAULT_API: &str = "https://ben.soy";
const MAX_WORKOUTS_PER_CHUNK: usize = 20;
// The Worker caps imports at 50 sets to stay below D1's statement budget.
// Whole workouts are never split (the audited export's largest has 39 sets).
const MAX_SETS_PER_CHUNK: usize = 50;

const USAGE: &str = "\
fitness_sync — sync a Strong workout CSV export to ben.soy

USAGE
  just sync-fitness <csv> [FLAGS]
  cargo run --bin fitness_sync -- <csv> [FLAGS]

FLAGS
  --dry-run       parse and diff, but upload nothing
  --json          print a machine-readable summary on stdout
  --api <origin>  API origin (default: https://ben.soy)
  --token <token> write token; otherwise $FITNESS_SYNC_TOKEN, otherwise
                  ~/.config/benjisponge/fitness.token
  -h, --help      this text

BEHAVIOR
  1. Parse every CSV row and group rows by the export's UTC workout start.
  2. GET  <api>/api/fitness/ids to learn which set ids already exist.
  3. POST <api>/api/fitness/import (Bearer token) with missing workouts and
     their exercises and sets, oldest first, in bounded whole-workout chunks.

  Stable ids and server-side duplicate handling make the command idempotent.
  If an upload is interrupted, run it again to resume. A dry run needs no
  token. Exit codes: 0 success, 1 parse/network/upload failure, 2 usage error.
";

#[derive(Debug)]
struct Args {
    csv: PathBuf,
    api: String,
    dry_run: bool,
    json: bool,
    token: Option<String>,
}

fn parse_args_from<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut api = DEFAULT_API.to_string();
    let mut dry_run = false;
    let mut json_output = false;
    let mut token = None;
    let mut csv = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--json" => json_output = true,
            "--api" => {
                api = args
                    .next()
                    .ok_or("--api needs a value")?
                    .trim_end_matches('/')
                    .to_string();
                if api.is_empty() {
                    return Err("--api cannot be empty".to_string());
                }
            }
            "--token" => token = Some(args.next().ok_or("--token needs a value")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown flag: {flag} (see --help)"));
            }
            path if csv.is_none() => csv = Some(PathBuf::from(path)),
            path => return Err(format!("unexpected second CSV path: {path}")),
        }
    }

    Ok(Args {
        csv: csv.ok_or("missing CSV path (see --help)")?,
        api,
        dry_run,
        json: json_output,
        token,
    })
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Workout {
    id: String,
    title: String,
    raw_title: String,
    started_at_utc: String,
    duration_seconds: i64,
    duration_suspicious: bool,
    notes: Option<String>,
    description: Option<String>,
    source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Exercise {
    name: String,
    tags: Vec<ExerciseTag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PersonalRecord {
    level: String,
    kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct FitnessSet {
    id: String,
    workout_id: String,
    ordinal: i64,
    exercise_name: String,
    raw_exercise_name: String,
    exercise_note: Option<String>,
    superset_id: Option<i64>,
    weight_milli: Option<i64>,
    weight_unit: String,
    reps: Option<i64>,
    effort_hundredths: Option<i64>,
    distance_milli: Option<i64>,
    set_time_seconds: Option<i64>,
    set_type: String,
    /// Parsed and validated from the CSV as a corruption tripwire, but
    /// never sent: records are derived server-side from set history
    /// (import contract v2), not imported from Lyfta's medal columns.
    #[serde(skip_serializing)]
    records: Vec<PersonalRecord>,
}

#[derive(Debug)]
struct WorkoutBundle {
    workout: Workout,
    sets: Vec<FitnessSet>,
}

#[derive(Debug)]
struct Export {
    workouts: Vec<WorkoutBundle>,
    exercises: BTreeMap<String, Exercise>,
    row_count: usize,
}

#[derive(Debug)]
struct HeaderIndexes {
    title: usize,
    date: usize,
    duration: usize,
    description: usize,
    notes: usize,
    exercise: usize,
    exercise_note: usize,
    superset_id: usize,
    weight: usize,
    reps: usize,
    effort: usize,
    distance: usize,
    time: usize,
    set_type: usize,
    record_levels: usize,
    record_types: usize,
}

impl HeaderIndexes {
    fn new(headers: &csv::StringRecord) -> Result<Self, String> {
        let mut indexes = HashMap::new();
        for (index, header) in headers.iter().enumerate() {
            let canonical = canonical_header(header);
            if indexes.insert(canonical.clone(), index).is_some() {
                return Err(format!(
                    "duplicate CSV header after trimming: {canonical:?}"
                ));
            }
        }

        let required = |name: &str| {
            indexes
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing required CSV header {name:?}"))
        };
        Ok(Self {
            title: required("Title")?,
            date: required("Date")?,
            duration: required("Duration")?,
            description: required("Description")?,
            notes: required("Notes")?,
            exercise: required("Exercise")?,
            exercise_note: required("Exercise Note")?,
            superset_id: required("Superset id")?,
            weight: required("Weight")?,
            reps: required("Reps")?,
            effort: required("RIR/RPE")?,
            distance: required("Distance")?,
            time: required("Time")?,
            set_type: required("Set Type")?,
            record_levels: required("RecordLevel: gold/silver/bronze")?,
            record_types: required("RecordType: 1RM / max_weight / volume / reps")?,
        })
    }
}

fn canonical_header(header: &str) -> String {
    header.trim_start_matches('\u{feff}').trim().to_string()
}

fn field<'a>(record: &'a csv::StringRecord, index: usize, name: &str) -> Result<&'a str, String> {
    record
        .get(index)
        .ok_or_else(|| format!("row has no {name:?} column"))
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        None
    } else {
        Some(value.to_string())
    }
}

fn required_display(value: &str, field_name: &str) -> Result<String, String> {
    let display = collapse_whitespace(value);
    if display.is_empty() || display.eq_ignore_ascii_case("null") {
        Err(format!("{field_name} is empty"))
    } else {
        Ok(display)
    }
}

/// Parse a decimal without passing through binary floating point. Extra
/// zeroes beyond the stored precision are harmless; non-zero precision loss
/// is rejected so an import can never silently change source data.
fn parse_scaled_decimal(value: &str, decimal_places: u32) -> Result<i64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty decimal".to_string());
    }
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let mut pieces = unsigned.split('.');
    let whole = pieces.next().unwrap_or_default();
    let fraction = pieces.next().unwrap_or_default();
    if pieces.next().is_some()
        || (whole.is_empty() && fraction.is_empty())
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid decimal {value:?}"));
    }

    let places = decimal_places as usize;
    if fraction.len() > places && !fraction[places..].chars().all(|c| c == '0') {
        return Err(format!(
            "decimal {value:?} has more than {decimal_places} places"
        ));
    }
    let factor = 10_i128.pow(decimal_places);
    let whole: i128 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| format!("decimal {value:?} is too large"))?
    };
    let kept = &fraction[..fraction.len().min(places)];
    let mut fraction_value: i128 = if kept.is_empty() {
        0
    } else {
        kept.parse()
            .map_err(|_| format!("decimal {value:?} is too large"))?
    };
    for _ in kept.len()..places {
        fraction_value *= 10;
    }
    let scaled = whole
        .checked_mul(factor)
        .and_then(|n| n.checked_add(fraction_value))
        .ok_or_else(|| format!("decimal {value:?} is too large"))?;
    let signed = if negative { -scaled } else { scaled };
    signed
        .try_into()
        .map_err(|_| format!("decimal {value:?} is too large"))
}

fn optional_decimal(value: &str, decimal_places: u32) -> Result<Option<i64>, String> {
    if optional_text(value).is_none() {
        Ok(None)
    } else {
        let parsed = parse_scaled_decimal(value, decimal_places)?;
        if parsed < 0 {
            Err(format!("decimal {value:?} must be non-negative"))
        } else {
            Ok(Some(parsed))
        }
    }
}

/// Weight is the one signed set quantity: negative pounds represent
/// assistance. Reps, effort, distance, and time keep using
/// `optional_decimal`, which rejects negatives.
fn optional_weight(value: &str) -> Result<Option<i64>, String> {
    if optional_text(value).is_none() {
        Ok(None)
    } else {
        parse_scaled_decimal(value, 3).map(Some)
    }
}

/// Strong combines RIR and RPE in one column. This archive stores only RPE:
/// values below 6 are interpreted as RIR and converted with RPE = 10 - RIR.
fn normalize_effort_to_rpe(effort_hundredths: i64) -> i64 {
    if effort_hundredths < 600 {
        1_000 - effort_hundredths
    } else {
        effort_hundredths
    }
}

fn parse_colon_seconds(value: &str) -> Result<i64, String> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, parse_u64(minutes)?, parse_u64(seconds)?),
        [hours, minutes, seconds] => (parse_u64(hours)?, parse_u64(minutes)?, parse_u64(seconds)?),
        _ => return Err(format!("invalid colon time {value:?}")),
    };
    if minutes >= 60 && parts.len() == 3 {
        return Err(format!("invalid minutes in time {value:?}"));
    }
    if seconds >= 60 {
        return Err(format!("invalid seconds in time {value:?}"));
    }
    let total = hours
        .checked_mul(3600)
        .and_then(|n| n.checked_add(minutes * 60))
        .and_then(|n| n.checked_add(seconds))
        .ok_or_else(|| format!("time {value:?} is too large"))?;
    total
        .try_into()
        .map_err(|_| format!("time {value:?} is too large"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("invalid non-negative integer {value:?}"))
}

fn parse_set_time(value: &str) -> Result<Option<i64>, String> {
    let Some(value) = optional_text(value) else {
        return Ok(None);
    };
    if value.contains(':') {
        parse_colon_seconds(&value).map(Some)
    } else {
        optional_decimal(&value, 0)
    }
}

/// Strong's `Date` column is a UTC timestamp without an explicit offset.
/// Keep its canonical wall-clock representation unchanged so existing stable
/// workout and set IDs remain stable.
fn canonical_utc_start(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() != 19 {
        return Err(format!("invalid UTC workout start {value:?}"));
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b' ' | b'T')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(format!("invalid UTC workout start {value:?}"));
    }
    for (start, end) in [(0, 4), (5, 7), (8, 10), (11, 13), (14, 16), (17, 19)] {
        if !bytes[start..end].iter().all(u8::is_ascii_digit) {
            return Err(format!("invalid UTC workout start {value:?}"));
        }
    }
    let year: i32 = value[0..4].parse().expect("validated digits");
    let month: u32 = value[5..7].parse().expect("validated digits");
    let day: u32 = value[8..10].parse().expect("validated digits");
    let hour: u32 = value[11..13].parse().expect("validated digits");
    let minute: u32 = value[14..16].parse().expect("validated digits");
    let second: u32 = value[17..19].parse().expect("validated digits");
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if day == 0 || day > month_days || hour >= 24 || minute >= 60 || second >= 60 {
        return Err(format!("invalid UTC workout start {value:?}"));
    }
    Ok(format!("{} {}", &value[..10], &value[11..]))
}

fn workout_id(started_at_utc: &str) -> String {
    format!("fitness:{}", started_at_utc.replacen(' ', "T", 1))
}

fn set_id(workout_id: &str, ordinal: usize) -> String {
    format!("{workout_id}:{ordinal:04}")
}

fn parse_records(levels: &str, kinds: &str) -> Result<Vec<PersonalRecord>, String> {
    let levels: Vec<&str> = levels
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .collect();
    let kinds: Vec<&str> = kinds
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .collect();
    if levels.len() != kinds.len() {
        return Err(format!(
            "record level/type counts differ ({} levels, {} types)",
            levels.len(),
            kinds.len()
        ));
    }
    levels
        .into_iter()
        .zip(kinds)
        .map(|(level, kind)| {
            let level = match level {
                "1" => "gold",
                "2" => "silver",
                "3" => "bronze",
                other => return Err(format!("unknown record level {other:?}")),
            };
            let kind = match kind {
                "1" => "1rm",
                "2" => "max-weight",
                "3" => "volume",
                "4" => "reps",
                other => return Err(format!("unknown record type {other:?}")),
            };
            Ok(PersonalRecord {
                level: level.to_string(),
                kind: kind.to_string(),
            })
        })
        .collect()
}

fn contextual<T>(result: Result<T, String>, row: u64, field: &str) -> Result<T, String> {
    result.map_err(|error| format!("CSV row {row}, {field}: {error}"))
}

fn parse_reader<R: Read>(reader: R) -> Result<Export, String> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_reader(reader);
    let headers = reader
        .headers()
        .map_err(|error| format!("CSV headers: {error}"))?
        .clone();
    let columns = HeaderIndexes::new(&headers)?;
    let mut workout_indexes: HashMap<String, usize> = HashMap::new();
    let mut workouts: Vec<WorkoutBundle> = Vec::new();
    let mut exercises = BTreeMap::new();
    let mut row_count = 0;

    for record in reader.records() {
        let record = record.map_err(|error| format!("CSV: {error}"))?;
        row_count += 1;
        let row = row_count as u64 + 1;
        let get = |index, name| field(&record, index, name);

        let raw_title = get(columns.title, "Title")?.to_string();
        let title = contextual(required_display(&raw_title, "title"), row, "Title")?;
        let start = contextual(canonical_utc_start(get(columns.date, "Date")?), row, "Date")?;
        let duration = contextual(
            parse_colon_seconds(get(columns.duration, "Duration")?),
            row,
            "Duration",
        )?;
        let raw_exercise_name = get(columns.exercise, "Exercise")?.to_string();
        let exercise_name = contextual(
            required_display(&raw_exercise_name, "exercise name"),
            row,
            "Exercise",
        )?;
        let set_type = contextual(
            required_display(get(columns.set_type, "Set Type")?, "set type"),
            row,
            "Set Type",
        )?;
        let records = contextual(
            parse_records(
                get(columns.record_levels, "RecordLevel")?,
                get(columns.record_types, "RecordType")?,
            ),
            row,
            "records",
        )?;
        let weight_milli = contextual(
            optional_weight(get(columns.weight, "Weight")?),
            row,
            "Weight",
        )?;
        let reps = contextual(optional_decimal(get(columns.reps, "Reps")?, 0), row, "Reps")?;
        let effort_hundredths = contextual(
            optional_decimal(get(columns.effort, "RIR/RPE")?, 2),
            row,
            "RIR/RPE",
        )?
        .map(normalize_effort_to_rpe);
        let distance_milli = contextual(
            optional_decimal(get(columns.distance, "Distance")?, 3),
            row,
            "Distance",
        )?;
        let set_time_seconds = contextual(parse_set_time(get(columns.time, "Time")?), row, "Time")?;
        let notes = optional_text(get(columns.notes, "Notes")?);
        let description = optional_text(get(columns.description, "Description")?);

        let index = match workout_indexes.get(&start) {
            Some(index) => *index,
            None => {
                let index = workouts.len();
                let id = workout_id(&start);
                workouts.push(WorkoutBundle {
                    workout: Workout {
                        id,
                        title,
                        raw_title: raw_title.clone(),
                        started_at_utc: start.clone(),
                        duration_seconds: duration,
                        duration_suspicious: duration == 0 || duration >= 4 * 60 * 60,
                        notes: notes.clone(),
                        description: description.clone(),
                        source: "workout-data-csv".to_string(),
                    },
                    sets: Vec::new(),
                });
                workout_indexes.insert(start.clone(), index);
                index
            }
        };

        let bundle = &mut workouts[index];
        if bundle.workout.raw_title != raw_title
            || bundle.workout.duration_seconds != duration
            || bundle.workout.notes != notes
            || bundle.workout.description != description
        {
            return Err(format!(
                "CSV row {row}: workout metadata changed within UTC start {start}"
            ));
        }
        let ordinal = bundle.sets.len() + 1;
        bundle.sets.push(FitnessSet {
            id: set_id(&bundle.workout.id, ordinal),
            workout_id: bundle.workout.id.clone(),
            ordinal: ordinal as i64,
            exercise_name: exercise_name.clone(),
            raw_exercise_name,
            exercise_note: optional_text(get(columns.exercise_note, "Exercise Note")?),
            superset_id: contextual(
                optional_decimal(get(columns.superset_id, "Superset id")?, 0),
                row,
                "Superset id",
            )?,
            weight_milli,
            weight_unit: "lbs".to_string(),
            reps,
            effort_hundredths,
            distance_milli,
            set_time_seconds,
            set_type,
            records,
        });
        exercises
            .entry(exercise_name.clone())
            .or_insert_with(|| Exercise {
                name: exercise_name.clone(),
                tags: exercise_tags(&exercise_name),
            });
    }

    if row_count == 0 {
        return Err("CSV contains no set rows".to_string());
    }
    Ok(Export {
        workouts,
        exercises,
        row_count,
    })
}

fn resolve_token(cli: Option<String>) -> Option<String> {
    if let Some(token) = cli.filter(|token| !token.trim().is_empty()) {
        return Some(token);
    }
    if let Ok(token) = std::env::var("FITNESS_SYNC_TOKEN")
        && !token.trim().is_empty()
    {
        return Some(token.trim().to_string());
    }
    let path = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".config/benjisponge/fitness.token");
    fs::read_to_string(path)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

async fn fetch_existing_ids(
    client: &reqwest::Client,
    api: &str,
) -> Result<HashSet<String>, String> {
    #[derive(Deserialize)]
    struct Ids {
        ids: Vec<String>,
    }

    let url = format!("{api}/api/fitness/ids");
    let ids: Ids = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("GET {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GET {url}: {error}"))?
        .json()
        .await
        .map_err(|error| format!("GET {url}: bad response: {error}"))?;
    Ok(ids.ids.into_iter().collect())
}

#[derive(Debug, Deserialize)]
struct ImportReceipt {
    received: u64,
    added: u64,
    skipped: u64,
    version: u64,
}

#[derive(Serialize)]
struct ImportPayload<'a> {
    workouts: Vec<&'a Workout>,
    exercises: Vec<&'a Exercise>,
    sets: Vec<&'a FitnessSet>,
}

fn payload_for<'a>(
    bundles: &[&'a WorkoutBundle],
    exercises: &'a BTreeMap<String, Exercise>,
) -> ImportPayload<'a> {
    let names: BTreeSet<&str> = bundles
        .iter()
        .flat_map(|bundle| bundle.sets.iter().map(|set| set.exercise_name.as_str()))
        .collect();
    ImportPayload {
        workouts: bundles.iter().map(|bundle| &bundle.workout).collect(),
        exercises: names
            .into_iter()
            .filter_map(|name| exercises.get(name))
            .collect(),
        sets: bundles
            .iter()
            .flat_map(|bundle| bundle.sets.iter())
            .collect(),
    }
}

fn workout_needs_upload(bundle: &WorkoutBundle, existing_set_ids: &HashSet<String>) -> bool {
    bundle
        .sets
        .iter()
        .any(|set| !existing_set_ids.contains(&set.id))
}

fn bounded_chunks(bundles: Vec<&WorkoutBundle>) -> Result<Vec<Vec<&WorkoutBundle>>, String> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_sets = 0;
    for bundle in bundles {
        if bundle.sets.len() > MAX_SETS_PER_CHUNK {
            return Err(format!(
                "workout {} has {} sets, above the API's whole-workout limit of {}",
                bundle.workout.id,
                bundle.sets.len(),
                MAX_SETS_PER_CHUNK
            ));
        }
        if !current.is_empty()
            && (current.len() >= MAX_WORKOUTS_PER_CHUNK
                || current_sets + bundle.sets.len() > MAX_SETS_PER_CHUNK)
        {
            chunks.push(std::mem::take(&mut current));
            current_sets = 0;
        }
        current_sets += bundle.sets.len();
        current.push(bundle);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

async fn upload_chunk(
    client: &reqwest::Client,
    api: &str,
    token: &str,
    payload: &ImportPayload<'_>,
) -> Result<ImportReceipt, String> {
    let url = format!("{api}/api/fitness/import");
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("POST {url}: {error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("unauthorized — token rejected (see --help for token sources)".to_string());
    }
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("unable to read response: {error}"));
        return Err(format!("POST {url}: HTTP {status}: {body}"));
    }
    response
        .json()
        .await
        .map_err(|error| format!("POST {url}: bad response: {error}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("fitness_sync: {error}");
            return ExitCode::from(2);
        }
    };
    let file = match fs::File::open(&args.csv) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("fitness_sync: {}: {error}", args.csv.display());
            return ExitCode::FAILURE;
        }
    };
    let export = match parse_reader(file) {
        Ok(export) => export,
        Err(error) => {
            eprintln!("fitness_sync: {}: {error}", args.csv.display());
            return ExitCode::FAILURE;
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("fitness-sync (+https://ben.soy/fitness)")
        .build()
        .expect("reqwest client");
    let existing = match fetch_existing_ids(&client, &args.api).await {
        Ok(ids) => ids,
        Err(error) => {
            eprintln!("fitness_sync: {error}");
            return ExitCode::FAILURE;
        }
    };

    let already_synced = export
        .workouts
        .iter()
        .filter(|bundle| !workout_needs_upload(bundle, &existing))
        .count();
    let already_synced_sets = export
        .workouts
        .iter()
        .flat_map(|bundle| &bundle.sets)
        .filter(|set| existing.contains(&set.id))
        .count();
    let mut missing: Vec<&WorkoutBundle> = export
        .workouts
        .iter()
        .filter(|bundle| workout_needs_upload(bundle, &existing))
        .collect();
    missing.sort_by(|left, right| {
        left.workout
            .started_at_utc
            .cmp(&right.workout.started_at_utc)
    });
    let missing_sets: usize = missing
        .iter()
        .flat_map(|bundle| &bundle.sets)
        .filter(|set| !existing.contains(&set.id))
        .count();
    // Re-send every set in a partially present workout. This preserves full
    // parent context; stable set ids let the server cheaply skip duplicates.
    let upload_sets: usize = missing.iter().map(|bundle| bundle.sets.len()).sum();
    let chunks = match bounded_chunks(missing) {
        Ok(chunks) => chunks,
        Err(error) => {
            eprintln!("fitness_sync: {error}");
            return ExitCode::FAILURE;
        }
    };
    let suspicious = export
        .workouts
        .iter()
        .filter(|bundle| bundle.workout.duration_suspicious)
        .count();

    let mut received = 0;
    let mut added = 0;
    let mut skipped = 0;
    let mut version = None;
    if !args.dry_run && !chunks.is_empty() {
        let Some(token) = resolve_token(args.token.clone()) else {
            eprintln!(
                "fitness_sync: {} new workouts but no write token — set FITNESS_SYNC_TOKEN, \
                 pass --token, or create ~/.config/benjisponge/fitness.token",
                chunks.iter().map(Vec::len).sum::<usize>()
            );
            return ExitCode::FAILURE;
        };
        for (index, chunk) in chunks.iter().enumerate() {
            let payload = payload_for(chunk, &export.exercises);
            match upload_chunk(&client, &args.api, &token, &payload).await {
                Ok(receipt) => {
                    received += receipt.received;
                    added += receipt.added;
                    skipped += receipt.skipped;
                    version = Some(receipt.version);
                    if !args.json {
                        println!(
                            "  chunk {}/{}: {} added, {} skipped",
                            index + 1,
                            chunks.len(),
                            receipt.added,
                            receipt.skipped
                        );
                    }
                }
                Err(error) => {
                    eprintln!("fitness_sync: {error}");
                    eprintln!(
                        "fitness_sync: aborted after {added} additions — rerun to resume \
                         (stable ids and server duplicate handling make this safe)"
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    let missing_workouts = chunks.iter().map(Vec::len).sum::<usize>();
    if args.json {
        println!(
            "{}",
            json!({
                "api": args.api,
                "csv": args.csv.display().to_string(),
                "rows": export.row_count,
                "workouts": export.workouts.len(),
                "exercises": export.exercises.len(),
                "duration_suspicious": suspicious,
                "already_synced_workouts": already_synced,
                "already_synced_sets": already_synced_sets,
                "new_workouts": missing_workouts,
                "new_sets": missing_sets,
                "upload_sets": upload_sets,
                "chunks": chunks.len(),
                "received": received,
                "added": added,
                "skipped": skipped,
                "version": version,
                "dry_run": args.dry_run,
            })
        );
    } else {
        println!(
            "parsed {} sets across {} workouts and {} exercises; {} suspicious durations",
            export.row_count,
            export.workouts.len(),
            export.exercises.len(),
            suspicious
        );
        println!(
            "{already_synced_sets} set(s) across {already_synced} fully synced workout(s) already in the database"
        );
        if args.dry_run {
            println!(
                "dry run: {missing_workouts} workout(s) containing {missing_sets} new set(s) \
                 would send {upload_sets} set row(s) in {} chunk(s)",
                chunks.len()
            );
        } else if chunks.is_empty() {
            println!("nothing to upload — in sync");
        } else {
            println!(
                "server received {received} row(s): {added} added, {skipped} skipped (version {})",
                version.unwrap_or_default()
            );
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = " Title,Date,Duration,Description,Notes,Exercise,Exercise Note,Superset id,Weight,Reps,RIR/RPE,Distance,Time,Set Type,RecordLevel: gold/silver/bronze,RecordType: 1RM / max_weight / volume / reps\n";

    fn has_tag(tags: &[ExerciseTag], kind: &str, value: &str) -> bool {
        tags.iter()
            .any(|tag| tag.kind == kind && tag.value == value)
    }

    #[test]
    fn parses_rows_losslessly_and_numbers_exactly() {
        let csv = format!(
            "{HEADER}\
             First workout ,2026-07-21 14:39:04,00:35:10,,,Lever Seated Leg Press ,note,1,45.125,10,0.5,null,9:30,NORMAL_SET,1,2\n\
             Other,2026-07-20 09:00:00,04:00:00,null,null,Running,,null,null,null,null,1.250,1800.0,NORMAL_SET,,\n\
             First workout ,2026-07-21 14:39:04,00:35:10,,,Full Squat,,,135.000,6,1.0,,null,WARMUP_SET,\"3,2\",\"3,4\"\n"
        );
        let export = parse_reader(csv.as_bytes()).unwrap();
        assert_eq!(export.row_count, 3);
        assert_eq!(export.workouts.len(), 2);
        assert_eq!(export.exercises.len(), 3);

        let first = &export.workouts[0];
        assert_eq!(first.workout.id, "fitness:2026-07-21T14:39:04");
        assert_eq!(first.workout.title, "First workout");
        assert_eq!(first.workout.raw_title, "First workout ");
        assert_eq!(first.workout.started_at_utc, "2026-07-21 14:39:04");
        assert_eq!(first.workout.duration_seconds, 2_110);
        assert_eq!(first.workout.source, "workout-data-csv");
        assert!(!first.workout.duration_suspicious);
        assert_eq!(first.sets.len(), 2);
        assert_eq!(first.sets[0].ordinal, 1);
        assert_eq!(first.sets[1].ordinal, 2);
        assert_eq!(first.sets[1].id, "fitness:2026-07-21T14:39:04:0002");
        assert_eq!(first.sets[0].exercise_name, "Lever Seated Leg Press");
        assert_eq!(first.sets[0].raw_exercise_name, "Lever Seated Leg Press ");
        assert_eq!(first.sets[0].weight_milli, Some(45_125));
        assert_eq!(first.sets[0].weight_unit, "lbs");
        assert_eq!(first.sets[0].superset_id, Some(1));
        assert_eq!(first.sets[0].effort_hundredths, Some(950));
        assert_eq!(first.sets[0].set_time_seconds, Some(570));
        assert_eq!(first.sets[1].weight_milli, Some(135_000));
        assert_eq!(
            first.sets[1].records,
            vec![
                PersonalRecord {
                    level: "bronze".into(),
                    kind: "volume".into(),
                },
                PersonalRecord {
                    level: "silver".into(),
                    kind: "reps".into(),
                }
            ]
        );
        let mut existing = HashSet::from([first.sets[0].id.clone()]);
        assert!(workout_needs_upload(first, &existing));
        existing.insert(first.sets[1].id.clone());
        assert!(!workout_needs_upload(first, &existing));

        let other = &export.workouts[1];
        assert!(other.workout.duration_suspicious);
        assert_eq!(other.workout.notes, None);
        assert_eq!(other.sets[0].distance_milli, Some(1_250));
        assert_eq!(other.sets[0].set_time_seconds, Some(1_800));
    }

    #[test]
    fn accepts_bom_and_trims_only_headers() {
        let csv = format!(
            "\u{feff}{HEADER}Title  ,2024-02-29T23:59:59,00:00:00,,,Exercise  ,,,,,,,,NORMAL_SET,,\n"
        );
        let export = parse_reader(csv.as_bytes()).unwrap();
        assert_eq!(export.workouts[0].workout.raw_title, "Title  ");
        assert_eq!(export.workouts[0].sets[0].raw_exercise_name, "Exercise  ");
        assert!(export.workouts[0].workout.duration_suspicious);
    }

    #[test]
    fn decimal_parser_rejects_silent_precision_loss() {
        assert_eq!(parse_scaled_decimal("45", 3).unwrap(), 45_000);
        assert_eq!(parse_scaled_decimal(".5", 2).unwrap(), 50);
        assert_eq!(parse_scaled_decimal("-1.25", 2).unwrap(), -125);
        assert_eq!(parse_scaled_decimal("1.2300", 2).unwrap(), 123);
        assert!(parse_scaled_decimal("1.234", 2).is_err());
        assert!(parse_scaled_decimal("1.2.3", 3).is_err());
        assert!(optional_decimal("-1", 3).is_err());
    }

    #[test]
    fn csv_weights_allow_negative_assistance_only() {
        let csv = format!(
            "{HEADER}\
             Assisted,2026-07-21 14:39:04,00:10:00,,,Pull Up (Assisted),,,\
             -45.125,8,,,,NORMAL_SET,,\n"
        );
        let export = parse_reader(csv.as_bytes()).unwrap();
        assert_eq!(export.workouts[0].sets[0].weight_milli, Some(-45_125));
        assert_eq!(optional_weight("-45.125").unwrap(), Some(-45_125));
        assert!(optional_decimal("-8", 0).is_err(), "reps stay unsigned");
    }

    #[test]
    fn mixed_effort_values_are_normalized_to_rpe() {
        assert_eq!(normalize_effort_to_rpe(0), 1_000);
        assert_eq!(normalize_effort_to_rpe(150), 850);
        assert_eq!(normalize_effort_to_rpe(599), 401);
        assert_eq!(normalize_effort_to_rpe(600), 600);
        assert_eq!(normalize_effort_to_rpe(850), 850);
    }

    #[test]
    fn utc_dates_are_validated_and_canonicalized() {
        assert_eq!(
            canonical_utc_start("2024-02-29 01:02:03").unwrap(),
            "2024-02-29 01:02:03"
        );
        assert!(canonical_utc_start("2023-02-29 01:02:03").is_err());
        assert!(canonical_utc_start("2024-13-01 01:02:03").is_err());
        assert!(canonical_utc_start("2024-01-01 24:02:03").is_err());
    }

    #[test]
    fn import_payload_sends_utc_start_without_changing_stable_ids() {
        let csv = format!(
            "{HEADER}UTC workout,2026-07-12 00:33:27,00:01:00,,,Squat,,,,,,,,NORMAL_SET,,\n"
        );
        let export = parse_reader(csv.as_bytes()).unwrap();
        let workout = &export.workouts[0];
        let payload = payload_for(&[workout], &export.exercises);
        let serialized = serde_json::to_value(payload).unwrap();
        let imported_workout = &serialized["workouts"][0];

        assert_eq!(
            imported_workout["started_at_utc"],
            serde_json::Value::String("2026-07-12 00:33:27".to_string())
        );
        assert!(imported_workout.get("started_at_local").is_none());
        assert_eq!(workout.workout.id, "fitness:2026-07-12T00:33:27");
        assert_eq!(workout.sets[0].id, "fitness:2026-07-12T00:33:27:0001");
        assert!(
            serialized["sets"][0].get("records").is_none(),
            "contract v2: records are derived server-side, never sent",
        );
    }

    #[test]
    fn record_codes_are_zipped_not_cross_producted() {
        assert_eq!(
            parse_records("1,3", "2,4").unwrap(),
            vec![
                PersonalRecord {
                    level: "gold".into(),
                    kind: "max-weight".into(),
                },
                PersonalRecord {
                    level: "bronze".into(),
                    kind: "reps".into(),
                }
            ]
        );
        assert!(parse_records("1,2", "1").is_err());
        assert!(parse_records("4", "1").is_err());
    }

    #[test]
    fn exact_squat_allowlist_matches_the_548_set_audit() {
        let audited_counts = [
            ("Barbell Zercher Squat", 39),
            ("Bulgarian Split Squat", 14),
            ("Bulgarian Split Squat (Smith Machine)", 4),
            ("Deficit Split Squat (Smith Machine)", 7),
            ("Dumbbell Assisted Bulgarian Split Squat", 2),
            ("Dumbbell Walking Lunges", 24),
            ("Full Squat", 315),
            ("Lever Horizontal One leg Press", 14),
            ("Lever Seated Leg Press", 32),
            ("Lunge", 28),
            ("Sissy Squat", 1),
            ("Sled 45° Leg Press", 28),
            ("Sled Hack Squat", 6),
            ("Smith Lateral Step-Up", 9),
            ("Smith Sprint Lunge", 7),
            ("Smith Squat", 14),
            ("Step-Up (Weighted)", 3),
            ("Step-up", 1),
        ];
        assert_eq!(
            audited_counts.iter().map(|(_, count)| count).sum::<i32>(),
            548
        );
        assert_eq!(
            audited_counts
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            SQUAT_TYPE_EXERCISES
        );
        for (name, _) in audited_counts {
            assert!(has_tag(&exercise_tags(name), "movement", "squat-type"));
        }

        let good_morning = exercise_tags("Good Morning (Squat Machine)");
        assert!(!has_tag(&good_morning, "movement", "squat-type"));
        assert!(has_tag(&good_morning, "movement", "hinge"));
        assert!(has_tag(&good_morning, "equipment", "machine"));
    }

    #[test]
    fn broad_taxonomy_emits_only_supported_kinds() {
        let bench = exercise_tags("Dumbbell Incline Bench Press");
        assert!(has_tag(&bench, "movement", "horizontal-push"));
        assert!(has_tag(&bench, "muscle", "chest"));
        assert!(has_tag(&bench, "equipment", "dumbbell"));

        let pulldown = exercise_tags("Cable Wide-Grip Lat Pulldown");
        assert!(has_tag(&pulldown, "movement", "vertical-pull"));
        assert!(has_tag(&pulldown, "muscle", "back"));
        assert!(has_tag(&pulldown, "equipment", "cable"));

        for exercise in [bench, pulldown] {
            assert!(
                exercise
                    .iter()
                    .all(|tag| matches!(tag.kind.as_str(), "movement" | "muscle" | "equipment"))
            );
        }
    }

    #[test]
    fn taxonomy_respects_words_and_curl_variants() {
        let throw = exercise_tags("Medicine Ball Standing Overhead Throw");
        assert!(has_tag(&throw, "movement", "throw"));
        assert!(!has_tag(&throw, "movement", "horizontal-pull"));
        assert!(!has_tag(&throw, "muscle", "back"));

        let row = exercise_tags("Bent Over One Arm Row (Dumbbell)");
        assert!(has_tag(&row, "movement", "horizontal-pull"));
        assert!(has_tag(&row, "muscle", "back"));

        let jefferson = exercise_tags("Barbell Jefferson Curl");
        assert!(has_tag(&jefferson, "movement", "hinge"));
        assert!(!has_tag(&jefferson, "movement", "elbow-flexion"));
        assert!(!has_tag(&jefferson, "muscle", "biceps"));

        let wrist = exercise_tags("Cable One Arm Wrist Curl");
        assert!(has_tag(&wrist, "movement", "grip-wrist"));
        assert!(has_tag(&wrist, "muscle", "forearms"));
        assert!(!has_tag(&wrist, "movement", "elbow-flexion"));
        assert!(!has_tag(&wrist, "muscle", "biceps"));
    }

    #[test]
    fn taxonomy_recognizes_unassisted_bodyweight_rows_and_dips() {
        let row = exercise_tags("Inverted Row (Bodyweight)");
        assert!(has_tag(&row, "equipment", "bodyweight"));

        let chest_dip = exercise_tags("Chest Dip");
        assert!(has_tag(&chest_dip, "equipment", "bodyweight"));

        let assisted_dip = exercise_tags("Chest Dip (Assisted)");
        assert!(!has_tag(&assisted_dip, "equipment", "bodyweight"));

        let machine_dip = exercise_tags("Lever Seated Dip");
        assert!(has_tag(&machine_dip, "equipment", "machine"));
        assert!(!has_tag(&machine_dip, "equipment", "bodyweight"));
    }

    #[test]
    fn chunks_keep_workouts_whole_and_obey_bounds() {
        let make_bundle = |number: usize, sets: usize| WorkoutBundle {
            workout: Workout {
                id: number.to_string(),
                title: String::new(),
                raw_title: String::new(),
                started_at_utc: String::new(),
                duration_seconds: 1,
                duration_suspicious: false,
                notes: None,
                description: None,
                source: String::new(),
            },
            sets: (0..sets)
                .map(|ordinal| FitnessSet {
                    id: ordinal.to_string(),
                    workout_id: number.to_string(),
                    ordinal: ordinal as i64,
                    exercise_name: String::new(),
                    raw_exercise_name: String::new(),
                    exercise_note: None,
                    superset_id: None,
                    weight_milli: None,
                    weight_unit: "lbs".to_string(),
                    reps: None,
                    effort_hundredths: None,
                    distance_milli: None,
                    set_time_seconds: None,
                    set_type: String::new(),
                    records: Vec::new(),
                })
                .collect(),
        };
        let bundles: Vec<WorkoutBundle> = (0..45).map(|n| make_bundle(n, 21)).collect();
        let chunks = bounded_chunks(bundles.iter().collect()).unwrap();
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 45);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 20));
        assert!(chunks.iter().all(|chunk| {
            chunk.iter().map(|bundle| bundle.sets.len()).sum::<usize>() <= MAX_SETS_PER_CHUNK
        }));

        let oversized = make_bundle(99, MAX_SETS_PER_CHUNK + 1);
        assert!(bounded_chunks(vec![&oversized]).is_err());
    }
}
