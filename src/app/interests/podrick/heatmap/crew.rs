//! Crew-wide kwerm / asynkwerm totals for the selected year.

use super::YearWindow;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CrewTotals {
    pub kwerms: usize,
    pub am_kwerms: usize,
    pub pm_kwerms: usize,
    pub kwerm_days: usize,
    pub asynkwerms: usize,
}

impl CrewTotals {
    pub fn query(window: &YearWindow<'_>) -> Self {
        let Some((start, end)) = window.scored_range() else {
            return Self::default();
        };
        window
            .by_date
            .range(start..=end)
            .fold(Self::default(), |mut totals, (_, day)| {
                totals.am_kwerms += usize::from(day.kwerm_am);
                totals.pm_kwerms += usize::from(day.kwerm_pm);
                totals.kwerms += usize::from(day.kwerm_count());
                totals.kwerm_days += usize::from(day.kwerm_count() > 0);
                totals.asynkwerms += usize::from(day.asynkwerm);
                totals
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::podrick::heatmap::fixtures::{message, window_from};
    use jiff::civil::Date;

    #[test]
    fn kwerms_are_counted_per_slot_and_asynkwerms_per_day() {
        let messages = [
            message("101", 0, "2026-01-01T11:07:00Z"),
            message("102", 0, "2026-01-01T23:07:00Z"),
            message("201", 1, "2026-01-01T11:07:01Z"),
            message("202", 1, "2026-01-01T23:07:01Z"),
            message("301", 2, "2026-01-01T11:07:02Z"),
            message("302", 2, "2026-01-01T23:07:02Z"),
            message("111", 0, "2026-01-02T11:07:00Z"),
            message("211", 1, "2026-01-02T23:07:01Z"),
            message("311", 2, "2026-01-02T23:07:02Z"),
        ];
        let start = Date::new(2026, 1, 1).unwrap();
        let owned = window_from(
            &messages,
            start,
            Date::new(2026, 12, 31).unwrap(),
            start,
            Date::new(2026, 7, 28).unwrap(),
            2026,
        );
        let totals = CrewTotals::query(&owned.window());
        assert_eq!(totals.kwerms, 2);
        assert_eq!(totals.am_kwerms, 1);
        assert_eq!(totals.pm_kwerms, 1);
        assert_eq!(totals.kwerm_days, 1);
        assert_eq!(totals.asynkwerms, 1);
    }
}
