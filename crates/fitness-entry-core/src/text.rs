use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{MAX_EFFORT_HUNDREDTHS, MAX_WEIGHT_MILLI, MIN_EFFORT_HUNDREDTHS};

/// The five structural kinds a set can have. Failure is deliberately absent:
/// it is an effort endpoint, represented by `failure` beside numeric RPE.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub enum SetType {
    #[serde(rename = "WARMUP_SET")]
    Warmup,
    #[default]
    #[serde(rename = "NORMAL_SET")]
    Normal,
    #[serde(rename = "PARTIAL_REPS_SET")]
    PartialReps,
    #[serde(rename = "DROP_SET")]
    Drop,
    #[serde(rename = "NEGATIVE_REPS_SET")]
    NegativeReps,
}

impl SetType {
    pub const ALL: [Self; 5] = [
        Self::Warmup,
        Self::Normal,
        Self::PartialReps,
        Self::Drop,
        Self::NegativeReps,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warmup => "WARMUP_SET",
            Self::Normal => "NORMAL_SET",
            Self::PartialReps => "PARTIAL_REPS_SET",
            Self::Drop => "DROP_SET",
            Self::NegativeReps => "NEGATIVE_REPS_SET",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Warmup => "WARM",
            Self::Normal => "WORK",
            Self::PartialReps => "PART",
            Self::Drop => "DROP",
            Self::NegativeReps => "NEG",
        }
    }

    pub const fn spoken_label(self) -> &'static str {
        match self {
            Self::Warmup => "warm-up set",
            Self::Normal => "working set",
            Self::PartialReps => "partial-reps set",
            Self::Drop => "drop set",
            Self::NegativeReps => "negative-reps set",
        }
    }

    pub const fn kind(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Normal => "working",
            Self::PartialReps => "partial",
            Self::Drop => "drop",
            Self::NegativeReps => "negative",
        }
    }
}

impl FromStr for SetType {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "WARMUP_SET" => Ok(Self::Warmup),
            "NORMAL_SET" => Ok(Self::Normal),
            "PARTIAL_REPS_SET" => Ok(Self::PartialReps),
            "DROP_SET" => Ok(Self::Drop),
            "NEGATIVE_REPS_SET" => Ok(Self::NegativeReps),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for SetType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

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
    value.parse::<SetType>().is_ok()
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
    fn set_type_owns_its_wire_value_and_presentational_kind() {
        let cases = [
            (SetType::Warmup, "WARMUP_SET", "WARM", "warmup"),
            (SetType::Normal, "NORMAL_SET", "WORK", "working"),
            (SetType::PartialReps, "PARTIAL_REPS_SET", "PART", "partial"),
            (SetType::Drop, "DROP_SET", "DROP", "drop"),
            (
                SetType::NegativeReps,
                "NEGATIVE_REPS_SET",
                "NEG",
                "negative",
            ),
        ];
        for (set_type, wire, label, kind) in cases {
            assert_eq!(set_type.as_str(), wire);
            assert_eq!(wire.parse::<SetType>(), Ok(set_type));
            assert_eq!(set_type.short_label(), label);
            assert_eq!(set_type.kind(), kind);
            assert_eq!(
                serde_json::to_string(&set_type).unwrap(),
                format!("\"{wire}\"")
            );
        }
        assert!("FAILURE_SET".parse::<SetType>().is_err());
    }

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
