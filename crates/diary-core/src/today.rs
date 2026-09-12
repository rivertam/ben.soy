//! Daily reflection policy. Civil days start at 04:00 America/New_York.
//! Today autosaves to the live server; Now retains its offline outbox.
use jiff::{Timestamp, civil::Date, tz::TimeZone};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

pub const BUDGET_MS: u32 = 15 * 60 * 1000;
pub const IDLE_MS: u32 = 5000;
pub const ZONE: &str = "America/New_York";

pub fn day_at(second: i64) -> Option<String> {
    let zoned = Timestamp::from_second(second)
        .ok()?
        .to_zoned(TimeZone::get(ZONE).ok()?);
    let date = if zoned.hour() < 4 {
        zoned.date().yesterday().ok()?
    } else {
        zoned.date()
    };
    Some(date.to_string())
}

pub fn valid_day(day: &str) -> bool {
    day.len() == 10
        && day
            .parse::<Date>()
            .is_ok_and(|date| date.to_string() == day)
}

pub fn day_end(day: &str) -> Option<i64> {
    let next = day.parse::<Date>().ok()?.tomorrow().ok()?;
    Some(
        next.at(4, 0, 0, 0)
            .to_zoned(TimeZone::get(ZONE).ok()?)
            .ok()?
            .timestamp()
            .as_second(),
    )
}

pub fn calendar(year: i16) -> Vec<String> {
    let Ok(mut date) = Date::new(year, 1, 1) else {
        return Vec::new();
    };
    let mut dates = Vec::new();
    while date.year() == year {
        dates.push(date.to_string());
        let Ok(next) = date.tomorrow() else { break };
        date = next;
    }
    dates
}

/// Shared by server rendering and the device mirror. Backdated Now entries
/// belong to their event day; deleted entries never become memory cues.
pub fn memory_cues<T>(
    entries: impl IntoIterator<Item = T>,
    day: &str,
    entry: impl Fn(&T) -> &crate::entry::DiaryEntry,
) -> Vec<T> {
    // Preserve the caller's wrapper: device cues must keep pending/failed
    // status rather than turn predicted links into server-confirmed links.
    let mut cues: Vec<_> = entries
        .into_iter()
        .filter(|item| {
            let entry = entry(item);
            !entry.deleted() && day_at(entry.occurred_at()).as_deref() == Some(day)
        })
        .collect();
    cues.sort_by(|a, b| {
        let (a, b) = (entry(a), entry(b));
        (a.occurred_at(), &a.id).cmp(&(b.occurred_at(), &b.id))
    });
    cues
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, SurrealValue)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub day: String,
    pub body: String,
    pub used_ms: u32,
    pub closed: bool,
    pub closed_at: Option<i64>,
    pub updated_at: i64,
    pub revision: u64,
}

impl Day {
    pub fn empty(day: String, now: i64) -> Self {
        Self {
            day,
            body: String::new(),
            used_ms: 0,
            closed: false,
            closed_at: None,
            updated_at: now,
            revision: 0,
        }
    }

    pub fn status(&self) -> &'static str {
        if self.closed { "closed" } else { "started" }
    }

    pub fn remaining_ms(&self) -> u32 {
        BUDGET_MS.saturating_sub(self.used_ms)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_day(&self.day)
            || self.used_ms > BUDGET_MS
            || self.body.chars().count() > crate::entry::MAX_ENTRY_CHARS
            || self.closed != self.closed_at.is_some()
            || (self.used_ms == BUDGET_MS && !self.closed)
            || self.revision == 0
        {
            return Err("invalid daily reflection".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_epoch: u16,
    pub days: Vec<Day>,
    pub emoji_usage: Vec<crate::emoji_usage::Usage>,
}

/// Monotonic interaction clock. Advancing after a delayed/background tick
/// charges only up to the last interaction plus the five-second idle grace.
#[derive(Clone, Debug, Default)]
pub struct ActiveClock {
    checkpoint: f64,
    active_until: Option<f64>,
}

impl ActiveClock {
    pub fn advance(&mut self, now_ms: f64) -> f64 {
        if !now_ms.is_finite() || now_ms < self.checkpoint {
            return 0.0;
        }
        let end = self
            .active_until
            .map(|end| end.min(now_ms))
            .unwrap_or(self.checkpoint);
        let debit = (end - self.checkpoint).max(0.0);
        self.checkpoint = now_ms;
        if self.active_until.is_some_and(|end| now_ms >= end) {
            self.active_until = None;
        }
        debit
    }
    pub fn interact(&mut self, now_ms: f64) -> f64 {
        let debit = self.advance(now_ms);
        if now_ms.is_finite() {
            self.active_until = Some(now_ms + f64::from(IDLE_MS));
        }
        debit
    }
    pub fn pause(&mut self, now_ms: f64) -> f64 {
        let debit = self.advance(now_ms);
        self.active_until = None;
        debit
    }
    pub fn active(&self) -> bool {
        self.active_until.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn epoch(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_second()
    }
    #[test]
    fn day_boundary_follows_local_civil_time_including_dst() {
        for (at, day) in [
            ("2026-09-10T07:59:59Z", "2026-09-09"),
            ("2026-09-10T08:00:00Z", "2026-09-10"),
            ("2026-03-08T07:30:00Z", "2026-03-07"),
            ("2026-03-08T08:00:00Z", "2026-03-08"),
            ("2026-11-01T08:59:59Z", "2026-10-31"),
            ("2026-11-01T09:00:00Z", "2026-11-01"),
        ] {
            assert_eq!(day_at(epoch(at)).as_deref(), Some(day));
        }
        assert_eq!(day_end("2026-03-07"), Some(epoch("2026-03-08T08:00:00Z")));
        assert_eq!(calendar(2024).len(), 366);
    }
    #[test]
    fn clock_pauses_at_idle_cutoff_and_immediately_on_blur() {
        let mut clock = ActiveClock::default();
        assert_eq!(clock.interact(100.0), 0.0);
        assert_eq!(clock.advance(1100.0), 1000.0);
        assert_eq!(clock.advance(60_000.0), 4000.0);
        assert!(!clock.active());
        assert_eq!(clock.interact(90_000.0), 0.0);
        assert_eq!(clock.pause(90_123.0), 123.0);
        assert_eq!(clock.advance(900_000.0), 0.0);
        assert_eq!(clock.interact(1_000_000.0), 0.0);
        assert_eq!(clock.interact(1_000_500.0), 500.0);
        assert_eq!(clock.advance(1_006_000.0), 5000.0);
    }
    #[test]
    fn memory_cues_follow_event_day_and_keep_their_local_status() {
        use crate::entry::DiaryEntry;
        let mut backdated =
            DiaryEntry::from_parts("later-id", epoch("2026-09-11T12:00:00Z"), "backdated");
        backdated.occurred_at = Some(epoch("2026-09-10T10:00:00Z"));
        let same_day =
            DiaryEntry::from_parts("earlier-id", epoch("2026-09-10T11:00:00Z"), "same day");
        let before_cutoff = DiaryEntry::from_parts(
            "previous-day",
            epoch("2026-09-10T07:59:59Z"),
            "previous day",
        );
        let cues = memory_cues(
            vec![
                (same_day, "synced"),
                (before_cutoff, "synced"),
                (backdated, "pending"),
            ],
            "2026-09-10",
            |item| &item.0,
        );
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].0.body, "backdated");
        assert_eq!(cues[0].1, "pending");
        assert_eq!(cues[1].0.body, "same day");
    }

    #[test]
    fn completed_credit_is_independent_of_duration_or_length() {
        let mut day = Day::empty("2026-09-10".into(), 1);
        day.revision = 1;
        assert_eq!(day.status(), "started");
        day.closed = true;
        day.closed_at = Some(2);
        assert_eq!(day.status(), "closed");
        assert!(day.validate().is_ok());
        day.used_ms = BUDGET_MS;
        assert_eq!(day.status(), "closed");
        assert!(day.validate().is_ok());
        day.closed = false;
        assert!(day.validate().is_err());
    }
}
