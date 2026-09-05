//! JavaScript-semantics validators shared by the import parser and the
//! filter parser. Lengths are UTF-16 code units (`"💪".length === 2`),
//! integer checks accept integral floats (`5.0`), and text bans the
//! Worker's exact control-character class (tab, CR, and LF stay legal).
//!
//! With D1's STRICT tables and CHECK constraints gone, these checks are
//! the archive's only line of defense — every bound here mirrors both the
//! old `fitness.ts` validation and the old schema's CHECKs.

use serde_json::{Map, Value};

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The Worker's control-character class `[\u0000-\u0008\u000b\u000c\u000e-\u001f]`
/// excludes tab (09), LF (0a), and CR (0d).
fn has_forbidden_control(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c, '\u{0000}'..='\u{0008}' | '\u{000b}' | '\u{000c}' | '\u{000e}'..='\u{001f}')
    })
}

/// The Worker's `validText` on an already-known string.
pub fn valid_text(text: &str, min: usize, max: usize) -> bool {
    let units = utf16_len(text);
    units >= min && units <= max && !has_forbidden_control(text)
}

/// JS `String.prototype.trim`: Unicode whitespace plus ZWNBSP (U+FEFF),
/// which Rust's `str::trim` does not strip.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `typeof v === "string" && validText(v, min, max)` on a JSON field.
pub fn text_value(value: Option<&Value>, min: usize, max: usize) -> Option<&str> {
    let text = value?.as_str()?;
    valid_text(text, min, max).then_some(text)
}

/// Explicit `null`, or a 1..=max string. Absent keys are invalid — they
/// were `undefined` in the Worker, which failed the type check.
#[allow(clippy::option_option)]
pub fn nullable_text_value(value: Option<&Value>, max: usize) -> Option<Option<String>> {
    match value {
        Some(Value::Null) => Some(None),
        Some(Value::String(text)) if valid_text(text, 1, max) => Some(Some(text.clone())),
        _ => None,
    }
}

/// `typeof v === "number" && Number.isInteger(v) && min <= v <= max`,
/// including JSON floats with integral values.
pub fn integer_value(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    let number = value?.as_number()?;
    if let Some(integer) = number.as_i64() {
        return (min..=max).contains(&integer).then_some(integer);
    }
    let float = number.as_f64()?;
    if float.fract() == 0.0 && float >= min as f64 && float <= max as f64 {
        return Some(float as i64);
    }
    None
}

#[allow(clippy::option_option)]
pub fn nullable_integer_value(value: Option<&Value>, min: i64, max: i64) -> Option<Option<i64>> {
    match value {
        Some(Value::Null) => Some(None),
        other => integer_value(other, min, max).map(Some),
    }
}

pub fn bool_value(value: Option<&Value>) -> Option<bool> {
    value?.as_bool()
}

/// `Object.keys(value).length === keys.length && keys.every(hasOwn)`.
pub fn has_only_keys(map: &Map<String, Value>, keys: &[&str]) -> bool {
    map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key))
}

/// `/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/`
pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && rest.len() <= 127
        && rest
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// `/^[a-z0-9][a-z0-9-]{0,63}$/`
pub fn valid_tag_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest.len() <= 63
        && rest
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

pub fn valid_set_type(value: &str) -> bool {
    value.parse::<fitness_entry_core::SetType>().is_ok()
}

pub const TAG_KINDS: [&str; 3] = ["movement", "muscle", "equipment"];

pub fn valid_tag_kind(value: &str) -> bool {
    TAG_KINDS.contains(&value)
}

/// The Worker's `validDate`: `YYYY-MM-DD` shape plus a `Date.UTC` round
/// trip. That round trip makes years 0000–0099 invalid (JS maps them to
/// 1900+y), so the check is: real calendar date with year >= 100.
pub fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits_ok = bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !digits_ok {
        return false;
    }
    let year: i16 = value[..4].parse().unwrap_or(0);
    let month: i8 = value[5..7].parse().unwrap_or(0);
    let day: i8 = value[8..10].parse().unwrap_or(0);
    year >= 100 && jiff::civil::Date::new(year, month, day).is_ok()
}

/// The Worker's `validLocalDateTime`: `YYYY-MM-DD HH:MM:SS` with a real
/// date and in-range clock fields.
pub fn valid_local_datetime(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 19 || bytes[10] != b' ' || bytes[13] != b':' || bytes[16] != b':' {
        return false;
    }
    if !valid_date(&value[..10]) {
        return false;
    }
    let clock_digits = [11, 12, 14, 15, 17, 18]
        .iter()
        .all(|&index| bytes[index].is_ascii_digit());
    if !clock_digits {
        return false;
    }
    let hours: u8 = value[11..13].parse().unwrap_or(99);
    let minutes: u8 = value[14..16].parse().unwrap_or(99);
    let seconds: u8 = value[17..19].parse().unwrap_or(99);
    hours <= 23 && minutes <= 59 && seconds <= 59
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_lengths_are_utf16_units() {
        assert!(valid_text("💪", 1, 2));
        assert!(!valid_text("💪", 1, 1), "surrogate pair counts as 2");
        assert!(valid_text("a\tb\nc\rd", 1, 100), "tab, LF, CR are legal");
        assert!(!valid_text("a\u{0000}b", 1, 100));
        assert!(!valid_text("a\u{000b}b", 1, 100));
        assert!(!valid_text("a\u{001f}b", 1, 100));
    }

    #[test]
    fn js_trim_strips_zwnbsp() {
        assert_eq!(js_trim("\u{feff} hi \u{feff}"), "hi");
    }

    #[test]
    fn integral_floats_count_as_integers() {
        assert_eq!(integer_value(Some(&serde_json::json!(5.0)), 0, 10), Some(5));
        assert_eq!(integer_value(Some(&serde_json::json!(5.5)), 0, 10), None);
        assert_eq!(integer_value(Some(&serde_json::json!("5")), 0, 10), None);
        assert_eq!(integer_value(Some(&serde_json::json!(11)), 0, 10), None);
        assert_eq!(integer_value(None, 0, 10), None);
    }

    #[test]
    fn id_and_tag_patterns() {
        assert!(valid_id("fitness:2026-07-21T14:39:04:0001"));
        assert!(!valid_id(""));
        assert!(!valid_id(":leading"));
        assert!(!valid_id(&"a".repeat(129)));
        assert!(valid_id(&"a".repeat(128)));
        assert!(valid_tag_value("squat-type"));
        assert!(!valid_tag_value("Squat"));
        assert!(!valid_tag_value("-leading"));
    }

    #[test]
    fn dates_match_the_workers_date_utc_round_trip() {
        assert!(valid_date("2026-02-28"));
        assert!(!valid_date("2026-02-30"));
        assert!(!valid_date("2026-2-28"));
        // Date.UTC(99, …) means 1999 in JS, so the round trip fails there.
        assert!(!valid_date("0099-01-01"));
        assert!(valid_date("0100-01-01"));
    }

    #[test]
    fn local_datetimes_check_the_clock() {
        assert!(valid_local_datetime("2026-07-21 14:39:04"));
        assert!(!valid_local_datetime("2026-07-21T14:39:04"));
        assert!(!valid_local_datetime("2026-07-21 24:00:00"));
        assert!(!valid_local_datetime("2026-07-21 14:60:00"));
    }
}
