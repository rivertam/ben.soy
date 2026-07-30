//! Shared date and copy helpers for the Pants Off calendars.

use jiff::{ToSpan, civil::Date};

pub(super) const DAYS_PER_WEEK: usize = 7;

pub(super) const NOTE: &str = "font-meta text-[0.7rem] leading-[1.55] text-muted";
pub(super) const META: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";

pub(super) fn inclusive_dates(start: Date, end: Date) -> Option<Vec<Date>> {
    if start > end {
        return Some(Vec::new());
    }
    let mut dates = Vec::new();
    let mut date = start;
    loop {
        dates.push(date);
        if date == end {
            break;
        }
        date = date.checked_add(1.days()).ok()?;
    }
    Some(dates)
}

pub(super) fn format_short(date: Date) -> String {
    date.strftime("%b %-d, %Y").to_string()
}

pub(super) fn format_long(date: Date) -> String {
    date.strftime("%A, %b %-d, %Y").to_string()
}

pub(super) fn format_compact(date: Date) -> String {
    date.strftime("%b %-d").to_string()
}

pub(super) const fn plural<'a>(value: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if value == 1 { singular } else { plural }
}

/// Competition rank label for a sorted position. Zeroes stay unranked.
pub(super) fn competition_rank(
    position: usize,
    value: usize,
    tied: bool,
    previous_value: &mut Option<usize>,
    rank: &mut usize,
) -> String {
    if previous_value
        .map(|previous| previous != value)
        .unwrap_or(true)
    {
        *rank = position + 1;
        *previous_value = Some(value);
    }
    if value == 0 {
        "—".to_string()
    } else if tied {
        format!("T{rank}")
    } else {
        rank.to_string()
    }
}
