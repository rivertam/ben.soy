//! Shared America/New_York projection for public, path-shaped timestamps.
//!
//! Both browser-side Rust modules and the native server compile this exact
//! calendar code. The bundled IANA database keeps DST behavior independent
//! of the host image and requires no browser clock access on wasm32.

use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::{AmbiguousOffset, TimeZone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EasternInstant {
    pub local: String,
    pub offset_minutes: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidTimestamp(pub String);

impl std::fmt::Display for InvalidTimestamp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid UTC timestamp: {}", self.0)
    }
}

impl std::error::Error for InvalidTimestamp {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidEasternTimestamp(pub String);

impl std::fmt::Display for InvalidEasternTimestamp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid or ambiguous America/New_York timestamp: {}",
            self.0
        )
    }
}

impl std::error::Error for InvalidEasternTimestamp {}

fn eastern_tz() -> &'static TimeZone {
    static TZ: OnceLock<TimeZone> = OnceLock::new();
    TZ.get_or_init(|| TimeZone::get("America/New_York").expect("bundled tzdb has America/New_York"))
}

pub fn utc_timestamp(utc: &str) -> Result<Timestamp, InvalidTimestamp> {
    if !is_plain_datetime_shape(utc) {
        return Err(InvalidTimestamp(utc.to_string()));
    }
    let civil = DateTime::strptime("%Y-%m-%d %H:%M:%S", utc)
        .map_err(|_| InvalidTimestamp(utc.to_string()))?;
    civil
        .to_zoned(TimeZone::UTC)
        .map(|zoned| zoned.timestamp())
        .map_err(|_| InvalidTimestamp(utc.to_string()))
}

pub fn eastern_local_to_utc(local: &str) -> Result<String, InvalidEasternTimestamp> {
    if !is_plain_datetime_shape(local) {
        return Err(InvalidEasternTimestamp(local.to_string()));
    }
    let civil = DateTime::strptime("%Y-%m-%d %H:%M:%S", local)
        .map_err(|_| InvalidEasternTimestamp(local.to_string()))?;
    let timestamp = eastern_tz()
        .to_ambiguous_timestamp(civil)
        .unambiguous()
        .map_err(|_| InvalidEasternTimestamp(local.to_string()))?;
    Ok(timestamp
        .to_zoned(TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string())
}

pub fn eastern_instant(utc: &str, add_seconds: i64) -> Result<EasternInstant, InvalidTimestamp> {
    let start = utc_timestamp(utc)?;
    let seconds = start
        .as_second()
        .checked_add(add_seconds)
        .ok_or_else(|| InvalidTimestamp(utc.to_string()))?;
    let instant = Timestamp::from_second(seconds).map_err(|_| InvalidTimestamp(utc.to_string()))?;
    Ok(project(instant))
}

pub fn eastern_date(instant: Timestamp) -> Date {
    instant.to_zoned(eastern_tz().clone()).date()
}

fn project(instant: Timestamp) -> EasternInstant {
    let zoned = instant.to_zoned(eastern_tz().clone());
    EasternInstant {
        local: zoned.strftime("%Y-%m-%d %H:%M:%S").to_string(),
        offset_minutes: zoned.offset().seconds() / 60,
    }
}

pub fn public_path(instant: &EasternInstant) -> String {
    let stamp = instant.local.replacen(' ', "T", 1).replace(':', "-");
    let sign = if instant.offset_minutes < 0 { '-' } else { '+' };
    let magnitude = instant.offset_minutes.unsigned_abs();
    format!("{stamp}{sign}{:02}-{:02}", magnitude / 60, magnitude % 60)
}

pub fn parse_public_path(segment: &str) -> Option<EasternInstant> {
    let bytes = segment.as_bytes();
    if bytes.len() != 25 || bytes[10] != b'T' {
        return None;
    }
    let sign = match bytes[19] {
        b'-' => -1i32,
        b'+' => 1i32,
        _ => return None,
    };
    if bytes[22] != b'-' {
        return None;
    }
    let local = format!("{} {}", &segment[..10], segment[11..19].replace('-', ":"));
    if !is_plain_datetime_shape(&local) {
        return None;
    }
    let civil = DateTime::strptime("%Y-%m-%d %H:%M:%S", &local).ok()?;
    let hours: i32 = segment[20..22].parse().ok()?;
    let minutes: i32 = segment[23..25].parse().ok()?;
    let offset_minutes = sign * (hours * 60 + minutes);
    if offset_minutes != -240 && offset_minutes != -300 {
        return None;
    }
    let offset_is_valid = match eastern_tz().to_ambiguous_timestamp(civil).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset.seconds() == offset_minutes * 60,
        AmbiguousOffset::Fold { before, after } => {
            before.seconds() == offset_minutes * 60 || after.seconds() == offset_minutes * 60
        }
        AmbiguousOffset::Gap { .. } => false,
    };
    if !offset_is_valid {
        return None;
    }
    Some(EasternInstant {
        local,
        offset_minutes,
    })
}

fn is_plain_datetime_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(local: &str, offset_minutes: i32) -> EasternInstant {
        EasternInstant {
            local: local.to_string(),
            offset_minutes,
        }
    }

    #[test]
    fn spring_forward_gap_and_fall_fold_use_iana_rules() {
        assert_eq!(
            eastern_instant("2026-03-08 06:59:00", 0).unwrap(),
            instant("2026-03-08 01:59:00", -300)
        );
        assert_eq!(
            eastern_instant("2026-03-08 07:00:00", 0).unwrap(),
            instant("2026-03-08 03:00:00", -240)
        );
        let first = eastern_instant("2025-11-02 05:30:00", 0).unwrap();
        let second = eastern_instant("2025-11-02 06:30:00", 0).unwrap();
        assert_eq!(first.local, second.local);
        assert_ne!(public_path(&first), public_path(&second));
    }

    #[test]
    fn public_paths_are_strict_and_round_trip() {
        let projected = eastern_instant("2026-07-11 00:33:27", 0).unwrap();
        let path = public_path(&projected);
        assert_eq!(path, "2026-07-10T20-33-27-04-00");
        assert_eq!(parse_public_path(&path), Some(projected));
        assert!(parse_public_path("2024-02-30T10-00-00-05-00").is_none());
        assert!(parse_public_path("2024-06-01T10-00-00-07-00").is_none());
        assert!(parse_public_path("2024-06-01T10-00-00-05-00").is_none());
        assert!(parse_public_path("2026-03-08T02-30-00-05-00").is_none());
        assert!(parse_public_path("2026-11-01T01-30-00-04-00").is_some());
        assert!(parse_public_path("2026-11-01T01-30-00-05-00").is_some());
    }

    #[test]
    fn end_projection_crosses_the_fall_transition() {
        let start = eastern_instant("2025-11-02 05:30:00", 0).unwrap();
        let end = eastern_instant("2025-11-02 05:30:00", 3600).unwrap();
        assert_eq!(start.local, end.local);
        assert_eq!((start.offset_minutes, end.offset_minutes), (-240, -300));
    }

    #[test]
    fn local_resolution_rejects_dst_gaps_and_folds() {
        assert!(eastern_local_to_utc("2026-03-08 02:30:00").is_err());
        assert!(eastern_local_to_utc("2026-11-01 01:30:00").is_err());
    }
}
