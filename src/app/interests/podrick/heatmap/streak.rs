//! Longest claim-streak yearbook board.

use benjisponge::data::podrick_models::PANTS_PARTICIPANTS;
use jiff::{ToSpan, civil::Date};

use super::{
    BoardCard, BoardEntry, YearWindow,
    util::{competition_rank, format_compact, plural},
};

#[derive(Clone, Copy, Debug, Default)]
struct StreakRow {
    participant_index: usize,
    longest_streak: usize,
    streak_start: Option<Date>,
    streak_end: Option<Date>,
}

pub(super) fn board(window: &YearWindow<'_>) -> BoardCard {
    let mut rows = std::array::from_fn(|participant_index| StreakRow {
        participant_index,
        ..StreakRow::default()
    });
    if let Some((start, end)) = window.scored_range() {
        let mut date = start;
        let mut current_streaks = [0_usize; 3];
        let mut current_starts = [None; 3];
        loop {
            for (participant_index, row) in rows.iter_mut().enumerate() {
                let claims = window
                    .by_date
                    .get(&date)
                    .map_or(0, |day| day.participants[participant_index].claims());
                if claims > 0 {
                    if current_streaks[participant_index] == 0 {
                        current_starts[participant_index] = Some(date);
                    }
                    current_streaks[participant_index] += 1;
                    if current_streaks[participant_index] >= row.longest_streak {
                        row.longest_streak = current_streaks[participant_index];
                        row.streak_start = current_starts[participant_index];
                        row.streak_end = Some(date);
                    }
                } else {
                    current_streaks[participant_index] = 0;
                    current_starts[participant_index] = None;
                }
            }
            if date == end {
                break;
            }
            date += 1.days();
        }
    }

    BoardCard {
        heading_id: format!("pants-board-{}-streak", window.selected_year),
        title: "Longest claim streak",
        entries: ranked(rows),
    }
}

fn ranked(rows: [StreakRow; 3]) -> Vec<BoardEntry> {
    let mut ordered = rows;
    ordered.sort_by_key(|row| (std::cmp::Reverse(row.longest_streak), row.participant_index));
    let mut previous_value = None;
    let mut rank = 0;
    ordered
        .into_iter()
        .enumerate()
        .map(|(position, row)| {
            let tied = row.longest_streak > 0
                && rows
                    .iter()
                    .filter(|r| r.longest_streak == row.longest_streak)
                    .count()
                    > 1;
            BoardEntry {
                rank: competition_rank(
                    position,
                    row.longest_streak,
                    tied,
                    &mut previous_value,
                    &mut rank,
                ),
                display_name: PANTS_PARTICIPANTS[row.participant_index].display_name,
                value: format!(
                    "{} {}",
                    row.longest_streak,
                    plural(row.longest_streak, "day", "days")
                ),
                detail: match (row.streak_start, row.streak_end) {
                    (Some(start), Some(end)) => {
                        format!("{} – {}", format_compact(start), format_compact(end))
                    }
                    _ => "no claim streak".to_string(),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::podrick::heatmap::fixtures::{message, window_from};
    use jiff::civil::Date;

    #[test]
    fn streaks_break_on_missing_and_non_claim_dates_and_doubles_count_once() {
        let messages = [
            message("101", 0, "2026-01-01T11:07:00Z"),
            message("102", 0, "2026-01-02T11:07:00Z"),
            message("103", 0, "2026-01-02T23:07:00Z"),
            message("104", 0, "2026-01-03T17:07:00Z"),
            message("105", 0, "2026-01-04T11:07:00Z"),
            message("106", 0, "2026-01-05T11:07:00Z"),
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
        let card = board(&owned.window());
        assert_eq!(card.entries[0].value, "2 days");
        assert_eq!(card.entries[0].detail, "Jan 4 – Jan 5");
        assert_eq!(
            card.entries[0].display_name,
            PANTS_PARTICIPANTS[0].display_name
        );
    }

    #[test]
    fn streaks_reset_at_the_year_boundary() {
        let messages = [
            message("101", 0, "2025-12-31T11:07:00Z"),
            message("102", 0, "2026-01-01T11:07:00Z"),
            message("103", 0, "2026-01-02T11:07:00Z"),
        ];
        let start = Date::new(2026, 1, 1).unwrap();
        let owned = window_from(
            &messages,
            start,
            Date::new(2026, 12, 31).unwrap(),
            Date::new(2025, 12, 31).unwrap(),
            Date::new(2026, 7, 28).unwrap(),
            2026,
        );
        let card = board(&owned.window());
        assert_eq!(card.entries[0].value, "2 days");
        assert_eq!(card.entries[0].detail, "Jan 1 – Jan 2");
    }
}
