//! Display formatting for set values and local workout time.

pub(super) fn format_integer(value: impl Into<u64>) -> String {
    let digits = value.into().to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

pub(super) fn format_scaled(value: u64, scale: u64) -> String {
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
    output
}

pub(super) fn format_signed_scaled(value: i64, scale: u64) -> String {
    let formatted = format_scaled(value.unsigned_abs(), scale);
    if value < 0 {
        format!("-{formatted}")
    } else {
        formatted
    }
}

pub(super) fn format_duration(seconds: u64) -> String {
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

pub(super) struct Timing {
    pub(super) date: String,
    pub(super) range: String,
}

/// Format a workout interval from the Eastern-local endpoints projected by the
/// Worker. In particular, do not derive an end wall-clock time by adding the
/// duration: that would get both DST transitions wrong.
pub(super) fn workout_timing(
    started_at_local: &str,
    ended_at_local: &str,
    eastern_offset_minutes: i32,
    end_eastern_offset_minutes: i32,
) -> Timing {
    let Some(start) = LocalDateTime::parse(started_at_local) else {
        return Timing {
            date: if started_at_local.is_empty() {
                "unknown date".to_string()
            } else {
                started_at_local.to_string()
            },
            range: "time unavailable".to_string(),
        };
    };

    let Some(end) = LocalDateTime::parse(ended_at_local) else {
        return Timing {
            date: start.format_date(),
            range: "time unavailable".to_string(),
        };
    };

    let labels = (eastern_offset_minutes != end_eastern_offset_minutes)
        .then(|| {
            (
                eastern_zone_name(eastern_offset_minutes),
                eastern_zone_name(end_eastern_offset_minutes),
            )
        })
        .filter(|(start, end)| start.is_some() && end.is_some());
    let start_time = format_time_with_zone(start, labels.and_then(|(start, _)| start));
    let end_time = format_time_with_zone(end, labels.and_then(|(_, end)| end));
    let range = if start.same_date(end) {
        format!("{start_time}–{end_time}")
    } else {
        format!("{start_time}–{} {end_time}", end.format_date(),)
    };
    Timing {
        date: start.format_date(),
        range,
    }
}

/// A machine-readable local timestamp which keeps the Eastern UTC offset
/// attached instead of silently presenting it as a timezone-free value.
pub(super) fn workout_datetime(raw: &str, eastern_offset_minutes: i32) -> String {
    let Some(local) = LocalDateTime::parse(raw) else {
        return raw.replace(' ', "T");
    };
    let offset = eastern_offset_minutes.unsigned_abs();
    let sign = if eastern_offset_minutes < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        local.year,
        local.month,
        local.day,
        local.hour,
        local.minute,
        local.second,
        offset / 60,
        offset % 60,
    )
}

fn eastern_zone_name(offset_minutes: i32) -> Option<&'static str> {
    match offset_minutes {
        -240 => Some("EDT"),
        -300 => Some("EST"),
        _ => None,
    }
}

fn format_time_with_zone(time: LocalDateTime, zone: Option<&str>) -> String {
    let time = time.format_time();
    match zone {
        Some(zone) => format!("{time} {zone}"),
        None => time,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalDateTime {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl LocalDateTime {
    fn parse(raw: &str) -> Option<Self> {
        if raw.len() != 19
            || !matches!(raw.as_bytes().get(4), Some(b'-'))
            || !matches!(raw.as_bytes().get(7), Some(b'-'))
            || !matches!(raw.as_bytes().get(10), Some(b' ' | b'T'))
            || !matches!(raw.as_bytes().get(13), Some(b':'))
            || !matches!(raw.as_bytes().get(16), Some(b':'))
        {
            return None;
        }
        let value = Self {
            year: raw[0..4].parse().ok()?,
            month: raw[5..7].parse().ok()?,
            day: raw[8..10].parse().ok()?,
            hour: raw[11..13].parse().ok()?,
            minute: raw[14..16].parse().ok()?,
            second: raw[17..19].parse().ok()?,
        };
        (value.month >= 1
            && value.month <= 12
            && value.day >= 1
            && value.day <= days_in_month(value.year, value.month)
            && value.hour < 24
            && value.minute < 60
            && value.second < 60)
            .then_some(value)
    }

    fn same_date(self, other: Self) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }

    fn format_date(self) -> String {
        format!("{} {}, {}", month_name(self.month), self.day, self.year)
    }

    fn format_time(self) -> String {
        let suffix = if self.hour < 12 { "AM" } else { "PM" };
        let hour = match self.hour % 12 {
            0 => 12,
            hour => hour,
        };
        format!("{hour}:{:02} {suffix}", self.minute)
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

pub(super) fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_formatters_match_the_old_browser_output() {
        assert_eq!(format_integer(5_561_u64), "5,561");
        assert_eq!(format_scaled(102_500, 1_000), "102.5");
        assert_eq!(format_scaled(100_000, 1_000), "100");
        assert_eq!(format_scaled(50, 100), "0.5");
        assert_eq!(format_signed_scaled(-45_125, 1_000), "-45.125");
        assert_eq!(format_signed_scaled(-1_250_000, 1_000), "-1,250");
        assert_eq!(format_duration(7_534), "2h 05m");
    }

    #[test]
    fn worker_projected_end_time_crosses_days_without_local_arithmetic() {
        let timing = workout_timing("2024-02-29 23:45:00", "2024-03-01 01:15:00", -300, -300);
        assert_eq!(timing.date, "Feb 29, 2024");
        assert_eq!(timing.range, "11:45 PM–Mar 1, 2024 1:15 AM");
    }

    #[test]
    fn dst_fall_back_shows_both_eastern_abbreviations() {
        let timing = workout_timing("2026-11-01 01:50:00", "2026-11-01 01:10:00", -240, -300);
        assert_eq!(timing.date, "Nov 1, 2026");
        assert_eq!(timing.range, "1:50 AM EDT–1:10 AM EST");
    }

    #[test]
    fn dst_spring_forward_uses_worker_end_and_offset() {
        let timing = workout_timing("2026-03-08 01:55:00", "2026-03-08 03:05:00", -300, -240);
        assert_eq!(timing.range, "1:55 AM EST–3:05 AM EDT");
    }

    #[test]
    fn machine_time_keeps_its_eastern_offset() {
        assert_eq!(
            workout_datetime("2026-07-11 20:33:27", -240),
            "2026-07-11T20:33:27-04:00"
        );
    }
}
