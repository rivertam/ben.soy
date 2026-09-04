use crate::{MAX_EFFORT_HUNDREDTHS, MAX_WEIGHT_MILLI, MIN_EFFORT_HUNDREDTHS};

pub const SET_TYPES: [&str; 6] = [
    "WARMUP_SET",
    "NORMAL_SET",
    "FAILURE_SET",
    "PARTIAL_REPS_SET",
    "DROP_SET",
    "NEGATIVE_REPS_SET",
];

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn valid_text(text: &str, min: usize, max: usize) -> bool {
    let units = utf16_len(text);
    units >= min
        && units <= max
        && !text.chars().any(|character| {
            matches!(
                character,
                '\u{0000}'..='\u{0008}' | '\u{000b}' | '\u{000c}' | '\u{000e}'..='\u{001f}'
            )
        })
}

pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|character: char| character.is_whitespace() || character == '\u{feff}')
}

pub fn normalize_title(text: &str) -> String {
    let mut normalized = String::new();
    let mut separating = false;
    for character in js_trim(text).chars() {
        if character.is_whitespace() || character == '\u{feff}' {
            separating = !normalized.is_empty();
        } else {
            if separating {
                normalized.push(' ');
                separating = false;
            }
            normalized.push(character);
        }
    }
    normalized
}

pub fn truncate_utf16(text: &str, max: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|character| {
            let width = character.len_utf16();
            if used + width > max {
                false
            } else {
                used += width;
                true
            }
        })
        .collect()
}

pub fn valid_set_type(value: &str) -> bool {
    SET_TYPES.contains(&value)
}

pub fn safe_local_id(value: &str) -> bool {
    (8..=80).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub fn pounds_to_milli(raw: &str) -> Option<i64> {
    let value = js_trim(raw);
    if value.is_empty() {
        return None;
    }
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |rest| (true, rest));
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, ""), |parts| parts);
    if whole.is_empty()
        || whole.len() > 7
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let mut fraction_text = fraction.to_string();
    while fraction_text.len() < 3 {
        fraction_text.push('0');
    }
    let fraction: i64 = if fraction_text.is_empty() {
        0
    } else {
        fraction_text.parse().ok()?
    };
    let magnitude = whole.checked_mul(1_000)?.checked_add(fraction)?;
    if magnitude > MAX_WEIGHT_MILLI {
        return None;
    }
    Some(if negative { -magnitude } else { magnitude })
}

pub fn reps_value(raw: &str) -> Option<u64> {
    let value = js_trim(raw);
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    let reps: u64 = value.parse().ok()?;
    (reps <= crate::MAX_REPS).then_some(reps)
}

pub fn effort_to_hundredths(raw: &str) -> Option<u64> {
    let value = js_trim(raw);
    if value.is_empty() {
        return None;
    }
    let (whole, fraction) = value.split_once('.').map_or((value, ""), |parts| parts);
    if whole.is_empty()
        || whole.len() > 2
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let mut fraction_text = fraction.to_string();
    while fraction_text.len() < 2 {
        fraction_text.push('0');
    }
    let fraction: u64 = if fraction_text.is_empty() {
        0
    } else {
        fraction_text.parse().ok()?
    };
    let value = whole.checked_mul(100)?.checked_add(fraction)?;
    (MIN_EFFORT_HUNDREDTHS..=MAX_EFFORT_HUNDREDTHS)
        .contains(&value)
        .then_some(value)
}

pub fn weight_text(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let absolute = value.unsigned_abs();
    let whole = absolute / 1_000;
    let fraction = absolute % 1_000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{:03}", fraction)
            .trim_end_matches('0')
            .to_string()
    }
}

pub fn hundredths_text(value: u64) -> String {
    let whole = value / 100;
    let fraction = value % 100;
    match fraction {
        0 => whole.to_string(),
        fraction if fraction % 10 == 0 => format!("{whole}.{}", fraction / 10),
        _ => format!("{whole}.{fraction:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_javascript_trim_semantics_are_explicit() {
        assert_eq!(utf16_len("💪"), 2);
        assert!(valid_text("a\tb\nc\rd", 1, 100));
        assert!(!valid_text("a\u{000b}b", 1, 100));
        assert_eq!(js_trim("\u{feff} hi \u{feff}"), "hi");
        assert_eq!(normalize_title("  Lunch\n\t lift  "), "Lunch lift");
    }

    #[test]
    fn scaled_number_parsing_is_exact() {
        assert_eq!(pounds_to_milli("225.5"), Some(225_500));
        assert_eq!(pounds_to_milli("-0.125"), Some(-125));
        assert_eq!(pounds_to_milli(""), None);
        assert_eq!(pounds_to_milli("1.0000"), None);
        assert_eq!(pounds_to_milli("1000000"), Some(1_000_000_000));
        assert_eq!(pounds_to_milli("1000000.001"), None);
        assert_eq!(reps_value("1000000"), Some(1_000_000));
        assert_eq!(reps_value("01"), None);
        assert_eq!(effort_to_hundredths("9.5"), Some(950));
        assert_eq!(effort_to_hundredths("5.99"), None);
        assert_eq!(weight_text(-125), "-0.125");
        assert_eq!(hundredths_text(950), "9.5");
    }
}
