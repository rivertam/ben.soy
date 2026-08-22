//! Query-filter parsing — the Rust `parseFilters`.
//!
//! Input is the decoded key/value pairs in query-string order (the same
//! thing `URLSearchParams` iterates). Validation order, first-error-wins
//! behavior, and every message are contract, pinned by the `err_*` golden
//! fixtures — including the two distinct `per_page` messages and the
//! vestigial-but-documented 50-byte LIKE pattern limit.

use super::validate;

const ALLOWED_FILTERS: [&str; 22] = [
    "q",
    "movement",
    "muscle",
    "equipment",
    "set_type",
    "exercise",
    "from",
    "to",
    "time_of_day",
    "weekday",
    "min_load",
    "max_load",
    "min_reps",
    "max_reps",
    "max_effort",
    "has_record",
    "has_superset",
    "has_notes",
    "incomplete",
    "duration",
    "page",
    "per_page",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeOfDay {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl TimeOfDay {
    pub fn contains_hour(self, hour: u8) -> bool {
        match self {
            TimeOfDay::Morning => (5..=11).contains(&hour),
            TimeOfDay::Afternoon => (12..=16).contains(&hour),
            TimeOfDay::Evening => (17..=20).contains(&hour),
            TimeOfDay::Night => hour >= 21 || hour <= 4,
        }
    }
}

/// A fully validated filter set, ready for the snapshot's predicate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    /// The trimmed search term; matching is ASCII-case-insensitive
    /// substring (SQLite `LIKE … COLLATE NOCASE` fidelity).
    pub q: Option<String>,
    pub movement: Vec<String>,
    pub muscle: Vec<String>,
    pub equipment: Vec<String>,
    pub set_types: Vec<String>,
    /// Compared against the canonical exercise name, ASCII-case-insensitively.
    pub exercise: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub time_of_day: Option<TimeOfDay>,
    /// 0 = Sunday, like SQLite's `strftime('%w', …)`.
    pub weekday: Option<u8>,
    pub min_load: Option<u64>,
    pub max_load: Option<u64>,
    pub min_reps: Option<u64>,
    pub max_reps: Option<u64>,
    pub max_effort: Option<u64>,
    pub has_record: Option<bool>,
    pub has_superset: Option<bool>,
    pub has_notes: Option<bool>,
    pub incomplete: Option<bool>,
    /// `duration=suspicious` → `Some(true)`; `normal` → `Some(false)`.
    pub duration_suspicious: Option<bool>,
    pub page: usize,
    pub per_page: usize,
}

impl Filters {
    /// Running activities participate only in the four activity-agnostic
    /// filters. Pagination is also harmless; every lifting-specific archive
    /// predicate excludes runs instead of pretending they have sets.
    pub(in crate::app::interests::lifting) fn admits_runs(&self) -> bool {
        self.q.is_none()
            && self.movement.is_empty()
            && self.muscle.is_empty()
            && self.equipment.is_empty()
            && self.set_types.is_empty()
            && self.exercise.is_none()
            && self.min_load.is_none()
            && self.max_load.is_none()
            && self.min_reps.is_none()
            && self.max_reps.is_none()
            && self.max_effort.is_none()
            && self.has_record.is_none()
            && self.has_superset.is_none()
            && self.has_notes.is_none()
            && self.incomplete.is_none()
            && self.duration_suspicious.is_none()
    }
}

pub fn parse_filters(pairs: &[(String, String)]) -> Result<Filters, String> {
    for (key, _) in pairs {
        if !ALLOWED_FILTERS.contains(&key.as_str()) {
            return Err(format!("unknown filter: {key}"));
        }
    }
    let mut filters = Filters {
        page: 1,
        per_page: 20,
        ..Filters::default()
    };

    if let Some(q) = single(pairs, "q")? {
        let term = validate::js_trim(q);
        if !validate::valid_text(term, 1, 100) {
            return Err("q must be 1-100 text characters".to_string());
        }
        if like_pattern_bytes(term) > 50 {
            return Err("q is too long after escaping (50-byte pattern limit)".to_string());
        }
        filters.q = Some(term.to_string());
    }

    filters.movement = repeated(pairs, "movement", 8)?;
    filters.muscle = repeated(pairs, "muscle", 8)?;
    filters.equipment = repeated(pairs, "equipment", 8)?;
    filters.set_types = repeated_set_types(pairs)?;

    if let Some(exercise) = single(pairs, "exercise")? {
        if !validate::valid_text(exercise, 1, 240) || validate::js_trim(exercise).is_empty() {
            return Err("exercise must be 1-240 non-whitespace characters".to_string());
        }
        filters.exercise = Some(exercise.to_string());
    }

    if let Some(from) = single(pairs, "from")? {
        if !validate::valid_date(from) {
            return Err("from must be a real YYYY-MM-DD date".to_string());
        }
        filters.from = Some(from.to_string());
    }
    if let Some(to) = single(pairs, "to")? {
        if !validate::valid_date(to) {
            return Err("to must be a real YYYY-MM-DD date".to_string());
        }
        filters.to = Some(to.to_string());
    }
    if let (Some(from), Some(to)) = (&filters.from, &filters.to)
        && from > to
    {
        return Err("from must not exceed to".to_string());
    }

    if let Some(time) = single(pairs, "time_of_day")? {
        filters.time_of_day = Some(match time {
            "morning" => TimeOfDay::Morning,
            "afternoon" => TimeOfDay::Afternoon,
            "evening" => TimeOfDay::Evening,
            "night" => TimeOfDay::Night,
            _ => {
                return Err("time_of_day must be morning, afternoon, evening, or night".to_string());
            }
        });
    }

    if let Some(weekday) = single(pairs, "weekday")? {
        let day = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
            .iter()
            .position(|name| *name == weekday);
        let Some(day) = day else {
            return Err("weekday must be sun, mon, tue, wed, thu, fri, or sat".to_string());
        };
        filters.weekday = Some(day as u8);
    }

    filters.min_load = scaled(pairs, "min_load", 3, 1_000_000_000)?;
    filters.max_load = scaled(pairs, "max_load", 3, 1_000_000_000)?;
    if let (Some(min), Some(max)) = (filters.min_load, filters.max_load)
        && min > max
    {
        return Err("min_load must not exceed max_load".to_string());
    }

    filters.min_reps = integer(pairs, "min_reps", 0, 1_000_000)?;
    filters.max_reps = integer(pairs, "max_reps", 0, 1_000_000)?;
    if let (Some(min), Some(max)) = (filters.min_reps, filters.max_reps)
        && min > max
    {
        return Err("min_reps must not exceed max_reps".to_string());
    }

    filters.max_effort = scaled(pairs, "max_effort", 2, 100_000)?;

    filters.has_record = boolean(pairs, "has_record")?;
    filters.has_superset = boolean(pairs, "has_superset")?;
    filters.has_notes = boolean(pairs, "has_notes")?;
    filters.incomplete = boolean(pairs, "incomplete")?;

    if let Some(duration) = single(pairs, "duration")? {
        match duration {
            "normal" => filters.duration_suspicious = Some(false),
            "suspicious" => filters.duration_suspicious = Some(true),
            _ => return Err("duration must be normal or suspicious".to_string()),
        }
    }

    if let Some(page) = integer(pairs, "page", 1, 100_000)? {
        filters.page = page as usize;
    }
    let per_page = integer(pairs, "per_page", 10, 40)?;
    if let Some(per_page) = per_page {
        if per_page != 10 && per_page != 20 && per_page != 40 {
            return Err("per_page must be 10, 20, or 40".to_string());
        }
        filters.per_page = per_page as usize;
    }

    Ok(filters)
}

/// The escaped `%…%` LIKE pattern's UTF-8 byte length — D1 capped patterns
/// at 50 bytes, and the limit (with its message) is documented contract.
fn like_pattern_bytes(term: &str) -> usize {
    let escapes = term
        .bytes()
        .filter(|b| matches!(b, b'\\' | b'%' | b'_'))
        .count();
    term.len() + escapes + 2
}

fn values<'a>(pairs: &'a [(String, String)], key: &str) -> impl Iterator<Item = &'a str> {
    pairs
        .iter()
        .filter(move |(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn single<'a>(pairs: &'a [(String, String)], key: &str) -> Result<Option<&'a str>, String> {
    let mut found = values(pairs, key);
    let first = found.next();
    if found.next().is_some() {
        return Err(format!("{key} may appear only once"));
    }
    Ok(first)
}

fn repeated(pairs: &[(String, String)], key: &str, limit: usize) -> Result<Vec<String>, String> {
    repeated_by(pairs, key, limit, validate::valid_tag_value)
}

fn repeated_set_types(pairs: &[(String, String)]) -> Result<Vec<String>, String> {
    repeated_by(pairs, "set_type", 8, validate::valid_set_type)
}

fn repeated_by(
    pairs: &[(String, String)],
    key: &str,
    limit: usize,
    valid: fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    let entries: Vec<&str> = values(pairs, key).collect();
    if entries.len() > limit {
        return Err(format!("{key} may appear at most {limit} times"));
    }
    let mut unique = std::collections::HashSet::new();
    for entry in &entries {
        if !valid(entry) {
            return Err(format!("bad {key} value"));
        }
        if !unique.insert(*entry) {
            return Err(format!("duplicate {key} value: {entry}"));
        }
    }
    Ok(entries.into_iter().map(str::to_string).collect())
}

fn integer(
    pairs: &[(String, String)],
    key: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, String> {
    let Some(value) = single(pairs, key)? else {
        return Ok(None);
    };
    let canonical_digits = value == "0"
        || (value.bytes().all(|b| b.is_ascii_digit())
            && !value.is_empty()
            && value.as_bytes()[0] != b'0');
    if !canonical_digits {
        return Err(format!("{key} must be an integer"));
    }
    match value.parse::<u64>() {
        Ok(parsed) if (min..=max).contains(&parsed) => Ok(Some(parsed)),
        // Overflow behaves like the Worker's Number(): a huge integral
        // float that then fails the range check.
        _ => Err(format!("{key} must be between {min} and {max}")),
    }
}

fn scaled(
    pairs: &[(String, String)],
    key: &str,
    places: u32,
    max_scaled: u64,
) -> Result<Option<u64>, String> {
    let Some(value) = single(pairs, key)? else {
        return Ok(None);
    };
    let error = || format!("{key} must be a non-negative decimal with at most {places} places");
    let (integer_part, fraction) = match value.split_once('.') {
        Some((integer_part, fraction)) => (integer_part, fraction),
        None => (value, ""),
    };
    // /^(0|[1-9]\d{0,8})(?:\.(\d+))?$/ with the fraction bounded by `places`.
    let integer_ok = integer_part == "0"
        || (!integer_part.is_empty()
            && integer_part.len() <= 9
            && integer_part.as_bytes()[0] != b'0'
            && integer_part.bytes().all(|b| b.is_ascii_digit()));
    let fraction_ok = if value.contains('.') {
        !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit())
    } else {
        true
    };
    if !integer_ok || !fraction_ok || fraction.len() > places as usize {
        return Err(error());
    }
    let scale = 10u64.pow(places);
    let whole: u64 = integer_part.parse().map_err(|_| error())?;
    let padded: u64 = if fraction.is_empty() {
        0
    } else {
        let mut digits = fraction.to_string();
        while digits.len() < places as usize {
            digits.push('0');
        }
        digits.parse().map_err(|_| error())?
    };
    let scaled = whole * scale + padded;
    if scaled > max_scaled {
        return Err(error());
    }
    Ok(Some(scaled))
}

fn boolean(pairs: &[(String, String)], key: &str) -> Result<Option<bool>, String> {
    match single(pairs, key)? {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn err(entries: &[(&str, &str)]) -> String {
        parse_filters(&pairs(entries)).unwrap_err()
    }

    #[test]
    fn defaults_and_happy_path() {
        let parsed = parse_filters(&[]).unwrap();
        assert_eq!(parsed.page, 1);
        assert_eq!(parsed.per_page, 20);

        let parsed = parse_filters(&pairs(&[
            ("q", " squat "),
            ("movement", "squat-type"),
            ("movement", "hinge-type"),
            ("min_load", "225"),
            ("max_effort", "8.5"),
            ("per_page", "40"),
            ("page", "2"),
        ]))
        .unwrap();
        assert_eq!(parsed.q.as_deref(), Some("squat"), "q is trimmed");
        assert_eq!(parsed.movement, vec!["squat-type", "hinge-type"]);
        assert_eq!(parsed.min_load, Some(225_000), "thousandths");
        assert_eq!(parsed.max_effort, Some(850), "hundredths");
        assert_eq!(parsed.per_page, 40);
        assert_eq!(parsed.page, 2);
    }

    #[test]
    fn error_messages_match_the_fixtures_verbatim() {
        assert_eq!(err(&[("bogus", "1")]), "unknown filter: bogus");
        assert_eq!(err(&[("page", "0")]), "page must be between 1 and 100000");
        assert_eq!(
            err(&[("page", "1"), ("page", "2")]),
            "page may appear only once",
        );
        assert_eq!(err(&[("per_page", "15")]), "per_page must be 10, 20, or 40");
        assert_eq!(
            err(&[("per_page", "50")]),
            "per_page must be between 10 and 40",
        );
        assert_eq!(
            err(&[("from", "2026-02-30")]),
            "from must be a real YYYY-MM-DD date",
        );
        assert_eq!(
            err(&[("from", "2025-01-01"), ("to", "2024-01-01")]),
            "from must not exceed to",
        );
        assert_eq!(
            err(&[("weekday", "monday")]),
            "weekday must be sun, mon, tue, wed, thu, fri, or sat",
        );
        assert_eq!(
            err(&[("time_of_day", "noon")]),
            "time_of_day must be morning, afternoon, evening, or night",
        );
        assert_eq!(
            err(&[("q", &"a".repeat(101))]),
            "q must be 1-100 text characters",
        );
        assert_eq!(
            err(&[("q", &"💪".repeat(13))]),
            "q is too long after escaping (50-byte pattern limit)",
        );
        assert_eq!(err(&[("movement", "SQUAT")]), "bad movement value");
        assert_eq!(
            err(&[
                ("movement", "a"),
                ("movement", "b"),
                ("movement", "c"),
                ("movement", "d"),
                ("movement", "e"),
                ("movement", "f"),
                ("movement", "g"),
                ("movement", "h"),
                ("movement", "i"),
            ]),
            "movement may appear at most 8 times",
        );
        assert_eq!(
            err(&[("movement", "squat-type"), ("movement", "squat-type")]),
            "duplicate movement value: squat-type",
        );
        assert_eq!(
            err(&[("exercise", "a"), ("exercise", "b")]),
            "exercise may appear only once",
        );
        assert_eq!(err(&[("set_type", "working")]), "bad set_type value");
        assert_eq!(
            err(&[("min_load", "abc")]),
            "min_load must be a non-negative decimal with at most 3 places",
        );
        assert_eq!(err(&[("min_reps", "abc")]), "min_reps must be an integer");
        assert_eq!(
            err(&[("min_load", "300"), ("max_load", "200")]),
            "min_load must not exceed max_load",
        );
        assert_eq!(
            err(&[("max_effort", "8.555")]),
            "max_effort must be a non-negative decimal with at most 2 places",
        );
        assert_eq!(
            err(&[("has_record", "yes")]),
            "has_record must be true or false",
        );
        assert_eq!(
            err(&[("duration", "weird")]),
            "duration must be normal or suspicious",
        );
    }

    #[test]
    fn first_unknown_key_wins_in_query_order() {
        assert_eq!(
            err(&[("zzz", "1"), ("aaa", "2")]),
            "unknown filter: zzz",
            "insertion order, not alphabetical",
        );
        // An unknown key is reported even when a known key is also invalid.
        assert_eq!(
            err(&[("bogus", "1"), ("page", "0")]),
            "unknown filter: bogus"
        );
    }

    #[test]
    fn scaled_decimal_mirrors_the_workers_regex() {
        let load = |v: &str| parse_filters(&pairs(&[("min_load", v)]));
        assert_eq!(load("0").unwrap().min_load, Some(0));
        assert_eq!(load("0.5").unwrap().min_load, Some(500));
        assert_eq!(load("225.125").unwrap().min_load, Some(225_125));
        assert!(load("01").is_err(), "leading zero");
        assert!(load(".5").is_err(), "bare fraction");
        assert!(load("5.").is_err(), "trailing dot");
        assert!(load("-1").is_err());
        assert!(load("1000000000").is_err(), "ten digits fails the regex");
    }

    #[test]
    fn scaled_values_respect_max_scaled() {
        assert_eq!(
            err(&[("min_load", "999999999")]),
            "min_load must be a non-negative decimal with at most 3 places",
        );
        assert_eq!(
            parse_filters(&pairs(&[("min_load", "1000000")]))
                .unwrap()
                .min_load,
            Some(1_000_000_000),
            "exactly max_scaled passes",
        );
        assert_eq!(
            err(&[("max_effort", "1000.01")]),
            "max_effort must be a non-negative decimal with at most 2 places",
        );
    }

    #[test]
    fn q_pattern_length_counts_escapes_and_wrapping_percents() {
        // 48 ASCII chars + %% wrapper = 50 bytes: passes.
        assert!(parse_filters(&pairs(&[("q", &"a".repeat(48))])).is_ok());
        // 48 chars with one escapable char = 48 + 1 escape + 2 = 51 bytes.
        let term = format!("{}%", "a".repeat(47));
        assert_eq!(
            err(&[("q", &term)]),
            "q is too long after escaping (50-byte pattern limit)",
        );
    }

    #[test]
    fn time_of_day_hour_bands() {
        assert!(TimeOfDay::Night.contains_hour(23));
        assert!(TimeOfDay::Night.contains_hour(4));
        assert!(!TimeOfDay::Night.contains_hour(5));
        assert!(TimeOfDay::Morning.contains_hour(5));
        assert!(TimeOfDay::Afternoon.contains_hour(12));
        assert!(TimeOfDay::Evening.contains_hour(20));
        assert!(!TimeOfDay::Evening.contains_hour(21));
    }

    #[test]
    fn only_universal_activity_filters_admit_runs() {
        let universal = parse_filters(&pairs(&[
            ("from", "2026-08-01"),
            ("to", "2026-08-31"),
            ("time_of_day", "morning"),
            ("weekday", "fri"),
            ("page", "2"),
            ("per_page", "10"),
        ]))
        .unwrap();
        assert!(universal.admits_runs());

        for pair in [
            ("q", "run"),
            ("movement", "cardio"),
            ("exercise", "Squat"),
            ("duration", "normal"),
            ("has_notes", "false"),
        ] {
            assert!(
                !parse_filters(&pairs(&[pair])).unwrap().admits_runs(),
                "{pair:?} is lifting-specific"
            );
        }
    }
}
