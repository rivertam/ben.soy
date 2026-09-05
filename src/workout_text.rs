//! Canonical plain-text workout rendering shared by the site and Podrick.

use serde::{Deserialize, Serialize};

const WARMUP_SET: &str = "WARMUP_SET";
const LEGACY_FAILURE_SET: &str = "FAILURE_SET";

/// The public workout fields needed by the text formatter. Unknown API fields
/// are intentionally ignored when Podrick deserializes the public endpoint.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Workout {
    pub title: String,
    #[serde(default)]
    pub started_at_local: String,
    #[serde(default)]
    pub ended_at_local: String,
    #[serde(default)]
    pub duration_seconds: i64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sets: Vec<Set>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Set {
    pub exercise_name: String,
    #[serde(default)]
    pub set_type: String,
    #[serde(default)]
    pub superset_id: Option<i64>,
    #[serde(default)]
    pub weight_milli: Option<i64>,
    #[serde(default)]
    pub weight_unit: String,
    #[serde(default)]
    pub reps: Option<i64>,
    #[serde(default)]
    pub effort_hundredths: Option<i64>,
    #[serde(default)]
    pub failure: bool,
    #[serde(default)]
    pub distance_milli: Option<i64>,
    #[serde(default)]
    pub set_time_seconds: Option<i64>,
}

/// Render Podrick's workout format. `permalink` may be absolute or a bare
/// site path. A caller-specific character cap drops only whole exercise
/// blocks and always retains the header and permalink.
pub fn format(workout: &Workout, permalink: &str, max_chars: Option<usize>) -> String {
    let header = message_header(workout);
    let groups = exercise_groups(&workout.sets);
    let mut body = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        body.push(String::new());
        body.extend(group_lines(group, index + 1));
    }

    let message = [
        header.clone(),
        body,
        vec![String::new(), permalink.to_string()],
    ]
    .concat()
    .join("\n");
    if max_chars.is_none_or(|limit| message.chars().count() <= limit) {
        return message;
    }
    truncated(
        &header,
        &groups,
        permalink,
        max_chars.expect("checked capped rendering"),
    )
}

fn message_header(workout: &Workout) -> Vec<String> {
    let mut header = vec![format!("**{}**", escape_markdown(&workout.title))];
    let mut facts = Vec::new();
    if let Some(date) = format_date(&workout.started_at_local) {
        facts.push(date);
    }
    if let Some(range) = format_time_range(&workout.started_at_local, &workout.ended_at_local) {
        facts.push(range);
    }
    if workout.duration_seconds > 0 {
        facts.push(format_duration(workout.duration_seconds));
    }
    let working = workout.sets.iter().filter(|set| is_working(set)).count();
    facts.push(plural(working, "working set", "working sets"));
    header.push(facts.join(" · "));

    for extra in [&workout.description, &workout.notes] {
        if let Some(text) = extra
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            header.push(String::new());
            header.push(escape_markdown(text));
        }
    }
    header
}

struct ExerciseGroup<'a> {
    name: &'a str,
    superset_id: Option<i64>,
    sets: Vec<&'a Set>,
}

fn exercise_groups(sets: &[Set]) -> Vec<ExerciseGroup<'_>> {
    let mut groups: Vec<ExerciseGroup<'_>> = Vec::new();
    for set in sets {
        match groups.last_mut() {
            Some(group) if group.name == set.exercise_name => group.sets.push(set),
            _ => groups.push(ExerciseGroup {
                name: &set.exercise_name,
                superset_id: set.superset_id,
                sets: vec![set],
            }),
        }
    }
    groups
}

fn group_heading(group: &ExerciseGroup<'_>, position: usize) -> String {
    let numbered = format!("{}. {}", roman(position), escape_markdown(group.name));
    match group.superset_id {
        Some(id) => format!("{numbered} · superset {id}"),
        None => numbered,
    }
}

fn is_working(set: &Set) -> bool {
    set.set_type != WARMUP_SET
}

fn group_lines(group: &ExerciseGroup<'_>, position: usize) -> Vec<String> {
    let mut lines = vec![group_heading(group, position)];
    let mut working = 0usize;
    for set in &group.sets {
        let marker = if is_working(set) {
            working += 1;
            format!("{working}.")
        } else {
            "W.".to_string()
        };
        lines.push(format!("{marker} {}", set_line(set)));
    }
    lines
}

pub fn set_line(set: &Set) -> String {
    let mut line = prescription(set);
    let failure = set.failure || set.set_type == LEGACY_FAILURE_SET;
    if failure {
        line.push_str(" · failure");
    } else if let Some(effort) = set.effort_hundredths {
        line.push_str(&format!(" @ RPE {}", format_scaled(effort, 100)));
    }
    line
}

pub fn roman(value: usize) -> String {
    const NUMERALS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut remaining = value;
    let mut output = String::new();
    for (amount, numeral) in NUMERALS {
        while remaining >= amount {
            output.push_str(numeral);
            remaining -= amount;
        }
    }
    output
}

fn prescription(set: &Set) -> String {
    let unit = if set.weight_unit.is_empty() {
        "lbs"
    } else {
        &set.weight_unit
    };
    match (set.weight_milli, set.reps) {
        (Some(load), Some(reps)) => format!("{} {unit} × {reps}", format_scaled(load, 1_000)),
        (Some(load), None) => format!("{} {unit}", format_scaled(load, 1_000)),
        (None, Some(reps)) => format!("{reps} reps"),
        (None, None) => match (set.distance_milli, set.set_time_seconds) {
            (Some(distance), _) => format!("distance {}", format_scaled(distance, 1_000)),
            (None, Some(seconds)) => format_duration(seconds),
            (None, None) => "logged".to_string(),
        },
    }
}

fn truncated(
    header: &[String],
    groups: &[ExerciseGroup<'_>],
    permalink: &str,
    limit: usize,
) -> String {
    let total_sets: usize = groups.iter().map(|group| group.sets.len()).sum();
    let mut kept = Vec::new();
    let mut shown_sets = 0usize;

    for (index, group) in groups.iter().enumerate() {
        let block = [vec![String::new()], group_lines(group, index + 1)].concat();
        let omitted = total_sets - (shown_sets + group.sets.len());
        let candidate = [
            header.to_vec(),
            kept.clone(),
            block.clone(),
            vec![
                String::new(),
                format!("… and {omitted} more"),
                String::new(),
                permalink.to_string(),
            ],
        ]
        .concat()
        .join("\n");
        if candidate.chars().count() > limit {
            break;
        }
        kept.extend(block);
        shown_sets += group.sets.len();
    }

    let mut lines = [header.to_vec(), kept].concat();
    let omitted = total_sets - shown_sets;
    if omitted > 0 {
        lines.push(String::new());
        lines.push(format!("… and {omitted} more"));
    }
    lines.push(String::new());
    lines.push(permalink.to_string());
    lines.join("\n")
}

pub fn format_scaled(value: i64, scale: i64) -> String {
    let negative = value < 0;
    let value = value.unsigned_abs();
    let scale = scale.unsigned_abs();
    let whole = value / scale;
    let remainder = value % scale;
    let mut output = format_integer(whole);
    if remainder > 0 {
        let width = scale.ilog10() as usize;
        let fraction = format!("{remainder:0width$}")
            .trim_end_matches('0')
            .to_string();
        output.push('.');
        output.push_str(&fraction);
    }
    if negative {
        output.insert(0, '-');
    }
    output
}

fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

pub fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse_local(value: &str) -> Option<(i64, usize, i64, i64, i64)> {
    let bytes = value.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
        return None;
    }
    let year = value[0..4].parse().ok()?;
    let month: usize = value[5..7].parse().ok()?;
    let day = value[8..10].parse().ok()?;
    let hour = value[11..13].parse().ok()?;
    let minute = value[14..16].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month, day, hour, minute))
}

pub fn format_date(local: &str) -> Option<String> {
    let (year, month, day, _, _) = parse_local(local)?;
    Some(format!("{} {day}, {year}", MONTHS[month - 1]))
}

fn format_time_range(start: &str, end: &str) -> Option<String> {
    let start = format_clock(start)?;
    Some(match format_clock(end) {
        Some(end) => format!("{start}–{end}"),
        None => start,
    })
}

pub fn format_clock(local: &str) -> Option<String> {
    let (_, _, _, hour, minute) = parse_local(local)?;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    Some(format!("{hour12}:{minute:02} {suffix}"))
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

fn escape_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '*' | '_' | '~' | '`' | '|' | '\\' | '>' | '#') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_wins_over_numeric_effort_in_legacy_input() {
        let set = Set {
            exercise_name: "Curl".into(),
            set_type: LEGACY_FAILURE_SET.into(),
            weight_milli: Some(50_000),
            weight_unit: "lbs".into(),
            reps: Some(8),
            effort_hundredths: Some(900),
            ..Set::default()
        };
        assert_eq!(set_line(&set), "50 lbs × 8 · failure");
    }
}
