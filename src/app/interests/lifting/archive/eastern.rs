//! America/New_York projection of UTC source instants.
//!
//! Port of the Worker's `easternInstant`/`publicWorkoutPath`/
//! `parsePublicWorkoutPath` (`fitness.ts`, since deleted). The source of truth
//! for a workout is its UTC wall-clock string (`YYYY-MM-DD HH:MM:SS`,
//! offset-less, always UTC); every reader-facing date, filter, calendar
//! bucket, and permanent URL uses the Eastern projection computed here.
//! DST is delegated entirely to the IANA database (jiff, bundled tzdb —
//! the production container image has no /usr/share/zoneinfo).

use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

/// An Eastern wall-clock projection of a UTC instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EasternInstant {
    /// `YYYY-MM-DD HH:MM:SS` in America/New_York.
    pub local: String,
    /// UTC offset in minutes at that instant: -240 (EDT) or -300 (EST).
    pub offset_minutes: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidTimestamp(pub String);

impl std::fmt::Display for InvalidTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid UTC timestamp: {}", self.0)
    }
}

impl std::error::Error for InvalidTimestamp {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidEasternTimestamp(pub String);

impl std::fmt::Display for InvalidEasternTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
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

/// Parse a `YYYY-MM-DD HH:MM:SS` UTC wall-clock string into an instant.
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

/// Resolve an America/New_York wall clock to its canonical UTC source
/// string. Lyfta's share text omits an offset, so DST gaps and folds are
/// rejected instead of silently choosing an instant.
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

/// Project a UTC source string (plus optional seconds, for workout ends)
/// into the Eastern wall clock.
pub fn eastern_instant(utc: &str, add_seconds: i64) -> Result<EasternInstant, InvalidTimestamp> {
    let start = utc_timestamp(utc)?;
    let instant = Timestamp::from_second(start.as_second() + add_seconds)
        .map_err(|_| InvalidTimestamp(utc.to_string()))?;
    Ok(project(instant))
}

fn project(instant: Timestamp) -> EasternInstant {
    let zoned = instant.to_zoned(eastern_tz().clone());
    let local = zoned.strftime("%Y-%m-%d %H:%M:%S").to_string();
    let offset_minutes = zoned.offset().seconds() / 60;
    EasternInstant {
        local,
        offset_minutes,
    }
}

/// The canonical public path segment for a workout start:
/// `YYYY-MM-DDThh-mm-ss±HH-MM` (Eastern local plus explicit offset, so the
/// repeated fall-DST hour stays distinct).
pub fn public_path(instant: &EasternInstant) -> String {
    let stamp = instant.local.replacen(' ', "T", 1).replace(':', "-");
    let sign = if instant.offset_minutes < 0 { '-' } else { '+' };
    let magnitude = instant.offset_minutes.unsigned_abs();
    format!("{stamp}{sign}{:02}-{:02}", magnitude / 60, magnitude % 60)
}

/// Parse a public path segment back into its Eastern local string and
/// offset. Returns `None` for anything but a well-formed, really-existing
/// datetime with an Eastern offset (-240 or -300) — mirrors
/// `parsePublicWorkoutPath`, where every rejection is a 404.
pub fn parse_public_path(segment: &str) -> Option<EasternInstant> {
    let bytes = segment.as_bytes();
    // YYYY-MM-DDTHH-MM-SS±HH-MM = 10 + 1 + 8 + 6 = 25 bytes.
    if bytes.len() != 25 || bytes[10] != b'T' {
        return None;
    }
    let sign = match bytes[19] {
        b'-' => -1i32,
        b'+' => 1i32,
        _ => return None,
    };
    let date = &segment[..10];
    let time = segment[11..19].replace('-', ":");
    let local = format!("{date} {time}");
    if !is_plain_datetime_shape(&local) || DateTime::strptime("%Y-%m-%d %H:%M:%S", &local).is_err()
    {
        return None;
    }
    let hours: i32 = segment[20..22].parse().ok()?;
    let minutes: i32 = segment[23..25].parse().ok()?;
    if bytes[22] != b'-' {
        return None;
    }
    let offset_minutes = sign * (hours * 60 + minutes);
    if offset_minutes != -240 && offset_minutes != -300 {
        return None;
    }
    Some(EasternInstant {
        local,
        offset_minutes,
    })
}

/// Strict `YYYY-MM-DD HH:MM:SS` shape check (digits and separators in exact
/// positions), so `strptime` leniency can never widen the accepted format.
fn is_plain_datetime_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let ok = match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        };
        if !ok {
            return false;
        }
    }
    true
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
    fn every_production_workout_projects_identically_to_the_worker() {
        // The full D1 dump of Worker-computed projections is the contract.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/d1/workout_triples.json"
        );
        let raw = std::fs::read_to_string(path).expect("run tests/fixtures/capture.sh first");
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let rows = parsed[0]["results"].as_array().unwrap();
        assert_eq!(rows.len(), 360, "expected the full production corpus");
        for row in rows {
            let utc = row["started_at_utc"].as_str().unwrap();
            let expected_local = row["started_at_local"].as_str().unwrap();
            let expected_offset = row["eastern_offset_minutes"].as_i64().unwrap() as i32;
            let projected = eastern_instant(utc, 0).unwrap();
            assert_eq!(projected.local, expected_local, "utc {utc}");
            assert_eq!(projected.offset_minutes, expected_offset, "utc {utc}");
        }
    }

    #[test]
    fn spring_forward_gap() {
        // 2026-03-08: 02:00 EST -> 03:00 EDT.
        assert_eq!(
            eastern_instant("2026-03-08 06:59:00", 0).unwrap(),
            instant("2026-03-08 01:59:00", -300),
        );
        assert_eq!(
            eastern_instant("2026-03-08 07:00:00", 0).unwrap(),
            instant("2026-03-08 03:00:00", -240),
        );
    }

    #[test]
    fn fall_back_repeated_hour_distinguished_by_offset() {
        // 2025-11-02: 02:00 EDT -> 01:00 EST; 01:30 happens twice.
        let first = eastern_instant("2025-11-02 05:30:00", 0).unwrap();
        let second = eastern_instant("2025-11-02 06:30:00", 0).unwrap();
        assert_eq!(first, instant("2025-11-02 01:30:00", -240));
        assert_eq!(second, instant("2025-11-02 01:30:00", -300));
        assert_ne!(public_path(&first), public_path(&second));
    }

    #[test]
    fn midnight_renders_as_00() {
        assert_eq!(
            eastern_instant("2026-07-22 04:00:00", 0).unwrap(),
            instant("2026-07-22 00:00:00", -240),
        );
    }

    #[test]
    fn workout_end_can_cross_the_fall_transition() {
        let start = eastern_instant("2025-11-02 05:30:00", 0).unwrap();
        let end = eastern_instant("2025-11-02 05:30:00", 3600).unwrap();
        assert_eq!(start.local, end.local, "same wall clock an hour apart");
        assert_eq!(start.offset_minutes, -240);
        assert_eq!(end.offset_minutes, -300);
    }

    #[test]
    fn public_path_round_trips() {
        let projected = eastern_instant("2026-07-11 00:33:27", 0).unwrap();
        let path = public_path(&projected);
        assert_eq!(path, "2026-07-10T20-33-27-04-00");
        assert_eq!(parse_public_path(&path).unwrap(), projected);
    }

    #[test]
    fn public_path_rejections_are_404s() {
        assert_eq!(parse_public_path("not-a-path"), None);
        // Real shape, non-Eastern offset (mirrors the by_path_bad_offset fixture).
        assert_eq!(parse_public_path("2024-06-01T10-00-00-07-00"), None);
        assert_eq!(parse_public_path("2024-06-01T10-00-00+04-00"), None);
        // Impossible datetime.
        assert_eq!(parse_public_path("2024-02-30T10-00-00-05-00"), None);
        assert_eq!(parse_public_path("2024-06-01T25-00-00-05-00"), None);
    }

    #[test]
    fn utc_parsing_is_strict() {
        assert!(utc_timestamp("2026-07-21 14:39:04").is_ok());
        assert!(utc_timestamp("2026-07-21T14:39:04").is_err());
        assert!(utc_timestamp("2026-7-21 14:39:04").is_err());
        assert!(utc_timestamp("2026-02-30 14:39:04").is_err());
        assert!(utc_timestamp("2026-07-21 14:39").is_err());
        assert!(utc_timestamp("2026-07-21 14:39:04 ").is_err());
    }

    #[test]
    fn eastern_local_time_resolves_to_utc() {
        assert_eq!(
            eastern_local_to_utc("2026-07-24 10:38:00").unwrap(),
            "2026-07-24 14:38:00"
        );
        assert_eq!(
            eastern_local_to_utc("2026-01-24 10:38:00").unwrap(),
            "2026-01-24 15:38:00"
        );
    }

    #[test]
    fn eastern_local_time_rejects_dst_gap_and_fold() {
        assert!(eastern_local_to_utc("2026-03-08 02:30:00").is_err());
        assert!(eastern_local_to_utc("2026-11-01 01:30:00").is_err());
    }
}
